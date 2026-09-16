# ADR-0143 — Keep applied BackendId identity association in Service Dataplane

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

A BPF Selection Receipt names `BackendId`, while gateway-client mTLS needs the
exact workload `SpiffeId` from the `Backend.alloc` value applied for that
selection. Current backend rows or address indexes may advance independently
after the socket's selection, so they cannot authoritatively interpret the
receipt.

## Decision

The existing **Service Dataplane** context owns the **Applied Backend Identity
Association** from `BackendId` to the exact applied `Backend.alloc` and owns the
resulting per-connect **Selected Backend Identity** read. Public Ingress may ask
to resolve the receipt's id but cannot publish, reconstruct or cache the
association.

Only an identity belonging to an applied selectable Dataplane generation is
readable. While reachable by a receipt, one BackendId never denotes a different
`Backend.alloc`. Exact Reserved/commit/Applied transaction mechanics belong to
ADR-0135 and the feature-delta Application contract, not this Domain ADR.

## Lifecycle Gate Ownership

**Selected Backend Identity Resolved** is owned by Service Dataplane for the
current selected receipt. It advances only from a readable Applied association;
absence or inconsistency fails that connect closed before gateway-client mTLS.
It does not change Gateway Applied Generation, BPF Hydrated, Backend Eligibility,
Allocation Running, Service Stable, future requests or the receipt owner.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Reconstruct identity in the gateway from `ServiceBackendRow` | The row can differ from the selected generation and creates a second backend-data consumer. |
| Resolve by connected address through `BackendIndex` | Address reuse and asynchronous index updates cannot prove the identity selected for this receipt. |
| Carry full `SpiffeId` directly in the BPF receipt | Viable, but couples transient selection correlation to identity representation and duplicates the Dataplane's applied Backend value. |

## Consequences

- BPF receipt interpretation and applied backend identity remain in the context
  that owns backend application.
- Gateway-client mTLS receives exact selected identity without a userspace
  backend selector or racy observer reread.
- Receipt correlation can evolve independently under ADR-0124.

## Links

- [Domain identity event model](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Exact Application identity-publication contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Application identity-publication decision](adr-0135-commit-gated-backend-identity-publication.md)
- [Application exact-peer decision](adr-0136-exact-peer-gateway-client-mtls.md)
- [Domain receipt decision](adr-0124-dataplane-selection-receipt-ownership.md)
- [System generation decision](adr-0129-keep-gateway-and-service-dataplane-generations-independent.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
