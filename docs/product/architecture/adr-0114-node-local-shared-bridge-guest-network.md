# ADR-0114 — Node-local shared bridge and guest-prefix network for microVMs

## Status

**Accepted — the current #295 contract is user-approved and independently
approved, including D-295-DISTILL-5 at review iteration 6 on 2026-09-16.**
Runtime ownership is defined by ADR-0124.

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
node-level dependency. Address derivation/recovery is decided independently by
ADR-0118 and DNS by ADR-0116. Cross-host routing is outside #295 and tracked by
[GH #298](https://github.com/overdrive-sh/overdrive/issues/298) without a
prescribed answer here. The hypervisor remains the isolation boundary; per-VM
cgroups remain the resource and process boundary.
