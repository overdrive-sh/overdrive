# ADR-0111 — Report gateway lifecycle through a separate redacted operator status projection

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Public HTTP response status describes one data-plane request. Operators need a
different view of Route/application, certified-key, identity, listener,
connect-path and independent BPF hydration state. Reusing a public response or
persisting per-request receipts would conflate lifecycle owners and risk
exposing backend or credential details.

## Decision

Expose one redacted `GatewayStatusResponse` from operator-mTLS
`GET /v1/gateway/status`, built from ID-scoped application/certified-key LWW
status rows plus the relevant existing Service hydration rows. Keep Gateway
Application, Public Certified-Key Custody and BPF hydration generations
separate in the projection. Per-connection receipts, selected BackendId,
private material, paths and raw adapter errors never enter status.

This is one decision about the operator observation boundary. It can evolve or
be replaced without changing public TLS/HTTP request semantics.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Infer lifecycle from public 404/502/503 responses | Those are request outcomes, not authoritative owner/generation state. |
| Fold BPF and Gateway Application into one generation/status | The owners converge independently and a unified generation would make a false atomicity claim. |
| Persist per-connect receipts and raw failures for diagnostics | It creates unbounded operational history and leaks transient/backend implementation detail. |

## Consequences

- Disabled gateway status remains readable without activating Route/custody or
  the public listener.
- Observation read failure is explicit; status does not guess from absence.
- Status writers remain their existing owners and no CQRS/event-history system
  is introduced.
- This projection adds no public data-plane response state and changes no
  Allocation, Service Stable, Backend Eligibility or BPF Hydrated meaning.

## Links

- [Exact status rows, DTOs and caller contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Status persistence boundary](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Canonical C4 operator status path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0112](adr-0112-public-certified-key-custody-domain-boundary.md),
  [ADR-0113](adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md),
  [ADR-0115](adr-0115-derived-gateway-frontend-demand-lifecycle.md), and
  [ADR-0116](adr-0116-state-based-public-ingress-domain-ownership.md)
- [Route ADR-0105](adr-0105-singleton-public-route-set-aggregate.md)
- [System consistency ADR-0119](adr-0119-keep-gateway-and-service-dataplane-generations-independent.md)
- [System listener-gate ADR-0121](adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md)
