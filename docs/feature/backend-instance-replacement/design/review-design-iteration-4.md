# DESIGN Wave Review — backend-instance-replacement

**Reviewer**: Codex, applying `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-30  
**Iteration**: 4  
**Verdict**: **CONDITIONALLY APPROVED — iteration-3 correctness blocker is resolved; one ADR-index handoff correction remains**

**Scope**: `docs/feature/backend-instance-replacement/design/wave-decisions.md`, `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md`, `docs/product/architecture/brief.md`, `docs/product/architecture/c4-diagrams.md`, `docs/feature/backend-instance-replacement/feature-delta.md`, and the slice briefs under `docs/feature/backend-instance-replacement/slices/`.

## Findings

### High: the ADR-0073 index row still summarizes the reconciler fix as generation-gating only, omitting the current-instance-scoped veto

**Dimension**: Cross-artifact consistency / Handoff clarity  
**Location**: `docs/product/architecture/brief.md:1734`, compared with `docs/product/architecture/brief.md:6319-6328`, `docs/product/architecture/c4-diagrams.md:1159`, `docs/product/architecture/c4-diagrams.md:1188-1189`, `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:549-579`, and `docs/feature/backend-instance-replacement/design/wave-decisions.md:44-45`

The detailed ADR, C4 diagrams, feature delta, wave decisions, and detailed brief section now correctly resolve the iteration-3 Critical: the operator-stop veto is `!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)`, where `current_alloc` selects the latest placed allocation by numeric `mint_alloc_id` suffix. That makes superseded prior-generation `Terminated{Operator}` rows unable to veto a fresh instance's later crash-restart.

The ADR index row in the architecture brief still says the `WorkloadLifecycle` reconciler "gates the stale line-520 operator-stop observation-veto on `observed_generation < generation`" and does not mention the current-instance scope or the `current_alloc` helper. Read literally, that is the iteration-2 shape the iteration-3 review rejected: a transient generation override that can re-arm stale historical Operator-stop rows after placement. Because the ADR table is a high-signal architecture index, this can send DELIVER back to the flawed predicate even though the long-form sections are correct.

**Recommendation**: update the ADR-0073 index row to match the accepted iteration-3 design. The row should say the veto is gated on restart pending **and scoped to the current instance**, e.g. `!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)`, and note that `current_alloc` is the new minimal pure helper using the numeric `mint_alloc_id` suffix. This is a documentation correction only; it does not reopen the ADR mechanism.

## Checks Passed

- **Iteration-1 atomicity blocker**: PASS. The current design consistently uses a store-side `TxnOp::IncrementU64` inside `IntentStore::txn`, with no reachable `Conflict` retry path and a concrete concurrency acceptance target.
- **Iteration-2 cardinality blocker**: PASS. The ADR, feature delta, wave decisions, C4 diagrams, and slice-01 consistently describe level-triggered coalescing for concurrent / pre-placement restarts and sequential cycling for post-placement restarts.
- **Iteration-3 stale-veto blocker**: PASS in the authoritative design. ADR-0073, `wave-decisions.md`, the detailed brief section, C4 diagrams, and the feature-delta all pin the current-instance-scoped veto, add the R1-crash case, and require a mutation-killing regression case for fresh-alloc crash after restart convergence across both stopped-origin and running-origin paths.
- **Delivery-facing verb/API handoff**: PASS. The slices drive `overdrive workload restart <id>` and `POST /v1/jobs/:id/restart`; `[D1]` is closed and historical DISCUSS-open text is bannered as superseded.
- **Architectural fit / reuse**: PASS. The design remains additive to the existing CLI, HTTP, IntentStore, and `WorkloadLifecycle` surfaces; there is no new crate, dependency, external integration, rkyv `AllocStatusRow` field, or ADR-0048 envelope bump.

## Approval Status

`conditionally_approved`

Critical issues: 0  
High issues: 1  
Medium issues: 0  
Low issues: 0

The design can proceed once the ADR index row in `brief.md` is corrected to include the current-instance-scoped veto. No remaining correctness blocker was found in the authoritative ADR/design handoff.
