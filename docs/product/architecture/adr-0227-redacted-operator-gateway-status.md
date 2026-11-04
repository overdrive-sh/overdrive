# ADR-0227 — Report gateway lifecycle through a separate redacted operator status projection

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

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
  the public listener, and without any observation read.
- The identity field is a pure projection of the held facts and the clock; the
  issuance-failure occurrence record is ADR-0274.
- Observation read failure is explicit; status does not guess from absence.
- Status writers remain their existing owners and no CQRS/event-history system
  is introduced.
- This projection adds no public data-plane response state and changes no
  Allocation, Service Stable, Backend Eligibility or BPF Hydrated meaning.

## Links

- [Status rows, DTOs and caller contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Status persistence boundary](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Canonical C4 operator status path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0221](adr-0221-singleton-public-route-set-aggregate.md), [ADR-0223](adr-0223-producer-neutral-public-certified-key-custody.md), [ADR-0225](adr-0225-dedicated-gateway-svid-identity-lifecycle.md)
- [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md)
- [ADR-0230](adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md)
