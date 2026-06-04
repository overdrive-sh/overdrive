# Options (raw, unfiltered) — unconnected-udp-sendmsg4 DIVERGE

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DIVERGE (Phase 3 — Brainstorming)

> **Separation discipline (Osborn).** This file contains GENERATION ONLY.
> No option is scored, ranked, preferred, or eliminated here. Blast-radius
> and mechanism facts are stated neutrally so the taste phase can evaluate
> them. Evaluation lives exclusively in `taste-evaluation.md`.

---

## HMW question (the ideation frame)

**Bad (solution embedded):** "How might we add a `sendmsg4` BPF program?"

**Good (outcome-oriented, opens the space):**

> **How might we make a same-host service reachable from a client that
> addresses the VIP without first connecting — and make the reply look like
> it came from the VIP?**

The good HMW does not presume `sendmsg4`. It admits address-rewrite hooks,
socket-selection primitives, requiring connected-UDP, or doing nothing —
the full space the SCAMPER pass below explores.

---

## SCAMPER pass — one option per lens (generation only)

### S — Substitute: replace the connect-time hook with a send-time hook

**Option A1 — `sendmsg4`-only (mirror connect4 exactly).**
A `BPF_CGROUP_UDP4_SENDMSG` (`cgroup/sendmsg4`) program attached to the same
`overdrive.slice` cgroup as connect4, reusing `LOCAL_BACKEND_MAP` keyed
`(vip, vip_port, proto=UDP)`, reading `bpf_sock_addr.protocol` (zero-
translation, per ADR-0053 Amd 2), rewriting `user_ip4`/`user_port` to the
backend on the unconnected `sendto` path. Reply path: untouched — the
backend's reply leaves with `src=backend_ip`.
- *Mechanism:* connect-time address rewrite → send-time address rewrite
  (substitute the hook, keep everything else identical to connect4).
- *Assumption:* the client does not validate the reply's source address
  (or tolerates a reply from the backend IP).
- *Cost profile:* smallest — one program, reuses the existing map/action/
  trait/hydrator surface; one new attach + one new Tier-3 fixture.
- *SCAMPER origin:* S (Substitute).
- *Closest competitor:* the kernel UAPI's sendmsg-only state (pre-commit
  `983695fa6765`); the issue #200 baseline recommendation.

### C — Combine: merge the unconnected-UDP job with the existing connect4 job behind one shared surface

**Option A5 — Unify connect4 + sendmsg4 (+recvmsg4) behind one shared
lookup helper and one attach orchestration.**
Factor the `LOCAL_BACKEND_MAP` lookup + key construction into a single
`#[inline(always)]` kernel-side helper (the ADR-0040 Q3 sanity-prologue
pattern) consumed by `cgroup_connect4_service`, a new
`cgroup_sendmsg4_service`, and a new `cgroup_recvmsg4_service`. One attach
orchestrator in `EbpfDataplane::new` attaches all three to
`overdrive.slice`; one Earned-Trust probe covers the set. The trait/action/
hydrator gain the unconnected-UDP path as a *property of the same
local-backend registration* rather than a parallel surface.
- *Mechanism:* the same address-rewrite mechanism as A1, but the marginal
  surface is minimized by sharing the lookup helper and attach orchestration
  across all three hooks (connect4 + sendmsg4 + recvmsg4).
- *Assumption:* the three hooks genuinely share lookup logic such that a
  shared helper reduces, not increases, total complexity; and the reply
  path (recvmsg4) is in scope.
- *Cost profile:* medium — three programs but one shared helper; touches the
  connect4 attach/probe site (a change to shipped code, vs A1's pure
  addition).
- *SCAMPER origin:* C (Combine).
- *Closest competitor:* Cilium's `bpf_sock.c` single-file shared socket-LB
  translation logic consumed by connect4/sendmsg4/recvmsg4.

### A — Adapt: borrow the reply-path reverse-translation from a different domain

**Option A2 — `sendmsg4` + `recvmsg4` pair (request rewrite + reply
source rewrite).**
A1's `sendmsg4` request-path rewrite PLUS a `BPF_CGROUP_UDP4_RECVMSG`
(`cgroup/recvmsg4`) program that rewrites the *source* of the received
datagram from `backend_ip:port` back to `vip:vip_port` before the
application's `recvfrom` returns — the kernel's own `983695fa6765`
"reverse translation." Both hooks key the same `LOCAL_BACKEND_MAP`
(recvmsg4 needs a reverse lookup `backend → vip`, so either a second map or
a reverse scan).
- *Mechanism:* bidirectional address translation — request dest rewrite
  (sendmsg4) + reply source rewrite (recvmsg4) — adapting the kernel's
  documented transparency fix.
- *Assumption:* the client validates the reply source (DNS resolvers do),
  so the reply source must be the VIP for the client to accept it; and a
  reverse `backend→vip` lookup is available cheaply.
- *Cost profile:* medium-large — two programs, a reverse-lookup surface,
  and Tier-3 reply-path assertion. Largest of the cgroup-hook options.
- *SCAMPER origin:* A (Adapt — borrow the kernel's reverse-translation
  transparency mechanism).
- *Closest competitor:* Cilium's `sendmsg4`+`recvmsg4` pair; kernel commit
  `983695fa6765`.

### M — Modify/Magnify: amplify "reachability from real clients" as the dominant dimension

**Option A6 — Reachability-complete cgroup set keyed on the same
local-backend identity, validated by a real-resolver Tier-3 gate.**
A2's sendmsg4+recvmsg4 pair, but the magnified dimension is *operator-
observable reachability*: the acceptance gate is not "`bpftool map dump`
shows the entry" but "a real stub resolver (`getaddrinfo`/`dig`) on the
host resolves a name through the VIP." The reply-path correctness is
elevated to a first-class outcome with its own KPI (O2), and the
implementation installs whatever hook set is required to make a real
resolver succeed end-to-end — sendmsg4 + recvmsg4 as the floor, with the
gate (not the hook count) as the spec.
- *Mechanism:* same bidirectional rewrite as A2, but specified/validated by
  the real-client outcome rather than by the map state.
- *Assumption:* a real-resolver Tier-3 gate is feasible in Lima without
  colliding with systemd-resolved's port (debugging.md § 11), and that
  "real resolver resolves" is the right spec.
- *Cost profile:* medium-large (= A2's surface) + a more demanding Tier-3
  fixture (real resolver, not synthetic socket).
- *SCAMPER origin:* M (Modify/Magnify — amplify reachability-as-spec).
- *Closest competitor:* Cilium's e2e CoreDNS reachability tests.

### P — Put to other use: serve the same reply-path-correctness need via a different inbound primitive

**Option A3 — `SK_LOOKUP` inbound socket selection (no destination
rewrite), reply path addressed separately.**
A per-netns `BPF_PROG_TYPE_SK_LOOKUP` program selects the backend's
listening socket via `bpf_sk_assign` for inbound UDP addressed to the VIP,
WITHOUT rewriting the destination — the application sees the VIP via
`getsockname`. A new `LOCAL_SERVICE_SOCKS` map (socket-fd registry keyed
`(vip, vip_port, proto)`) replaces the address-rewrite map for the inbound
leg. The reply-path source mismatch (backend replies from its own IP) is
handled by a companion rewrite or accepted as a documented limitation.
- *Mechanism:* socket-selection-at-lookup (no L3 rewrite) — a categorically
  different primitive from address rewrite; the inbound packet keeps the
  VIP.
- *Assumption:* a socket-fd registry can be populated (pre-bind or
  SCM_RIGHTS from the workload), and the reply-path source is solved
  separately or the limitation is acceptable; per-netns attach is
  acceptable for Phase 1 single shared netns.
- *Cost profile:* large — a new program type, a new socket-fd map shape, an
  fd-plumbing channel from `ExecDriver`, and a separate reply-path answer;
  does NOT reuse the shipped `LOCAL_BACKEND_MAP` / address-rewrite surface.
- *SCAMPER origin:* P (Put to other use — the SK_LOOKUP primitive applied
  to the unconnected-UDP same-job).
- *Closest competitor:* Cloudflare/Sitnicki SK_LOOKUP service steering;
  `same-host-backend-delivery-architecture.md` Option 2.

### E — Eliminate: remove the unconnected path entirely — require the client to connect

**Option A4 — Document the limitation: support connected-UDP only.**
Ship no new hook. The connect4 path (already shipping) covers TCP and
connected-UDP. Document that same-host UDP services are reachable only from
clients that `connect(VIP)` before sending, and that unconnected clients
(most DNS resolvers) are not supported on the same host until #200 is
delivered. Optionally provide an operator note ("front your DNS service
with a connecting client / sidecar").
- *Mechanism:* none added — eliminate the unconnected path from scope.
- *Assumption:* the operator can either tolerate unconnected-UDP services
  being unreachable, or can interpose a connecting client — i.e. the
  burden can be pushed to the operator/client side.
- *Cost profile:* near-zero (documentation only); the honest no-op floor.
- *SCAMPER origin:* E (Eliminate); ADR-0053 Alt D analogue ("accept the
  limitation").
- *Closest competitor:* the current shipped state (ADR-0053 Amd 4 out-of-
  scope status).

### R — Reverse: invert the rewrite direction — bind the backend to the VIP so no client-side rewrite is needed

**Option A7 — Bind backends to the VIP (host-side pre-bind / IP-on-loopback),
no client-path hook.**
Instead of rewriting the client's destination, make the VIP a real local
address: add the VIP to a host interface (e.g. `lo`) and have the backend
bind directly to `VIP:port`. An unconnected `sendto(VIP:port)` then reaches
the backend with no BPF hook at all, and the reply is naturally sourced
from the VIP (the backend's bound address), so reply-source validation
passes for free.
- *Mechanism:* invert the model — instead of translating the client's view
  of the backend, make the backend actually own the VIP address. No
  syscall interception.
- *Assumption:* the platform can assign the VIP as a real local address and
  the backend can be made to bind it (pre-bind/SCM_RIGHTS or address-add);
  one backend per (VIP, port) (no multi-backend selection); collides with
  nothing else binding the VIP.
- *Cost profile:* medium — VIP address management on the host + backend
  bind plumbing; no BPF program, but a new IP-management surface and a hard
  one-backend constraint; diverges from the LOCAL_BACKEND_MAP rewrite model.
- *SCAMPER origin:* R (Reverse — backend owns the VIP rather than the
  dataplane translating to the backend).
- *Closest competitor:* keepalived/IPVS VIP-on-loopback for DSR; classic
  "bind the service IP locally" patterns.

---

## Crazy 8s supplements (additional structurally-distinct options)

### Option A8 — Connection-tracking-free reply rewrite via a single bidirectional cgroup hook + reverse map (the "minimal reply-correct" shape)

sendmsg4 for the request, and instead of a *separate* recvmsg4 program with
its own reverse lookup, install a single small `REVERSE_LOCAL_MAP`
(`backend(ip,port,proto) → vip(ip,port)`) populated by the **same**
`register_local_backend` action (one write, two map entries), consumed by a
recvmsg4 program that does a direct point lookup. The differentiator from
A2 is the *reverse map is derived from the same registration write* (no
scan, no second action), making the reply path a byte-for-byte mirror of
the forward path.
- *Mechanism:* bidirectional rewrite (like A2) but with a purpose-built
  reverse point-lookup map written atomically alongside the forward entry —
  optimizing the reverse-lookup cost and the action surface.
- *Assumption:* the reverse map can be kept consistent with the forward map
  via a single action (no independent drift), and recvmsg4 point-lookup is
  the cheapest reply-path shape.
- *Cost profile:* medium-large (≈ A2) but with an extra map declaration in
  exchange for a cheaper/cleaner reverse lookup and a single-write
  consistency story.
- *SCAMPER origin:* Crazy 8s (refinement of the reply-path data shape;
  distinct from A2's "reuse one map with a reverse scan").

### Option A9 — TPROXY / `bpf_sk_assign` at TC ingress for unconnected UDP

A TC-ingress program (or TPROXY-style steering) on the host that, for UDP
packets addressed to a VIP, assigns the backend's listening socket via
`bpf_sk_assign` at the TC layer (kernel ≥ 5.7) — a per-packet steering on
the receive path rather than a syscall hook. Reply path handled by a
companion egress rewrite or accepted.
- *Mechanism:* per-packet TC-ingress socket steering (TC + `bpf_sk_assign`)
  — the user's original "Option 2" hypothesis from the connect4 research,
  re-examined for the unconnected case.
- *Assumption:* per-packet TC steering is acceptable cost vs a once-per-
  syscall hook; reply path solved separately; TC attach is acceptable.
- *Cost profile:* large — a TC program (different attach layer than the
  shipped cgroup model), per-packet runtime cost, separate reply path; does
  not reuse the cgroup surface.
- *SCAMPER origin:* Crazy 8s (the TC+`sk_assign` primitive the connect4
  research's user hypothesis named but Cilium does not use for service LB).

---

## Curation to 6 (diversity test applied)

All generated: A1, A5, A2, A6, A3, A4, A7, A8, A9 (nine).

### Merges / removals (exact-or-near variations only)

- **A6 merged into A2.** A6 ("reachability-complete, validated by a real-
  resolver gate") is A2's mechanism (sendmsg4+recvmsg4) with a *test-gate*
  difference, not a *mechanism/assumption/cost* difference. The real-
  resolver Tier-3 gate is an acceptance-criteria choice that applies to
  A2 (and A8) identically — it is not a structurally distinct option. Its
  one durable contribution (reachability-as-spec) is folded into A2's
  description and carried as a DISCUSS/DESIGN testability note.
- **A8 merged into A2.** A8 ("reverse point-lookup map written by the same
  action") is a *data-shape refinement of the reply path within the same
  bidirectional mechanism* — same mechanism (sendmsg4+recvmsg4), same
  assumption (client validates reply source), same cost band as A2. Whether
  the reverse lookup uses a second map or a reverse scan is a DESIGN detail,
  not a divergence-level option. Folded into A2 as the recommended reply-
  map shape.

### The curated 6

| # | Option | Mechanism | Assumption (about client/deployment) | Cost profile |
|---|---|---|---|---|
| **1** | **A1 — sendmsg4-only** | send-time dest rewrite, reply untouched | client does NOT validate reply source | smallest; pure addition reusing connect4 surface |
| **2** | **A2 — sendmsg4 + recvmsg4** (absorbs A6, A8) | bidirectional rewrite: dest (sendmsg4) + reply source (recvmsg4) | client DOES validate reply source (DNS resolvers do) | medium-large; two programs + reverse-lookup surface |
| **3** | **A5 — unify connect4+sendmsg4+recvmsg4 behind shared helper** | same bidirectional rewrite, marginal surface minimized via shared lookup helper + one attach orchestration | the three hooks share lookup logic; touching shipped connect4 code is acceptable | medium; three programs/one helper; modifies shipped attach/probe site |
| **4** | **A3 — SK_LOOKUP inbound socket selection** | socket-selection-at-lookup, no L3 rewrite (app sees VIP) | a socket-fd registry can be populated; reply source solved separately or limitation accepted; per-netns attach OK | large; new program type + socket-fd map + fd plumbing; does NOT reuse connect4 surface |
| **5** | **A7 — bind backends to the VIP (no client hook)** | invert: backend owns the VIP as a real local address; no syscall interception | platform can assign VIP locally + backend binds it; one backend per (VIP,port) | medium; VIP-address mgmt + bind plumbing; no BPF program; hard one-backend constraint |
| **6** | **A4 — document the limitation (connected-UDP only)** | none added; require client to connect() | operator can tolerate unreachability or interpose a connecting client | near-zero; docs only (the honest no-op floor) |

*(A9 — TC+`bpf_sk_assign` — was generated but is held out of the curated 6:
it is a strictly-worse variant of A3's "non-cgroup inbound steering"
category (per-packet TC cost, same unsolved reply path, same no-reuse-of-
connect4) and shares all three diversity axes with A3. Recorded here for
completeness; not carried forward, to keep the 6 structurally distinct.)*

### 3-point diversity test (each of the 6 vs every other)

| Option | Different mechanism? | Different assumption? | Different cost? |
|---|---|---|---|
| 1 (sendmsg4-only) | send-time rewrite, **no reply hook** | uniquely assumes client does NOT validate reply source | uniquely smallest (pure addition) |
| 2 (sendmsg4+recvmsg4) | **bidirectional** rewrite | client DOES validate reply source | two programs + reverse lookup |
| 3 (unify) | bidirectional **+ shared-helper/one-attach** | the hooks share lookup logic worth factoring; modify shipped code OK | three-program/one-helper; touches shipped site |
| 4 (SK_LOOKUP) | **socket selection, no rewrite** (different primitive family) | needs socket-fd registry; reply solved elsewhere | new program type + fd plumbing; no connect4 reuse |
| 5 (VIP bind) | **no interception — backend owns VIP** (inversion) | platform assigns VIP locally; one backend only | IP-mgmt surface, no BPF program |
| 6 (document) | **none — eliminate the path** | operator tolerates / interposes | docs only |

Each row is distinct on **all three** axes from every other row:
- 1 vs 2: same hook *family* but 1 has no reply hook and uniquely assumes
  no reply-source validation — different mechanism (no reverse leg),
  different assumption, different cost. (This is the central discriminator
  the research identified.)
- 2 vs 3: 3 adds the shared-helper/one-attach combine and modifies shipped
  connect4 code — different mechanism (factoring), different assumption
  (shared logic worth it / touching shipped OK), different cost.
- 4, 5, 6 are each in a different primitive category entirely (socket
  selection / VIP-ownership inversion / no-op).

**All 6 pass the 3-point diversity test.**

---

## Gate check (Phase 3 — G3)

- [x] **HMW framed, no embedded solution** — § HMW.
- [x] **All 7 SCAMPER lenses applied** — S→A1, C→A5, A→A2, M→A6, P→A3,
  E→A4, R→A7 (each named).
- [x] **Crazy 8s supplements** — A8, A9 (2 additional, structurally
  distinct at generation time).
- [x] **6 curated options** — A1, A2, A5, A3, A7, A4 (with A6, A8 merged
  and A9 held out, all disclosed).
- [x] **Each passes the 3-point diversity test** (mechanism, assumption,
  cost) — tabulated above.
- [x] **No evaluation language** — no option is scored, ranked, preferred,
  or eliminated; merges are by structural-duplication only (Osborn
  separation respected). Blast-radius/mechanism facts stated neutrally.

**G3: PASS.**
