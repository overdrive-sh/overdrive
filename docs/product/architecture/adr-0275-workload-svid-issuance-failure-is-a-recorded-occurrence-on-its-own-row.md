# ADR-0275 — A failed workload SVID issuance is a recorded occurrence on its own per-allocation row

## Status

Proposed — pending independent review. User decision 2026-11-04 (R-46); provenance of the related rulings is in the feature delta. Extends the
operative executor of [ADR-0067](adr-0067-workload-identity-manager-svid-lifecycle.md) D3 with a new observation row; it changes no decision of
ADR-0067 and no existing persisted envelope.

## Context

`action_shim::issue_svid::dispatch_issue` returns the `CaIssuanceError` (`Ca` or `Audit`) to the convergence
loop, whose only sink is a tracing event. An allocation whose SVID never issues reads the same as one that has
just started, and `overdrive workload describe` shows nothing. The appliance OS has no operator shell. The
per-allocation row the CLI reads, `AllocStatusRow`, has one authoring route, `write_alloc_lifecycle`, whose
transaction rejects an update that does not dominate the stored `updated_at`
(`crates/overdrive-store-local/src/observation_backend.rs:517-525`), so a second writer of that row would have
its writes dropped or would drop the lifecycle writer's. The reconciler's retry memory (`IssueRetry`) is
stamped when the action is emitted, not when the failure is confirmed.

## Decision

Introduce one observation row, `WorkloadSvidIssueStatusRowV1`, keyed by `AllocationId`, holding a monotone
count of failed issuances and a depth-1 snapshot of the latest failure (cause class and instant). Its only
writer is the `IssueSvid` executor, through a single atomic `ObservationStore` operation that reads the prior
row, increments the count and installs the row, called with the executor's confirmed `Err` after
`issue_and_audit` returns. A success never clears it. `GET /v1/allocs` joins it onto each allocation row and
`overdrive workload describe` renders it. `AllocStatusRow` and its envelope are unchanged.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Add the count and last failure to `AllocStatusRow` (a `V2` envelope bump) | Makes the executor a second writer of a row whose sole compound writer owns lifecycle state, under the dominance rule cited above; it also forces a bump of an operative envelope for a fact that is not a lifecycle transition. |
| Append the failure to the lifecycle occurrence ring (`AllocLifecycleOccurrenceRow`) | Those occurrences are state transitions of the allocation row; a failed issuance changes no allocation state. |
| The executor reads the prior row and writes through the generic `write` | The read and the write are not atomic; two failures for one allocation could lose a count. |
| Stamp the surface from the reconciler View on emit | The marker outlives the effect it claims to record (`reconcilers.md`); an emit is not a confirmed failure. |
| Keep tracing only | Not an operator surface on an appliance OS with no shell. |

## Consequences

- An operator sees the count, cause class and instant per allocation; error text never leaves the node.
- Two new `ObservationStore` operations (the atomic record and a point read), implemented by the Local and Sim
  adapters and covered by an equivalence test. The row is read by point read only; it is not emitted to
  subscribers because nothing consumes it that way.
- A failure whose recording also fails is not absorbed: the executor surfaces both failures, and the failure
  is not counted. The recording shares the `ObservationStore` with the audit write, so an outage of every
  store write loses the count while it lasts.
- The trust-bundle refresh failure after a successful issuance is a different occurrence and is not recorded
  here. It is the open, disjoint issue #223 (a standing-state row); this decision neither covers nor
  constrains it.
- `IssueRetry` stays stamped on emit; deriving it from this row is the PROPOSED P-40 in the feature delta.

## Links

- [SVID issuance failure occurrence](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-svid-issuance-failure-occurrence) (contracts, rulings and their provenance)
- [ADR-0067](adr-0067-workload-identity-manager-svid-lifecycle.md), [ADR-0048](adr-0048-rkyv-versioned-envelope.md), [ADR-0078](adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md)
- [ADR-0274](adr-0274-gateway-svid-issuance-failure-is-a-recorded-occurrence-on-the-application-status-row.md) (the gateway counterpart)
