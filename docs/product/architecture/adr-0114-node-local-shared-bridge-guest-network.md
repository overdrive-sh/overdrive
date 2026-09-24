# ADR-0114 — Node-local shared bridge and guest-prefix network for microVMs

## Status

**Accepted — the current #295 contract is user-approved and independently
approved, including D-295-DISTILL-5 at review iteration 6 on 2026-09-16.**
Runtime ownership is defined by ADR-0124. **Amended 2026-09-24** by the
accepted #295 correctness-recovery replacement (see § *Accepted amendment
2026-09-24* at the end): the allocation TAP's activation timing and the VMM's
attachment mechanism are decided by ADR-0127 through ADR-0131, and TAP egress
delivery by ADR-0142.

## Context

The live microVM path assigns each allocation a `NetSlot`, a per-workload
network namespace and veth pair, transit and guest `/30` networks, and a TAP
inside the namespace. GH #293 removed the host Exec driver, so those mechanisms
no longer provide a workload isolation boundary. Part A of the #295 spike proved
two real microVM TAPs can use one Linux bridge and one subnet with transparent
capture and DNS. Part B executed the production mTLS, identity, resolver, and
cgroup core on that topology. Both parts are same-node evidence only.

## Decision

Replace the microVM network topology with one node-local Linux bridge. Each
allocation connects one host-netns TAP directly to that bridge. The bridge is
the sole node-local L2 switch for running microVMs.

Delete the per-workload netns, veth pair, transit/guest `/30` carves,
`NetSlot`, `host_veth`, setns launch, and `/etc/netns` resolver path. Keep the
persisted `workload_addr` as the observed address that health, backend, and
cleanup consumers read.

Exact implementation-facing alternatives are in
`docs/feature/netns-density-295/feature-delta.md`.

The shared guest-switch owner is singular across node and allocation effects.
The same injected object owns the startup probe, post-VMM stale sweep,
production convergence, non-repairing audit, TAP quiescence, and the inherited
per-allocation provision/teardown boundary. Its plan, ports, facts, and
source-bearing orchestration error live in
`overdrive-control-plane::guest_network`; the private host implementation
continues to compose the existing netlink and dataplane adapters. Core carries
only grouped driver/VMM handoffs, the fixed bridge MAC, and dependency-neutral
EXEC capabilities/request values. This placement creates neither a second
network owner nor a low-level netlink/BPF port. Exact signatures live only in
the #295 feature delta.

The private host owner also owns the startup scratch algorithm. A module-private
typed I/O boundary supplies only raw netlink/TCX/socket effects and per-family
counts; it cannot construct the public complement or cleanup aggregate. The
host owner retains setup and semantic-probe ordering, unconditional reverse
cleanup, continuation after the first cleanup failure, all-fifteen inventory,
and the final source-honest disposition. This is effect isolation inside the
same owner, not another shared-switch owner or a public fault interface.

This ADR does not decide the allocation TAP's administrative-state timing or
the VMM's attachment mechanism. The accepted zero-frame outcome (ADR-0088)
constrains both, and ADR-0127 through ADR-0131 decide them.

## Alternatives considered

### Keep per-workload netns and raise the slot ceiling

Rejected. It retains the container-CNI mechanism and its per-workload veth,
routes, `/30` waste, naming ceiling, and recovery machinery. Raising a constant
does not change that structural cost.

### One bridge per workload

Rejected. It removes netns but preserves an O(N) switching object and does not
provide the one shared-switch density model the spike proved.

### Use the public shared-owner port to inject completed startup results

Rejected as proof of the host algorithm. Public sim-port scripting remains
valid for deterministic composition behavior, but a completed owner result
cannot prove that the private host implementation actually attempted cleanup
or observed kernel scratch inventory. The private typed I/O boundary exercises
that algorithm without widening the public port.

## Consequences

Positive: one TAP/address/MAC per guest, no per-workload namespace/veth/routes,
and one node-level switch owner. Negative: the shared bridge becomes a
node-level dependency. The shared bridge also exposes each guest to ambient
L2 traffic, which is why the TAP-activation decisions (ADR-0127 through
ADR-0131) are load-bearing for the zero-frame outcome. On one learning bridge a
queue holder can also redirect another guest's host-to-guest frames by changing
its own TAP's host-side MAC; ADR-0142 closes that at TAP egress. Address
derivation/recovery is decided independently by ADR-0118 and DNS by ADR-0116.
Cross-host routing is outside #295 and tracked by
[GH #298](https://github.com/overdrive-sh/overdrive/issues/298) without a
prescribed answer here. The hypervisor remains the isolation boundary; per-VM
cgroups remain the resource and process boundary.

## Accepted amendment 2026-09-24 — #295 correctness-recovery replacement

Accepted by the user on 2026-09-24 with the GH #295 correctness-recovery
replacement DESIGN (`docs/feature/netns-density-295/feature-delta.md`).

**Why this is stated as an amendment.** The shared-bridge contract above is
operative in code committed at HEAD `db3af700` on the #295 feature branch
(steps 01-01 to 03-03; not merged to `main`; no persisted or wire state). That
committed owner raises each allocation TAP inside provision and reads it back
up, and Cloud Hypervisor attaches the TAP by name.

**What changes.** Both are superseded:

- The TAP is created owned by uid 0 and stays administratively down through
  VMM start, READY, and Running
  ([ADR-0130](adr-0130-guest-taps-carry-no-unprivileged-owner-grant.md)).
- Cloud Hypervisor receives one inherited queue descriptor
  ([ADR-0127](adr-0127-inherited-tap-queue-descriptor-guest-nic-attachment.md),
  [ADR-0128](adr-0128-vmm-adapter-owns-per-launch-tap-queue-descriptor.md),
  [ADR-0129](adr-0129-safe-descriptor-mapping-for-vmm-launch.md)).
- The same shared owner raises the TAP only after the allocation's mTLS
  install-success event and before EXEC
  ([ADR-0131](adr-0131-activate-allocation-tap-after-intercept-live.md)).
- Each managed TAP also carries a TCX egress classifier that delivers unicast
  only to its registered guest MAC
  ([ADR-0142](adr-0142-guest-tap-egress-drops-frames-to-foreign-destination-mac.md)).

The single shared-switch owner, the node-local bridge, and the deleted netns
mechanism are unchanged. The superseded behaviour committed on the feature
branch is replaced in the single #295 implementation cut.
