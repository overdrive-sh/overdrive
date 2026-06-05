# Recommendation — unconnected-udp-sendmsg4 (DIVERGE)

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DIVERGE → handoff to DISCUSS (product-owner)
· **Decision under divergence:** how to close the unconnected-UDP
(`sendto(VIP)` without `connect()`) same-host delivery gap that ADR-0053
Amendment 4 left open and tracked as #200.

> **Scope guard.** ADR-0053's connect4 decision is **locked and shipping** —
> not reopened here. This DIVERGE diverges only on the *new* unconnected-UDP
> hook, which plugs into the existing `LOCAL_BACKEND_MAP` `(vip, vip_port,
> proto)` surface.

---

## Decision statement (for the DISCUSS wave)

> **Proceed with Option 2 — `sendmsg4` + `recvmsg4` (bidirectional
> address rewrite).** Add a `BPF_CGROUP_UDP4_SENDMSG` (`cgroup/sendmsg4`)
> program that rewrites the unconnected datagram's destination
> `VIP → backend` against the existing `LOCAL_BACKEND_MAP` (keyed
> `(vip, vip_port, proto=UDP)`, proto read zero-translation from
> `bpf_sock_addr.protocol` per ADR-0053 Amd 2), **AND** a
> `BPF_CGROUP_UDP4_RECVMSG` (`cgroup/recvmsg4`) program that rewrites the
> reply's source `backend → VIP` so a source-validating client (every DNS
> resolver) accepts the reply. **Assuming** the one key risk is acceptable:
> that recvmsg4's reverse `backend → vip` lookup is served by a reverse map
> written atomically alongside the forward `register_local_backend` write
> (DESIGN detail — second map vs reverse scan), NOT by a per-connection
> conntrack table (UDP is connectionless; there is no per-connection state
> to reuse).

The matrix selects Option 2 at **4.65**, ahead of Option 3 (unify,
**4.07**) and Option 1 (sendmsg4-only, **3.48**). The choice is **not**
inside scoring noise — the gap from #1 to #2 is 0.58, and the gap from #2
(the winner) to #3 (sendmsg4-only) is 1.17, driven by the reply-path
discriminator (O2). recvmsg4 is **required, not optional**: kernel commit
[`983695fa6765`](https://github.com/torvalds/linux/commit/983695fa6765)
demonstrates the exact `nslookup`-rejects-the-backend-sourced-reply failure
that sendmsg4-alone produces, and Cilium ships recvmsg4 for precisely this.

---

## Top 3 options

### Option 2 — `sendmsg4` + `recvmsg4` — Score 4.65 (RECOMMENDED)

Bidirectional cgroup-hook rewrite: `sendmsg4` rewrites the request
destination `VIP→backend`; `recvmsg4` rewrites the reply source
`backend→VIP`. Reuses `LOCAL_BACKEND_MAP` for the forward leg; adds a
reverse `backend→vip` lookup for the reply leg.

- **Why it scores well:** DVF 5.00 (exact parity with the kernel's own
  "connected by connect, unconnected by sendmsg" design [LWN 755902] +
  Cilium's shipped recvmsg4 reply fix; all kernel floors below 5.10), T4=5
  and T3=5 (the delivered outcome is *real*: `dig @vip` returns correctly,
  no latent asymmetry; the reply passes source validation). T2=4 — the
  recvmsg4 reverse leg is the mirror of the forward leg the engineer
  already reads on connect4, so ~one new concept, well-anchored.
- **Core trade-off:** two programs + a reverse-lookup surface, vs the
  one-program sendmsg4-only (which scores T1=5 to this option's T1=4). The
  second program is the *price of the reply path working* — it is not
  removable subtraction; removing it removes core value (the resolver
  rejects the reply).
- **Key risk (must be true):** the reverse `backend→vip` lookup is cheap
  and consistent — served by a `REVERSE_LOCAL_MAP` written atomically
  alongside the forward `LOCAL_BACKEND_MAP` entry by the same
  `register_local_backend` action (one logical write, two map entries),
  NOT by a conntrack table. If a future requirement forces per-flow state
  (it should not for stateless same-host UDP), the cost story changes.
- **Hire criteria:** choose this when the operator must run a *real* UDP
  service reachable from real (source-validating) clients — DNS being the
  canonical, day-one driver — and you want exact parity with the kernel's
  design intent and the dominant production reference (Cilium), reusing the
  shipped connect4 surface as a pure addition.

### Option 3 — Unify connect4 + sendmsg4 + recvmsg4 behind a shared helper — Score 4.07 (RUNNER-UP)

Option 2's bidirectional rewrite, but the `LOCAL_BACKEND_MAP` lookup + key
construction is factored into one shared `#[inline(always)]` kernel helper
consumed by all three hooks, with one attach orchestration and one
Earned-Trust probe covering the set.

- **Why it scores well:** identical delivered outcome to Option 2 (DVF 4.67,
  T4=5) — the operator-facing result is the same. The marginal-surface win
  (O4) is real: one lookup helper instead of three copies.
- **Core trade-off:** it **modifies shipped connect4 code** (the attach
  orchestration, the probe, and a refactor of the connect4 lookup into the
  shared helper). That is a change to a working, shipped path — Feasibility
  4 not 5, T2=3 (two interdependent new concepts: the shared-helper refactor
  AND the three-hook orchestration), against the single-cut-greenfield
  posture's preference for additive change.
- **Key risk:** the refactor of the shipped connect4 path introduces no
  regression — a real risk that Option 2 (pure addition) does not carry.
- **Hire criteria:** choose this if DESIGN judges the three hooks' shared
  lookup logic substantial enough that one helper *reduces* total
  complexity, AND the team accepts touching the shipped connect4 attach/probe
  in the same PR. This is a strong DESIGN-time refinement *of Option 2*, not
  a different destination — Option 2 and Option 3 are the same architectural
  family (bidirectional cgroup rewrite); the split is "additive (2) vs
  refactor-to-share (3)."

### Option 1 — `sendmsg4`-only — Score 3.48 (THE DISSENT — see below)

Send-time destination rewrite only; reply path untouched.

- **Why it scores lower:** DVF 3.33 and T4=2 — it delivers the query but the
  canonical client **rejects the backend-sourced reply** (kernel
  `983695fa6765`; research Competitor 3/4), producing the "service times
  out / appears flaky" failure T4 exists to penalize. It scores T1=5
  (maximal subtraction) — but the subtraction *removes core value* (the
  reply path), which is exactly the kind of subtraction the T1 rubric warns
  against ("feature accumulation" is the bottom, but value-destroying
  minimalism is not a 5-worthy 'nothing removable without breaking core
  value' — it breaks it).
- **Why it might still be right:** see § Dissenting case.
- **Hire criteria:** choose this ONLY if the target client provably does
  NOT validate the reply source (a bespoke, controlled UDP client — NOT a
  DNS resolver), or if recvmsg4 is deferred to a fast follow-up and the
  interim is honestly documented as "connecting-or-non-validating clients
  only."

---

## REQUIRED analysis per surviving option

### Blast radius (concrete; single-cut, no shims, per ADR-0053 migration posture)

| Option | Kernel-side programs | Maps | Action / trait | Hydrator | Attach + probe | New surface? |
|---|---|---|---|---|---|---|
| **2 (rec)** | + `cgroup_sendmsg4_service`, + `cgroup_recvmsg4_service` | reuse `LOCAL_BACKEND_MAP` (forward); + `REVERSE_LOCAL_MAP` (reply) | `register_local_backend` already carries `(vip, vip_port, proto, backend)` (ADR-0053 Amd 3) — the reverse entry is derived from the SAME action; no new action variant required for the forward path; reply map written in the same shim | classifier already emits `RegisterLocalBackend` for local backends — UNCHANGED (it already covers UDP local backends; the new hooks just *consume* the existing map for the unconnected path) | + 2 attach calls on `overdrive.slice`; + probe steps for the 2 hooks | `REVERSE_LOCAL_MAP` + 2 programs (pure addition; connect4 untouched) |
| **3 (unify)** | + 2 programs + refactor connect4 to shared helper | same as 2 | same as 2 | UNCHANGED | refactor the single attach orchestration to cover 3 hooks; one probe set | `REVERSE_LOCAL_MAP` + 2 programs + shared helper (modifies shipped connect4 attach/probe) |
| **1 (sendmsg4-only)** | + `cgroup_sendmsg4_service` | reuse `LOCAL_BACKEND_MAP` only | none new | UNCHANGED | + 1 attach + 1 probe step | 1 program (pure addition; no reply path) |

**Key blast-radius fact:** Options 2 and 1 are **pure additions** — the
shipped connect4 program, `LOCAL_BACKEND_MAP` forward shape, action variants
(already proto-carrying per ADR-0053 Amd 3), and the hydrator classifier are
**unchanged**. The hydrator already emits `RegisterLocalBackend` for UDP
local backends (the connected-UDP path uses it today); the new sendmsg4/
recvmsg4 hooks *consume the same forward map* for the unconnected datagram
path. The only genuinely-new surface in Option 2 is `REVERSE_LOCAL_MAP` +
the two programs + their attach/probe steps. Option 3's extra cost is the
connect4 refactor (a change to shipped code).

### Reply-path correctness (the crux — O2) per option

| Option | Reply source the client sees | Source-validating client (DNS) accepts? |
|---|---|---|
| **2 (rec)** | **VIP** (recvmsg4 reverse-rewrites `backend→VIP`) | **Yes** — the documented kernel fix (`983695fa6765`) |
| **3 (unify)** | **VIP** (same recvmsg4) | **Yes** |
| **1 (sendmsg4-only)** | backend IP (no reply rewrite) | **No** — discarded on source validation |
| 4 (SK_LOOKUP) | backend IP (no reply rewrite; SK_LOOKUP steers inbound only) | No (reply unsolved) |
| 5 (VIP bind) | VIP (backend bound to VIP) | Yes — but one backend only |
| 6 (document) | n/a (unreachable) | n/a |

This table is the decision in one view: **only Options 2, 3, and 5 produce
a VIP-sourced reply; of those, 5 is one-backend-only and architecturally
fragile (DVF 2.33), and 3 is 2-plus-a-refactor. Option 2 is the clean
VIP-sourced-reply winner.**

### Kernel viability (hard gate — all options below the 5.10 floor)

| Hook | Since | Field availability (verified 2026-06-05) |
|---|---|---|
| `cgroup/sendmsg4` (`BPF_CGROUP_UDP4_SENDMSG`) | **4.18** | `user_ip4` writable (4-byte, *only valid for this attach type* among UDP hooks), `user_port` writable NBO, `protocol` populated (IPPROTO_UDP=17, zero-translation) |
| `cgroup/recvmsg4` (`BPF_CGROUP_UDP4_RECVMSG`) | **4.20** | fires on non-NULL `msg_name` (unconnected `recvfrom`); rewrites the source `sockaddr` the app sees |
| (SK_LOOKUP, Option 4) | 5.9 | per-netns; below floor but does not solve reply path |

No kernel-floor bump for any option. The `user_port` low-16-NBO-in-u32
hazard (`.claude/rules/development.md`) applies identically to sendmsg4 as
to connect4 — a known, documented care-point, not a viability blocker.

### Testability (O5 — no Tier-2 backstop for `cgroup_sock_addr`)

`BPF_PROG_TEST_RUN` returns ENOTSUPP for `cgroup_sock_addr` on kernel ≤ 6.8
(`.claude/rules/development.md` § sock_addr-no-Tier-2). Both the forward
(sendmsg4) and reply (recvmsg4) correctness are **Tier-3-only** gates — a
real unconnected `sendto`/`recvfrom` through `overdrive.slice` with
`tcpdump`/`bpftool` evidence (and ideally a real stub resolver, the folded-in
A6 reachability-as-spec note). This cost is **identical for Options 1, 2,
3** (same program type), so O5 does not separate them — it is a shared
DELIVER discipline, not a discriminator. (Option 4's SK_LOOKUP *can* use
`BPF_PROG_TEST_RUN` — a testability point in its favour — but its other
weaknesses sink it.) **Tier-3 fixture must avoid the systemd-resolved UDP
5353 collision** (`.claude/rules/debugging.md` § 11).

### VIP-semantics on `recvfrom` source — per option (the dispatch's load-bearing differentiator (a)/(b))

- **Option 2/3:** `recvfrom` returns **VIP** as the source (recvmsg4
  rewrites it). This is *more* VIP-preserving than the connect4 path's
  ClusterIP semantic (where `getpeername` returns the backend) — a
  deliberate, correct asymmetry: connected sockets have a stored peer the
  app can introspect (ClusterIP-style), unconnected datagram replies must
  *look like* they came from the VIP or the client discards them.
- **Option 1:** `recvfrom` returns the **backend** source → client discards.
- **Option 5:** `recvfrom` returns VIP (backend bound to VIP) — for free,
  but one backend only.

---

## Dissenting case — for Option 1 (`sendmsg4`-only)

**The honest case the cheapest option could still be right:**

1. **If the target client does not validate the reply source.** The entire
   case against Option 1 rests on the canonical client (DNS resolver)
   rejecting a backend-sourced reply. A *bespoke, operator-controlled* UDP
   client (a custom telemetry agent, a game protocol the operator wrote, a
   syslog forwarder configured to accept any source) may not validate the
   source at all. For that client, sendmsg4-only delivers the full job at
   T1=5 minimal cost, and recvmsg4 is genuinely unnecessary surface. The
   matrix penalizes Option 1 on T4/DVF *because it weights the DNS-resolver
   client as the driver* (per the job analysis O1/O2). If DISCUSS establishes
   that Phase-1 UDP services are NOT consumed by source-validating clients,
   the T4 penalty evaporates and Option 1 rises to ~4.4 (T4 5, DVF ~4.7) —
   ahead of Option 3. **To make Option 1 win you would re-establish the
   client model (no source validation) — an explicit, documentable
   assumption change, not a silent override.**

2. **Incremental landing.** Option 1 could ship first (close the request-
   path half of #200, unblock connecting-or-non-validating clients) with
   recvmsg4 as an immediate fast-follow — *if* the interim is honestly
   documented ("unconnected-UDP delivery lands request-path-first; reply-
   source rewrite for validating clients follows"). This trades one
   half-working interim for a smaller first PR. The risk: a "half-working"
   interim is exactly the operator-trust failure J-OPS-004 names, so the
   interim documentation must be unambiguous, and the fast-follow must
   actually follow.

**Verdict on the dissent:** legitimate but conditional. Option 1 wins ONLY
if (a) the Phase-1 UDP client model is "does not validate reply source"
(contradicted by the DNS-resolver driver the whole feature exists for), OR
(b) the team deliberately ships request-path-first with a tracked, honest
interim. Neither is established by the validated job (J-OPS-004's "reachable
from real clients I do not control" is precisely the source-validating-DNS
case). Absent that, the matrix's choice (Option 2) stands.

---

## Sub-deferral surfaced (NEEDS USER/ORCHESTRATOR DECISION — not actioned here)

**recvmsg4 as a separate follow-up vs in-scope with sendmsg4.** The
recommendation lands recvmsg4 **in scope with sendmsg4** (Option 2),
because the evidence makes recvmsg4 load-bearing for the canonical client.
However, the dispatch flagged recvmsg4 as a candidate sub-deferral. **If**
the orchestrator/user prefers to land sendmsg4 first and recvmsg4 as a
tracked follow-up (the Option-1-as-interim path in the dissent), that is a
*new deferral* requiring a GitHub issue. **Per CLAUDE.md, this DIVERGE does
NOT create that issue** — it surfaces the choice for the user to approve.
The Flux recommendation is to keep recvmsg4 in scope (Option 2); a
sendmsg4-first split should only proceed with explicit user approval and a
real tracking issue created by the user/orchestrator, not by an agent.
#200 itself already exists and covers the unconnected-UDP work; no new issue
is needed for the recommended Option 2.

---

## Handoff to DISCUSS (product-owner)

- **Recommended:** Option 2 (`sendmsg4` + `recvmsg4`), score 4.65.
- **Runner-up / same family:** Option 3 (unify behind shared helper), 4.07
  — adopt as a DESIGN-time refinement of Option 2 IF the connect4-refactor
  risk is judged worth the marginal-surface win.
- **Dissent:** Option 1 (sendmsg4-only), 3.48 — adopt ONLY if the Phase-1
  UDP client model is re-established as non-source-validating, OR as a
  documented request-path-first interim with a tracked recvmsg4 follow-up
  (user-approved deferral + issue).
- **Decision is traceable:** job (O1/O2 reply-path) → research (kernel
  `983695fa6765` makes recvmsg4 load-bearing; Cilium ships it) → matrix
  (Option 2 = 4.65, top, driven by DVF + T4 on the O2 discriminator) →
  recommendation. No "feels right" override.
- **ADR amendment** (extending ADR-0053 with the sendmsg4+recvmsg4 path and
  `REVERSE_LOCAL_MAP`) is the **architect's job in DESIGN** — this DIVERGE
  forward-points only; it does NOT edit ADRs.

---

## Peer review

See `diverge/review.yaml` for the nw-diverger-reviewer (Prism) verdict.
