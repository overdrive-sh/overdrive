# ADR-0347 — The dataplane association carries the workload identity and follows the workload programmed at the endpoint

## Status

Accepted — approved by the user 2026-11-21 (D4 and DR-1, option b). Supersedes
[ADR-0241](adr-0241-dataplane-applied-backend-identity-association.md) (per-allocation
association; rejection of a different identity at a memoised endpoint). ADR-0241 keeps its
text; its ownership, commit-gating and Lifecycle Gate Ownership statements are carried
forward here unchanged.

## Context

ADR-0241 gave the Service Dataplane the Applied Backend Identity Association from
`BackendId` to the applied `Backend` identity, and kept it fixed by rejecting any update
that would re-associate a live id with another identity. With per-allocation identities, a
replacement allocation at a reused guest address *was* "another identity", so the rule
turned the normal replacement into a terminal error that the hydrator could only retry
(RCA RC-1; the serve-restart path is where it surfaces,
[RCA](../../analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md)).

With a workload-level identity
([ADR-0344](adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md)) `Backend`
carries the identity of its workload, so a replacement of the same workload at the same
endpoint is the same `Backend`. A different workload at an endpoint the dataplane still
memoises is also reachable: the guest address pool hands a freed address to any later
allocation, and the memo entry of a departed backend outlives it until the orphan sweep.
Refusing that update reproduces the RCA wedge for a different workload.

## Decision

The Service Dataplane owns the Applied Backend Identity Association from `BackendId` to the
**workload identity** of the backend programmed at the id's endpoint, and the per-connect
Selected Backend Identity read that returns it.

An endpoint has one `BackendId`; the id's association is the workload identity currently
programmed there.

- The same workload at an endpoint the dataplane already programmed reuses that endpoint's
  `BackendId` and changes no association.
- A different workload at a memoised endpoint keeps the endpoint's `BackendId`, which is
  re-associated with the new workload identity inside the commit-guarded transaction of
  [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md): the new association is
  readable only at publication, a pre-commit failure leaves the prior association, and a
  reader waits on the commit guard and never sees a reserved or half-applied value.
- `update_service` has no identity-related outcome: the error that reported an identity
  conflict is removed. An endpoint whose reverse-NAT key another Service's VIP owns is still
  refused (that key is the endpoint and does not depend on identity).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep per-allocation association and mint a fresh `BackendId` per replacement | Repairs one symptom, keeps the per-instance identity, and burns an id and an orphan sweep per replacement. |
| Keep the rejection; add a hydrator withdraw-then-apply on conflict, or release the memo when an allocation terminates | The first opens a no-backend window on every replacement and encodes dataplane internals in a reconciler; the second depends on a `ServiceLifecycle` evaluation landing between termination and replacement (RCA RC-2), which is not guaranteed. |
| Refuse the update for a *different* workload only | Wedges the hydrator for as long as the memo entry lives. |
| Key the memo by (endpoint, workload), or evict the entry, and mint a fresh `BackendId` for the different workload | Keeps the earlier id's association for receipts already taken, but gives two live ids per endpoint until the sweep and one consumed id per takeover; the user ruled for one id per endpoint whose identity follows the workload programmed there. |
| Drop the association and carry the identity in the BPF receipt | Couples selection correlation to identity representation (already rejected by ADR-0241). |

## Consequences

- A replacement of the same workload at the same endpoint is a no-op to the dataplane: the
  backend set fingerprint, `BackendId` and association are unchanged.
- `ServiceMapHydrator` cannot wedge on a replacement or a takeover, whatever the order of
  `ServiceLifecycle` and `WorkloadLifecycle` evaluations.
- A connection whose receipt was taken while the id meant workload A, and whose identity is
  read after the re-association to workload B, is verified against workload B. The gateway
  accepts, for that connection, the workload now programmed at the endpoint, not the one the
  receipt was taken for. The read waits on the commit guard, so it resolves the prior or the
  re-associated value, never a mixed one.
- The association of an id is no longer immutable. The property that holds instead: whenever
  the guard is free, the association of every id equals the workload most recently
  committed for it.
- `DataplaneError::BackendIdentityConflict` has no producer and is deleted (single cut).

## Links

- [feature-delta](../../feature/workload-spiffe-identity/feature-delta.md) (interface contract, Lifecycle Gate Ownership, Formal protocol model)
- [Quint model](../../../specs/quint/public-ingress-gateway/README.md)
- [ADR-0233](adr-0233-commit-gated-backend-identity-publication.md), [ADR-0228](adr-0228-dataplane-selection-receipt-ownership.md), [ADR-0046](adr-0046-collision-free-backend-id-allocator.md), [ADR-0346](adr-0346-gateway-client-mtls-requires-the-selected-backends-workload-identity.md)
