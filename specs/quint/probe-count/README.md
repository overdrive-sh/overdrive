# Quint specification: probe-run counting (netns-density-295)

Model-checked specification of the protocol between the probe task of one key (alloc, role, probe_idx) and
ServiceLifecycle, per decisions PROBE-RUN-COUNT (ADR-0191; the producer-counted `same_status_runs` and the consumer act-once-per-row rule) and PROBE-NOSAMPLE (ADR-0187 decisions 1-3).
`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint specification";
oracle for DISTILL `quint-connect` drivers (`.claude/rules/testing.md` § "Quint conformance").

Run: `cargo xtask lima run -- cargo xtask quint typecheck`, then
`cargo xtask lima run -- cargo xtask quint check --subsystem probe-count [--name <check>]`.
Evidence: `evidence/summary.md` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem probe-count`).

## Abstraction and rule

One key, one producer, one consumer. Module `probe_count` (parameters `OFF`, `REC`); main instance `probe_streak` (`OFF = {}`, `REC = false`); conformance main `probe_streak_conf` (`REC = true`).
LT = 3 (liveness), RT = 2 (readiness), bound MAXRUNS = 7 producer runs of any outcome (`Fail x3, Pass, Fail x3` fits); looks are unbounded (the state space is finite: A, running, run_seq are bounded by MAXRUNS).

Producer action `ProbeRun(outcome, reason)`, outcome in Pass | Fail | NoSample | WriteFail (reason: `node_capacity` |
`quiesced`). Pass / Fail write a row `{run_seq = last written + 1, status, same_status_runs}` where
`same_status_runs` is counted by the producer: first row 1; same status as the previous written row => previous + 1;
else 1. NoSample / WriteFail write nothing, consume no number, and neither break nor extend the streak.
Store: one latest row, replaced iff the written run_seq is strictly greater. Environment action:
`ClockBack` (backward wall-clock step; observed-at decides nothing).

Consumer action `ConsumerLook` reads the latest row (N, st, s); it keeps no streak and no "runs since last look"
arithmetic. Liveness crossed iff `st == Fail and s >= LT`; readiness eligible iff `st == Pass and s >= RT` (a level
re-derived at every look; no row => not eligible). Act once per row: the view keeps `A` (run_seq at which it last
emitted, 0 never); it emits a liveness terminate iff `st == Fail and s >= LT` (crossed) AND the allocation is Running
AND `N > A`, and only when it emits sets `A := N` (persisted before dispatch). A re-read of the same row does nothing;
a new crossed row while still Running (the previous stop failed) emits again: a retry once per new written row.

Dispatch outcome: the action-shim dispatch of StopAllocation is awaited inside the reconcile tick (PC-4), so the
outcome is decided in the same `ConsumerLook(stopOk)` step, `stopOk` chosen nondeterministically by `step`. A look
that emits with `stopOk` sets `running := false` (the shim wrote the terminal row inline); emitting with `not stopOk`
leaves `running` true. With `running` false a look emits nothing (the existing Running guard). Producer runs and
looks may continue after a successful stop (allowed, not restricted; they cannot terminate). Terminates are counted
in ghost state (`terms`, `tseqs`).

Conformance projection: `rseq`, `rst`, `rsame` (the stored row), `A` (consumer view) and `running` (projected from the
allocation's `AllocStatusRow`: not terminal). Model-only (not projected): `hist`, `terms`, `tseqs`, `g`, the producer
variables, `clock`/`rat`, and `fs`/`M`, which exist only for the `reconstruct` hazard and stay 0 in the main instance.

## The conformance step record `conf_step` (D-UP-1)

`quint test` (which `#[quint_test]` runs) emits no `mbt::actionTaken` / `mbt::nondetPicks`, so the module records the
action that produced each state in `conf_step`: a variant of `probe_count_conf::ConfStep` whose tag is the action's
name (`init`, `ProbeRun`, `ClockBack`, `ConsumerLook`, `Idle`) and whose value is the record of the action's parameters
by parameter name (`ProbeRun { out, reason }`, `ConsumerLook { stopOk }`; the unit value for `init`, `ClockBack`,
`Idle`). The variant type sits in the definitions-only module `probe_count_conf` and each action builds its value with
the builtin `variant`, because a constructor cannot share an action's name. `conf_step` is model-only and write-only
(no guard or property reads it).

It changes only on the conformance main `probe_streak_conf` (`REC = true`); each action assigns
`conf_step' = if (REC) variant(…) else conf_step`, so `probe_streak` and every hazard instance (`REC = false`) keep it
at `init` and their TLC state spaces are those of the rules alone. Only the named runs are checked on
`probe_streak_conf`. A driver names it in `#[quint_test(main = "probe_streak_conf", …)]`, `#[quint_run]` and
`Config::nondet` by its ITF name `probe_streak_conf::probe_count::conf_step` (Quint qualifies a variable with the main
module and the module that declares it).

## Invariants (named after the decision they defend)

| Invariant | Decision | Meaning |
|---|---|---|
| `NoEarlyTerminate` | PROBE-RUN-COUNT | every terminate happens at a look where the true trailing count of written Fail rows >= LT |
| `NoReactOnSameRow` | PROBE-RUN-COUNT | no two terminates on the same row (same run_seq) |
| `NoTerminateWhenStopped` | PROBE-RUN-COUNT | no terminate after a stop succeeded (`running` false) |
| `NoMissedTerminate` | PROBE-RUN-COUNT | progress as a per-look obligation: at every look on a Running allocation whose latest row is crossed with N > A, a terminate is emitted at that look |
| `NoEarlyReady` | PROBE-RUN-COUNT | eligible only if the true trailing written Pass count >= RT |
| `StreakExact` | PROBE-RUN-COUNT | stored `same_status_runs` == true trailing count of written rows with its status |
| `MissingSampleInert` | PROBE-NOSAMPLE | NoSample / WriteFail runs write or change no row |
| `StoreHoldsLatest` | PROBE-RUN-COUNT | the stored row is the last written row (merge by run_seq) |
| `NoStaleChange` | PROBE-RUN-COUNT | a look that sees no new row changes no consumer state and emits no terminate |
| `Safety` | all | conjunction of the invariants above  |

No witnesses: the named runs `HappyPathThreeFailsTerminateAtThreshold` and `ReadinessReachesThreshold` reach a terminate and readiness.

## Finding

A same-key reset of the producer's streak (run_seq continuing) was examined and ruled unreachable: the streak counter and run_seq are task-private to one probe task, which is never re-spawned for a live allocation, and every restart mints a new key (PC-1), so there is no bounded exposure.

## Not modelled

A probe-runner restart is not modelled as a same-key action, because a restart only happens with a new allocation (PC-1), whose key starts afresh (run_seq 1, same_status_runs 1, no act-once entry), so it can only delay an action; the predecessor allocation's leaked probe tasks (GH #318) write under their own key.

## Named runs (critical scenario -> run, all in `probe_count.qnt`, listed in `checks.toml`, main instance)

| Scenario | Run |
|---|---|
| happy path: look after each Fail, terminate at the third | `HappyPathThreeFailsTerminateAtThreshold` |
| three Fails then one look terminates | `CoalescedFailsTerminateOnFirstLook` |
| NoSample / WriteFail between Fails are inert, number reused | `MissingSampleChangesNothing` |
| failed stop, re-read with no new row: no re-terminate | `ReReadAfterFailedStopDoesNotReTerminate` |
| failed stop, one more Fail row: retried; after a successful stop nothing more | `FailedStopRetriedOnNextRow` |
| failed stop, Pass, new Fail streak crossing again while Running: terminate again | `NewStreakAfterPassTerminatesAgain` |
| Fail, look, Pass, Fail, look: no early terminate | `FlipBetweenLooksNoEarlyTerminate` |
| readiness reaches RT, then drops on Fail | `ReadinessReachesThreshold` |
| backward clock step does not lose a row | `ClockStepBackDoesNotLoseRow` |

## Hazards (`hazard/probe_count_hazards.qnt`, `expect = "violation"`; each has a failing run)

| Instance | Rule removed | Violated | Failing run |
|---|---|---|---|
| `streak_bugReconstruct` | consumer-side reconstruction: consumer keeps `fs`, advances by N - M, resets on the other status | `NoEarlyTerminate` | `FlipBetweenLooksNoEarlyTerminate` |
| `streak_bugNoActOnce` | no `A` check | `NoReactOnSameRow` | `ReReadAfterFailedStopDoesNotReTerminate` |
| `streak_bugOncePerStreak` | condition `N - s >= A` (once per Fail streak) | `NoMissedTerminate` | `FailedStopRetriedOnNextRow` |
| `streak_bugOnceEver` | emit only if `A == 0` | `NoMissedTerminate` | `FailedStopRetriedOnNextRow` |
| `streak_bugNoSampleFail` | NoSample written as a Fail row | `MissingSampleInert` | `MissingSampleChangesNothing` |
| `streak_bugLwwObservedAt` | merge by observed-at with ClockBack | `StoreHoldsLatest` | `ClockStepBackDoesNotLoseRow` |

## Assumptions

| Id | Assumption | Discharged by |
|---|---|---|
| PC-1 | the producer is the single writer of its key; a probe task is never re-spawned for a live allocation (`crates/overdrive-worker/src/probe_runner/mod.rs:297-300,364-369`); a restart mints a new AllocationId, so run_seq never goes backwards. A crash-restart's predecessor probe tasks keep writing under their own (old) AllocationId: a different key, so not a second writer of this key; that leak is GH #318 | R5-PROBE-COUNT (DISTILL seeded-sim scenario of the production ServiceLifecycle + probe runner, defined in the feature delta) |
| PC-2 | hydration yields the latest stored row per key | R5-PROBE-COUNT |
| PC-3 | a run that writes no row leaves the store unchanged | R5-PROBE-COUNT; for the host store also R5-PROBE-MERGE (LocalObservationStore failed-write case) |
| PC-4 | the action-shim dispatch of StopAllocation is awaited inside the reconcile tick (`reconciler_runtime.rs:1604-1640`), so after a successful stop the allocation is no longer Running at the next look, and a failed stop (`Driver::stop` Err, `action_shim/mod.rs:3229-3233`, or a later cleanup step, `:3240-3248`) leaves it Running; `A` is persisted before dispatch | R5-PROBE-COUNT fault (h): a scripted `Driver::stop` failure |

No Apalache checks: the instances are small and finite; TLC is exhaustive.
