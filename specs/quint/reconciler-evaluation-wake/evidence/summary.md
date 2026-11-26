# Quint evidence — `reconciler-evaluation-wake`

Written by `cargo xtask quint check --subsystem reconciler-evaluation-wake --record`; do not edit. `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake` checks it against the specs.

| | |
|---|---|
| Result | 15 of 15 check(s) matched `expect` |
| Run | `1801623-1795664029.276`, 2026-11-26T03:33:49.275625472Z → 2026-11-26T03:37:46.119592704Z |
| Jobs | 16 |
| Git | `809023b0c19ed95db1ef9263434296f76db45613` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| evaluation-wake-progress-by-pass | evaluation_wake.qnt | wake_full | temporal `ProgressByPass` | tlc | exhaustive | holds | holds | PASS | 1 | 182.3 | 0.0 | 182.3 |  |
| evaluation-wake-retention-schedules-backed-off-key | evaluation_wake.qnt | wake_retention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | holds | holds | PASS | 1 | 54.2 | 0.0 | 54.2 |  |
| evaluation-wake-hazard-no-pass | hazard/evaluation_wake_hazards.qnt | wake_noPass | temporal `ProgressByPass` | tlc | exhaustive | violation | violation | PASS | 1 | 14.7 | 0.0 | 14.7 | [traces/evaluation-wake-hazard-no-pass.txt](traces/evaluation-wake-hazard-no-pass.txt) [traces/evaluation-wake-hazard-no-pass.tlc.txt](traces/evaluation-wake-hazard-no-pass.tlc.txt) |
| evaluation-wake-hazard-no-retention | hazard/evaluation_wake_hazards.qnt | wake_noRetention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | violation | violation | PASS | 1 | 11.7 | 0.0 | 11.7 | [traces/evaluation-wake-hazard-no-retention.txt](traces/evaluation-wake-hazard-no-retention.txt) [traces/evaluation-wake-hazard-no-retention.tlc.txt](traces/evaluation-wake-hazard-no-retention.tlc.txt) |
| evaluation-wake-run-healthy-edge-converges | evaluation_wake.qnt | wake_conf | run `healthy_edge_converges` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| evaluation-wake-run-hydrate-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `hydrate_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| evaluation-wake-run-persist-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `persist_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-crash-between-fsync-and-dispatch-recovers-by-boot-pass | evaluation_wake.qnt | wake_conf | run `crash_between_fsync_and_dispatch_recovers_by_boot_pass` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| evaluation-wake-run-failed-dataplane-inside-backoff-recovers-without-row-write | evaluation_wake.qnt | wake_conf | run `failed_dataplane_inside_backoff_recovers_without_row_write` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-fresh-deploy-without-row-is-evaluated | evaluation_wake.qnt | wake_conf | run `fresh_deploy_without_row_is_evaluated` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-enumeration-failure-of-one-reconciler-does-not-starve-others | evaluation_wake.qnt | wake_conf | run `enumeration_failure_of_one_reconciler_does_not_starve_others` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-edge-wake-during-burst-is-admitted-after-burst | evaluation_wake.qnt | wake_conf | run `edge_wake_during_burst_is_admitted_after_burst` | evaluator | run | holds | holds | PASS | 1 | 3.5 | 0.0 | 3.5 |  |
| evaluation-wake-run-pass-submit-merges-retained-not-before-to-immediate | evaluation_wake.qnt | wake_conf | run `pass_submit_merges_retained_not_before_to_immediate` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-world-changes-between-drain-and-finish | evaluation_wake.qnt | wake_conf | run `world_changes_between_drain_and_finish` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-empty-published-row-then-service-change-requeues-hydrator | evaluation_wake.qnt | wake_conf | run `empty_published_row_then_service_change_requeues_hydrator` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |

## Inputs (sha256)

- `checks.toml` `6ec6f1d0a9a10425d12d38c03afe503ad8ff2650bd983d1fcfab3960b83b881e`
- `evaluation_wake.qnt` `7532082e027003225eefa7c7d5a156b65123f5d48c1cd972981dee7c675d061c`
- `hazard/evaluation_wake_hazards.qnt` `1400eabddea8942da71f2b82f4c0e751308c861ae1c15ba4246e90dcdfe1b97a`
