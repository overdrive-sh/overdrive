# DESIGN Wave Review — backend-instance-replacement

**Reviewer**: Codex, applying `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-29  
**Iteration**: 1  
**Verdict**: **REJECTED PENDING REVISIONS — one correctness blocker in the generation bump contract**

**Scope**: `docs/feature/backend-instance-replacement/design/wave-decisions.md`, `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md`, `docs/product/architecture/brief.md` and `c4-diagrams.md` diffs, `docs/feature/backend-instance-replacement/feature-delta.md` DESIGN append, and the slice briefs under `docs/feature/backend-instance-replacement/slices/`.

## Findings

### Critical: the non-idempotent restart contract relies on a transaction conflict that the store API cannot produce

**Dimension**: Implementation feasibility / Reliability / Testability  
**Location**: `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:157-161`, `:252-268`, `:367-369`, `:450-454`; `crates/overdrive-core/src/traits/intent_store.rs:63-71`, `:193-195`; `crates/overdrive-store-local/src/redb_backend.rs:302-358`

ADR-0073 makes `restart` explicitly non-idempotent: every call must bump generation and yield a fresh instance. The ADR says the handler reads the current generation, writes `g+1` with `TxnOp::Put`, deletes `/stop`, retries on `TxnOutcome::Conflict`, and therefore two concurrent restarts produce two fresh instances.

That conflict path is not available on the current store surface. `TxnOp` has only blind `Put` and `Delete`; there is no expected-current-value or compare operation. `LocalIntentStore::txn` applies all ops in one redb write transaction and returns `TxnOutcome::Committed` unconditionally after commit. Two concurrent handlers can both read generation `0`, both commit `Put generation=1`, and both return success. That loses one restart despite the non-idempotent contract.

**Recommendation**: revise the design before DISTILL/DELIVER. Either add an explicit compare-and-swap/precondition primitive to the store contract, or add a store-side atomic generation-bump operation that reads and writes inside the same write transaction. Then update ADR-0073, the six pinned signatures, C4 text, and tests to include the concurrent-restart case: two simultaneous `restart` calls must leave generation advanced by 2 and must not report two successes for one replacement generation.

### High: `RestartOutcome` is both pinned and deferred

**Dimension**: ADR quality / Cross-artifact consistency / Acceptance testability  
**Location**: `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:142-154`, `:195-209`, `:348-350`; `docs/feature/backend-instance-replacement/design/wave-decisions.md:45-50`, `:107-110`; `docs/feature/backend-instance-replacement/feature-delta.md:582-592`, `:666-670`

The ADR pins `RestartWorkloadResponse { workload_id, outcome }` and a two-variant `RestartOutcome` (`Restarted`, `Resumed`). It also says the handler classifies running vs stopped origin for the response label. But the DESIGN handoff later lists this as an open question: whether the handler reads observations to label `RestartOutcome` or returns a single outcome.

That leaves DISTILL unable to write stable executable acceptance for the API, and DELIVER could validly implement a single outcome while still claiming to follow the "open question" section, contradicting the accepted ADR.

**Recommendation**: make one decision. If the two variants stay, pin the classification source and race semantics in ADR-0073 (for example, stop-sentinel present before mutation means `Resumed`; running allocation observed before mutation means `Restarted`; define fallback precedence). If the distinction is cosmetic and not worth a read, collapse to one outcome and update ADR-0073, feature-delta, API shape, and OpenAPI expectations.

### High: delivery-facing artifacts still describe `[D1]` as DESIGN-open after the DESIGN append closes it

**Dimension**: Handoff clarity / Cross-artifact consistency  
**Location**: `docs/feature/backend-instance-replacement/feature-delta.md:150-190`, `:376-388`, `:419-431`, `:507`, `:523`, `:532`, `:575-593`; `docs/feature/backend-instance-replacement/slices/slice-01-replace-action-new-instance-intent-retained.md:5`, `:31-37`, `:53-56`; `docs/feature/backend-instance-replacement/slices/slice-02-stable-frontend-survives-cycle.md:27-31`; `docs/feature/backend-instance-replacement/slices/slice-03-in-flight-churn-fails-fast.md:26-29`

The DESIGN append correctly closes `[D1]` with `overdrive workload restart <id>`, `POST /v1/jobs/:id/restart`, and the generation precursor. But earlier feature-delta sections and slice briefs still tell implementers that the verb shape, API, sentinel mechanics, and reconciler edit are DESIGN-owned or DESIGN-open.

Because slices are the direct DELIVER handoff, this is more than historical noise. It can send the crafter back into API design or cause tests to keep using "replace action" placeholders instead of the concrete production verb.

**Recommendation**: preserve the old DISCUSS text only if clearly marked historical/superseded. Update delivery-facing sections and slice briefs to point to ADR-0073 and the concrete command/API/mechanism. Replace "DESIGN owns API/mechanism" with "implemented per ADR-0073" in the slice dependencies and behavior sections.

### Medium: running-origin sequencing is deferred to DELIVER despite being part of the architecture contract

**Dimension**: Decision quality / Implementation feasibility  
**Location**: `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:329-339`; `docs/feature/backend-instance-replacement/feature-delta.md:666-668`

The ADR states the model for running-origin replacement (`restart_pending + Running => StopAllocation`; then place fresh once the prior is Terminated), but it defers exact tick ordering to DELIVER. This is implementable, but the sequencing is load-bearing: repeated evaluations while the old allocation is still Running must not emit unsafe duplicate stop actions, and `observed_generation` must not be stamped until the fresh placement actually happens.

**Recommendation**: add a short state-transition table to ADR-0073 for `restart_pending` across Running, Terminated/Operator, no active alloc, and placement-emitted states. Include the expected action and view update for each row. That is enough for DISTILL to write focused state-machine tests without reverse-engineering the intended tick behavior.

## Checks Passed

- **Architectural fit**: PASS. The design extends the existing hexagonal/reconciler topology and avoids new crates, new dependencies, or a new reconciler.
- **Reuse analysis**: PASS with one correction dependency. The EXTEND/CREATE NEW split is mostly justified; if the generation bump requires a new CAS/bump store primitive, the reuse analysis must be updated honestly.
- **Alternatives analysis**: PASS. The ADR gives credible rejection rationale for narrow veto edit, observation-row relabelling, and full #180 revision lineage pull-forward.
- **Observation honesty**: PASS. Rejecting observation relabelling preserves the intent/observation boundary.
- **Forward compatibility**: PASS in concept. The thin `generation` / `observed_generation` seam is a reasonable precursor for #180/#64/#253/#254, once the atomic bump primitive is corrected.

## Approval Status

`rejected_pending_revisions`

Critical issues: 1  
High issues: 2  
Medium issues: 1

The architecture should not proceed to DISTILL/DELIVER until the generation bump is made atomic against concurrent restarts and the API outcome contract is made singular. After those revisions, the remaining work is handoff cleanup and a small sequencing clarification.
