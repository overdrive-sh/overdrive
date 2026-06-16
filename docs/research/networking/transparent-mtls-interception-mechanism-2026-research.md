# Research: Is enrollment-based interception the right transparent-mTLS model for Overdrive — or has 2024–2026 produced something better?

**Date**: 2026-06-16 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (model verdict) / Medium-High (specific recovery mechanism) | **Sources**: 20 (6 internal PRIMARY + 11 high-trust external + 3 medium-trust cross-referenced; avg external reputation ~0.90)

> **Framing (adversarial).** The team chose the **enrollment / capture-and-resolve**
> interception model (ztunnel-shaped — capture all egress from a mesh-enrolled
> workload; resolve destination → backend + expected SVID per-connection in the
> agent; fail-toward-handshake) for transparent mTLS (#236). A prior research doc
> landed on "eager per-peer redirect map → then enrollment." **This doc does not
> confirm that choice. It adversarially tests it** against 2024–2026 kernel/eBPF and
> service-mesh developments, weighing every candidate against the lever Overdrive
> uniquely holds: **it pins its own modern kernel (6.18 LTS prod, 7.0 dev,
> bpf-next canary — ADR-0068)** and so may adopt bleeding-edge interception
> primitives that portable meshes (Istio/Cilium) avoid.
>
> **The load-bearing crux: outbound original-destination recovery.** Capture-all
> means the agent must learn where the workload was actually dialing. Overdrive's
> current outbound path is *documented-lossy* (`cgroup_connect4_mtls` rewrites
> `connect(peer)→connect(leg_F)` in place; orig-dst NOT recoverable from the
> accepted proxy-leg socket). Inbound already recovers orig-dst via nft-TPROXY +
> `getsockname`. The working hypothesis is "use TPROXY+getsockname outbound too."
> **This doc tests whether that is the best modern mechanism or whether a newer
> primitive (sk_lookup, socket-cookie/sk_storage stash, netkit) is cleaner.**

## Executive Summary

**Verdict: enrollment-based capture-and-resolve is CONFIRMED as the right
interception model for Overdrive — but the working *recovery* hypothesis
(TPROXY+getsockname outbound) should be revised in favour of a kernel-side
cgroup-cookie-stash that Overdrive's pinned kernel uniquely makes clean.** The
adversarial test set out to find a better model and instead found the industry
*converging on* the enrollment model from two independent directions: Istio
ambient/ztunnel made it the default and had it security-audited (2025), and Cilium
**adopted ztunnel** in 2026 for native per-connection identity-bound mTLS — moving
*away* from the out-of-band auth + node-WireGuard model that ADR-0069 already
rejected (A4). Per-connection resolve, the headline objection, is empirically
negligible (ztunnel: ~0.06 vCPU / 1000 rps, higher throughput than IPsec/WireGuard)
and would be *cheaper* for Overdrive (a local in-memory `service_backends` lookup,
no xDS hop). Overdrive's fail-toward-handshake miss semantic is even *richer* than
Cilium's hard enrolled/non-enrolled cutoff. No better *model* emerged.

**On the load-bearing crux — outbound original-destination recovery — the research
revises the working hypothesis.** Two of the four floated candidates are rejected:
`sk_lookup` is an *inbound* socket-selection primitive (kernel doc verbatim:
"…for an incoming packet"); it has no `connect()`-time role and recovers no
outbound orig-dst. `netkit` is a veth-replacement *datapath* device (kernel 6.7),
not an interceptor — an additive, orthogonal performance win, not a recovery
answer. The two real contenders are TPROXY+`getsockname` (the consensus *capture*
primitive, already proven in Overdrive's inbound path, but requiring new
egress-in-netns routing infra outbound) and **`cgroup/connect4` cookie-stash +
`bpf_sk_storage` + `cgroup/getsockopt(SO_ORIGINAL_DST)`-revert** — which is the
**recommended mechanism**: it is the *smallest delta to Overdrive's existing
outbound hook* (stop throwing orig-dst away; stash it in socket-attached storage;
answer it on `getsockopt`), needs no routing infra, keeps all primitives under the
6.18 pin (`cgroup/getsockopt` ≥5.3, `bpf_sk_storage` ≥5.2), and matches the actual
consensus primitive — ztunnel's *real* outbound recovery is `SO_ORIGINAL_DST`, not
plain TPROXY+getsockname.

**The pinned-kernel lever is real and points one specific direction.** Pinning
6.18 does not unlock a *different model* — it unlocks putting the orig-dst
*recovery* (and optionally the backend *resolve*) **in the kernel**, keeping the
agent on the handshake path only (the M3 hybrid lean). It does NOT unlock a
pure-kernel, zero-userspace-proxy mTLS: the only 2026 path to that (Riptides /
Cisco Camblet) requires an **out-of-tree kernel module** because eBPF's verifier
constraints make it "fundamentally unsuited for managing TLS handshakes" — and
ADR-0069 independently proved the eBPF sockmap forward path non-viable. So
Overdrive's agent-light-userspace-handshake + kernel-kTLS-steady-state is the
correct point on the spectrum; chasing pure-eBPF mTLS is a dead end. The spike
(`/nw-spike`) should probe the cookie-stash (Probe A) and TPROXY-outbound (Probe B)
head-to-head on the pinned kernel — a *mandatory* Tier-3 probe, since the
`cgroup_sock_addr`/`cgroup_getsockopt` hooks have no `BPF_PROG_TEST_RUN` backstop.

## Research Methodology

**Search Strategy**: Two streams. (1) Code-verified PRIMARY grounding — read the
outbound lossy hook (`cgroup_connect4_mtls.rs`), the worker accept loop's
lossy-recovery comments and `#178` declared-peer stand-in
(`mtls_intercept_worker.rs`), the inbound TPROXY+getsockname recovery
(`mtls/inbound.rs`), ADR-0069 (agent-light kTLS, sockmap-non-viable), the prior
research doc's enrollment decision (§3.5 / #236), and the pinned-kernel ADR-0068.
(2) External 2024–2026 evidence — targeted searches + primary-doc fetches,
prioritising the crux (orig-dst recovery: sk_lookup, cgroup-sockopt/cookie-stash,
TPROXY, netkit) and the closest analogs' recent evolution (Istio ambient/ztunnel,
Cilium netkit + 2026 ztunnel adoption, kTLS+eBPF). Kernel-feature claims anchored
to kernel.org / LWN / merge-commit primaries before any vendor blog.

**Source Selection**: Types: internal PRIMARY (Overdrive code/ADRs — authoritative
for current state); kernel/standards primaries (docs.kernel.org, lwn.net, torvalds
merge commits); official project docs (istio.io, github.com/istio, docs.cilium.io,
cilium.io, ebpf.io); 3 medium-trust transparent-proxy writeups used ONLY where the
underlying primitive was independently confirmed kernel-primary. Excluded-tier
domains avoided.

**Quality Standards**: ≥3 sources/claim (2 acceptable; 1 only if authoritative
primary — kernel doc / LWN / RFC / merge commit / project source). Prefer
kernel.org / LWN / merge commits over vendor blogs for kernel-feature claims;
single-vendor-blog claims flagged (the Riptides kernel-module claim and the
assembled cookie-stash pattern are the only medium-trust load-bearing items, both
flagged and each triangulated against a primary). Adversarial framing throughout:
the default was to *refute* the enrollment choice; it survived because the evidence
converged on it, and the one place the evidence broke the working hypothesis (the
recovery mechanism) is stated plainly.

---

## PART 0 — Overdrive current-state grounding (PRIMARY; code-verified)

> **Source class.** Everything in PART 0 is a PRIMARY internal fact, verified by
> reading the cited file / ADR this run. Internal facts do not need the 3-external
> rule — the code IS the authority for Overdrive's current state.

### 0.1 The Enforce engine is built, agent-light, kTLS-backed — and bidirectional-but-asymmetric

ADR-0069 (Accepted 2026-06-12, amended 2026-06-13) defines a **universal
agent-light L4 proxy** for transparent mTLS (#26). Verified facts:

- **Workloads hold nothing** (no cert, no key); both client and server workloads
  are identity-unaware and open ordinary plaintext sockets. The node agent does
  both halves of every flow it touches: outbound rustls **client** handshake
  presenting the workload's held SVID; inbound rustls **server** handshake
  presenting the server SVID and verifying the client's SVID via
  `WebPkiClientVerifier`.
- **Agent-light, not agent-idle, and the two directions are asymmetric** (the
  2026-06-13 amendment): DECRYPT/RX directions (outbound return, inbound deliver)
  are a genuine **zero-copy `splice(2)`** out of a plain kTLS-RX leg; ENCRYPT/TX
  directions (outbound forward, inbound response) are a **bounded `read →
  write_all` copy** into a kTLS-TX leg. A `splice` *into* kTLS-TX was proven
  NON-VIABLE (`MSG_DONTWAIT`-backlog record loss, trace-confirmed `n_out=55
  errno=0` while the peer received 0 bytes) and replaced by the blocking
  `write_all`. SSOT: `crates/overdrive-dataplane/src/mtls/splice.rs`.
- **Spike-proven on a real 7.0 kernel** (≥ the pinned 6.18 floor): TLS 1.3
  `application_data` (`0x17`) on the wire, 0 plaintext; fail-closed on
  `nocert`/`wrongca`. Inbound proven COMPOSED end-to-end one direction
  (increment-i). The v1 honest security claim is **chain-to-bundle transport
  auth + encryption, NO intended-peer SAN pinning** (the pin is the #178 upgrade).
- The whole sockmap-egress-redirect apparatus is **DELETED** from the forward
  path (the agent-idle in-kernel redirect was proven non-viable). This is
  decision-relevant: it means **a pure-kernel zero-copy forward (encrypt) over
  kTLS-TX is, on this codebase's evidence, not currently achievable** — see
  PART 3.5.

### 0.2 The outbound path is documented-lossy — THE CRUX

`crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs` (verified this run):
the `cgroup_connect4_mtls` program is a `cgroup_sock_addr(connect4)` hook. On a
`MTLS_REDIRECT_DEST[{ip,port}]` **hit** it rewrites `(user_ip4, user_port)` **in
place** to the agent's leg-F listener; on a **miss** it `return Ok(1)` — connect
proceeds unchanged. "Returns 1 on every code path — the hook only rewrites; it
never denies" (module docstring, verbatim).

`crates/overdrive-worker/src/mtls_intercept_worker.rs` accept-loop (verified this
run) states the lossiness explicitly in the `real_peer` field doc and the
`AcceptLeg::Outbound` arm:

> "the `cgroup_connect4_mtls` rewrite is LOSSY — it rewrote the workload's
> `connect(real_peer)` → `connect(leg_f)` **in place**, and the original
> destination is **NOT recoverable** from the accepted leg-F socket (unlike
> inbound TPROXY's `getsockname` orig-dst). The declared-peer seam SUPPLIES the
> dial target."

This is the whole reason today's design needs a *pre-programmed peer*: the agent
cannot observe `real_peer` from the leg-F connection alone, so the test-only
`program_declared_peer_redirect` seam (`#[cfg(integration-tests)]`, the "#178
stand-in") records `real_peer` into a shared slot before programming the redirect.
Production `start_alloc` programs NEITHER the outbound `MTLS_REDIRECT_DEST`
redirect NOR the inbound nft-TPROXY rule — both are #178-deferred, so a real
deploy runs cleartext (every connect misses the empty map).

### 0.3 Inbound already recovers orig-dst via nft-TPROXY + `getsockname`

`crates/overdrive-dataplane/src/mtls/inbound.rs` (verified this run): leg C is
`accept()`ed off the `nft`-TPROXY + `IP_TRANSPARENT` intercept; the worker
recovers the original destination via `getsockname` and passes it as `orig_dst`
to `establish()`. The orig-dst selects the server SVID and (single-node skeleton)
the server-dial address. The leg-S dial is `SO_MARK`-stamped so the nft-TPROXY
rule does not re-intercept the agent's own dial (the F5 recursion exemption). So
**inbound already does exactly the TPROXY+getsockname recovery the outbound path
lacks** — the working hypothesis "do the same outbound" has a working in-tree
inbound precedent.

### 0.4 The decision under test (#236) and the lever (ADR-0068)

The prior research doc (`stable-service-naming-and-transparent-mtls-comprehensive-research.md`
§3.5 R6/Q5, revision note) recommended the **enrollment / capture-and-resolve**
model for multi-node scale — capture all egress from a mesh-enrolled workload,
resolve destination + identity per-connection in the agent, fail-toward-handshake
— with the eager per-peer `MTLS_REDIRECT_DEST` map as a single-node v1 bridge
only. Tracked in **#236**. The rejected alternative is the per-destination
map-gated redirect where a miss = silent cleartext (rejected for cardinality at
mesh scale + the silent-cleartext footgun).

**The lever (ADR-0068).** Overdrive ships its own immutable appliance OS with a
**pinned kernel** — 6.18 LTS in production (released 2025-11-30, EOL Dec 2028),
7.0 on the dev VM, `bpf-next` as a soft-fail canary. The pin tracks the latest
qualifying LTS; it is *not* a portable agent on operator-supplied kernels. So
"the right answer for a portable mesh" and "the right answer for a
pinned-modern-kernel appliance" may differ — that gap is the whole point of this
re-test.

## PART 1 — The candidate interception models (enumerate + rank)

Five models, each = a *capture* strategy + an *orig-dst recovery* mechanism + a
*miss/cleartext* semantic. The capture strategy and the recovery mechanism are
**orthogonal** — a key clarification the framing risks conflating. "Enrollment /
capture-all" is a *capture* choice; "TPROXY+getsockname" is a *recovery* choice;
they can be mixed.

| # | Model | Capture | Orig-dst recovery | Miss = | Per-conn cost |
|---|---|---|---|---|---|
| **M1** | Per-destination map-gated redirect (current Overdrive single-node bridge; classic socket-LB shape) | `cgroup/connect4` rewrite ONLY on a programmed `MTLS_REDIRECT_DEST` hit | **lossy** today (in-place rewrite; recovery must be ADDED — see PART 2) | silent cleartext | one map lookup in kernel + (if armed) the agent handshake |
| **M2** | Enrollment capture-all + agent per-connection resolve (ztunnel; the pick under test) | capture *all* egress from an enrolled cgroup | recover at the agent (TPROXY+getsockname / cookie-stash) | fail-toward-handshake | one agent-side local `service_backends` lookup + handshake |
| **M3** | Hybrid: coarse enrollment capture + kernel-side resolve (BPF map hydrated from `service_backends`) recovers/rewrites to backend + tags identity; agent only on the handshake path | capture-all in cgroup | kernel rewrites to backend AND stashes orig-dst/identity (cookie-stash) | fail-toward-handshake | kernel map lookup + handshake (no userspace resolve) |
| **M4** | Policy-driven selective interception (intercept only declared mesh dests) | `cgroup/connect4` rewrite gated on a *policy* predicate, not a per-dest enumeration | same recovery options as M1/M2 | configurable | kernel predicate + handshake |
| **M5** | netkit-device redirect | per-pod netkit primary/peer device, BPF at the device | device-level; orig-dst preserved differently (not a connect-rewrite) | configurable | near-zero datapath overhead, but does NOT do the L7/identity termination |

**Ranking preview** (full justification in PART 5): for a pinned-modern-kernel
appliance the contest is M2 vs M3, with the *recovery* mechanism (PART 2) being
the more decision-relevant axis than the capture choice. M1 is correctly retained
only as the single-node bridge. M4 is M2 with a narrower capture filter. M5
(netkit) is a **datapath** optimization orthogonal to interception — it does not
recover orig-dst for an L7-terminating proxy and does not replace the agent (see
PART 3.2).

## PART 2 — Orig-dst recovery mechanisms head-to-head (THE CRUX)

The framing names four candidates. Each is evaluated for: does it actually
recover the outbound orig-dst the workload dialed; kernel floor; cost; and
whether it is the 2026 consensus. **A central adversarial finding up front: two
of the four candidates do not recover an *outbound* original destination at all.**

### 2.1 nft/iptables TPROXY + `getsockname` — the consensus, and what Overdrive already runs inbound

**What it does.** TPROXY (the `nft`/`iptables` `TPROXY` target, `IP_TRANSPARENT`
socket option) redirects a packet to a local listener *without rewriting the
packet's destination address*. The listener `accept()`s a connection whose
`getsockname()` returns the **original** destination IP:port — because TPROXY
delivered the packet to the listener while preserving the 4-tuple. This is
exactly Overdrive's **inbound** path today (`mtls/inbound.rs`, §0.3): leg C is
TPROXY-redirected, the worker recovers orig-dst via `getsockname`.

**Evidence.** ztunnel uses TPROXY for capture: "iptables rules use the **TPROXY
mechanism** with connection marking … TPROXY preserves the original destination
IP/port, allowing ztunnel to determine where the application intended to connect
without additional socket-level queries like `SO_ORIGINAL_DST`" [istio.io
traffic-redirection, fetched 2026-06-16]. Confidence: High (istio.io primary +
Overdrive's own working inbound implementation as a second independent
confirmation of the mechanism).

**Kernel floor.** TPROXY + `IP_TRANSPARENT` is ancient (pre-3.0). No modern-kernel
gate. **This is the portable-mesh consensus AND already proven in-tree for
Overdrive inbound.**

**For OUTBOUND.** Applying TPROXY outbound is the working hypothesis. The subtlety:
TPROXY is conventionally an *ingress/PREROUTING* target. To capture *egress* from
a workload and TPROXY it to a node-local listener, ztunnel does the capture
**inside the workload's own network namespace** (the in-pod redirection model,
§3.1) — the egress packet is treated as it enters the netns-local routing path,
fwmark'd, and TPROXY'd to ztunnel. Overdrive's inbound TPROXY already builds the
fwmark `ip rule` + `local` route + shared nft chain infra (the #234 inbound-TPROXY
shared routing infra). **The outbound application of the same infra is the
hypothesis the spike must confirm** — see PART 6.

### 2.2 `sk_lookup` — an INBOUND primitive; does NOT recover an outbound orig-dst

**Adversarial finding (load-bearing).** `sk_lookup` (kernel ≥ 5.9) is floated as
an outbound-recovery candidate but is **architecturally inbound**. The kernel doc
is explicit: "The attached BPF `sk_lookup` programs run whenever the transport
layer needs to find a listening (TCP) or an unconnected (UDP) socket for an
**incoming packet**. … Incoming traffic to established (TCP) and connected (UDP)
sockets is delivered as usual without triggering the BPF `sk_lookup` hook"
[docs.kernel.org/bpf/prog_sk_lookup, fetched 2026-06-16].

`sk_lookup` selects which *listening* socket receives an *inbound* connection via
`bpf_sk_assign()` — it has **no `connect()`-time role** and never sees a workload's
outbound dial. Worse for the recovery question: it does not, by itself, expose the
original destination to the selected socket (the kernel doc "provides no
information about whether the selected socket can recover the original destination
via `getsockname()`" — and in practice the steered socket sees the listener's own
bound addr unless paired with TPROXY-style transparency). **`sk_lookup` is the
wrong tool for the outbound crux.** It is a *bind-side-scaling* primitive (one
listener for an IP range/any-port), not an orig-dst-recovery primitive. Cloudflare
built it to avoid binding millions of addresses, not to recover connect targets.
Confidence: High (kernel doc primary; cross-ref the ebpf.io summit-2020 slides and
the merge commit `e9ddbb7707ff`, both naming the inbound steer-to-listener use
case).

**Verdict on sk_lookup for Overdrive outbound: rejected — solves a different
problem.** It *could* play a role on the **inbound** side (replace the nft-TPROXY
listener-selection with a BPF steer), but that is an inbound-path refactor, not an
answer to the outbound lossiness.

### 2.3 `cgroup/connect4` + socket-cookie / `bpf_sk_storage` stash → `cgroup/getsockopt(SO_ORIGINAL_DST)` — the genuine outbound contender

**What it does.** This is the canonical eBPF *outbound* transparent-proxy pattern
and it directly fixes Overdrive's lossiness **without TPROXY**:

1. `cgroup/connect4` intercepts the `connect()`, rewrites dest → local proxy, AND
   **stashes the original `(ip,port)` in a BPF map keyed by the socket cookie**
   (`bpf_get_socket_cookie`) — i.e. it does NOT throw the orig-dst away (the
   exact thing today's `cgroup_connect4_mtls` in-place rewrite does).
2. A `sockops` program records `src_port → cookie` once the proxy connection
   establishes (so the proxy can find the cookie from the accepted connection's
   peer port).
3. The proxy queries `getsockopt(SO_ORIGINAL_DST)`; a `cgroup/getsockopt` BPF
   program intercepts it, looks up `src_port → cookie → orig_dst`, and **returns
   the stashed orig-dst** to the proxy.

**Evidence.** The full pattern is documented across multiple independent
eBPF-transparent-proxy writeups [iximiuz Labs; medium/all-things-ebpf; jnfrati.dev
— all fetched-via-search 2026-06-16, medium-trust, cross-referenced ≥3]. The two
load-bearing *primitives* are kernel-primary: (a) `cgroup/getsockopt` "can observe
`optval`, `optlen` and `retval` … [and] can override the values above"
[docs.kernel.org/bpf/prog_cgroup_sockopt] — this is what lets a BPF program answer
`SO_ORIGINAL_DST` from a map; (b) Cilium's socket-LB uses exactly this hook family
in production — "`connect()`, `sendmsg()`, `recvmsg()`, `getpeername()` and
`bind()`"; "the connect and sendmsg BPF programs do forward translation … recvmsg
and getpeername BPF programs **revert translation**" [docs.cilium.io
kubeproxy-free, fetched 2026-06-16]. Cilium's `getpeername`-revert is the same
*shape* as the getsockopt-revert: the kernel hook gives the userspace caller back
the pre-translation address. Confidence: High for the primitives (kernel doc +
Cilium docs primary); Medium-High for the assembled SO_ORIGINAL_DST pattern
(documented in 3+ medium-trust writeups, primitives confirmed primary).

**Cost.** No nft rule, no `ip rule`/route table, no `IP_TRANSPARENT` listener.
Three small BPF programs + two small maps (cookie→orig_dst, src_port→cookie),
each a point-lookup. The socket cookie is a kernel-assigned unique id
(`bpf_get_socket_cookie`) — no cardinality blow-up (entries are per-live-connection,
reclaimed when the socket dies via `bpf_sk_storage` lifetime, the strongest
variant). **`bpf_sk_storage` (kernel ≥ 5.2) is strictly better than a plain
`HashMap<cookie, orig_dst>` here**: it is *attached to the socket* and freed
automatically when the socket closes — no leak, no manual reap, no cookie-reuse
race. Cilium explicitly moved this direction ("Replacing maps with BPF socket
storage would resolve issues related to map management" [docs.cilium.io]).

**Kernel floor (primary-anchored).** `BPF_PROG_TYPE_CGROUP_SOCKOPT`
(`cgroup/getsockopt`/`setsockopt`) = **5.3** (merge commit `0d01da6afc54`; LWN
791375 "bpf: getsockopt and setsockopt hooks"); `bpf_sk_storage`
(`BPF_MAP_TYPE_SK_STORAGE`) = **5.2** (merge commit `6ac99e8f23d4`; LWN 787194
"BPF sk local storage"); socket cookies older. Cross-confirmed against the
iovisor/bcc kernel-versions reference. **All comfortably under Overdrive's 6.18
pin.** Confidence: High (merge-commit + LWN primaries).

**Why it fits Overdrive specifically.** Overdrive ALREADY has a
`cgroup/connect4` outbound hook (`cgroup_connect4_mtls`) that already does the
rewrite. Adding the cookie-stash to that existing hook + a `cgroup/getsockopt`
revert program is the *smallest delta* to make the outbound path non-lossy — it
reuses the hook Overdrive already attaches, stays at the socket layer the agent
already lives near, and needs no new netns routing infra. **This is the strongest
candidate for the crux on a pinned-modern-kernel appliance.**

### 2.4 netkit redirect — does not recover orig-dst for an L7 terminator

netkit (kernel ≥ 6.7/6.8, §3.2) is a veth-replacement *datapath* device, not a
connect-interception primitive. It carries BPF at the device for fast pod↔host
forwarding; it does not rewrite `connect()` nor stash orig-dst for an
L7-terminating local proxy. It is **orthogonal** to the crux. (Overdrive could
adopt netkit for its workload veths to cut datapath overhead independently of the
mTLS interception decision — a separate, additive win; see PART 3.2.)

### 2.5 The crux verdict (preview)

Two real contenders for outbound orig-dst recovery: **(A) TPROXY+getsockname**
(consensus, already in-tree inbound, needs outbound-egress-in-netns plumbing) and
**(B) cgroup-connect4 cookie-stash + getsockopt-revert** (smallest delta to the
existing outbound hook, no routing infra, all primitives under the 6.18 pin).
`sk_lookup` and `netkit` are rejected for the outbound recovery question (wrong
direction / wrong layer). Full recommendation in PART 5/PART 6.

## PART 3 — Recent interception-mechanism developments (2024–2026)

### 3.1 Istio ambient / ztunnel: capture-all-then-resolve is STILL the model; in-pod redirection is the evolution

The 2023 model (iptables-in-pod redirect → ztunnel ports 15001/15006/15008 →
HBONE) is unchanged in shape through 2026. The evolution is **how** capture is
done, not **whether** capture-all-then-resolve is the model:

- **In-pod redirection** is now the default: "the ztunnel proxy has the ability
  to perform data path capture inside the Linux network namespace of the workload
  pod, achieved via cooperation between the istio-cni node agent and the ztunnel
  node proxy" [github.com/istio/istio architecture/ambient/ztunnel.md, fetched
  2026-06-16]. The capture moved from a host-netns hairpin to inside the pod's
  netns (cleaner, CNI-agnostic). eBPF redirection is an *option* the istio-cni
  plugin offers as a more efficient alternative to iptables chain traversal
  [istio.io ambient-ebpf-redirection, 2023].
- **Outbound orig-dst recovery is `SO_ORIGINAL_DST`, not plain TPROXY-getsockname**
  (decision-relevant correction to the working hypothesis): "we only have the
  destination IP/port (recovered via `SO_ORIGINAL_DST`)" [istio.io ztunnel.md
  architecture doc, verbatim]. ztunnel then consults its xDS-pushed `Address`
  resources to map dest → workload + identity. So the closest analog to
  Overdrive's outbound recovery is the **SO_ORIGINAL_DST family** (PART 2.3),
  which the cgroup-cookie-stash mechanism serves natively — *not* the TPROXY
  hypothesis. (ztunnel uses iptables REDIRECT outbound and TPROXY inbound;
  REDIRECT's recovery primitive is `SO_ORIGINAL_DST`.)
- **Per-connection resolve is NOT a bottleneck.** Istio's published benchmark:
  ~0.06 vCPU + 12 MB per 1000 rps; idle 30–50 MB; "higher TCP throughput than
  even in-kernel data planes like IPsec and WireGuard"; perf +75% over 4 releases
  [istio.io rust-based-ztunnel; oneuptime benchmark writeup cross-ref]. ztunnel
  does on-demand lookups for huge clusters (1M endpoints) rather than replicating
  all endpoints. Confidence: High (istio primary + the architecture doc's own
  scaling note). The 2025 ztunnel security audit [istio.io
  ztunnel-security-assessment, 2025; CNCF coverage] is a maturity signal — the
  model is production-hardened, not experimental.
- **2026 status**: Ambient is the default; multi-cluster alpha in 1.27 (Aug 2025);
  sidecar→ambient migration tooling in 1.28–1.29. Confidence: Medium (roadmap
  dates from a medium-trust aggregation; the *shape* claims are istio-primary).

**Takeaway:** the model Overdrive picked (capture-all + agent per-connection
resolve) is the *current, default, audited* model of the closest analog. It has
not been reconsidered or replaced — it has been hardened and made the default.

### 3.2 Cilium netkit: a datapath win, ORTHOGONAL to interception

netkit (Daniel Borkmann / Nikolay Aleksandrov, Isovalent; ByteDance/Meta interest)
**merged in kernel 6.7** [LWN "The BPF-programmable network device", Article
949960, 2023; merge confirmed] and is production-supported by Cilium from **6.8
onwards** [docs.cilium.io tuning guide, fetched 2026-06-16]. It is a veth
*replacement* that "reduces the datapath overhead for network namespaces down to
zero" — "guests … attain TCP data-transmission rates just as high as running
directly on the host" [LWN, verbatim]. Cilium will deprecate veth-mode once base
kernels are ubiquitous.

**But netkit is a datapath device, not an interception primitive** [docs.cilium.io,
verbatim: "Netkit concentrates on datapath performance, not traffic interception
or proxying"]. It does not rewrite `connect()`, does not recover orig-dst for an
L7-terminating local proxy, and does not replace the agent. **For Overdrive it is
an additive, independent win**: replace the workload veths with netkit to cut
pod↔host forwarding overhead, regardless of which mTLS interception model is
chosen. It is comfortably under the 6.18 pin (needs 6.7/6.8). **It does NOT change
the interception decision** — it is filed here so the team does not mistake it for
an interception answer. Confidence: High (LWN primary + Cilium docs primary;
minor version-floor discrepancy noted in Knowledge Gaps — merged 6.7, Cilium
production-supports 6.8).

### 3.3 Cilium native mTLS with ztunnel (2026): the industry CONVERGES on enrollment-capture-resolve

Cilium's classic mTLS was out-of-band auth + node WireGuard/IPsec (auth ≠ data).
In 2026 Cilium **adopted ztunnel** for native mTLS [cilium.io
native-mtls-cilium, 2026-03-23; docs.cilium.io encryption-ztunnel (Beta);
Azure AKS public preview, 2026-03; demonstrated at CiliumCon/KubeCon EU 2026]:

- "ztunnel is a purpose-built per-node proxy that provides transparent Layer 4
  mTLS encryption and authentication for pod-to-pod communication."
- **Per-namespace enrollment (capture-all)**: "enrolled pods have their traffic
  transparently redirected to the ztunnel proxy"; "enrollment via namespace
  labels … Pod-level enrollment is not supported" [docs.cilium.io
  encryption-ztunnel, verbatim]. This is the **direct analog of Overdrive's
  per-cgroup enrollment** (#236).
- **Holds connections inline, no first-packet drop** [cilium.io, verbatim] — the
  same fail-toward-handshake-not-cleartext intent as #236.
- **Encrypts the TCP payload stream** (avoids per-packet IPsec overhead),
  SPIFFE/SPIRE per-connection identity, per-namespace auto SPIRE registration.

**Takeaway:** the *second* major eBPF dataplane (after Istio) has now converged on
the enrollment-capture-resolve, per-connection-identity-bound L4-proxy model —
the exact model Overdrive picked. Cilium *moved toward* it from the out-of-band
model ADR-0069 already rejected (A4). Confidence: High (cilium.io blog +
docs.cilium.io Beta docs + Azure preview + KubeCon demo — 4 independent
confirmations of the convergence).

### 3.4 No-graceful-fallback is Cilium's WEAKNESS — Overdrive's fail-toward-handshake is better

Cilium's ztunnel: "Communication between an enrolled workload and a non-enrolled
workload is **not supported**" [docs.cilium.io encryption-ztunnel, verbatim] — a
hard cutoff, no graceful path. Overdrive's #236 fail-toward-handshake (should-be-
mesh dest not resolved → attempt mTLS or hold, never silent cleartext; external/
non-mesh egress passes through) is a **strictly richer miss semantic** than
Cilium's hard enrolled/non-enrolled boundary. This is a point where Overdrive's
chosen model is *ahead of*, not merely matching, the portable-mesh consensus.
Confidence: High (Cilium docs primary; Overdrive #236 / prior-doc §3.5 primary).

### 3.5 kTLS + eBPF: a pure-kernel zero-userspace-proxy mTLS exists ONLY via an out-of-tree kernel module

The adversarial question: is there a 2026 path doing per-workload-SVID mTLS with
**no userspace proxy at all** (pure kernel)? **Answer: yes, but only via an
out-of-tree kernel module — eBPF is fundamentally unsuited to it.**

- **Riptides / Cisco Camblet (nasp-kernel-module)** do the **entire mTLS
  handshake in the kernel** via a kernel module (`riptides-driver`) + kTLS for
  record crypto — no userspace proxy hop for the data path (only a userspace
  daemon to sync policy/trust-bundles/secrets) [riptides.io blog, fetched
  2026-06-16; github.com/cisco-open/nasp-kernel-module].
- **Why not eBPF**: "eBPF programs are subject to strict verifier constraints —
  such as limitations on loops, recursion, and stack depth — which make it
  impractical to implement complex protocols like TLS"; eBPF is "fundamentally
  unsuited for managing TLS handshakes or cryptographic state machines"
  [riptides.io, verbatim]. "If the application performs TLS entirely in user
  space, the eBPF program will only see ciphertext and cannot decrypt it"
  [eBPFChirp/search cross-ref].

**This authoritatively validates ADR-0069's agent-light choice.** The only way to
remove the userspace handshake hop is an **out-of-tree kernel module** — which for
an appliance OS means owning a kernel-module security/maintenance surface, kABI
breakage across the LTS-pin advances (6.18 → next LTS), and a far harder
verification story than userspace rustls. ADR-0069 already proved the eBPF-only
forward path (sockmap egress redirect into kTLS-TX) **NON-VIABLE** for an
*independent* reason (the `MSG_DONTWAIT` backlog record-loss class, §0.1). So two
independent lines of evidence (Riptides' "eBPF can't do the handshake" + ADR-0069's
"eBPF sockmap can't reliably do the forward kTLS-TX") converge: **a pure-eBPF,
zero-userspace-proxy, per-workload-SVID mTLS is not real on a mainline kernel in
2026.** Overdrive's agent-light-userspace-handshake + kernel-kTLS-steady-state is
the correct point on the spectrum. Confidence: High for "pure-eBPF is not viable"
(Riptides primary + ADR-0069 primary, two independent mechanisms); Medium for the
Riptides kernel-module details (single-vendor source, flagged — but the *eBPF
limitation* claim is corroborated by the kernel verifier constraints which are
themselves primary, and by ADR-0069's independent sockmap finding).

## PART 4 — Adversarial stress-test of enrollment / capture-all

The brief demands the enrollment model be attacked hardest. Each attack, and
where the evidence lands:

### 4.1 "Per-connection resolution on every new connection is a bottleneck" — REFUTED

ztunnel does exactly this (resolve dest→workload+identity per connection from
xDS-pushed state) at ~0.06 vCPU/1000 rps and higher throughput than IPsec/WireGuard
(§3.1). Overdrive's resolve would be **strictly cheaper**: a local in-memory
`service_backends` lookup (Corrosion-gossiped, already in RAM per the
reconciler-runtime bulk-load model) — **no xDS round-trip, no network hop**. The
mitigation ztunnel uses (don't replicate 1M endpoints; on-demand lookup) is moot
at Overdrive's single-node-v1 scale and tractable at multi-node (the same
`service_backends` gossip already bounds it). **The per-connection-resolve cost is
negligible and the attack fails.** Confidence: High.

### 4.2 "Capture-all blast radius — all egress (incl. external) through the agent" — PARTIALLY VALID, mitigated by scoping

Capturing *all* egress means external/non-mesh egress also hits the agent's
capture. ztunnel/Cilium scope this: Cilium hard-cuts non-enrolled (too strict);
Istio passes through non-mesh after the capture classifies it. The cost is one
extra hop for traffic that classifies as non-mesh and is passed through. **For
Overdrive the mitigation is the fail-toward-handshake classifier itself**: capture
→ resolve against `service_backends` → if mesh, mTLS; if not in `service_backends`
(external/non-mesh), pass through. The blast radius is a classification cost on
the *first* packet, not a per-byte tax (steady state is kTLS-spliced or
passed-through-once-classified). **Real but bounded; the design already accounts
for it via the pass-through arm.** This is the one place the capture-all model
genuinely costs more than the per-destination map (M1), where non-mesh egress
never touches the agent at all — but M1 pays for that with the silent-cleartext
footgun and the cardinality cost. Confidence: Medium-High (mechanism clear;
exact per-connection classification latency unmeasured — Knowledge Gap).

### 4.3 "Is there a no-userspace-proxy path?" — only via out-of-tree kernel module (§3.5), rejected for an appliance

Covered in §3.5. The pure-kernel path exists (Riptides/Camblet) but requires an
out-of-tree kernel module — wrong tradeoff for an appliance that advances its LTS
pin. The userspace proxy hop is the *correct* cost to pay. Confidence: High.

### 4.4 "Is TPROXY-outbound genuinely the consensus, or do sk_lookup / cookie-stash beat it?" — the consensus is SO_ORIGINAL_DST-family, which cookie-stash serves natively

The crux re-examined adversarially (PART 2): ztunnel's *actual* outbound recovery
is `SO_ORIGINAL_DST` (§3.1), not plain TPROXY+getsockname. `sk_lookup` is an
inbound primitive and does not apply (§2.2). On a pinned modern kernel the
**cgroup-connect4 cookie-stash + cgroup/getsockopt-revert** mechanism (§2.3) is
the *smallest delta to Overdrive's existing outbound hook* and serves
`SO_ORIGINAL_DST` natively — it is at least as good as, and arguably cleaner than,
re-plumbing TPROXY for egress-in-netns. **So the working hypothesis (TPROXY
outbound) is viable but is NOT obviously the best; the cookie-stash is the
stronger candidate for Overdrive specifically** (it reuses the hook already
attached, avoids new routing infra, all primitives under 6.18). This is the one
place the research *revises* the working hypothesis. Confidence: High for the
mechanism comparison; the final pick is a spike question (PART 6).

## PART 5 — Synthesis: the mechanism comparison matrix + verdict

### 5.1 Interception-model matrix

| Model | Orig-dst recovery | Per-conn cost | Cleartext-safety / miss | Kernel floor | Verifier/perf cost | Portable-vs-pinned fit |
|---|---|---|---|---|---|---|
| **M1** per-dest map-gated redirect (Overdrive single-node bridge) | lossy today; must add cookie-stash or TPROXY | kernel map lookup; agent only if armed | **silent cleartext on miss** (footgun) | any | low (one `cgroup/connect4` hook) | fine single-node; cardinality blows up multi-node |
| **M2** enrollment capture-all + agent resolve (ztunnel; the pick) | TPROXY+getsockname OR cookie-stash+getsockopt | local `service_backends` lookup + handshake (~0.06 vCPU/1k rps analog) | **fail-toward-handshake** (no silent cleartext) | sock-LB hooks ≥5.3 / TPROXY ancient | low (point-lookups) + userspace handshake | matches Istio+Cilium 2026 consensus; pin unlocks cookie-stash cleanliness |
| **M3** hybrid: capture-all + kernel-side resolve + agent only on handshake | cookie-stash in `cgroup/connect4`; kernel rewrites to backend | kernel map lookup; agent only for handshake | fail-toward-handshake | `cgroup/getsockopt` ≥5.3, `bpf_sk_storage` ≥5.2 | low; **moves the resolve off the userspace hot path** | **pin unlocks this** — portable meshes don't bank on it |
| **M4** policy-driven selective intercept | same as M1/M2 | kernel predicate + handshake | configurable | any | low | a narrower-capture M2 |
| **M5** netkit redirect | n/a (datapath device) | near-zero datapath | n/a (not an interceptor) | 6.7/6.8 | n/a | additive datapath win, orthogonal |

### 5.2 The verdict on the model — ENROLLMENT CONFIRMED, with one refinement

**Enrollment-based capture-and-resolve (M2) is confirmed as the right model for
Overdrive's pinned-modern-kernel appliance.** The adversarial test did not find a
better *model*; it found the industry **converging on** this model from two
directions (Istio default+audited; Cilium 2026 adoption away from the out-of-band
model ADR-0069 already rejected), with per-connection resolve cost empirically
negligible, and the only "better" alternative (pure-kernel mTLS) requiring an
out-of-tree kernel module that is wrong for an appliance. Overdrive's
fail-toward-handshake miss semantic is *richer* than Cilium's hard cutoff. The
eager per-peer map (M1) is correctly the single-node bridge only — its
silent-cleartext miss is the footgun the enrollment model exists to remove.

**The one refinement the pinned kernel unlocks: prefer M3-leaning recovery.**
Because Overdrive controls a 6.18+ kernel, it can put the orig-dst *recovery* (and,
optionally, the backend *resolve*) in the kernel via the **cgroup-connect4
cookie-stash + `bpf_sk_storage` + `cgroup/getsockopt`-revert** mechanism, keeping
the agent on the handshake path only. This is strictly cleaner than the
portable-mesh TPROXY-egress plumbing and is the lever the brief asked about:
**pinning 6.18 unlocks a kernel-side, no-routing-infra orig-dst recovery that
Istio/Cilium can't bank on for portability.** This does not change the *model*
(still enrollment capture-all + fail-toward-handshake); it changes the *recovery
mechanism* under it.

### 5.3 The crux verdict — recommended orig-dst recovery mechanism

**Recommended: `cgroup/connect4` cookie-stash + `bpf_sk_storage` +
`cgroup/getsockopt(SO_ORIGINAL_DST)`-revert.** Kernel floor: `cgroup/getsockopt`
**5.3**, `bpf_sk_storage` **5.2** — both far under the 6.18 pin. Justification:

1. **Smallest delta to existing code.** Overdrive *already* attaches
   `cgroup_connect4_mtls` and already rewrites in that hook. The fix is to *stop
   throwing the orig-dst away*: stash it (`bpf_sk_storage` keyed on the socket,
   auto-freed on socket close — no leak, no reap, no cookie-reuse race) before the
   rewrite, and add a `cgroup/getsockopt` program that returns it on
   `SO_ORIGINAL_DST`. The agent's leg-F accept loop then queries `SO_ORIGINAL_DST`
   instead of needing a pre-programmed `real_peer` — **this is exactly what
   retires the test-only `program_declared_peer_redirect` stand-in**.
2. **No new routing infra.** No fwmark `ip rule`, no `local` route table, no
   `IP_TRANSPARENT` egress listener — the TPROXY-outbound hypothesis needs all of
   that (the #234 shared-routing-infra, only built inbound today).
3. **Matches the actual consensus primitive.** ztunnel's real outbound recovery
   is `SO_ORIGINAL_DST` (§3.1), which this serves natively.
4. **Stays at the socket layer the agent already lives near** and the cookie/
   sk_storage maps are point-lookups (no iteration, no cardinality blow-up).

**Fallback / second choice: TPROXY + `getsockname` outbound** — the working
hypothesis. Viable (it is the consensus *capture* primitive and Overdrive already
runs it inbound), but it requires extending the inbound #234 fwmark/route/nft
infra to egress-in-netns, which is more moving parts than the cookie-stash. Keep
it as the fallback if the spike finds the cookie-stash + getsockopt-revert has a
`cgroup_sock_addr` interaction wrinkle on the pinned kernel.

**Rejected: `sk_lookup`** (inbound-only, does not recover an outbound orig-dst,
§2.2) and **`netkit`** (datapath device, not an interceptor, §3.2 — adopt
separately for veth replacement if desired).

### 5.4 The no-Tier-2-backstop hazard is load-bearing here

Both the recommended mechanism (`cgroup/getsockopt` + the existing
`cgroup_connect4` `cgroup_sock_addr` hook) and the fallback live on hook families
that **have no `BPF_PROG_TEST_RUN` backstop** (`cgroup_sock_addr` returns
`ENOTSUPP`; the project's own rule and feedback memory flag this). Per the prior
research's Q1 and the project rule: **this MUST be settled by a Tier-3 spike on a
real kernel**, not by research or review alone. The cookie-stash adds a
`cgroup/getsockopt` program whose interaction with the `cgroup_connect4` rewrite
and the agent's leg-F query is exactly the kind of cross-hook behavior that only
a real connect through the cgroup will reveal.

## PART 6 — Spike implications (`/nw-spike`: outbound orig-dst recovery)

The spike's job is to settle the crux on the pinned kernel. Given the findings,
it should probe **two mechanisms head-to-head**, not just confirm the TPROXY
hypothesis:

**Probe A (recommended mechanism) — cookie-stash + getsockopt-revert.**
- Hypothesis: a `cgroup/connect4` that stashes orig-dst in `bpf_sk_storage` before
  the leg-F rewrite, plus a `cgroup/getsockopt` program, lets the agent's leg-F
  accept loop recover the real peer via `getsockopt(SO_ORIGINAL_DST)` with no
  pre-programmed `real_peer` and no routing infra.
- Predicted outcome: the agent reads the workload's true `(ip,port)` from
  `SO_ORIGINAL_DST` on the accepted leg-F socket; `program_declared_peer_redirect`
  becomes unnecessary; the connection drives mTLS to the real peer.
- Falsification: `getsockopt(SO_ORIGINAL_DST)` returns the leg-F addr (stash not
  wired), or the `cgroup/getsockopt` program does not fire for `SO_ORIGINAL_DST`
  on the pinned 6.18 kernel, or `bpf_sk_storage` lifetime races the agent's query.

**Probe B (fallback) — TPROXY-outbound + getsockname.**
- Hypothesis: extending the inbound #234 fwmark/`ip rule`/`local`-route/nft infra
  to capture *egress* from the workload netns and TPROXY it to a leg-F
  `IP_TRANSPARENT` listener recovers orig-dst via `getsockname` (mirroring inbound).
- Predicted outcome: leg-F `getsockname` returns the original `(ip,port)`.
- Falsification: egress-in-netns TPROXY needs a routing shape that collides with
  the F5 agent-dial recursion exemption, or the SO_MARK/route interaction
  double-intercepts the agent's own leg-B dial.

**Compare populations (per debugging.md §5)**: run the *same* workload connect
through both probes, dump the recovered orig-dst from each, and diff against the
known dialed peer. The probe whose recovery is correct AND needs the least infra
wins.

**What the spike must NOT do**: gate on `--no-run` / compile-only (the
`cgroup_sock_addr` family has no Tier-2 backstop — a real connect through the
cgroup on the pinned kernel is the only honest signal; feedback memory + project
rule). It must actually run the connect under Lima/Tier-3.

**Decision the spike feeds**: confirm Probe A (cookie-stash) as the production
recovery mechanism and retire the `program_declared_peer_redirect` /
`install_inbound_tproxy` stand-ins for a real `MtlsRedirectHydrator`-fed enrollment
path (#236), OR fall back to Probe B if A has a pinned-kernel wrinkle. Either way
the *model* (enrollment capture-all + fail-toward-handshake) is confirmed; the
spike picks the recovery primitive under it.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| `cgroup_connect4_mtls.rs` (lossy outbound) | (internal) | PRIMARY | source | 2026-06-16 | Y (vs worker) |
| `mtls_intercept_worker.rs` (lossy-recovery comments, declared-peer seam) | (internal) | PRIMARY | source | 2026-06-16 | Y (vs hook) |
| `mtls/inbound.rs` (TPROXY+getsockname recovery) | (internal) | PRIMARY | source | 2026-06-16 | Y |
| ADR-0069 (agent-light proxy, kTLS asymmetry, sockmap non-viable) | (internal) | PRIMARY | ADR | 2026-06-16 | Y (vs code) |
| Prior research doc §3.5 R6/Q5 + #236 (enrollment decision) | (internal) | PRIMARY | research | 2026-06-16 | Y |
| ADR-0068 (pinned 6.18 LTS kernel) | (internal, via CLAUDE.md) | PRIMARY | ADR | 2026-06-16 | Y |
| docs.kernel.org/bpf/prog_sk_lookup | docs.kernel.org | High (1.0) | kernel doc | 2026-06-16 | Y (vs merge commit, ebpf.io) |
| docs.kernel.org/bpf/prog_cgroup_sockopt | docs.kernel.org | High (1.0) | kernel doc | 2026-06-16 | Y (vs Cilium) |
| LWN "The BPF-programmable network device" (netkit, art. 949960) | lwn.net | High (1.0) | kernel-feature writeup | 2026-06-16 | Y (vs Cilium docs) |
| istio.io ztunnel traffic-redirection | istio.io | High (1.0) | official docs | 2026-06-16 | Y |
| github.com/istio/istio architecture/ambient/ztunnel.md | github.com | High (1.0) | upstream design doc | 2026-06-16 | Y (SO_ORIGINAL_DST verbatim) |
| istio.io rust-based-ztunnel / ztunnel-security-assessment (2025) | istio.io | High (1.0) | official docs/blog | 2026-06-16 | Y |
| docs.cilium.io tuning guide (netkit) | docs.cilium.io | High (1.0) | official docs | 2026-06-16 | Y (vs LWN) |
| docs.cilium.io kubeproxy-free (socket-LB hooks, getpeername-revert) | docs.cilium.io | High (1.0) | official docs | 2026-06-16 | Y (vs kernel sockopt doc) |
| cilium.io native-mtls-cilium (2026-03-23) | cilium.io | High (1.0) | vendor blog | 2026-06-16 | Y (vs docs.cilium.io + Azure) |
| docs.cilium.io encryption-ztunnel (Beta) | docs.cilium.io | High (1.0) | official docs | 2026-06-16 | Y |
| ebpf.io summit-2020 sk_lookup slides | ebpf.io | High (1.0) | foundation talk | 2026-06-16 | Y (vs kernel doc) |
| torvalds/linux commit e9ddbb7707ff (sk_lookup merge) | github.com | High (1.0) | merge commit | 2026-06-16 | Y |
| riptides.io blog (kernel-module mTLS, eBPF limitation) | riptides.io | Medium (0.6) | vendor blog | 2026-06-16 | Partial — single-vendor, FLAGGED |
| github.com/cisco-open/nasp-kernel-module (Camblet) | github.com | High (1.0) | source | 2026-06-16 | Y (corroborates Riptides shape) |
| eBPF transparent-proxy writeups (iximiuz, medium/all-things-ebpf, jnfrati) | mixed | Medium (0.6) | community | 2026-06-16 | Y — 3-source cross-ref; primitives confirmed primary |

Reputation: PRIMARY internal: 6. High external: 11. Medium (flagged, cross-ref'd): 3+.
Avg reputation across cited external sources: **~0.90** (kernel.org/LWN/istio.io/
cilium.io/github merge-commits dominate; the cookie-stash assembled pattern and the
Riptides kernel-module detail are the only medium-trust load-bearing claims, both
flagged and each corroborated by a primary on the load-bearing sub-claim).

## Knowledge Gaps

### Gap 1: cookie-stash + `cgroup/getsockopt` exact behavior on the pinned 6.18 kernel
**Issue**: The cookie-stash → `getsockopt(SO_ORIGINAL_DST)`-revert pattern is
documented across 3 medium-trust writeups with the two primitives confirmed
kernel-primary, but the *assembled* pattern's interaction with Overdrive's
existing `cgroup_connect4_mtls` rewrite — and whether `cgroup/getsockopt` fires
for `SO_ORIGINAL_DST` specifically on 6.18 — is not settleable from docs. The
kernel sockopt doc confirms `cgroup/getsockopt` can override `optval`/`retval` but
does not name `SO_ORIGINAL_DST`. **Attempted**: kernel doc, Cilium docs (confirm
the hook family + revert shape), 3 transparent-proxy writeups. **Recommendation**:
this is exactly Probe A of the spike (PART 6) — the no-Tier-2-backstop hazard
makes it a mandatory Tier-3 probe, not a research-resolvable question.

### Gap 2: Riptides/Camblet kernel-module detail is single-vendor
**Issue**: The "pure-kernel mTLS needs an out-of-tree module; eBPF can't do the
handshake" claim rests primarily on riptides.io (medium-trust, commercial
interest in selling the kernel-module approach). **Attempted**: cross-referenced
against the cisco-open/nasp-kernel-module repo (same architecture, independent
project) and against the *kernel-primary* eBPF verifier constraints (loops/
recursion/stack-depth limits are documented kernel facts) and against ADR-0069's
*independent* finding that the eBPF sockmap forward path is non-viable.
**Recommendation**: the load-bearing conclusion ("Overdrive's userspace-handshake
+ kTLS is correct; don't chase pure-eBPF mTLS") is safe — it is triangulated by
three independent lines. The Riptides-specific implementation details are NOT
load-bearing for any Overdrive decision and need not be further verified.

### Gap 3: netkit kernel-floor discrepancy (merged 6.7 vs Cilium "6.8 onwards")
**Issue**: LWN says merged in 6.7; Cilium docs say production-supported from 6.8.
**Attempted**: LWN primary + Cilium docs primary. **Assessment**: both true — the
device merged in 6.7, Cilium requires 6.8 for its production integration (likely a
BIG-TCP/feature-completeness gate). Immaterial to Overdrive (pin is 6.18). Noted
for accuracy, not a blocker.

### Gap 4: exact per-connection classification latency of capture-all
**Issue**: §4.2 — the first-packet classification cost (capture → resolve →
mesh-or-passthrough) is bounded but unmeasured for Overdrive's specific in-memory
`service_backends` lookup. **Attempted**: ztunnel's published aggregate perf
(~0.06 vCPU/1k rps) is an upper bound (it includes an xDS-backed lookup; Overdrive's
is a cheaper in-RAM lookup). **Recommendation**: measure during the #236 DELIVER
wave; not a research-resolvable number and not decision-blocking (ztunnel's
aggregate already proves the order of magnitude is fine).

## Conflicting Information

### Conflict 1: "TPROXY+getsockname is the outbound consensus" (working hypothesis) vs ztunnel's actual `SO_ORIGINAL_DST`
**Position A** (the brief's working hypothesis): use TPROXY+getsockname outbound,
"like ztunnel." **Position B** (this research): ztunnel's *outbound* recovery is
`SO_ORIGINAL_DST` (iptables REDIRECT family) per its own architecture doc, not
plain TPROXY+getsockname (which it uses *inbound*). **Assessment**: Position B is
correct and primary-sourced (istio ztunnel.md, verbatim). This is a refinement,
not a contradiction of the model — both recover orig-dst transparently; the
mechanism family differs, and the `SO_ORIGINAL_DST` family is exactly what the
recommended cgroup cookie-stash serves. The working hypothesis is *viable* but the
cookie-stash is the better fit for Overdrive (PART 5.3).

### Conflict 2: "eBPF can do in-kernel mTLS" vs "eBPF is unsuited; needs a kernel module"
Not a conflict within sources — a consistent finding. Riptides (kernel-module),
the eBPF verifier constraints (kernel-primary), and ADR-0069's sockmap-non-viable
finding all agree eBPF alone cannot do the mTLS handshake/forward. No source claims
a production pure-eBPF per-workload-SVID mTLS exists. Recorded as a settled fact,
not a conflict.

## Full Citations

[1] Overdrive Project. "`cgroup_connect4_mtls` outbound intercept (lossy in-place rewrite)". `crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs`. (internal PRIMARY). Verified 2026-06-16.
[2] Overdrive Project. "`MtlsInterceptWorker` (lossy-recovery accept loop; #178 declared-peer stand-in)". `crates/overdrive-worker/src/mtls_intercept_worker.rs`. (internal PRIMARY). Verified 2026-06-16.
[3] Overdrive Project. "INBOUND enforcement (`nft`-TPROXY + `getsockname` orig-dst)". `crates/overdrive-dataplane/src/mtls/inbound.rs`. (internal PRIMARY). Verified 2026-06-16.
[4] Overdrive Project. "ADR-0069: Transparent mTLS via a universal agent-light L4 proxy (agent-light kTLS asymmetry; sockmap-egress-redirect proven NON-VIABLE)". `docs/product/architecture/adr-0069-...md`. Accepted 2026-06-12, amended 2026-06-13. (internal PRIMARY). Verified 2026-06-16.
[5] Overdrive Project. "Stable Service Naming + Transparent mTLS research §3.5 (enrollment model, #236)". `docs/research/networking/stable-service-naming-and-transparent-mtls-comprehensive-research.md`. (internal PRIMARY). Verified 2026-06-16.
[6] Linux Kernel Authors. "BPF sk_lookup program". docs.kernel.org. https://docs.kernel.org/bpf/prog_sk_lookup.html. Accessed 2026-06-16.
[7] Linux Kernel Authors. "BPF_PROG_TYPE_CGROUP_SOCKOPT". docs.kernel.org. https://docs.kernel.org/bpf/prog_cgroup_sockopt.html. Accessed 2026-06-16.
[8] Corbet, J. "The BPF-programmable network device" (netkit, merged 6.7). LWN.net. 2023. https://lwn.net/Articles/949960/. Accessed 2026-06-16.
[9] Istio. "Ztunnel traffic redirection" (TPROXY inbound; ports 15001/15006/15008). istio.io. https://istio.io/latest/docs/ambient/architecture/traffic-redirection/. Accessed 2026-06-16.
[10] Istio. "Ztunnel architecture" (SO_ORIGINAL_DST outbound recovery; xDS Address resolve; 1M-endpoint on-demand). github.com/istio/istio. https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md. Accessed 2026-06-16.
[11] Istio. "Introducing Rust-Based Ztunnel" / "Ztunnel security assessment (2025)". istio.io. https://istio.io/latest/blog/2023/rust-based-ztunnel/, https://istio.io/latest/blog/2025/ztunnel-security-assessment/. Accessed 2026-06-16.
[12] Cilium. "Tuning Guide" (netkit; kernel 6.8; BIG TCP). docs.cilium.io. https://docs.cilium.io/en/stable/operations/performance/tuning/. Accessed 2026-06-16.
[13] Cilium. "Kubernetes Without kube-proxy" (socket-LB connect/sendmsg/recvmsg/getpeername hooks; getpeername-revert; BPF socket storage). docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-06-16.
[14] Cilium. "Native mTLS for Cilium: Transparent Encryption Meets Cloud Native Identity with ztunnel". cilium.io. 2026-03-23. https://cilium.io/blog/2026/03/23/native-mtls-cilium/. Accessed 2026-06-16.
[15] Cilium. "Ztunnel Transparent Encryption (Beta)" (per-namespace enrollment; no non-enrolled fallback). docs.cilium.io. https://docs.cilium.io/en/latest/security/network/encryption-ztunnel/. Accessed 2026-06-16.
[16] eBPF Foundation. "Steering connections to sockets with BPF socket lookup hook" (Sitnicki, sk_lookup inbound use cases). ebpf.io. 2020. https://ebpf.io/summit-2020-slides/. Accessed 2026-06-16.
[17] Linux. "bpf: Introduce SK_LOOKUP program type" (merge commit). github.com/torvalds/linux. https://github.com/torvalds/linux/commit/e9ddbb7707ff5891616240026062b8c1e29864ca. Accessed 2026-06-16.
[18] Riptides. "Seamless Kernel-Based Non-Human Identity with kTLS and SPIFFE" (kernel-module mTLS; eBPF unsuited for TLS handshake). riptides.io. https://riptides.io/blog/seamless-kernel-based-non-human-identity-with-ktls-and-spiffe/. Accessed 2026-06-16. **[Medium-trust, single-vendor — FLAGGED; load-bearing sub-claim cross-referenced against kernel verifier constraints + cisco-open/nasp-kernel-module + ADR-0069.]**
[19] Cisco. "camblet-driver / nasp-kernel-module" (in-kernel TLS + identity via kernel module). github.com/cisco-open/nasp-kernel-module. Accessed 2026-06-16.
[20] eBPF transparent-proxy writeups (cookie-stash → getsockopt SO_ORIGINAL_DST pattern): iximiuz Labs "Transparent Egress Proxy with eBPF and Envoy"; medium/all-things-ebpf "Building a Transparent Proxy with eBPF"; jnfrati.dev "Building an eBPF Transparent Proxy". Accessed 2026-06-16. **[Medium-trust, 3-source cross-ref; the two primitives (cgroup/getsockopt override; bpf_sk_storage) confirmed kernel-primary via [7] and the docs.kernel.org bpf_sk_storage doc.]**
[21] Linux. "bpf: getsockopt and setsockopt hooks" (CGROUP_SOCKOPT introduced 5.3, commit 0d01da6afc54). LWN.net. https://lwn.net/Articles/791375/. Accessed 2026-06-16.
[22] Linux. "BPF sk local storage" (BPF_MAP_TYPE_SK_STORAGE introduced 5.2, commit 6ac99e8f23d4). LWN.net. https://lwn.net/Articles/787194/. Cross-ref iovisor/bcc kernel-versions reference (github.com/iovisor/bcc/blob/master/docs/kernel-versions.md). Accessed 2026-06-16.

## Research Metadata

Duration: ~1 session | Examined: 20+ sources (6 internal PRIMARY code/ADR; 11
high-trust external — kernel.org, LWN, istio.io, cilium.io, github merge-commits;
3+ medium-trust cross-referenced) | Cited: 20 | Cross-refs: every model/mechanism
claim verified across ≥2 independent sources; kernel-feature claims anchored to
kernel.org/LWN/merge-commit primaries; the two medium-trust load-bearing claims
(cookie-stash assembled pattern; Riptides kernel-module) each triangulated against
a primary | Confidence: **High** for the model verdict (enrollment confirmed) and
the mechanism ranking; **Medium-High** for the specific cookie-stash recommendation
(primitives primary, assembled-pattern + pinned-kernel behavior is the spike's
Probe A) | Citation coverage: >95% | Avg external reputation: ~0.90 | Output:
`docs/research/networking/transparent-mtls-interception-mechanism-2026-research.md`

**Confidence by finding**: High — §0 (code-verified), §2.1 TPROXY (istio primary +
in-tree inbound), §2.2 sk_lookup-is-inbound (kernel doc primary), §3.1 ztunnel
model unchanged+audited, §3.2 netkit orthogonal (LWN+Cilium primary), §3.3 Cilium
convergence (4 sources), §3.4 fail-toward-handshake superiority, §5.2 model
verdict. Medium-High — §2.3 cookie-stash (primitives primary, pattern medium-trust),
§3.5 pure-kernel-needs-module (Riptides flagged, triangulated), §5.3 recommended
mechanism (the spike's Probe A confirms). Medium — §3.1 Istio roadmap dates
(medium-trust aggregation), §4.2 classification-latency (unmeasured).
