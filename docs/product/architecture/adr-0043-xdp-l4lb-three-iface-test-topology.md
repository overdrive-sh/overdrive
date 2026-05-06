# ADR-0043 — XDP L4LB three-iface transit test topology (`client-ns ←veth1→ lb-ns ←veth2→ backend-ns`)

## Status

Accepted. 2026-05-06. Decision-makers: Morgan (proposing). Tags:
phase-2, dataplane, test-topology, xdp, l4lb, integration-test.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic-
swap primitive), ADR-0041 (weighted Maglev + REVERSE_NAT shape +
endianness lockstep), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService`).

## Context

Phase 2.2 (XDP service map) ships an XDP+TC L4 load-balancer whose
production hot path returns `XDP_TX` after rewriting the destination
header. `XDP_TX` bounces the rewritten frame back out the *same*
ingress iface; the kernel routing table on the LB host then forwards
it to the chosen backend. This is the shape every credible production
XDP L4LB implements — Cilium's standalone L4LB (PR #16338,
`bpf-lb-mode: dsr`, `bpf-lb-acceleration: native`) and Katran both
attach on a single iface where the LB host has reachability to the
backend network via host routing.

The acceptance test for Slice 15 (S-2.2-15) drives a real `nc`
TCP connection through the LB program to a backend in a different
netns. Three attempts against a two-namespace topology
(`client-ns ←veth-pair→ backend-ns`, with the XDP program loaded on
the LB-side veth peer) have failed in the same shape: the SYN arrives
at the LB iface from the client; `XDP_TX` rewrites the destination
and returns the frame back out the same iface — toward the client,
not toward the backend. There is no second iface on the LB side, no
routing entry, no path that resolves "the rewritten dest IP routes
out a different egress."

This is **not** a case of "the test environment is too constrained to
express production." A two-namespace veth pair with the XDP program
attached to one peer is not a smaller version of the production
shape — it is a *different* shape, one that strips out the routing
host entirely. Production XDP L4LBs (Cilium with `bpf-lb-mode: dsr` +
`bpf-lb-acceleration: native`, Katran) attach on a single iface of a
host whose kernel routing table reaches the backend network on that
same NIC; `XDP_TX` returns the rewritten frame, and the host's
routing layer — running on the same machine, the same kernel — picks
it up and forwards it. The routing host is not incidental scaffolding
around the XDP program; it IS half the L4LB system. Two-namespace
veth has no routing host on the LB side, so it cannot exercise an
L4LB at all — there is no system under test, only a packet rewriter
detached from the routing context that gives those rewrites meaning.
The peer-stub XDP program required by the kernel for
`XDP_TX`/`XDP_REDIRECT` delivery on veth (kernel patch v7 09/10,
"veth: Add XDP TX and REDIRECT") is a delivery prerequisite that
applies in either topology; it does not, and cannot, supply the
missing routing host.

The three-iface transit topology restores the routing host as
`lb-ns`: a netns whose routing table reaches the backend network on
the same iface `XDP_TX` returned the frame on. This is the production
shape expressed in netns form, not a test-side workaround that
compensates for a topology limitation.

The research memo at
`docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`
(2026-05-06) enumerates four candidate options and concludes that
Option A (three-iface transit) is the answer Cilium's own production
integration test (PR #16338) implements: `ip l a l4lb-veth0 type veth
peer l4lb-veth1` + `ip l s dev l4lb-veth1 netns <ns>`, with
`bpf_xdp_veth_host.o` loaded on both peers. Cilium's adoption of this
topology is decisive precisely because Cilium's production L4LB
deployments have the *same* property the three-iface test models: the
LB host routes the rewritten frame through its kernel routing table
on the same iface `XDP_TX` returned it on. Two-namespace topologies
do not appear in the published reference set because they cannot
support `XDP_TX`-style L4LB at all — the routing host is not
something to be added later for fidelity, it is the missing half of
the system.

## Decision

### 1. Adopt the three-iface transit topology for end-to-end LB tests

Tier 3 integration tests that drive real packets through the
production XDP+TC programs run against:

```
        veth1                   veth2
client-ns ←──── lb-ns ────→ backend-ns
   │            │   │            │
client app   peer-A peer-B    backend app
             (XDP+TC programs attached on peer-B = veth2 in lb-ns)
```

Three netns, two veth pairs:

- `client-ns` carries `veth1`'s client-side peer; the test's `nc`
  client opens its connection from inside this netns.
- `lb-ns` carries `veth1`'s LB-side peer (no programs attached) AND
  `veth2`'s LB-side peer (the production XDP+TC programs attach
  here). The lb-ns routing table is configured so the rewritten
  destination IP (the chosen backend) routes via `veth2`. **`lb-ns`
  is not a test-side concession; it IS the production routing host
  expressed at a netns boundary** — the same single-NIC routing
  layer Cilium and Katran rely on in their bare-metal L4LB
  deployments, scaled down to a netns and a programmatically
  configured route.
- `backend-ns` carries `veth2`'s backend-side peer; the test's
  backend listener (`nc -l`) accepts inside this netns. A stub
  XDP program (`return XDP_PASS`) is loaded on `veth2`'s
  backend-side peer to satisfy the kernel's veth peer-program
  requirement (kernel patch v7 09/10). The stub is the project's
  test-only equivalent of Cilium's `bpf_xdp_veth_host.o`.

The `XDP_TX` return code on the production hot path is **unchanged**.
The kernel-side architecture (XDP_TX as the primary return on
SERVICE_MAP hit; reverse-NAT on TC egress; endianness lockstep) is
the same shape ADR-0040, ADR-0041, and ADR-0042 lock — this ADR
extends test-topology only.

### 2. Scope is `crates/overdrive-dataplane/tests/integration/` test helpers

The decision is **test-side only**. No production code, no
kernel-side BPF program, no `Dataplane` port body, no `aya` loader
call site changes shape as a result of this ADR. Concretely:

- A new helper at `crates/overdrive-dataplane/tests/integration/helpers/netns.rs`
  (already in the working tree per `git status`) gains a
  `ThreeNamespaceTopology` shape that composes the existing
  `NetNs` RAII helper and the existing `veth.rs` pair-creation
  helper. The shape mirrors Cilium PR #16338's `ip l a` /
  `ip l s dev … netns …` sequence exactly.
- The peer-stub XDP program is added to `crates/overdrive-bpf` as a
  named test-only program (analogous to Cilium's
  `bpf_xdp_veth_host.o`). It returns `XDP_PASS` unconditionally and
  exists solely to satisfy the kernel's veth peer-program delivery
  prerequisite.
- The XDP+TC production programs (the SERVICE_MAP lookup, the
  REVERSE_NAT TC-egress program) attach to `veth2`'s lb-ns peer via
  the same `EbpfDataplane` loader path production uses. No special
  test-only loader path; `EbpfDataplane::new` is the entry.
- The lb-ns routing table is populated programmatically via the
  existing helper sequence (`ip addr add … dev veth2`,
  `ip route add … dev veth2`) — no new infrastructure.

### 3. Slice coverage stays unchanged

Prior slices that assert on `XDP_TX` semantics (S-2.2-04 SERVICE_MAP
hit, S-2.2-06 atomic backend swap, S-2.2-08 truncated-frame handling,
S-2.2-15 nc-driven end-to-end, S-2.2-17 endianness lockstep,
S-2.2-20 REVERSE_NAT lockstep, S-2.2-21 non-IPv4 passthrough)
**keep their `XDP_TX` assertions verbatim**. The three-iface helper
makes those assertions exercisable in netns; it does not require
restating them. Slices that already pass against Tier 2
(`BPF_PROG_TEST_RUN` triptych) — most of the kernel-side correctness
— do not depend on this ADR at all.

Future LB-shaped tests inherit the helper rather than reinventing
topology per slice. This is the analogue of how `NetNs` and
`VethPair` are reused across the existing acceptance suite.

## Alternatives Considered

### B — Switch the kernel-side return from `XDP_TX` to `XDP_PASS` (DSR-style)

Rewrite headers in XDP, return `XDP_PASS` on the hit path, let the
kernel routing stack forward the rewritten frame to the backend.
This is a recognized pattern in Cilium's codebase as the
`punt_to_stack` *fallback*, but never as the primary L4LB hot path
in either Cilium or Katran. **Rejected**: switching the production
fast path from `XDP_TX` to `XDP_PASS` permanently degrades hot-path
throughput (`XDP_TX` emits via the driver hook with no skb
allocation; `XDP_PASS` allocates an skb and traverses the full
kernel networking stack on every load-balanced packet) to fix a
*test-topology* problem. It also forces every prior slice asserting
`XDP_TX` (S-2.2-04, S-2.2-06, S-2.2-15, S-2.2-17, S-2.2-20,
S-2.2-21) to be reworked. The user's intuition that `XDP_PASS`
shaves verifier budget by skipping checksum work is structurally
wrong: both return shapes require identical RFC 1624 incremental
checksum updates because the kernel's RX-checksum-offload verdict
is computed against the *original* bytes before XDP runs and is not
recomputed on the `XDP_PASS` path; if XDP mutates headers without
fixing checksums the kernel drops the malformed packet at socket
demux. The verifier walks every reachable path either way, so
flipping the hit-path return changes neither instruction count nor
checksum cost. See research memo § 2.1, § 2.3, § 3.1, § 3.2.

### C — Defer the real-`nc` test with `#[ignore]`

Land the kernel-side correctness via Tier 2 (`BPF_PROG_TEST_RUN`
triptych) only and mark S-2.2-15's real-`nc` end-to-end test
`#[ignore]` until "later." **Rejected**: this is not "deferring a
test that needs an exotic topology" — it is deferring the *only*
test that exercises the production L4LB shape. The three-iface
topology is not exotic, it is the production shape; the ignored
test is the one that exercises the actual deployment shape end-to-
end. Ignoring it is the largest possible concession in this
decision space, not the smallest: every subsequent change to the
XDP+TC programs lands without a real-traffic gate, and the "every
change validated end-to-end" property the four-tier testing posture
exists to provide silently erodes. `#[ignore]` per
`.claude/rules/testing.md` § "What about `#[ignore]`?" is also
mismatched on its own terms — it is reserved for tests waiting on
**external** resources the implementation cannot synthesize (a
kernel matrix only available in CI, real BPF ELF the upstream
pipeline doesn't yet emit, etc.). The routing host is not an
external resource; the implementation can synthesize it — Cilium
PR #16338 is the proof. Erosion accumulating across slices is
exactly what the research memo flags as the dominant long-term
cost of this option. See research memo § Risks (C).

### D — `XDP_REDIRECT` to a peer veth ifindex

Switch the production hot path from `XDP_TX` to `XDP_REDIRECT`,
populating a `BPF_MAP_TYPE_DEVMAP` with the backend-side veth
ifindex; the kernel delivers directly across the veth boundary.
**Rejected**: this is a kernel-side architectural change (not a
test-side change) with substantially the same blast radius as
Option B — every prior slice asserting `XDP_TX` reworks, plus a new
DEVMAP map family and userspace handle work, plus a new typed
`Action::DataplaneUpdateDevmap` shape and ObservationStore row to
mirror ADR-0042's hydrator pattern. And it does not even solve the
underlying problem any better than Option A: `XDP_REDIRECT` to a
veth peer requires the *same* peer-stub XDP program in
`backend-ns` that Option A's `XDP_TX` setup needs (kernel patch v7
09/10 governs both paths identically). On top of all that,
`XDP_REDIRECT`-to-ifindex is the wrong production model for a
single-NIC L4LB — Cilium and Katran both use `XDP_TX` for
`bpf-lb-acceleration: native`, with `XDP_REDIRECT` reserved for
multi-NIC routing/firewall scenarios. Option D pays the full cost
of an architectural change and buys no test-fidelity over Option
A. See research memo § Q4 (Findings 4.1, 4.2, 4.3) and § Risks (D).

## Consequences

**Positive:**

- S-2.2-15's real-`nc` end-to-end gate becomes structurally
  achievable; the production hot path is exercised against real
  packets in netns rather than only via Tier 2 synthetic
  `BPF_PROG_TEST_RUN` input.
- Production code is unchanged. ADR-0040, ADR-0041, ADR-0042 remain
  accepted as-is — no amendments to their decisions, no rework of
  the SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP three-map split, the
  weighted Maglev shape, or the `ServiceMapHydrator` reconciler.
- Test topology mirrors Cilium PR #16338 directly. Future
  contributors reading the integration suite recognize the shape;
  the helper is portable to any LB-shaped slice that needs the same
  property.
- The peer-stub XDP program (`return XDP_PASS`) is a single
  kernel-side BPF source file (~10 LoC) added once; reusable by
  every subsequent test that needs veth peer-program delivery
  satisfaction without imposing real LB logic on the peer side.

**Negative:**

- Test helpers gain a routing-host-in-miniature: a third netns
  (`lb-ns`), a second veth pair, a routing-table setup step inside
  `lb-ns`, and the peer-stub XDP program. This is not "extra"
  relative to the production shape — it is the production shape.
  Without `lb-ns` there is no L4LB system under test, only an XDP
  program detached from any routing context. Per-test setup cost
  grows correspondingly (a small handful of additional `ip`
  invocations and one extra ELF load); negligible against typical
  Tier 3 wall-clock budget but visible.
- Netns-as-routing-host is a *scaled-down model* of "the LB host's
  routing table reaches the backend on the same NIC" — a netns test
  that passes in this shape is not, by itself, evidence that the
  same program attached to a physical i40e/mlx5 NIC will work
  end-to-end on a bare-metal host. The architectural shape is
  identical (the routing host IS the routing host whether expressed
  as a netns or as a physical machine); the residual gap is driver
  and hardware behaviour, not topology. This is the same fidelity
  gap Cilium accepts in PR #16338 and is mitigated at higher tiers:
  Tier 4 (xdp-bench against real virtio-net NICs in CI; veristat
  against the kernel matrix) and per-release real-hardware
  validation per `.claude/rules/testing.md` § "Scope boundaries".
  The fidelity gap is structural to all netns testing, not specific
  to this decision.

**Operational implications:**

- ADR-0040, ADR-0041, ADR-0042 require **no amendments**. Their
  status lines stay "Accepted"; no supersession, no amending
  reference. The kernel-side architecture they lock is fully
  consistent with this ADR's test-side topology choice.
- `crates/overdrive-dataplane/tests/integration/helpers/netns.rs`
  becomes the canonical entry point for LB-shaped Tier 3 tests.
  Future slices that need single-iface (non-LB) topology continue
  to use the existing `NetNs` + `VethPair` two-namespace shape
  unchanged.
- The peer-stub XDP program lives in `crates/overdrive-bpf` under
  a test-only program name (e.g., `xdp_veth_peer_stub`) and is
  loaded by the test helper, not by `EbpfDataplane`. It is not
  part of the production loader's program set.
- Tier 2 PKTGEN/SETUP/CHECK coverage is unaffected — synthetic
  `BPF_PROG_TEST_RUN` input does not need a topology at all and
  continues to gate kernel-side correctness independently.

## References

- `docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md`
  — Decisive research memo (Cilium PR #16338, Katran's
  `BPF_PROG_TEST_RUN`-only test rig, kernel patch v7 09/10 veth
  peer-program requirement, `XDP_TX` vs `XDP_PASS` checksum-budget
  analysis).
- Cilium PR #16338 — "helm,test: Add standalone L4LB XDP tests in a
  form of Github Action" — production-fidelity reference for
  multi-veth, multi-netns L4LB integration testing.
- Cilium L4LB blog post (2022-04-12) — `bpf-lb-mode: dsr`,
  `bpf-lb-acceleration: native`, `bpf-lb-dsr-dispatch: ipip`.
- Katran `DEVELOPING.md` and `BPF_PROG_TEST_RUN` test fixtures —
  reference for the choice not to do real-netns L4LB testing at all.
- netdev mailing list, "[PATCH v7 bpf-next 09/10] veth: Add XDP TX
  and REDIRECT" — kernel patch establishing the veth peer-program
  delivery requirement for `XDP_TX` and `XDP_REDIRECT`.
- xdp-project/xdp-tutorial `packet03-redirecting` — corroborates the
  peer-stub requirement.
- `docs/whitepaper.md` § 7 *eBPF Dataplane / XDP — Fast Path Packet
  Processing*; § 15 *Zero Downtime Deployments*.
- `.claude/rules/testing.md` § Tier 3 — Real-Kernel Integration; §
  "What about `#[ignore]`?"; § "Scope boundaries".
- `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side
  patterns" — XDP return code semantics; attach-mode (native vs SKB)
  fallback shape.
- ADR-0040 (SERVICE_MAP three-map split + HASH_OF_MAPS) — companion;
  not amended.
- ADR-0041 (weighted Maglev + REVERSE_NAT shape + endianness
  lockstep) — companion; not amended.
- ADR-0042 (`ServiceMapHydrator` reconciler +
  `Action::DataplaneUpdateService` + `service_hydration_results`) —
  companion; not amended.
