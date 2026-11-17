# Quint specification: reconciler evaluation wake (reconciler-convergence-guarantees)

Model-checked specification of decision D-1 (ADR-0296: every reconciler declares its managed targets; the runtime
re-evaluates all of them each period; no local requeue on a hydrate or persist error) and D-3
(`ServiceMapHydrator::next_evaluation_at`). `.claude/rules/design.md` § "Concurrent protocols carry a model-checked
Quint specification"; oracle for the DISTILL `quint-connect` driver (`.claude/rules/testing.md` § "Quint conformance").

Run: `cargo xtask metal run -- cargo xtask quint typecheck`, then
`cargo xtask metal run -- cargo xtask quint check --subsystem reconciler-evaluation-wake [--name <check>] [--ci]`.
Evidence: `evidence/summary.md` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake`).

## Abstraction

Module `evaluation_wake` (parameters `OFF`, `FAULTS`, `DROPS`, `REC`). Four evaluation keys `(reconciler, target)` of one shared
workload (a Service with a listener). Abstract names sort like the production names, so the resync pass's registered
(`BTreeMap`) order is `r1S, r2M, r3V, r4W`.

| Model key | Model reconciler | Production reconciler | Target |
|---|---|---|---|
| `k3` | `r1S` | `service-lifecycle` | `t1` = `workload/<id>` |
| `k2` | `r2M` | `service-map-hydrator` | `t2` = `service/<id>` of the workload's own listener (alone on its target) |
| `k4` | `r3V` | `svid-lifecycle` | `t1` |
| `k1` | `r4W` | `workload-lifecycle` | `t1` |

`r2M` (the hydrator) and `r3V` (svid-lifecycle) have a View backoff window and `next_evaluation_at` (the hydrator's is D-3; svid-lifecycle's is existing behaviour, D11). `k1`, `k3`, `k4` share `t1`, so the loop's
per-TARGET exclusion (`active_targets` is a set of `TargetResource`) is exercised. CAP = 2 stands for the 8-evaluation cap.
World per key: `has` (the managed set, from intent and observation) and `need` (desired != actual). More world facts, each a
production fact: `vip` (the listener has an assigned VIP) and `held` (an SVID is held for some allocation of the workload);
`running` (a Running allocation exists) is derived as `has(k1) and not need(k1)`. Two View facts decide whether a View write can fail
(D9, below): `v3` (service-lifecycle's next tick would change its View) and `r4` (svid-lifecycle's stored View holds an `IssueRetry`
entry). Both are model-only (never projected).

### Managed-set coupling (`SharedIntentImpliesBase`)

`has(k3) => has(k1)`, `has(k4) => has(k1)`, `has(k4) => has(k3)` (D7: an allocation row makes service-lifecycle manage the Service),
`has(k2) => has(k3)`; `has` never shrinks. An intent alone makes only `k1` managed: service-lifecycle manages a Service once it has an
allocation row or an assigned VIP, svid-lifecycle once it has an allocation row or held SVID, the hydrator once a backend row exists.
So `has(k3)` arises from a `k3`, `k4` or `k2` world change or from a landed `k1` start, never from a `k1` world change.
`managed_targets` is still a DISTILL scaffold in the tree; the rule is the user ruling (`conformance-driver.md` "World construction").

### What each key needs, and what a world change does

| Key | `need` (production reconcile) | Source |
|---|---|---|
| `k1` | intent persisted and no Running allocation (nothing is emitted while one exists) | `workload_lifecycle.rs` `running_alloc` branch |
| `k4` | a Running allocation with no held SVID (`IssueSvid`), or a held SVID of an allocation no longer Running (`DropSvid`); `running and held and not near-expiry` emits nothing (D6) | `svid_lifecycle.rs` `reconcile` |
| `k3` | with a VIP: the observed backend row differs from the projection of the Running allocations; without a VIP: a Running allocation whose Stable is not committed (the startup opt-out commits it at the first Running row) | `service_lifecycle.rs` branch (a'), `service_backend_row_actions` |
| `k2` | a backend row the result row does not cover | `service_map_hydrator.rs` |

| World change (driver stimulus) | Effect on `has` / `vip` / `held` / `need` |
|---|---|
| `k1`: persist the intent, replace the Running allocation with a Failed row | `has +k1`; `need(k1)`; `need(k3) := vip` (the projection lost a backend; without a VIP a Failed row needs nothing); `need(k4) := held` (nothing to issue, a held SVID of the failed allocation is to be dropped) |
| `k4`: persist the intent, add a fresh Running allocation with no SVID | `has +k1,k3,k4`; `need(k4)`, `need(k3)`; `need(k1)` cleared (a Running allocation exists) |
| `k3`: assign the VIP, persist the intent | `has +k1,k3`; `vip`; `need(k3)`; `need(k1)` if the intent was new (no allocation exists) |
| `k2`: assign the VIP, persist the intent, write a backend row | `has +k1,k2,k3`; `vip`; `need(k2)`, `need(k3)` (the written row differs from the projection); `need(k1)` if the intent was new |

### View changes and backoff windows (D9, D11; existing behaviour)

`persist_view` returns before the durable write when `next_view == current` (`reconciler_runtime.rs`, `if current == view` in every arm), so an
injected View-write failure (`persistErr`) is reached only when the tick changes the View. Otherwise the tick dispatches and requeues as `ok`.
The model offers `persistErr` only where production's View changes (the `Finish` guard `acts and viewChanges`):

| Key | The tick changes the View when it acts | Source |
|---|---|---|
| `k1` | always: a new allocation id enters `restart_counts` | `workload_lifecycle.rs` (`restart_counts.insert`) |
| `k2` | always: the attempt is bumped; an in-window Noop changes nothing, so `persistErr` is not offered there | `service_map_hydrator.rs` `reconcile` |
| `k3` | only if it records an allocation it has not yet recorded (`v3`): a Running unstable allocation not in `observed`, a Failed one not in `terminal_announced`; a VIP with no allocation, or a committed-Stable allocation, changes nothing | `service_lifecycle.rs` `reconcile` |
| `k4` | an `IssueSvid` always (the `IssueRetry` is bumped); a `DropSvid` only when an entry for the dropped allocation is still stored (`r4`) | `svid_lifecycle.rs` `reconcile` |

`v3` is set by a `k4` world change (a fresh Running row), a `k1` world change that replaces a Running allocation (a Failed row) and a landed `k1`
start (a new allocation), and cleared by any `k3` tick that persists. `r4` is, after any persisted `k4` tick, `need(k4) and running`
(the entry is written on emit and removed by the first tick that sees the allocation held or gone).

svid-lifecycle has its own backoff window. The `IssueRetry` is recorded when `IssueSvid` is **emitted**, not when it fails
(`svid_lifecycle.rs`: "Record the attempt"); a landed issue leaves the allocation held, so the entry never gates anything and the next tick
removes it. A **failed** issue (`dispatchErr` on a Running, unheld allocation) leaves it unheld with the entry: the key is inside its
window (`inWin(k4)`), the self-requeue is Immediate (the tick emitted an action), and an evaluation inside the window emits `Noop`
and requeues with a `NotBefore` retention, because svid-lifecycle implements `next_evaluation_at` (the retry deadline of a Running, unheld,
never-issued allocation). That retention is existing behaviour and stays on in every instance; the `noRetention` hazard switches off the
hydrator's (D-3) only. A `k1` world change ends `k4`'s window (the allocation is no longer Running). `backoff_for_attempt` ignores the attempt
(one constant, `RESTART_BACKOFF_DURATION`), so `WindowEnds` is one clock advance that ends every open window and makes every retained
`NotBefore` eligible. `RetentionSchedulesBackedOffKey` covers `k4` (it ranges over every key in a window); the `wake_retention` instance enables `dispatchErr` beside `dpFail` so that `k4` can enter its window.

### Handoff wakes and landed consequences (existing behaviour)

| When | Handoff submitted (Immediate, before the self-requeue) | Consequence when the action lands (`ok`) | Source |
|---|---|---|---|
| `k1` acts (allocation-mutating action) | `k3` then `k4` | the SVID is held (D6); `need(k3)`; `has +k3,k4`; **`need(k4)` is not set**: the svid evaluation after the start is a Noop, so the handoff wakes a key that is not needy | `workload_lifecycle.rs:264-306`, `action_shim/mod.rs:187-213` (called at `:1870`, `:2306`), `:2899`, `svid_lifecycle.rs` |
| `k3` acts, VIP assigned (`WriteServiceBackendRow`) | `k2` | `need(k2)`, `has +k2` (D4) | `service_lifecycle.rs:1152-1166` (push at `:1161`) |
| `k3` acts, no VIP (`FinalizeFailed` Stable only) | none (D5) | Stable committed; nothing for `k2` | `service_lifecycle.rs:852-885`, `:1096-1099` |
| `k2`, `k4` act | none | `k4`: SVID held for every Running allocation, held SVID of a non-Running allocation dropped (`held := running`) | |

Per outcome of `Finish(k, o)`:

| Outcome | Handoffs | Lands | Requeue of `k` itself |
|---|---|---|---|
| `ok`, `k` acts | submitted (the table above) | yes | Immediate |
| `ok`, `k` not needy (Noop) | none | no | none (`has_action` false; `next_evaluation_at` is `None` for `k1`, `k3` here); a hydrator or svid-lifecycle Noop inside its window requeues NotBefore |
| `dispatchErr`, `k` acts | submitted (dispatch isolates actions); `k3` only with a VIP | no | Immediate; a failed `k4` `IssueSvid` also enters `k4`'s window |
| `dpFail` (hydrator) | none | no (result row) | Immediate |
| `hydrateErr`, `persistErr` (View changes), `Crash` | none (end before dispatch) | no | none |

A handed-off or submitted key that is not needy is admitted (counted in `dispatched`) and evaluates to a Noop; a submit at an already
pending key collapses and counts in `cancelled` whether or not the key is needy (`EvaluationBroker::submit` reads no world;
`enqueue_evaluation.rs` submits Immediate through `ReconcilerRuntime::submit_evaluation`).

| Part | Model | Source |
|---|---|---|
| Broker | `pending` FIFO list of unique keys, `elig` `imm` / `nb`; same-key submit collapses and keeps its slot; Immediate merges a pending NotBefore to Immediate | `eval_broker.rs` `submit`, `merge_eligibility` |
| Admission | `Drain`: oldest first, up to CAP - active, skip a blocked target, skip a non-Immediate key, atomic batch | `eval_broker.rs` `drain_pending`; `lib.rs:5165-5260` |
| Evaluation | `Drain` admits only; `Finish(k, outcome)` is `run_convergence_tick`, which hydrates at its start (the world is read at `Finish`, so a change between `Drain` and `Finish` is seen); outcomes `ok`, `dpFail`, `hydrateErr`, `persistErr` (only where the View changes, D9), `dispatchErr`; a tick that emitted an action self-requeues Immediate on every path including `dispatchErr`; an in-window Noop (hydrator, svid-lifecycle) requeues NotBefore when `next_evaluation_at` is `Some` | `reconciler_runtime.rs:1580-1750` |
| Resync pass (D-1b) | `Tick` (period timer, weak fairness) sets `due`; `PassStep` walks `r1S`, `r2M`, `r3V`, `r4W` in registered (BTreeMap) order, each either submitting the reconciler's whole managed set Immediate or (enumeration fault) nothing; the first pass is due at boot and after a crash | feature-delta D-1a, D-1b |
| Faults | hydrate / persist / dispatch error, dataplane failure, enumeration failure, crash (broker, its `dispatched` / `cancelled` counters and in-flight set lost; View, intent, observation survive), dropped edge wake, drift; budgeted by `MAXF = 2`, `MAXD = 2` | feature-delta "Formal protocol model" |

## Properties, named after the decision they defend

| Property | Kind | Defends | Hazard that breaks it |
|---|---|---|---|
| `Safety` = `ActiveWithinCap`, `OneActivePerTarget`, `PendingUnique`, `EligCanonical`, `NeedImpliesManaged`, `WindowOnlyForNeedy`, `SharedIntentImpliesBase` | `expect` clause in the named runs only | model well-formedness; every conjunct holds by construction of the transition relation, so there is no standalone check | none (see Limits) |
| `ProgressByPass` | temporal, `wake_full` | D-1: every needy key is evaluated to convergence again after any error outcome, dropped wake or crash | `wake_noPass` (D-1) |
| `RetentionSchedulesBackedOffKey` | invariant, `wake_retention` (faults: the dataplane fails, a dispatch fails) | D-3 (and svid-lifecycle's existing retention, D11): a key inside its backoff window is always scheduled (in flight or retained), so its retry needs neither a row write nor a pass | `wake_noRetention` (D-3) |

Hazards (one per decision): `wake_noPass` removes the pass (dropped wakes disabled, so the wedge is the one ADR-0296
names: an error outcome ends the evaluation and nothing re-submits the key); `wake_noRetention` makes
the hydrator's `next_evaluation_at` return `None` (the pre-D-3 hydrator: the in-window Noop is not requeued and the key is
unscheduled). svid-lifecycle's retention is existing behaviour and is not switched off.

## Named runs (critical scenario -> run, all on `wake_conf`)

| Scenario | Run |
|---|---|
| happy path (edge wake converges; the start hands off to `k3` and `k4`, `k3` commits Stable, the `k4` evaluation is a Noop) | `healthy_edge_converges` |
| slice 01 fault 1: hydrate error | `hydrate_error_recovers_by_pass` |
| slice 01 fault 2: View persist error | `persist_error_recovers_by_pass` |
| crash between View fsync and dispatch | `crash_between_fsync_and_dispatch_recovers_by_boot_pass` |
| slice 01 fault 3: failed dataplane inside the backoff window (D-3) | `failed_dataplane_inside_backoff_recovers_without_row_write` |
| fresh deploy: intent, no row, no edge wake | `fresh_deploy_without_row_is_evaluated` |
| enumeration failure of one reconciler (`r4W`, last in the pass order; the other reconcilers' keys are evaluated, D7 and D8 visible) | `enumeration_failure_of_one_reconciler_does_not_starve_others` |
| edge wake during a pass burst (also target exclusion between `k3` and `k1` on `t1`, and the handoffs, the `k3` publish to `k2` with a VIP) | `edge_wake_during_burst_is_admitted_after_burst` |
| V-4: pass submit merges a retained NotBefore (discharged through admission: the next `Drain` admits the key) | `pass_submit_merges_retained_not_before_to_immediate` |
| D-12 d: world changes between `Drain` and `Finish`; `Finish` acts on the world as read at `Finish` | `world_changes_between_drain_and_finish` |

## Conformance step record and projection

`conf_step` records the action that produced each state (variant tagged by action name, value = record of its
parameters); it changes only on `wake_conf` (`REC = true`), which also counts
`dispatched` (Drain admissions) and `cancelled` (same-key collapses); both are in-memory broker state, so `Crash` resets them to 0. Driver ITF name:
`wake_conf::evaluation_wake::conf_step`.

The pinned observable projections (accessors, signatures, model-only variables) and the action-to-driving-port mapping are
in the feature-delta, "Formal protocol model", "Pinned observable projections for the conformance drivers"
(`docs/feature/reconciler-convergence-guarantees/feature-delta.md`). Not copied here.

## Assumptions

WK-1 to WK-8 are the `ASSUMPTIONS` block of `evaluation_wake.qnt`, each with its discharging item; the feature-delta
carries the same table. WK-7 states the handoff edges, the landed consequences and the world needs in the tables above (which action emits which
`EnqueueEvaluation`, in which order, what a landed start or publish leaves needy, that the svid evaluation after a `k1` start is a Noop
because the start holds its own SVID, that `k3` publishes and hands off only with a VIP). WK-8 states the View-change facts (D9) and svid-lifecycle's window and retention (D11) above; both are discharged by the fidelity probe
and the quint-connect replay of the named runs against the real reconcilers. Not modelled: the held SVID set is volatile in a real
process restart (svid-lifecycle's restart recovery re-issues), but the conformance `Crash` rebuilds the runtime only (WK-5).

## Check results and timings

One faithful model: `wake_full` at `MAXF = 2`, `MAXD = 2`, every key, fault and drift kind (the verified model is the
conformance oracle; it is not split into an abstract and a faithful main). There is no per-check cap beyond the runner's
`timeout_secs`; `ci = true` only for checks under two minutes (`.claude/rules/design.md`). Recorded run (TLC, exhaustive,
16 CPU / 61 GiB test machine). The recorded evidence does not carry TLC state counts.

| Check | Main | Verdict | Time | Peak memory | `ci` |
|---|---|---|---|---|---|
| `evaluation-wake-progress-by-pass` | `wake_full` | holds | 4794.2 s (79.9 min) | 6.8 GiB (heap 40 GiB, 12 workers) | false |
| `evaluation-wake-retention-schedules-backed-off-key` | `wake_retention` (faults `dpFail`, `dispatchErr`) | holds | 634.0 s | 3.0 GiB (heap 12 GiB, 4 workers) | false |
| `evaluation-wake-hazard-no-pass` | `wake_noPass` | violation | 13.2 s | 1.4 GiB | true |
| `evaluation-wake-hazard-no-retention` | `wake_noRetention` | violation | 12.1 s | 1.4 GiB | true |
| 10 named runs | `wake_conf` | holds | 2.5 s to 4.1 s each | about 0.2 GiB | true |

## Limits

The model checks the pass and edge-wake protocol, not the duration bound: the period, anchored re-arming and "within one
period plus queue time" are fairness assumptions here (WK-3). All `Safety` conjuncts are true by construction of the transition relation (broker rules are existing
behaviour, not D-1 or D-3 decisions, so no hazard and no standalone check); their teeth are the broker unit tests
(V-4). `Safety` stays as an `expect` clause in the named runs. A pass never submits a partial set because `managed_targets`
returns the whole set or `Err` (D-1a), assumption WK-4.
