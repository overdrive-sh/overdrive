# Quint specification: reconciler evaluation wake (reconciler-convergence-guarantees)

Model-checked specification of decision D-1 (ADR-0296: every reconciler declares its managed targets; the runtime
re-evaluates all of them each period; no local requeue on a hydrate or persist error) and D-3
(`ServiceMapHydrator::next_evaluation_at`). `.claude/rules/design.md` § "Concurrent protocols carry a model-checked
Quint specification"; oracle for the DISTILL `quint-connect` driver (`.claude/rules/testing.md` § "Quint conformance").

Run: `cargo xtask metal run -- cargo xtask quint typecheck`, then
`cargo xtask metal run -- cargo xtask quint check --subsystem reconciler-evaluation-wake [--name <check>] [--ci]`.
Evidence: `evidence/summary.md` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake`).

## Abstraction

Module `evaluation_wake` (parameters `OFF`, `FAULTS`, `DROPS`, `REC`). Three evaluation keys `(reconciler, target)`:
`k1 = (rA, t1)`, `k2 = (rH, t1)`, `k3 = (rA, t2)`; `rA` is generic, `rH` is hydrator-shaped (the only one with a View
backoff window and `next_evaluation_at`). `k1` and `k2` share the target `t1`, so the loop's per-TARGET exclusion
(`active_targets` is a set of `TargetResource`) is exercised. CAP = 2 stands for the 8-evaluation cap. World per key:
`has` (the managed set, from intent and observation) and `need` (desired != actual).

| Part | Model | Source |
|---|---|---|
| Broker | `pending` FIFO list of unique keys, `elig` `imm` / `nb`; same-key submit collapses and keeps its slot; Immediate merges a pending NotBefore to Immediate | `eval_broker.rs` `submit`, `merge_eligibility` |
| Admission | `Drain`: oldest first, up to CAP - active, skip a blocked target, skip a non-Immediate key, atomic batch | `eval_broker.rs` `drain_pending`; `lib.rs:5165-5260` |
| Evaluation | `Drain` admits only; `Finish(k, outcome)` is `run_convergence_tick`, which hydrates at its start (the world is read at `Finish`, so a change between `Drain` and `Finish` is seen); outcomes `ok`, `dpFail`, `hydrateErr`, `persistErr`, `dispatchErr`; a tick that emitted an action self-requeues Immediate on every path including `dispatchErr`; an in-window Noop requeues NotBefore when `next_evaluation_at` is `Some` | `reconciler_runtime.rs:1580-1750` |
| Resync pass (D-1b) | `Tick` (period timer, weak fairness) sets `due`; `PassStep` walks `rA`, `rH` in order, each either submitting the reconciler's whole managed set Immediate or (enumeration fault) nothing; the first pass is due at boot and after a crash | feature-delta D-1a, D-1b |
| Faults | hydrate / persist / dispatch error, dataplane failure, enumeration failure, crash (broker and in-flight lost; View, intent, observation survive), dropped edge wake, drift; budgeted by `MAXF = 2`, `MAXD = 2` | feature-delta "Formal protocol model" |

## Properties, named after the decision they defend

| Property | Kind | Defends | Hazard that breaks it |
|---|---|---|---|
| `Safety` = `ActiveWithinCap`, `OneActivePerTarget`, `PendingUnique`, `EligCanonical`, `NeedImpliesManaged`, `WindowOnlyForNeedy` | `expect` clause in the named runs only | model well-formedness; every conjunct holds by construction of the transition relation, so there is no standalone check | none (see Limits) |
| `ProgressByPass` | temporal, `wake_full` | D-1: every needy key is evaluated to convergence again after any error outcome, dropped wake or crash | `wake_noPass` (D-1) |
| `RetentionSchedulesBackedOffKey` | invariant, `wake_retention` (only fault: the dataplane fails) | D-3: a key inside its backoff window is always scheduled (in flight or retained), so its retry needs neither a row write nor a pass | `wake_noRetention` (D-3) |

Hazards (one per decision): `wake_noPass` removes the pass (dropped wakes disabled, so the wedge is the one ADR-0296
names: an error outcome ends the evaluation and nothing re-submits the key); `wake_noRetention` makes
`next_evaluation_at` return `None` (the pre-D-3 hydrator: the in-window Noop is not requeued and the key is
unscheduled).

## Named runs (critical scenario -> run, all on `wake_conf`)

| Scenario | Run |
|---|---|
| happy path (edge wake converges) | `healthy_edge_converges` |
| slice 01 fault 1: hydrate error | `hydrate_error_recovers_by_pass` |
| slice 01 fault 2: View persist error | `persist_error_recovers_by_pass` |
| crash between View fsync and dispatch | `crash_between_fsync_and_dispatch_recovers_by_boot_pass` |
| slice 01 fault 3: failed dataplane inside the backoff window (D-3) | `failed_dataplane_inside_backoff_recovers_without_row_write` |
| fresh deploy: intent, no row, no edge wake | `fresh_deploy_without_row_is_evaluated` |
| enumeration failure of one reconciler | `enumeration_failure_of_one_reconciler_does_not_starve_others` |
| edge wake during a pass burst (also target exclusion) | `edge_wake_during_burst_is_admitted_after_burst` |
| V-4: pass submit merges a retained NotBefore (discharged through admission: the next `Drain` admits the key) | `pass_submit_merges_retained_not_before_to_immediate` |
| D-12 d: world changes between `Drain` and `Finish`; `Finish` acts on the world as read at `Finish` | `world_changes_between_drain_and_finish` |

## Conformance step record and projection

`conf_step` records the action that produced each state (variant tagged by action name, value = record of its
parameters); it changes only on `wake_conf` (`REC = true`), which also counts
`dispatched` (Drain admissions) and `cancelled` (same-key collapses). Driver ITF name:
`wake_conf::evaluation_wake::conf_step`.

The pinned observable projections (accessors, signatures, model-only variables) and the action-to-driving-port mapping are
in the feature-delta, "Formal protocol model", "Pinned observable projections for the conformance drivers"
(`docs/feature/reconciler-convergence-guarantees/feature-delta.md`). Not copied here.

## Assumptions

WK-1 to WK-6 are the `ASSUMPTIONS` block of `evaluation_wake.qnt`, each with its discharging item; the feature-delta
carries the same table.

## Limits

The model checks the pass and edge-wake protocol, not the duration bound: the period, anchored re-arming and "within one
period plus queue time" are fairness assumptions here (WK-3). All six `Safety` conjuncts are true by construction of the transition relation (broker rules are existing
behaviour, not D-1 or D-3 decisions, so no hazard and no standalone check); their teeth are the broker unit tests
(V-4). `Safety` stays as an `expect` clause in the named runs. A pass never submits a partial set because `managed_targets`
returns the whole set or `Err` (D-1a), assumption WK-4.
