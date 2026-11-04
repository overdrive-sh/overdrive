# Quint specification: terminal-hook ownership (feature service-allocation-readiness-and-ending)

Model-checked specification of the protocols pinned by decisions D1, D3, D5, D6, D7, D8, D9, D10 and D11, and the B1 (ADR-0258) and B2 (ADR-0259) rulings of 2026-11-03, of
`docs/feature/service-allocation-readiness-and-ending/feature-delta.md` (ADR-0248 to ADR-0260), per
`.claude/rules/design.md` § "Concurrent protocols carry a model-checked Quint specification". It is the oracle for DISTILL
`quint-connect` drivers (`.claude/rules/testing.md` § "Quint conformance"; scenario S-9). D2 (the hook contract is
row-independent and idempotent) and D4 (the two shim hooks stay, `cleanup_restart_predecessor` gains none) are contract and
non-change decisions: they are covered by named runs and by assumption HO-3, with no hazard of their own. B3 (ADR-0251) is
text only: `failed_crash` abstracts the two Failed reasons the exit observer writes for a guest that ended on its own,
`WorkloadCrashedImmediately` and `VmOutOfMemory`; the shim's fail-closed supersessions of a `Running` row
(`MtlsInterceptInstallFailed`, `WorkloadNetnsProvisionFailed`, `DriverInternalError`) are not guest crashes and are outside this
class (they are `failed_other`). D12 (backend eligibility) has no model: it is a pure predicate over one hydrated fact, covered
by proptest S-46 and the composed S-47; `restart_authority` and `liveness_gate` carry no eligibility.

Run (the user's directive: metal, never Lima): `cargo xtask metal run -- cargo xtask quint typecheck`, then
`cargo xtask metal run -- cargo xtask quint check --subsystem service-allocation-readiness-and-ending [--name <check>]`.
Evidence: `evidence/summary.md` (written only by `--record`; `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending`).

## Modules (each is a state machine with `init`, `step` and named runs)

Every module is parameterised by `OFF` (the rules switched off, for hazards) and `REC` (record `conf_step`, the action that
produced each state, for conformance: `quint test` emits no `mbt::actionTaken`, as in probe-count D-UP-1). Instances:
`*_ok` (REC = false, the design, model-checked), `*_conf` (REC = true; only the named runs are checked on it),
`*_bug*` (in `hazard/`, one decision's rule off).

| File | Decisions | Abstraction |
|---|---|---|
| `hook_ownership.qnt` | D1, D3 (D2, D4 by runs) | per allocation id (two ids: an attempt and its fresh-id successor): VM started, `on_alloc_running` (supervisor registered), `release_for_exit_emission` (gate), guest died, `ExitEvent` none/queued/consumed, shim stop (effects, terminal row write, then hook: B2) / finalize hooks, supervisor present, terminal row. Actors: shim start, watcher, observer, shim stop, shim finalize |
| `restart_authority.qnt` | D5, D6, D7, D8 | one Service, attempts 0..CEIL with fresh ids; per attempt row state (`running`, `failed_crash`, `failed_other`, `t_startup`, `t_liveness`, `t_operator`, `t_clean`), startup probe row (`absent/fail/pass`, sequence, producer-counted streak), deadline, Stable, `terminal_announced`, `startup_last_acted_run`, stop outcome and stop failure stage (`landed`, `pre_hook`, `row_write`: B2), VM slot, the failure claim and its author. Actors: `ServiceLifecycle` (`stable`, `startup_stop`, `liveness_stop`), `WorkloadLifecycle` (`restart`, `give_up`), environment (crash, clean exit, operator stop, probe rows, deadline) |
| `liveness_gate.qnt` | D9 | one allocation: supervisor, retained liveness descriptors, liveness role absent/live, `committed` (the row carries Stable), the Stable write outcome (`landed`/`failed`: B1), `on_alloc_stable` hook, liveness streak and rows since Stable |
| `progress_deadline.qnt` | D10 | the Service stream's timer racing the lifecycle terminals; time in 10 s ticks (FLOOR 9 = 90 s, MARGIN 3 = 30 s, startup deadline 1 or 12 ticks); stream slots: one in the checked instance, two in the conformance instance (a redeploy opens a new stream with a new clock) |
| `stream_snapshot.qnt` | D11 | the stream's subscribe, recorded-verdict read, relist; the shim's commit and publish per attempt; `lag`, `read_fault`, fresh-id restart; `service_event_from_terminal` modelled by `proj` (restartable stops project to none) |

D10 and D11 are two modules of one stream (the design lists them as one abstraction): D11's `armed` flag stands for D10's
deadline being armed after the read, and D10's `Open` arms at stream open. A combined module was built first and checked;
the split keeps each exhaustive check small and loses no property the
design names, because the one interplay (the deadline arms only when the read found no verdict) is the `armed` flag asserted
by the D11 runs.

`restart_authority`: the ceiling is the module parameter `CEIL`. The checks run on `ra_ok` (CEIL = 1, two attempts: the
restart, the give-up and the successor are all reached); the named runs run on `ra_conf` (CEIL = 2, three attempts, so a
restart sits between two different causes). The startup max_attempts is 2, the bound on startup rows per attempt is 3. The
dispatch of a stop is awaited inside the reconcile tick (RA-2, as probe-count PC-4), so its landed/failed outcome is decided
in the step that emits it.

## Invariants (named after the decision they defend)

| Module | Invariant | Decision | Meaning |
|---|---|---|---|
| hook_ownership | `NoRegisterAfterEnd` | D3 | a supervisor is never registered for an id after its end was claimed |
| | `EndedImpliesEventuallyUnsupervised` (TLC temporal, weak fairness) | D1 | an id with an `ExitEvent` out is eventually unsupervised, whatever the write outcome |
| restart_authority | `NoStartupVerdictOverCrash` | D5 | no startup stop is decided for an allocation whose row is not `running` |
| | `CrashedAllocEventuallyDecidedByRestartAuthority` (temporal) | D5 | a failed allocation is eventually restarted or given up on by `WorkloadLifecycle` |
| | `OneAuthorPerEnding` | D6 | every failure claim on any row was authored by `workload_lifecycle` |
| | `StartupFailureNamedOnlyAtGiveUp` | D7 | a `startup_probe_failed` claim exists only as the final attempt's give-up |
| | `NoStrandedStop` | B2 (ADR-0259) | a `running` row whose VM slot is not live has a supervisor present, so the next probe row re-emits the stop |
| | `NoStableOverPendingStop` | D8 | `stable` is never announced for an announced id unless the latest startup row is `pass`; `terminal_announced` and Stable stay disjoint |
| | `FailedStartupStopIsRetried` (temporal) | D8 | an id that stays `running` with the gates holding, whose stop failed and that has a newer row, is stopped again |
| | `LatePassUnstrandsTheInstance` (temporal) | D8 | an announced `running` id whose latest startup row is `pass` reaches Stable |
| liveness_gate | `LivenessOnlyAfterStable` | D9, B1 | the liveness role is live only for an id whose row carries Stable (`committed`) |
| | `StableEventuallyCommitted` (temporal, weak fairness) | B1 (ADR-0258) | a `Running`, supervised allocation with no committed Stable is eventually committed: a failed Stable write is emitted again |
| | `NoLivenessStopBeforeStable` | D9 | a liveness stop is never enabled or emitted for an id whose row does not carry Stable |
| | `NoLivenessForEndedId` | D9 | the liveness role is never live for an id whose supervisor was removed |
| | `LivenessStreakStartsAtStable` | D9 | the streak is at most the liveness rows written since Stable |
| progress_deadline | `HealthySlowStartNeverTimesOut` | D10, scoped by DL-6 | a Stable committed at or before the startup deadline never coexists with a `timeout` closure; the model starts the stream clock and the startup clock together (assumption DL-6: boot plus placement plus one evaluation is at most the 30 s margin; where it fails the timeout is honest and retracts nothing, exposure E-4) |
| stream_snapshot | `NoMissedTerminal` (temporal) | D11 | an open stream that has subscribed and read, whose current attempt has a committed projecting terminal, closes |
| | `RestartableStopNeverEndsStream` | D7, D11 | no closure from a startup or liveness stop, on the live path or the read path |
| | `FailedReadNeverWaits` | D11 | a failed read closes the stream as `error` |
| | `StreamEndsOnceHonestly` | D11 | the stream yields at most one verdict line, never `stable` unless a Stable was committed |

Invariants that hold by construction were dropped from the checks, each after a guard-deletion trial (the action guard that
makes it hold removed in a scratch copy; a kept invariant, hazard or named run then failed, so the dropped invariant was
redundant): `SupervisorImpliesNotEnded` (subsumed by `EndedImpliesEventuallyUnsupervised` and the stop and finalize runs),
`StartupStopAtMostOncePerRow` (the retry runs), `StartupStopStopsTheInstance`, `StartupStopIsRestartable` and
`EarlyExitIffPreStartupCrash` (the runs that name the effect), `PreStartupCrashStaysObservable` (`NoStartupVerdictOverCrash`).
The real functions are tested directly: S-1, S-7, S-10, S-15, S-16, S-22, S-23, S-29, S-34, S-39, S-40. `NoStableOverPendingStop`
stays: no other check failed when its guards (the `terminal_announced` clause and the declared-probe pass requirement of the
Stable action) were deleted. Assumption SS-6 and the `Restart` guard that encodes it stay.

## Named runs (critical scenario -> run; all listed in `checks.toml`, `backend = "evaluator"`, on the `*_conf` instance)

| Scenario | Module | Run |
|---|---|---|
| #334 crash without restart | hook_ownership | `crash_without_restart` |
| #318 crash then fresh-id restart | hook_ownership | `crash_then_fresh_id_restart` |
| D1 outcome independence (write exhausted / NoWrite) | hook_ownership | `write_exhaustion_still_stops_probes` |
| D4 / B2 operator or liveness stop: effects, failed row write keeps the supervisor, retry, hook, no ExitEvent | hook_ownership | `operator_or_liveness_stop_emits_no_exit_event` |
| D4 finalize on a live VM | hook_ownership | `finalize_on_live_vm_stops_probes` |
| D2 double fire | hook_ownership | `double_fire_is_idempotent` |
| D5 crash after streak at max, then deadline | restart_authority | `crash_after_attempts_exhausted_then_deadline_passes` |
| D5 live path still stops | restart_authority | `running_alloc_with_exhausted_probes_past_deadline_is_stopped` |
| D6 pre-startup crash loop | restart_authority | `pre_startup_crash_loop_gives_up_early_exit` |
| D6 successor passes | restart_authority | `pre_startup_crash_then_successor_passes` |
| D6 post-pass crash loop | restart_authority | `crash_after_startup_pass_gives_up_backoff_exhausted` |
| D6 opt-out crash loop | restart_authority | `opt_out_service_crash_loop_gives_up_backoff_exhausted` |
| D6 non-crash failed row | restart_authority | `non_crash_failed_row_gives_up_backoff_exhausted` |
| D6 liveness-kill precedence | restart_authority | `liveness_kill_loop_gives_up_liveness_probe_failed` |
| D7 never passes, stop, restart, give-up | restart_authority | `startup_never_passes_loop_gives_up_startup_probe_failed` |
| R-2 latest startup row gates the stop (pass, fail, fail) | restart_authority | `pass_then_fails_before_stable_is_stopped` |
| D7 successor passes | restart_authority | `startup_stop_then_successor_passes_is_stable` |
| D7 last cause wins (both orders) | restart_authority | `startup_stop_then_crash_names_last_cause`, `crash_then_startup_stop_names_last_cause` |
| D7 not an intentional stop | restart_authority | `startup_stop_is_not_intentional_stop` |
| D8 failed stop retried on a new row | restart_authority | `failed_startup_stop_is_retried_on_next_row` |
| D8 late pass reaches Stable | restart_authority | `late_pass_after_failed_stop_reaches_stable` |
| D8 View reload keeps the acted run | restart_authority | `control_plane_restart_keeps_acted_run` |
| B2 a failed stop row write is retried (startup and liveness stop) | restart_authority | `stop_row_write_failure_is_retried` |
| D9 liveness failing during startup | liveness_gate | `liveness_failing_during_startup_is_not_stopped` |
| D9 starts at Stable, stops on failure | liveness_gate | `liveness_starts_at_stable_and_stops_on_failure` |
| D9 Stable after the end | liveness_gate | `stable_after_end_starts_no_liveness` |
| D9 opt-out | liveness_gate | `opt_out_service_starts_liveness_at_first_running_stable` |
| B1 a lost Stable write is re-announced | liveness_gate | `lost_stable_write_is_reannounced` |
| D10 slow start | progress_deadline | `slow_start_within_startup_deadline_reaches_stable_not_timeout` |
| D10 never ready | progress_deadline | `never_ready_times_out_with_last_attempt_cause` |
| D10 give-up inside the deadline | progress_deadline | `giveup_inside_deadline_wins_over_timeout` |
| D10 redeploy opens a new clock | progress_deadline | `redeploy_unchanged_opens_new_clock` |
| D11 redeploy of a Stable service | stream_snapshot | `redeploy_unchanged_of_stable_ends_stable` |
| D11 commit between subscribe and read | stream_snapshot | `terminal_between_subscribe_and_read_is_not_lost` |
| D11 restartable stop is not a verdict | stream_snapshot | `restartable_stop_recorded_is_not_a_verdict` |
| D11 superseded attempt | stream_snapshot | `superseded_attempt_terminal_is_not_reported` |
| D11 lag then relist | stream_snapshot | `lagged_relist_recovers_terminal` |
| D11 failed read (G-8 failure projection) | stream_snapshot | `failed_read_ends_with_error` |

`failed_read_ends_with_error` is the G-8 unavailable boundary (`FailedReadNeverWaits`, S-42); `control_plane_restart_keeps_acted_run` is
the G-5 replay boundary; `startup_stop_is_not_intentional_stop` is the G-5 unrelated-state boundary (a startup-stopped row is
restarted and an operator-stopped row beside it is not). Each is the run a conformance driver pins for a boundary
the design names. `commit before or after the read` (the D11 race) is covered by `terminal_between_subscribe_and_read_is_not_lost`
together with the exhaustive `NoMissedTerminal`.

## Hazards (`hazard/`, `expect = "violation"`; one per decision)

| Decision | Instance | Rule removed | Violated |
|---|---|---|---|
| D1 | `hooks_bugHookGatedOnRowWrite` | the observer fires the hook only after a successful row write | `EndedImpliesEventuallyUnsupervised` |
| D3 | `hooks_bugGateBeforeRegistration` | the gate is released before registration | `NoRegisterAfterEnd` |
| D5 | `ra_bugGuardAcceptsFailed` | the startup-failure guard accepts a `failed` row | `NoStartupVerdictOverCrash` |
| D6 | `ra_bugSlFinalisesCrash` | `ServiceLifecycle` finalises a crash row with `early_exit` while `WorkloadLifecycle` restarts it | `OneAuthorPerEnding` |
| D7 | `ra_bugSlFinalisesStartup` | `ServiceLifecycle` writes `startup_probe_failed` on a live row (no stop) | `StartupFailureNamedOnlyAtGiveUp` (and `OneAuthorPerEnding`) |
| B2 | `ra_bugHookBeforeRowWrite` | a stop's row-write failure has already removed the supervisor (the hook runs before the row write) | `NoStrandedStop` |
| D8 | `ra_bugStampAtEmission` | the dedup is `terminal_announced` membership, stamped at emission | `FailedStartupStopIsRetried` |
| B1 | `lg_bugStampAtEmission` | `Stable` emission is guarded by a record that a `Stable` write was attempted, set at emission | `StableEventuallyCommitted` |
| D9 | `lg_bugSpawnAtRunning` | `on_alloc_running` spawns the liveness tasks | `LivenessOnlyAfterStable` (and `NoLivenessStopBeforeStable`, `LivenessStreakStartsAtStable`) |
| D10 | `pd_bugFixedCap` (one stream slot) | the fixed 90 s cap with a startup deadline above it | `HealthySlowStartNeverTimesOut` |
| D11 | `ss_bugReadBeforeSubscribe` | read the recorded verdict, then subscribe | `NoMissedTerminal` |

No witnesses: every state of interest is reached by a named run. No Apalache checks: all instances are small and finite, TLC is
exhaustive.

## Assumptions (kernel, VMM, store, environment; each maps to a validation item or obligation)

| Id | Assumption | Discharged by |
|---|---|---|
| HO-1 | the gate send is the last effect of `release_for_exit_emission`; the watcher emits only after it | V5, S-4 |
| HO-2 | `VmDriver::stop` emits no `ExitEvent` | V1, S-6 |
| HO-3 | the hook is infallible, synchronous, idempotent; a no-op for an unknown id | V2, S-7 |
| HO-4 | a registered supervisor implies a held VM slot until the hook runs, except the state a failed stop row write creates (vacant slot, supervisor registered, row `Running`), which has no host residue and is retried by the next probe row | V7 |
| HO-5 | the observer's driver upgrade succeeds while the process runs | V27 (`exit_observer.rs:262-265`, D1 Q2) |
| RA-1 | the D1 hook ran at a crash or stop: the supervisor is gone and the startup streak stops growing | S-1, S-5, V2 |
| RA-2 | the shim's `StopAllocation` dispatch is awaited inside the `ServiceLifecycle` reconcile tick, so a stop's outcome is known before the next evaluation, and the arm's cleanup effects are idempotent when it runs again | V28 (reads `mtls_lifecycle.stop_alloc` and the other effects), S-30 (scripted `SimDriver::stop` failure); the production claim is probe-count PC-4 (`reconciler_runtime.rs:1604-1640`) |
| RA-3 | a terminal row is never deleted or rewritten by a successor (fresh ids) | V9 |
| RA-4 | a dead guest cannot produce a startup `Pass`; probe rows are written only while the supervisor is present | V18, S-30 |
| RA-5 | a probe row of a `Running` allocation arrives every interval after the deadline | V17 |
| LG-1 | `Stable` is committed before `on_alloc_stable` fires, on every Stable path | V19, S-31 |
| LG-2 | `start_liveness` is atomic with `stop_alloc` | V20, S-31 |
| LG-3 | the first liveness row lands one interval after the role goes live, streak 1 | S-31, S-32 |
| LG-4 | a committed `Stable` claim is never dropped while the allocation is `Running` (the premise that `committed` is monotone and that D12 reads) | V31, S-43 |
| DL-2 | the give-up reaches the bus no earlier than `6 x sd + 1` ticks | V15 |
| DL-3 | the stream receives every attempt-ending event published after it opened | V21 |
| DL-4 | the timer is the injected Clock, armed at stream open | S-35 |
| DL-6 | boot plus placement plus one evaluation is at most the 30 s margin (`PROGRESS_DEADLINE_MARGIN` = `VM_BOOT_DEADLINE`); scopes `HealthySlowStartNeverTimesOut`; where it fails the timeout is honest (exposure E-4) | V30, S-35 (f) |
| DL-5 | a CLI disconnect persists nothing; a redeploy opens a new stream with a new clock | S-35 (d) |
| SS-1 | the shim commits a row before it publishes the event for it | V23, S-41 |
| SS-2 | the stream receives every event published after it subscribed, except those a lagged receiver drops | V21, S-40 |
| SS-3 | `current_alloc` selects the current attempt | V25 |
| SS-4 | the recorded terminal equals the event's terminal | S-41 |
| SS-5 | the read and the relist are single atomic store scans | S-40 |
| SS-6 | an attempt's event is published before its successor is minted | V29 |

## Findings (underspecified points and exposures)

The architect's resolutions R-1 to R-4 (the reclaimed row, the latest startup row in the three gates, the E-2 exposure, the
SS-6 ordering) are recorded in the feature delta's § Pinned readings.

5. B2 modelling choices: `Crash` and `CleanExit` need a live VM slot (a guest whose VM was stopped by a failed-row-write stop
   cannot crash), and a startup `Pass` needs a live VM (RA-4). The liveness stop's retry is modelled as re-enabling the stop
   while the row is `Running` and Stable (the liveness row streak is `liveness_gate`'s). Neither is stated verbatim by the design;
   both follow V18 and ADR-0259's "probe rows against a dead guest are harmless".
6. Not modelled, stated by the design: a disconnected bus (`Closed` gives `StreamInterrupted`, G-8), backend membership and
   readiness, the Job kind, and the reconciler backoff (a restart is enabled any time after its terminal row). A clean exit
   (`Terminated`, `Stopped { Process }`) is modelled as restartable and budget-consuming because D6.b says it gives up as
   `BackoffExhausted` in production; D6 and D7 do not otherwise state its restart treatment.
7. Liveness-stop retry in conformance: the run `stop_row_write_failure_is_retried` drives the liveness stop's retry by a new
   liveness probe row. The model re-enables the stop while the row is `Running` and `Stable`; the streak is `liveness_gate`'s,
   so the conformance driver supplies the new liveness row through the recording prober.
