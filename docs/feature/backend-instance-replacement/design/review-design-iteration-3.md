# DESIGN Wave Review — backend-instance-replacement

**Reviewer**: Codex, applying `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-30  
**Iteration**: 3  
**Verdict**: **REJECTED PENDING REVISIONS — superseded Operator-stop rows re-arm the stale veto after restart placement**

**Scope**: `docs/feature/backend-instance-replacement/design/wave-decisions.md`, `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md`, `docs/product/architecture/brief.md`, `docs/product/architecture/c4-diagrams.md`, `docs/feature/backend-instance-replacement/feature-delta.md`, and the slice briefs under `docs/feature/backend-instance-replacement/slices/`.

## Findings

### Critical: the generation gate only overrides old Operator-stop rows while restart is pending, so those rows can veto future convergence after the fresh placement

**Dimension**: Implementation feasibility / Reliability / ADR quality  
**Location**: `docs/product/architecture/adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md:76-87`, `:543-590`; `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:477-520`, `:593-620`

ADR-0073 correctly identifies that an observed `Terminated { by: Operator }` row currently implies a stop sentinel once existed, and warns that DELIVER must not introduce a no-sentinel Operator-stop path without revisiting the gate. The running-origin restart design then introduces exactly that path: `restart` deletes or no-ops `/stop`, bumps generation, and the reconciler emits `StopAllocation { terminal: Stopped { by: Operator } }` for the current Running allocation before placing the fresh one.

The deeper problem is not only running-origin restart. Both origins leave a superseded Operator-stopped row behind:

- stopped-origin: `payments-0` was already `Terminated/Operator`; R4 places `payments-1` and stamps `observed_generation = desired.generation`.
- running-origin: R2 creates `payments-0 Terminated/Operator`; R3 places `payments-1` and stamps `observed_generation = desired.generation`.

After that stamp, `restart_pending` is false. ADR-0073 says the veto "re-arms" after placement, but the proposed predicate is only `!restart_pending && any(is_operator_stopped)`. It does not distinguish a current operator stop from a superseded pre-restart operator row. In the live reconciler, `is_operator_stopped` scans all allocation rows before crash-restart/backoff handling, while `active_allocs_vec` merely filters intentional stops out of running/restartable selection. If the fresh `payments-1` later fails or terminates, the old `payments-0 Terminated/Operator` row can make line 520 return no actions before the failed fresh alloc reaches the restart path. That regresses J-OPS-003 after the first successful restart: the platform can again refuse to converge a declared workload because of an obsolete terminal row.

This also weakens the Bug-3 preservation argument. The design preserves "same-spec deploy does not resurrect a stopped workload" by re-arming the veto, but it re-arms it globally over every historical Operator-stop row. The needed contract is narrower: a restart must permanently supersede the specific prior Operator-stop rows it consumed, while a later explicit operator stop of the new instance must still suspend the workload.

**Recommendation**: revise ADR-0073 before DISTILL/DELIVER to make the veto generation-aware or otherwise scoped to non-superseded Operator-stop rows. Acceptable shapes include:

- record consumed Operator-stop allocation ids in `WorkloadLifecycleView` when R3/R4 places, and have the line-520 veto ignore those ids;
- add a placement/restart generation marker that lets the reconciler tell whether an Operator-stop row belongs to the current generation or an older consumed generation;
- change the stop/restart transition model so R2 does not create a bare no-sentinel Operator-stop row, while still preserving the user-visible terminal reason and Bug-3 semantics.

Whichever option is chosen, add state-machine acceptance for: stopped-origin restart → fresh placement → fresh alloc fails → restart/backoff still runs despite the old Operator-stopped row; running-origin restart → R2/R3 fresh placement → fresh alloc fails → restart/backoff still runs; later `overdrive job stop` of the fresh alloc still suspends and same-spec `deploy` still does not resurrect it.

## Checks Passed

- **Iteration-1 atomicity blocker**: PASS. The current detailed design consistently uses `TxnOp::IncrementU64` inside `IntentStore::txn`, with no reachable `Conflict` retry path.
- **Iteration-2 cardinality blocker**: PASS. The ADR, feature delta, wave decisions, and slice-01 now consistently describe level-triggered coalescing for concurrent / pre-placement restarts and sequential cycling for post-placement restarts.
- **Delivery-facing verb/API handoff**: PASS. The slices now point to `overdrive workload restart <id>` and `POST /v1/jobs/:id/restart` rather than leaving `[D1]` open.
- **Reuse / dependency posture**: PASS. The design remains additive to the existing CLI, HTTP, IntentStore, and `WorkloadLifecycle` surfaces; no new crate, dependency, or external integration is introduced.

## Approval Status

`rejected_pending_revisions`

Critical issues: 1  
High issues: 0  
Medium issues: 0  
Low issues: 0

The design should not proceed to DISTILL/DELIVER until old Operator-stop rows consumed by a restart can no longer veto future convergence of the fresh instance. The iteration-2 coalescing and atomicity revisions are otherwise coherent.
