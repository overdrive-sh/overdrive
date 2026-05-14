# workload-gc-absent-stale-allocs — Workload GC for absent-intent stale allocations

**Date:** 2026-05-14
**Issue:** [#148](https://github.com/overdrive-sh/overdrive/issues/148)
**Branch:** `marcus-sa/fix-issue-148-rename`
**Status:** Implemented

## Summary

Closed the GC gap in `WorkloadLifecycle::reconcile`'s `None` arm at
`crates/overdrive-core/src/reconciler.rs`. When a `Job` intent disappears
(hard-delete, multi-node drain, crash recovery surgery) while non-terminal
`AllocStatusRow`s remain, the reconciler now emits one
`Action::StopAllocation { terminal: Some(TerminalCondition::Stopped { by:
StoppedBy::SystemGc }) }` per Running orphan row. The new `StoppedBy::SystemGc`
variant (ADR-0037 Amendment 2026-05-14, appended at rkyv discriminant index 3)
gives operators a distinct terminal class to audit against.

## Business context

GH issue #148 — "WorkloadLifecycle reconciler: cleanup stale allocations when
desired Job is absent". Single SSOT (no DISCUSS wave). Three triggering
scenarios named in the issue: hard-delete, multi-node drain, crash recovery.
Acceptance criteria: convergence to terminal, distinct `TerminalCondition`,
DST scenario.

## Key decisions (D-1 .. D-7)

- **D-1: Extend `WorkloadLifecycle::reconcile` in place (Option A).** Sibling
  `WorkloadGC` reconciler rejected — fails F-1 default of EXTEND over CREATE
  NEW; would require new `AnyReconciler`/hydrator/registration story.
- **D-2: `StoppedBy::SystemGc` as a new variant on the existing enum.** New
  top-level `TerminalCondition::Withdrawn` variant rejected (inflates
  hierarchy for a sub-classification). Reusing `StoppedBy::Reconciler`
  rejected (collapses two distinct semantics).
- **D-3: Append at discriminant index 3 (after `Process`).** The "Process MUST
  remain last" comment is a discriminant-pin claim, not aesthetic — renumbering
  `Process` IS the breaking change the comment forbids.
- **D-4: Amend ADR-0037, no new ADR.** Falls inside ADR-0037's documented
  SemVer-additive amendment scope.
- **D-5: No grace window — immediate stop on first orphan tick.** Premature
  complexity; #148's AC does not require it.
- **D-6: Workload-kind agnostic GC.** Withdrawal of intent is a system action
  distinct from any kind-specific natural exit.
- **D-7: Two DST scenarios with three invariants.** `gc.converges`,
  `gc.terminal_claim`, `gc.no_fresh_alloc` (orphan convergence);
  `resubmit.places_fresh`, `resubmit.preserves_prior_gc_terminal` (race shape).

## Steps completed (5)

| Step | Title | Outcome |
|---|---|---|
| 01-01 | Add `StoppedBy::SystemGc` variant + schema-evolution fixture pin | Variant appended at index 3; existing `FIXTURE_V1` untouched; new forward-roundtrip test added. |
| 01-02 | Reconcile None-arm emits `StopAllocation { terminal: SystemGc }` per orphan Running row | Mirror of operator-stop branch; kind-parametrised tests (a)-(d). |
| 01-03 | DST scenarios for orphan convergence and resubmit race | `orphan_workload_converges_to_terminal_gc` GREEN; `resubmit_after_gc_creates_fresh_alloc` landed RED — surfaced production gap (see scope expansion below). |
| 01-04 | Close resubmit-after-SystemGc gap; promote `WorkloadGcResubmitCreatesFresh` into `Invariant::ALL` | Symmetric `is_intentionally_stopped` helper introduced; SystemGc-Terminated rows now filtered from Run-branch `active_allocs_vec`; resubmit DST GREEN. |
| 01-05 | Kill 3 missed mutants on `is_intentionally_stopped` + `is_natural_exit` | Mutation kill-rate 100% (was 84.2%, +15.8pp); 19/19 caught. |

## Scope expansions (user-approved)

Both expansions were surfaced for user approval at the moment of discovery —
neither was a unilateral widening.

- **Step 01-04 — resubmit-after-SystemGc gap.** Step 01-03's RED scaffold for
  `resubmit_after_gc_creates_fresh_alloc` revealed that the architecture's § 5
  promise ("the next tick of the same target sees `desired.job = Some(...)`
  again and follows the Run branch — placing a fresh allocation") was
  speculative until structurally enforced. The Run branch's `is_natural_exit`
  helper would mis-classify SystemGc-stopped rows and emit `FinalizeFailed`
  every tick instead of placing fresh. User approved option 1 (introduce
  symmetric `is_intentionally_stopped` helper, filter from
  `active_allocs_vec`).
- **Step 01-05 — mutation gap.** Step 01-04's mutation run reported 84.2%
  kill-rate (PR-gate passed at ≥80%) with 3 surviving mutants on
  `reconciler.rs:1713`/`1745`/`1746`. Per CLAUDE.md ("Missed mutations are
  actionable, not aspirational"), user approved adding tests (i) and (j) to
  close the gap rather than documenting the misses.

## Asymmetry to remember (load-bearing for future maintainers)

`is_operator_stopped(row)` and `is_intentionally_stopped(row)` are NOT
interchangeable. The helpers encode different precedence rules at different
call sites:

- **Operator-stop short-circuits the Run branch with `(Vec::new(),
  view.clone())`.** Operator's intent overrides re-submit; if the operator
  stopped the workload, a fresh re-submit does not undo the stop.
- **SystemGc-stop falls through to fresh placement.** The system stopped the
  workload because intent was withdrawn; the re-submit IS the operator's
  intent and re-creates the workload.

`is_operator_stopped` remains for audit / classification paths that want
operator-only semantics. `is_intentionally_stopped` is the broader
"intentional-stop class" query used at restart / natural-exit / placement
gates. Per-call-site decision lives in `reconciler.rs`; do not collapse the
two helpers.

## Architectural promise made structurally true

Architecture.md § 5 stated that resubmit after SystemGc produces a fresh
allocation. Until step 01-04 landed the `is_intentionally_stopped` filter on
`active_allocs_vec`, that statement was a documentation claim with no
structural enforcement — the Run branch would have hit `is_natural_exit ==
true` and emitted `FinalizeFailed` instead. Step 01-04 made the promise true.

## Lessons learned

- **DST RED scaffolds catch architectural promises that are not structurally
  enforced.** The `resubmit_after_gc_creates_fresh_alloc` scenario was
  intended as a regression guard for already-working behaviour; it instead
  surfaced that the behaviour didn't yet exist. The convention paid for itself
  inside one step.
- **Mutation gate >= 80% is the floor, not the goal.** When 3 specific
  mutants survive on a small surface (3 lines), the discipline is to add
  tests, not to log "diff-scoped passed at 84.2%." Roadmap step 01-05 was
  inserted mid-execution after user approval.
- **Amend ADRs in place when the change is additive, not a new ADR.** The
  ADR-0037 amendment block landed in the same commit as the variant addition
  (commit `ac02653e`). New-ADR-per-change inflates the ADR index without
  matching architectural significance.

## Migrated artifacts

- `docs/architecture/workload-gc-absent-stale-allocs/architecture.md` (was
  `docs/feature/.../design/architecture.md`)
- `docs/architecture/workload-gc-absent-stale-allocs/c4-component.md` (was
  `docs/feature/.../design/c4-component.md`)
- ADR-0037 amendment landed in place at
  `docs/product/architecture/adr-0037-reconciler-emits-typed-terminal-condition.md`
  (commit `ac02653e`).

## Commits

- `ac02653e` — feat(transition-reason): add StoppedBy::SystemGc for absent-intent GC
- `3cb064a0` — feat(reconciler): emit SystemGc stops for absent-intent orphan allocs
- `c8e47a25` — test(sim): DST scenarios for absent-intent workload GC + resubmit race
- `a3fc5191` — feat(reconciler): symmetric intentional-stop class for SystemGc + Operator
- `a0f27ba1` — test(reconciler): kill 3 missed mutants on intentional-stop helpers

Plus DESIGN-wave commits `c82d060f` and `69c4bcb5`.

## Verification

- All 5 DES steps verified PASS via `verify_deliver_integrity` (exit 0).
- Adversarial review: APPROVED WITH NOTES (no blockers).
- Mutation gate: 100% kill-rate post step 01-05 (19/19 caught).
- Full test suite green via Lima.
