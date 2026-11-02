# ADR-0222 — Use one owner for coherent Gateway Application admission

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13; its demand clause is retired by D1/D2 (user-approved 2026-11-01, see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

[ADR-0221](adr-0221-singleton-public-route-set-aggregate.md) owns the singleton
Public Route Set. Application runtime must turn its accepted Route plus
independently changing Service Frontend and Public Certified Key facts into
one coherent value for new connections. Splitting that authority among
handlers or resource-specific tasks permits torn admission. The gateway is
node infrastructure inside `overdrive serve`, not an Allocation or another
deployed process.

The ordinary `ServiceMapHydrator` path already programs VM backends into the
Service maps without any gateway input
([backend programming](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-backend-programming-and-selection-on-main-d1-d3)),
so the owner has no backend-programming role.

## Decision

Use one active, in-process `GatewayApplicationOwner` in the non-deployable
`overdrive-gateway` library. It alone publishes each whole
Route/resolved-frontend/authorized-certified-key admission generation, moves
the prior generation to Draining, retires it after its users reach zero, and
owns public-listener admission. It never waits for a backend: backend
availability is the typed outcome of the gateway's consuming port
([ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)).
Derived application state is rebuilt rather than persisted as a second source
of desired truth.

Certificate custody, BPF forwarding, internal identity, public protocol policy
and operator status are independently reversible decisions recorded elsewhere.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Let HTTP handlers resolve Route/key/frontend independently per request | It permits torn generations and gives request tasks desired-state ownership. |
| Give Route, custody and listener tasks independent publication authority | Their independently scheduled changes can expose a new Route with an old key/frontend. |
| Put the admission state machine directly in `ServerHandle` | It mixes per-generation Route/key/frontend convergence with the process-wide shutdown composition and gives no independently testable owner boundary. |

## Consequences

- One owner provides atomic admission, staged/current/draining retention and
  ordered withdrawal without redefining Service or Allocation state.
- The new crate adds a module boundary, not a process, daemon or datastore.
- Reversing this decision replaces/removes the derived admission owner while
  leaving the Public Route Set, Public Certified-Key Custody, Service dataplane
  and internal identity independently reusable.

## Links

- [Route/owner contracts](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Component ownership and lifecycle](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-application-component-decomposition)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0220](adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md)
- [ADR-0221](adr-0221-singleton-public-route-set-aggregate.md)
- [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md)
- [ADR-0230](adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md)
- [ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)
