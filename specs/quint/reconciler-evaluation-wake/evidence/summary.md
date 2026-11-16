# Quint evidence — `reconciler-evaluation-wake`

Written by `cargo xtask quint check --subsystem reconciler-evaluation-wake --record`; do not edit. `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake` checks it against the specs.

| | |
|---|---|
| Result | 14 of 14 check(s) matched `expect` |
| Run | `2193479-1794826701.278`, 2026-11-16T10:58:21.278259968Z → 2026-11-16T10:59:18.407046144Z |
| Jobs | 16 |
| Git | `cd5dd64b64a4231f5f99c1741917d47e6ff5ceb9` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| evaluation-wake-progress-by-pass | evaluation_wake.qnt | wake_full | temporal `ProgressByPass` | tlc | exhaustive | holds | holds | PASS | 1 | 56.8 | 0.0 | 56.8 |  |
| evaluation-wake-retention-schedules-backed-off-key | evaluation_wake.qnt | wake_retention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | holds | holds | PASS | 1 | 17.5 | 0.0 | 17.5 |  |
| evaluation-wake-hazard-no-pass | hazard/evaluation_wake_hazards.qnt | wake_noPass | temporal `ProgressByPass` | tlc | exhaustive | violation | violation | PASS | 1 | 11.9 | 0.0 | 11.9 | [traces/evaluation-wake-hazard-no-pass.txt](traces/evaluation-wake-hazard-no-pass.txt) [traces/evaluation-wake-hazard-no-pass.tlc.txt](traces/evaluation-wake-hazard-no-pass.tlc.txt) |
| evaluation-wake-hazard-no-retention | hazard/evaluation_wake_hazards.qnt | wake_noRetention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | violation | violation | PASS | 1 | 9.9 | 0.0 | 9.9 | [traces/evaluation-wake-hazard-no-retention.txt](traces/evaluation-wake-hazard-no-retention.txt) [traces/evaluation-wake-hazard-no-retention.tlc.txt](traces/evaluation-wake-hazard-no-retention.tlc.txt) |
| evaluation-wake-run-healthy-edge-converges | evaluation_wake.qnt | wake_conf | run `healthy_edge_converges` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-hydrate-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `hydrate_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-persist-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `persist_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-crash-between-fsync-and-dispatch-recovers-by-boot-pass | evaluation_wake.qnt | wake_conf | run `crash_between_fsync_and_dispatch_recovers_by_boot_pass` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| evaluation-wake-run-failed-dataplane-inside-backoff-recovers-without-row-write | evaluation_wake.qnt | wake_conf | run `failed_dataplane_inside_backoff_recovers_without_row_write` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-fresh-deploy-without-row-is-evaluated | evaluation_wake.qnt | wake_conf | run `fresh_deploy_without_row_is_evaluated` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-enumeration-failure-of-one-reconciler-does-not-starve-others | evaluation_wake.qnt | wake_conf | run `enumeration_failure_of_one_reconciler_does_not_starve_others` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-edge-wake-during-burst-is-admitted-after-burst | evaluation_wake.qnt | wake_conf | run `edge_wake_during_burst_is_admitted_after_burst` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-pass-submit-merges-retained-not-before-to-immediate | evaluation_wake.qnt | wake_conf | run `pass_submit_merges_retained_not_before_to_immediate` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-world-changes-between-drain-and-finish | evaluation_wake.qnt | wake_conf | run `world_changes_between_drain_and_finish` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |

## Inputs (sha256)

- `checks.toml` `5b011c482d202a290122ffed2580de244614c3d11d8b74b07bafaa718a0e4c5c`
- `evaluation_wake.qnt` `5a5d43de00f0d6d5de46467a20a26295fab11e0888cae213406b887f9bdb212b`
- `hazard/evaluation_wake_hazards.qnt` `dc5d42126ae69c37a48e4ce56e8809d1b5993b84ac9197879a655e0d245c9ce0`
