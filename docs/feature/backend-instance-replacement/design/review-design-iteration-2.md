# DESIGN Wave Review — backend-instance-replacement

**Reviewer**: Codex, applying `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-29  
**Iteration**: 2  
**Verdict**: **REJECTED PENDING REVISIONS — the restart cardinality contract does not match the generation-gate state machine**

**Scope**: `docs/feature/backend-instance-replacement/design/wave-decisions.md`, `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md`, `docs/product/architecture/brief.md`, `docs/product/architecture/c4-diagrams.md`, `docs/feature/backend-instance-replacement/feature-delta.md`, and the slice briefs under `docs/feature/backend-instance-replacement/slices/`.

## Findings

### Critical: the design says each restart call yields a fresh instance, but the state machine coalesces multiple bumps into one placement

**Dimension**: Implementation feasibility / Reliability / ADR quality  
**Location**: `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:194-198`, `:337-343`, `:470-473`, `:496-518`, `:559-562`; `docs/feature/backend-instance-replacement/design/wave-decisions.md:20-23`, `:52-54`; `docs/feature/backend-instance-replacement/feature-delta.md:646-648`, `:710-718`

ADR-0073 pins `restart` as non-idempotent: every call bumps generation and "two `restart` calls produce two fresh instances." The post-review `TxnOp::IncrementU64` fix correctly prevents a lost bump, so concurrent restarts can leave `desired.generation == 2`.

The reconciler contract then consumes that level as a single pending replacement. The R3/R4 table and the prose say to stamp `observed_generation = desired.generation` on the placement tick. If two restarts arrive before the reconciler places, the reconciler sees `observed_generation = 0`, `desired.generation = 2`, emits one `StartAllocation`, and stamps `observed_generation = 2`. That produces one fresh allocation, not two. The second bump is not lost in storage, but it is collapsed by the state machine.

This also affects stopped-origin restarts: two concurrent `overdrive workload restart payments` calls against a stopped workload produce one `payments-1` placement and then `observed == desired`. For running-origin restarts, the same collapse happens after the stop-then-start cycle if both bumps arrive before placement. That contradicts the non-idempotent operator contract and the required concurrency test semantics.

**Recommendation**: choose one contract and make all artifacts consistent.

Option A: keep non-idempotent "one call = one fresh instance." Change the reconciler contract to consume exactly one generation per placement, e.g. stamp `observed_generation = view.observed_generation.saturating_add(1)` or otherwise record the placed generation value, then re-enter the state machine while `observed_generation < desired.generation`. Add state-machine acceptance for two pre-placement restarts yielding two sequential fresh allocations.

Option B: make restart level-triggered/coalescing. Keep `observed_generation = desired.generation`, but update ADR-0073, feature-delta, wave-decisions, slices, and tests to say concurrent or pre-placement restarts coalesce to one replacement for the latest desired generation. This aligns more closely with the Kubernetes research's level-triggered Finding 7, but changes the current operator-visible idempotency posture.

### High: the ADR index row still records the rejected pre-review atomicity design

**Dimension**: Cross-artifact consistency / Handoff clarity  
**Location**: `docs/product/architecture/brief.md:1734`, compared with `docs/product/architecture/brief.md:6310-6325`, `:6370`; `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:289-343`, `:564-572`

The detailed brief section, changelog, ADR, C4 diagrams, feature-delta, and wave-decisions all say the atomic bump uses the new `TxnOp::IncrementU64` primitive with no `Conflict` retry. The ADR index row still says "existing surface — `TxnOp::Put` + `TxnOp::Delete`, retry on `Conflict`, monotonic `saturating_add`" and lists only three minimal CREATE-NEW surfaces, omitting `TxnOp::IncrementU64`.

This resurrects the exact critical issue that iteration 1 rejected and creates an SSOT conflict in the architecture brief's ADR table.

**Recommendation**: update the ADR-0073 row in `brief.md` to match the accepted post-review design: `TxnOp::IncrementU64 + Delete`, no `Conflict` retry, and four minimal CREATE-NEW surfaces including the store primitive.

### Low: review history is stale unless iteration 2 is the current handoff

**Dimension**: Documentation hygiene  
**Location**: `docs/feature/backend-instance-replacement/design/review-design.md:6`, `:62-70`; this file

`review-design.md` still says the design is rejected for the original atomicity blocker. That is useful history, but it is no longer the current review state after the ADR was revised. Without an iteration-2 review artifact or a pointer from the original review, implementers may not know which verdict is current.

**Recommendation**: keep the old review as iteration 1 history, and make this iteration-2 review the current handoff. If the design is revised again, add iteration 3 or update an index/pointer in `design/wave-decisions.md`.

## Checks Passed

- **Original atomicity blocker**: PASS in the detailed design. `TxnOp::IncrementU64` fixes the lost-bump / backwards-wedge problem at the store boundary, provided DELIVER implements the trait contract and concurrency test.
- **Outcome contract**: PASS. `RestartOutcome` is pinned to `Restarted` / `Resumed` and classified from the `/stop` lookup in the existence read.
- **Delivery-facing handoff**: PASS for the slice briefs. The slices now point to ADR-0073 and the concrete `overdrive workload restart <id>` / `POST /v1/jobs/:id/restart` surface.
- **Architectural fit**: PASS. The design still extends the existing hexagonal/reconciler topology and avoids new crates, dependencies, or external integrations.

## Approval Status

`rejected_pending_revisions`

Critical issues: 1  
High issues: 1  
Medium issues: 0  
Low issues: 1

The design should not proceed to DISTILL/DELIVER until the restart cardinality contract is made coherent. The smallest revision is either to consume generation one step at a time, or to explicitly adopt coalescing restart semantics and update the operator contract accordingly.
