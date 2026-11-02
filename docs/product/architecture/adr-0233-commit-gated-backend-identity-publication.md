# ADR-0233 — Publish backend identity only after Dataplane map commit

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

## Context

Applying one Service generation in `EbpfDataplane::update_service` performs
fallible `BACKEND_MAP`, inner-map and reverse-NAT work before the atomic outer
`SERVICE_MAP` swap. Publishing a BackendId-to-workload identity independently
could label an unselectable backend Applied or prevent safe BackendId slot reuse
after failure. The generation is applied by the ordinary `ServiceMapHydrator`
path; the gateway adds no application input (D1).

## Decision

Serialize each affected Service application behind one Dataplane commit guard.
Reserve the not-yet-readable BackendId identity association, stage the fallible
map changes, treat the atomic outer-map swap as the selection commit point, and
publish the corresponding immutable BackendId-to-applied-`Backend.alloc`
identity only after commit. Pre-commit failure restores the captured prior map
and reservation state before the guard releases.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Publish identity before fallible BPF work | Failure can expose identity as Applied while no committed generation can select it. |
| Reconstruct identity from current backend rows on receipt | Current observations can differ from the committed generation the socket encountered. |
| Create one unified Gateway/BPF generation | The owners, inputs and update cadence are independent, so the generation would claim false atomicity. |

## Consequences

- Multi-map kernel writes are not claimed globally atomic; only the external
  applied-identity publication is commit-guarded.
- Failed application cannot poison identity visibility or later slot reuse.
- Gateway Application and Service Dataplane generations remain independently
  owned per ADR-0229.

## Links

- [Transaction contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 Dataplane owner](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0241](adr-0241-dataplane-applied-backend-identity-association.md), [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md)
- ADR-0040 and ADR-0042
