# Quint evidence — `service-allocation-readiness-and-ending`

Written by `cargo xtask quint check --subsystem service-allocation-readiness-and-ending --record`; do not edit. `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` checks it against the specs.

| | |
|---|---|
| Result | 84 of 84 check(s) matched `expect` |
| Run | `2701863-1794536983.789`, 2026-11-13T02:29:43.788714752Z → 2026-11-13T02:31:42.755789824Z |
| Jobs | 16 |
| Git | `e4635dd3fb536ba71d3034ce180ee7e4f3475f5b` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| hooks-no-register-after-end | hook_ownership.qnt | hooks_ok | invariant `NoRegisterAfterEnd` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| hooks-ended-implies-eventually-unsupervised | hook_ownership.qnt | hooks_ok | temporal `EndedImpliesEventuallyUnsupervised` | tlc | exhaustive | holds | holds | PASS | 1 | 7.3 | 0.0 | 7.3 |  |
| hooks-hazard-hook-gated-on-row-write | hazard/hook_ownership_hazards.qnt | hooks_bugHookGatedOnRowWrite | temporal `EndedImpliesEventuallyUnsupervised` | tlc | exhaustive | violation | violation | PASS | 1 | 7.3 | 0.0 | 7.3 | [traces/hooks-hazard-hook-gated-on-row-write.txt](traces/hooks-hazard-hook-gated-on-row-write.txt) [traces/hooks-hazard-hook-gated-on-row-write.tlc.txt](traces/hooks-hazard-hook-gated-on-row-write.tlc.txt) |
| hooks-hazard-gate-before-registration | hazard/hook_ownership_hazards.qnt | hooks_bugGateBeforeRegistration | invariant `NoRegisterAfterEnd` | tlc | exhaustive | violation | violation | PASS | 1 | 7.1 | 0.0 | 7.1 | [traces/hooks-hazard-gate-before-registration.txt](traces/hooks-hazard-gate-before-registration.txt) [traces/hooks-hazard-gate-before-registration.tlc.txt](traces/hooks-hazard-gate-before-registration.tlc.txt) |
| hooks-run-crash-without-restart | hook_ownership.qnt | hooks_conf | run `crash_without_restart` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| hooks-run-crash-then-fresh-id-restart | hook_ownership.qnt | hooks_conf | run `crash_then_fresh_id_restart` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| hooks-run-write-exhaustion-still-stops-probes | hook_ownership.qnt | hooks_conf | run `write_exhaustion_still_stops_probes` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| hooks-run-operator-or-liveness-stop-emits-no-exit-event | hook_ownership.qnt | hooks_conf | run `operator_or_liveness_stop_emits_no_exit_event` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| hooks-run-finalize-on-live-vm-stops-probes | hook_ownership.qnt | hooks_conf | run `finalize_on_live_vm_stops_probes` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| hooks-run-double-fire-is-idempotent | hook_ownership.qnt | hooks_conf | run `double_fire_is_idempotent` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| ra-no-startup-verdict-over-crash | restart_authority.qnt | ra_ok | invariant `NoStartupVerdictOverCrash` | tlc | exhaustive | holds | holds | PASS | 1 | 28.1 | 0.0 | 28.1 |  |
| ra-one-author-per-ending | restart_authority.qnt | ra_ok | invariant `OneAuthorPerEnding` | tlc | exhaustive | holds | holds | PASS | 1 | 28.1 | 0.0 | 28.1 |  |
| ra-startup-failure-named-only-at-give-up | restart_authority.qnt | ra_ok | invariant `StartupFailureNamedOnlyAtGiveUp` | tlc | exhaustive | holds | holds | PASS | 1 | 28.6 | 0.0 | 28.6 |  |
| ra-no-stable-over-pending-stop | restart_authority.qnt | ra_ok | invariant `NoStableOverPendingStop` | tlc | exhaustive | holds | holds | PASS | 1 | 24.6 | 0.0 | 24.6 |  |
| ra-crashed-alloc-eventually-decided-by-restart-authority | restart_authority.qnt | ra_ok | temporal `CrashedAllocEventuallyDecidedByRestartAuthority` | tlc | exhaustive | holds | holds | PASS | 1 | 33.5 | 0.0 | 33.5 |  |
| ra-failed-startup-stop-is-retried | restart_authority.qnt | ra_ok | temporal `FailedStartupStopIsRetried` | tlc | exhaustive | holds | holds | PASS | 1 | 28.2 | 0.0 | 28.2 |  |
| ra-late-pass-unstrands-the-instance | restart_authority.qnt | ra_ok | temporal `LatePassUnstrandsTheInstance` | tlc | exhaustive | holds | holds | PASS | 1 | 26.9 | 0.0 | 26.9 |  |
| ra-hazard-guard-accepts-failed | hazard/restart_authority_hazards.qnt | ra_bugGuardAcceptsFailed | invariant `NoStartupVerdictOverCrash` | tlc | exhaustive | violation | violation | PASS | 1 | 12.6 | 0.0 | 12.6 | [traces/ra-hazard-guard-accepts-failed.txt](traces/ra-hazard-guard-accepts-failed.txt) [traces/ra-hazard-guard-accepts-failed.tlc.txt](traces/ra-hazard-guard-accepts-failed.tlc.txt) |
| ra-hazard-sl-finalises-crash | hazard/restart_authority_hazards.qnt | ra_bugSlFinalisesCrash | invariant `OneAuthorPerEnding` | tlc | exhaustive | violation | violation | PASS | 1 | 11.1 | 0.0 | 11.1 | [traces/ra-hazard-sl-finalises-crash.txt](traces/ra-hazard-sl-finalises-crash.txt) [traces/ra-hazard-sl-finalises-crash.tlc.txt](traces/ra-hazard-sl-finalises-crash.tlc.txt) |
| ra-hazard-sl-finalises-startup | hazard/restart_authority_hazards.qnt | ra_bugSlFinalisesStartup | invariant `StartupFailureNamedOnlyAtGiveUp` | tlc | exhaustive | violation | violation | PASS | 1 | 11.6 | 0.0 | 11.6 | [traces/ra-hazard-sl-finalises-startup.txt](traces/ra-hazard-sl-finalises-startup.txt) [traces/ra-hazard-sl-finalises-startup.tlc.txt](traces/ra-hazard-sl-finalises-startup.tlc.txt) |
| ra-hazard-stamp-at-emission | hazard/restart_authority_hazards.qnt | ra_bugStampAtEmission | temporal `FailedStartupStopIsRetried` | tlc | exhaustive | violation | violation | PASS | 1 | 13.9 | 0.0 | 13.9 | [traces/ra-hazard-stamp-at-emission.txt](traces/ra-hazard-stamp-at-emission.txt) [traces/ra-hazard-stamp-at-emission.tlc.txt](traces/ra-hazard-stamp-at-emission.tlc.txt) |
| ra-run-crash-after-attempts-exhausted-then-deadline-passes | restart_authority.qnt | ra_conf | run `crash_after_attempts_exhausted_then_deadline_passes` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| ra-run-running-alloc-with-exhausted-probes-past-deadline-is-stopped | restart_authority.qnt | ra_conf | run `running_alloc_with_exhausted_probes_past_deadline_is_stopped` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| ra-run-pre-startup-crash-loop-gives-up-early-exit | restart_authority.qnt | ra_conf | run `pre_startup_crash_loop_gives_up_early_exit` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-pre-startup-crash-then-successor-passes | restart_authority.qnt | ra_conf | run `pre_startup_crash_then_successor_passes` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-crash-after-startup-pass-gives-up-backoff-exhausted | restart_authority.qnt | ra_conf | run `crash_after_startup_pass_gives_up_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| ra-run-opt-out-service-crash-loop-gives-up-backoff-exhausted | restart_authority.qnt | ra_conf | run `opt_out_service_crash_loop_gives_up_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| ra-run-non-crash-failed-row-gives-up-backoff-exhausted | restart_authority.qnt | ra_conf | run `non_crash_failed_row_gives_up_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| ra-run-liveness-kill-loop-gives-up-liveness-probe-failed | restart_authority.qnt | ra_conf | run `liveness_kill_loop_gives_up_liveness_probe_failed` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-startup-never-passes-loop-gives-up-startup-probe-failed | restart_authority.qnt | ra_conf | run `startup_never_passes_loop_gives_up_startup_probe_failed` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-startup-stop-then-successor-passes-is-stable | restart_authority.qnt | ra_conf | run `startup_stop_then_successor_passes_is_stable` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-startup-stop-then-crash-names-last-cause | restart_authority.qnt | ra_conf | run `startup_stop_then_crash_names_last_cause` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ra-run-crash-then-startup-stop-names-last-cause | restart_authority.qnt | ra_conf | run `crash_then_startup_stop_names_last_cause` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| ra-run-startup-stop-is-not-intentional-stop | restart_authority.qnt | ra_conf | run `startup_stop_is_not_intentional_stop` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| ra-run-failed-startup-stop-is-retried-on-next-row | restart_authority.qnt | ra_conf | run `failed_startup_stop_is_retried_on_next_row` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| ra-run-late-pass-after-failed-stop-reaches-stable | restart_authority.qnt | ra_conf | run `late_pass_after_failed_stop_reaches_stable` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| ra-run-control-plane-restart-keeps-acted-run | restart_authority.qnt | ra_conf | run `control_plane_restart_keeps_acted_run` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| lg-liveness-only-after-stable | liveness_gate.qnt | lg_ok | invariant `LivenessOnlyAfterStable` | tlc | exhaustive | holds | holds | PASS | 1 | 6.1 | 0.0 | 6.1 |  |
| lg-no-liveness-stop-before-stable | liveness_gate.qnt | lg_ok | invariant `NoLivenessStopBeforeStable` | tlc | exhaustive | holds | holds | PASS | 1 | 6.3 | 0.0 | 6.3 |  |
| lg-no-liveness-for-ended-id | liveness_gate.qnt | lg_ok | invariant `NoLivenessForEndedId` | tlc | exhaustive | holds | holds | PASS | 1 | 6.3 | 0.0 | 6.3 |  |
| lg-liveness-streak-starts-at-stable | liveness_gate.qnt | lg_ok | invariant `LivenessStreakStartsAtStable` | tlc | exhaustive | holds | holds | PASS | 1 | 6.3 | 0.0 | 6.3 |  |
| lg-hazard-spawn-at-running | hazard/liveness_gate_hazards.qnt | lg_bugSpawnAtRunning | invariant `LivenessOnlyAfterStable` | tlc | exhaustive | violation | violation | PASS | 1 | 6.3 | 0.0 | 6.3 | [traces/lg-hazard-spawn-at-running.txt](traces/lg-hazard-spawn-at-running.txt) [traces/lg-hazard-spawn-at-running.tlc.txt](traces/lg-hazard-spawn-at-running.tlc.txt) |
| lg-run-liveness-failing-during-startup-is-not-stopped | liveness_gate.qnt | lg_conf | run `liveness_failing_during_startup_is_not_stopped` | evaluator | run | holds | holds | PASS | 1 | 1.6 | 0.0 | 1.6 |  |
| lg-run-liveness-starts-at-stable-and-stops-on-failure | liveness_gate.qnt | lg_conf | run `liveness_starts_at_stable_and_stops_on_failure` | evaluator | run | holds | holds | PASS | 1 | 1.6 | 0.0 | 1.6 |  |
| lg-run-stable-after-end-starts-no-liveness | liveness_gate.qnt | lg_conf | run `stable_after_end_starts_no_liveness` | evaluator | run | holds | holds | PASS | 1 | 1.6 | 0.0 | 1.6 |  |
| lg-run-opt-out-service-starts-liveness-at-first-running-stable | liveness_gate.qnt | lg_conf | run `opt_out_service_starts_liveness_at_first_running_stable` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| pd-healthy-slow-start-never-times-out | progress_deadline.qnt | pd_ok | invariant `HealthySlowStartNeverTimesOut` | tlc | exhaustive | holds | holds | PASS | 1 | 8.1 | 0.0 | 8.1 |  |
| pd-hazard-fixed-cap | hazard/progress_deadline_hazards.qnt | pd_bugFixedCap | invariant `HealthySlowStartNeverTimesOut` | tlc | exhaustive | violation | violation | PASS | 1 | 7.3 | 0.0 | 7.3 | [traces/pd-hazard-fixed-cap.txt](traces/pd-hazard-fixed-cap.txt) [traces/pd-hazard-fixed-cap.tlc.txt](traces/pd-hazard-fixed-cap.tlc.txt) |
| pd-run-slow-start-within-startup-deadline-reaches-stable-not-timeout | progress_deadline.qnt | pd_conf | run `slow_start_within_startup_deadline_reaches_stable_not_timeout` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| pd-run-never-ready-times-out-with-last-attempt-cause | progress_deadline.qnt | pd_conf | run `never_ready_times_out_with_last_attempt_cause` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| pd-run-giveup-inside-deadline-wins-over-timeout | progress_deadline.qnt | pd_conf | run `giveup_inside_deadline_wins_over_timeout` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| pd-run-redeploy-unchanged-opens-new-clock | progress_deadline.qnt | pd_conf | run `redeploy_unchanged_opens_new_clock` | evaluator | run | holds | holds | PASS | 1 | 1.8 | 0.0 | 1.8 |  |
| ss-stream-ends-once-honestly | stream_snapshot.qnt | ss_ok | invariant `StreamEndsOnceHonestly` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| ss-restartable-stop-never-ends-stream | stream_snapshot.qnt | ss_ok | invariant `RestartableStopNeverEndsStream` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| ss-failed-read-never-waits | stream_snapshot.qnt | ss_ok | invariant `FailedReadNeverWaits` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| ss-no-missed-terminal | stream_snapshot.qnt | ss_ok | temporal `NoMissedTerminal` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| ss-hazard-read-before-subscribe | hazard/stream_snapshot_hazards.qnt | ss_bugReadBeforeSubscribe | temporal `NoMissedTerminal` | tlc | exhaustive | violation | violation | PASS | 1 | 7.3 | 0.0 | 7.3 | [traces/ss-hazard-read-before-subscribe.txt](traces/ss-hazard-read-before-subscribe.txt) [traces/ss-hazard-read-before-subscribe.tlc.txt](traces/ss-hazard-read-before-subscribe.tlc.txt) |
| ss-run-redeploy-unchanged-of-stable-ends-stable | stream_snapshot.qnt | ss_conf | run `redeploy_unchanged_of_stable_ends_stable` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| ss-run-terminal-between-subscribe-and-read-is-not-lost | stream_snapshot.qnt | ss_conf | run `terminal_between_subscribe_and_read_is_not_lost` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| ss-run-restartable-stop-recorded-is-not-a-verdict | stream_snapshot.qnt | ss_conf | run `restartable_stop_recorded_is_not_a_verdict` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| ss-run-superseded-attempt-terminal-is-not-reported | stream_snapshot.qnt | ss_conf | run `superseded_attempt_terminal_is_not_reported` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| ss-run-lagged-relist-recovers-terminal | stream_snapshot.qnt | ss_conf | run `lagged_relist_recovers_terminal` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| ss-run-failed-read-ends-with-error | stream_snapshot.qnt | ss_conf | run `failed_read_ends_with_error` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| ra-run-pass-then-fails-before-stable-is-stopped | restart_authority.qnt | ra_conf | run `pass_then_fails_before_stable_is_stopped` | evaluator | run | holds | holds | PASS | 1 | 4.1 | 0.0 | 4.1 |  |
| lg-stable-eventually-committed | liveness_gate.qnt | lg_ok | temporal `StableEventuallyCommitted` | tlc | exhaustive | holds | holds | PASS | 1 | 6.3 | 0.0 | 6.3 |  |
| lg-hazard-stamp-at-emission | hazard/liveness_gate_hazards.qnt | lg_bugStampAtEmission | temporal `StableEventuallyCommitted` | tlc | exhaustive | violation | violation | PASS | 1 | 6.3 | 0.0 | 6.3 | [traces/lg-hazard-stamp-at-emission.txt](traces/lg-hazard-stamp-at-emission.txt) [traces/lg-hazard-stamp-at-emission.tlc.txt](traces/lg-hazard-stamp-at-emission.tlc.txt) |
| lg-run-lost-stable-write-is-reannounced | liveness_gate.qnt | lg_conf | run `lost_stable_write_is_reannounced` | evaluator | run | holds | holds | PASS | 1 | 1.5 | 0.0 | 1.5 |  |
| ra-no-stranded-stop | restart_authority.qnt | ra_ok | invariant `NoStrandedStop` | tlc | exhaustive | holds | holds | PASS | 1 | 20.5 | 0.0 | 20.5 |  |
| ra-hazard-hook-before-row-write | hazard/restart_authority_hazards.qnt | ra_bugHookBeforeRowWrite | invariant `NoStrandedStop` | tlc | exhaustive | violation | violation | PASS | 1 | 11.6 | 0.0 | 11.6 | [traces/ra-hazard-hook-before-row-write.txt](traces/ra-hazard-hook-before-row-write.txt) [traces/ra-hazard-hook-before-row-write.tlc.txt](traces/ra-hazard-hook-before-row-write.tlc.txt) |
| ra-run-stop-row-write-failure-is-retried | restart_authority.qnt | ra_conf | run `stop_row_write_failure_is_retried` | evaluator | run | holds | holds | PASS | 1 | 4.1 | 0.0 | 4.1 |  |
| gr-listen-before-exit | guest_report.qnt | gr_ok | invariant `ListenBeforeExit` | tlc | exhaustive | holds | holds | PASS | 1 | 7.3 | 0.0 | 7.3 |  |
| gr-delivered-before-exit | guest_report.qnt | gr_ok | invariant `DeliveredBeforeExit` | tlc | exhaustive | holds | holds | PASS | 1 | 7.8 | 0.0 | 7.9 |  |
| gr-exit-eventually-reported | guest_report.qnt | gr_ok | temporal `ExitEventuallyReported` | tlc | exhaustive | holds | holds | PASS | 1 | 8.1 | 0.0 | 8.1 |  |
| gr-hazard-no-drain-before-exit | hazard/guest_report_hazards.qnt | gr_bugNoDrainBeforeExit | invariant `DeliveredBeforeExit` | tlc | exhaustive | violation | violation | PASS | 1 | 7.3 | 0.0 | 7.3 | [traces/gr-hazard-no-drain-before-exit.txt](traces/gr-hazard-no-drain-before-exit.txt) [traces/gr-hazard-no-drain-before-exit.tlc.txt](traces/gr-hazard-no-drain-before-exit.tlc.txt) |
| gr-run-listen-then-exit-drains-first | guest_report.qnt | gr_conf | run `listen_then_exit_drains_first` | evaluator | run | holds | holds | PASS | 1 | 5.6 | 0.0 | 5.6 |  |
| gr-run-ring-drop-leaves-set-partial | guest_report.qnt | gr_conf | run `ring_drop_leaves_set_partial` | evaluator | run | holds | holds | PASS | 1 | 5.9 | 0.0 | 5.9 |  |
| gr-run-host-all-declared-reported | guest_report.qnt | gr_host_conf | run `host_all_declared_reported` | evaluator | run | holds | holds | PASS | 1 | 4.9 | 0.0 | 4.9 |  |
| gr-run-host-garbage-line-is-dropped | guest_report.qnt | gr_host_conf | run `host_garbage_line_is_dropped` | evaluator | run | holds | holds | PASS | 1 | 4.8 | 0.0 | 4.8 |  |
| gr-run-host-beacon-closes-with-partial-set | guest_report.qnt | gr_host_conf | run `host_beacon_closes_with_partial_set` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| gr-run-host-listen-after-exit-is-ignored | guest_report.qnt | gr_host_conf | run `host_listen_after_exit_is_ignored` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| ra-run-first-fail-at-deadline-with-no-earlier-row-stops | restart_authority.qnt | ra_conf | run `first_fail_at_deadline_with_no_earlier_row_stops` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| ra-run-explicit-probe-fail-after-deadline-keeps-counted-streak | restart_authority.qnt | ra_conf | run `explicit_probe_fail_after_deadline_keeps_counted_streak` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| ra-run-crashed-and-rejected-allocs-join-terminal-announced | restart_authority.qnt | ra_conf | run `crashed_and_rejected_allocs_join_terminal_announced` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| ss-run-publish-after-relist-reaches-channel | stream_snapshot.qnt | ss_conf | run `publish_after_relist_reaches_channel` | evaluator | run | holds | holds | PASS | 1 | 1.3 | 0.0 | 1.3 |  |

## Inputs (sha256)

- `checks.toml` `e5569b54093c32619d466cb0680e52394d74bcf4753cb66bf312162d8cbc6d64`
- `guest_report.qnt` `4efde0c1edae3ce198b5b6daa7b6164eceb5e92b1536c67a28e107b6b21e2a29`
- `hazard/guest_report_hazards.qnt` `c84c6c8b77c5f2b05730e60bac9c5d560e0c4e6a105d1a57ed0339564a76faec`
- `hazard/hook_ownership_hazards.qnt` `6e900656e0a281938f43a30fa844dcc44cba66c4aa0fadd48647eff49f6de7e7`
- `hazard/liveness_gate_hazards.qnt` `13dcd004b9d8914aafce4dfde0cdb2ca121d6569f24d923a44b9f775324956bd`
- `hazard/progress_deadline_hazards.qnt` `a17f31289bec7c2e39c0a5b55973c9892a6a7e2192b10e8e3f242a1908857889`
- `hazard/restart_authority_hazards.qnt` `d9500f5f70451951cd5eceb6447103fbc03f6416337bac7e6eb0c9b6c63d895a`
- `hazard/stream_snapshot_hazards.qnt` `f65b2af7b3899703ec222008d7a59f1011aa3bafb8ece35256bf2330c82d85e8`
- `hook_ownership.qnt` `c6e990eaf29adfc8b698d07a568c01408af0e77f19721e9a626142b0c5662864`
- `liveness_gate.qnt` `fd5c89476df752c9b1e87fabedaab9c1897702f3c98f9ab668eaaa5d1cf08b38`
- `progress_deadline.qnt` `051add9635ecd632a1b68bba01b6131958a36b8b8408956977177e37867f859a`
- `restart_authority.qnt` `6d666e2eddbc973ff41aa7cac1ec015eea23bbcd4fbd1bca5c6b7732be9e9769`
- `stream_snapshot.qnt` `221f32fb71d5b87ddca703316c269f24bfa0c02fd652723bc3952ce3508f41d6`
