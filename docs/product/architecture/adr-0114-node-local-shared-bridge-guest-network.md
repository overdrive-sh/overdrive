# ADR-0114 — Node-local shared bridge and guest-prefix network for microVMs

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. Runtime ownership is completed by ADR-0124.

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

## Alternatives considered

### Keep per-workload netns and raise the slot ceiling

Rejected. It retains the container-CNI mechanism and its per-workload veth,
routes, `/30` waste, naming ceiling, and recovery machinery. Raising a constant
does not change that structural cost.

### One bridge per workload

Rejected. It removes netns but preserves an O(N) switching object and does not
provide the one shared-switch density model the spike proved.

## Consequences

Positive: one TAP/address/MAC per guest, no per-workload namespace/veth/routes,
and one node-level switch owner. Negative: the shared bridge becomes a
node-level dependency. Address derivation/recovery is decided independently by
ADR-0118 and DNS by ADR-0116. Cross-host routing is outside #295 and tracked by
[GH #298](https://github.com/overdrive-sh/overdrive/issues/298) without a
prescribed answer here. The hypervisor remains the isolation boundary; per-VM
cgroups remain the resource and process boundary.
