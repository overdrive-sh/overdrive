# ADR-0229 — Keep Gateway Application and Service Dataplane generations independent

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

## Context

Gateway Application atomically publishes Route, exact Service Frontend and
Public Certified Key state. ServiceMapHydrator/Dataplane independently converge
eligible backends into BPF maps. Their inputs, update cadence and failure modes
are different, so one shared generation would claim atomicity the system cannot
provide.

## Decision

Keep Gateway Applied Generation and BPF Hydrated generation independently owned
and observable. A gateway request uses its retained Gateway Application
generation, while a transient selection receipt identifies the actual backend
chosen by the BPF generation encountered by that connect. Neither owner claims
the other is current.

This ADR decides only the cross-owner consistency model.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| One Route/certificate/backend generation | Requires an unavailable cross-owner transaction and makes a false atomicity claim. |
| Block every Route application on continuous BPF currency | Couples public admission to independent health/hydration convergence and still cannot freeze per-connect selection. |
| Expose no generation distinction | Makes operator status unable to explain a coherent Route with temporarily empty or stale backend hydration. |

## Consequences

- A coherent current Route may race backend convergence and receive the typed
  `NoBackend` result ([ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)) rather than userspace fallback.
- Status reports Gateway Application and BPF hydration separately.
- Receipt correlation is per connect and does not become a persisted unified
  generation or backend cache.

## Links

- [ADR-0222](adr-0222-public-ingress-route-and-gateway-application-owner.md)
- [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md)
- [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md)
- [ADR-0227](adr-0227-redacted-operator-gateway-status.md)
- [Status contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
