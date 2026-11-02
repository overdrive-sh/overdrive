# ADR-0241 — Keep applied BackendId identity association in Service Dataplane

## Status

Proposed — pending independent review (2026-11-02). Ratified by the user
2026-09-13; decisions D1–D5 approved 2026-11-01 (see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).
User ruling 2026-11-02: P-2 (R-3), the backend-ID allocator is unchanged.

## Context

A BPF Selection Receipt names `BackendId`, while gateway-client mTLS needs the
exact workload `SpiffeId` from the `Backend.alloc` value applied for that
selection. Current backend rows or address indexes may advance independently
after the socket's selection, so they cannot authoritatively interpret the
receipt. On main `EbpfDataplane::update_service` assigns `BackendId`s through a
`BackendIdAllocator` that memoizes by `(ip, port, proto)`, so the association
must be owned where that assignment happens.

## Decision

The existing **Service Dataplane** context owns the **Applied Backend Identity
Association** from `BackendId` to the exact applied `Backend.alloc` and owns the
resulting per-connect **Selected Backend Identity** read. Public Ingress may ask
to resolve the receipt's id but cannot publish, reconstruct or cache the
association.

Only an identity belonging to an applied selectable Dataplane generation is
readable. While reachable by a receipt, one BackendId never denotes a different
`Backend.alloc`: an application that would associate a live id with another
allocation is rejected before the outer-map swap and the prior applied
generation is preserved. Reserved/commit/Applied transaction mechanics belong to
ADR-0233.

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
| Change the allocator: key the memo by the owning `Backend.alloc`, or release the memo of backends leaving a Service before allocating the new set | Either removes the re-lease wedge. The first is an allocator signature change; the second reorders `update_service` and needs the prior per-id allocation to know which memos to drop early. Neither is justified while no production path re-leases a guest address with a live memo. Rejected by ruling P-2 (R-3). |

## Consequences

- BPF receipt interpretation and applied backend identity remain in the context
  that owns backend application.
- Gateway-client mTLS receives exact selected identity without a userspace
  backend selector or racy observer reread.
- Receipt correlation can evolve independently under ADR-0228.
- A re-leased guest address held by the id memo is rejected rather than
  re-associated. `BackendIdAllocator` is unchanged (user ruling P-2 / R-3,
  2026-11-02): its memo stays keyed by `(ip, port, proto)`, and neither an
  allocation-aware key nor a release-before-allocate reorder is adopted. The
  exposure this leaves is recorded, not defended: a Service whose backend
  address was re-leased to another allocation while the old memo entry is live
  cannot hydrate until the id is released. No production path that re-leases a
  guest address while its memo is live has been shown.

## Links

- [Domain identity event model](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-event-model-and-lifecycle-gates)
- [Identity-publication contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md), [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md), [ADR-0228](adr-0228-dataplane-selection-receipt-ownership.md), [ADR-0229](adr-0229-keep-gateway-and-service-dataplane-generations-independent.md)
- [Route/domain context map](c4-diagrams.md#public-ingress-gateway-route-domain-context)
