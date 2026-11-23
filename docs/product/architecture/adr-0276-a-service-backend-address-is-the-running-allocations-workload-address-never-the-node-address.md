# ADR-0276 — A Service backend's address is the Running allocation's workload address, never the node address

## Status

Accepted on user ruling, 2026-11-06 (rulings 1 and 2 recorded in the
[feature delta](../../feature/no-host-ip-backend-fallback/feature-delta.md#user-rulings)).
The mechanism below was chosen by the architect within that ruling; independent
DESIGN review is pending. Changes ADR-0101 D1's "Backend identity and address"
row, which is operative.

## Context

ADR-0101 D1 states that `ServiceLifecycle` advertises the allocation's
`workload_addr` "when present, otherwise the configured host IPv4". The code
spells this `row.workload_addr.unwrap_or(ctx.host_ipv4)` in the Service
allocation hydration. The fallback was copied from the retired
`BackendDiscoveryBridge` (ADR-0079 D8).

The premise is grounded in the feature delta: on the production path
(`run_server` and the `deploy` action shim) every allocation is assigned a guest
network plan before `Driver::start`, and the Running row copies that plan's
workload address. A Running row without a workload address is produced only by
the cfg-gated legacy exec/process provisioning branch that test compositions
select. The fallback therefore never fires in production; it masks a test
configuration, and in a real fault it would publish the node's own address as a
backend instead of surfacing the fault. The user ruled
(2026-11-06) that two Services never share a backend endpoint, that the fallback
is removed, and that skipping such an allocation is not an acceptable fix: an
allocation that can be a Service backend must be unable to lack the address.

Rows legitimately lack the address in other states (Pending, a failed start,
terminal rows written before any address was assigned), so the persisted
`AllocStatusRow.workload_addr` cannot be made required without inventing an
address for allocations that never had one.

## Decision

A Service backend identity can be constructed only from a Running allocation
row that carries a workload address. The Service allocation fact holds a
Running allocation's backend identity as part of its Running phase, so a
Running fact without an address does not exist; a Running row that lacks the
address is refused at hydration with a typed error naming the allocation, and
is neither skipped nor given a node address. The node's host IPv4 is no longer
an input to Service hydration and is removed from the hydration context.

The persisted allocation-status row is unchanged: no envelope version, no
persisted or wire format changes.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Make `AllocStatusRow.workload_addr` required through a new rkyv envelope version | Pending, failed-start and terminal rows have no address to record; a required field would force an invented value, which is the same masking in a different place. It also changes the persisted format and every one of the roughly 210 row construction sites for a defect that exists only on the backend path. |
| Keep the optional field and skip Running allocations that lack an address | Ruled out by the user. A silent skip hides a malformed Running row and leaves a Service with fewer backends and no signal. |
| Keep `Option` for the backend address inside the Service allocation fact and rely on the hydrator to fill it for Running rows | Leaves a Running fact with no address representable. The next constructor that forgets the rule reintroduces the defect. |
| Keep the fallback but log it | A fallback that cannot occur in production defends only a test configuration (the #248 lesson); the user ruled it out. |

## Consequences

- The `ServiceAllocFact` shape changes; the contract is in the feature delta.
  Fixtures that build facts or seed Running rows without an address change with
  it.
- A Running Service row without an address makes that Service's hydration fail
  with a typed error until the row is corrected. On the production path this
  state is unreachable, so the refusal has no production cost.
- Test compositions that tick a Service workload through the legacy exec
  provisioning forms no longer get a backend; they use the guest-network
  composition the production path uses.
- `HydrationContext` loses its `host_ipv4` field. `AppState.host_ipv4`,
  `ServiceMapHydrator` and the dataplane composer keep theirs; they have other
  consumers.

## Links

- [Feature delta: interface contracts, producers, fixtures, obligations](../../feature/no-host-ip-backend-fallback/feature-delta.md)
- [ADR-0101](adr-0101-service-backend-health-observed-convergence.md) (D1 changed), [ADR-0079](adr-0079-backend-discovery-bridge-converges-on-the-rows-it-manages.md) (D8 fallback origin; the bridge is retired)
