# ADR-0129 — Keep Gateway Application and Service Dataplane generations independent

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

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
  `NoBackend` result rather than userspace fallback.
- Status reports Gateway Application and BPF hydration separately.
- Receipt correlation is per connect and does not become a persisted unified
  generation or backend cache.

## Links

- [ADR-0116](adr-0116-public-ingress-route-and-gateway-application-owner.md)
- [ADR-0134](adr-0134-gateway-connect-selected-backend-receipt.md)
- [ADR-0135](adr-0135-commit-gated-backend-identity-publication.md)
- [ADR-0121](adr-0121-redacted-operator-gateway-status.md)
- [Application status contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
