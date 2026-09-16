# ADR-0116 — Use one owner for coherent Gateway Application admission

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

ADR-0115 already owns the singleton Public Route Set. Application runtime must
turn its accepted Route plus independently changing Service Frontend and Public
Certified Key facts into one coherent value for new connections. Splitting
that authority among handlers or resource-specific tasks permits torn
admission and early demand retirement. The gateway is node infrastructure
inside `overdrive serve`, not an Allocation or another deployed process.

## Decision

Use one active, in-process `GatewayApplicationOwner` in the non-deployable
`overdrive-gateway` library. It alone stages exact frontend demand, publishes
each whole Route/resolved-frontend/authorized-certified-key admission
generation, moves the prior generation to Draining, retires it after its users
reach zero, and owns public-listener admission. Derived application state is
rebuilt rather than persisted as a second source of desired truth.

This is one decision about the derived admission ownership boundary.
Certificate custody, BPF forwarding, internal identity, public protocol policy
and operator status are independently reversible decisions recorded elsewhere.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Let HTTP handlers resolve Route/key/frontend independently per request | It permits torn generations and gives request tasks desired-state ownership. |
| Give Route, demand and listener tasks independent publication authority | Their independently scheduled changes can expose a new Route with old key/frontend or retire demand still retained by an admitted connection. |
| Put the admission state machine directly in `ServerHandle` | It mixes per-generation Route/key/frontend convergence with the process-wide shutdown composition and gives no independently testable owner boundary. |

## Consequences

- One owner provides atomic admission, staged/current/draining retention and
  ordered withdrawal without redefining Service or Allocation state.
- The new crate adds a module boundary, not a process, daemon or datastore.
- Reversing this decision replaces/removes the derived admission owner while
  leaving the Public Route Set, Public Certified-Key Custody, Service dataplane
  and internal identity independently reusable.

## Links

- [Implementation-facing Route/owner contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Component ownership and lifecycle](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-application-component-decomposition)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0114](adr-0114-embed-single-node-public-ingress-in-overdrive-serve.md)
- [ADR-0115](adr-0115-singleton-public-route-set-aggregate.md)
- [System consistency ADR-0129](adr-0129-keep-gateway-and-service-dataplane-generations-independent.md)
- [System listener-gate ADR-0131](adr-0131-gate-public-listener-without-redefining-workload-service-readiness.md)
