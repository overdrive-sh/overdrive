# Quint evidence — `reconciler-evaluation-wake`

Written by `cargo xtask quint check --subsystem reconciler-evaluation-wake --record`; do not edit. `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake` checks it against the specs.

| | |
|---|---|
| Result | 15 of 15 check(s) matched `expect` |
| Run | `437712-1794780120.923`, 2026-11-15T22:02:00.923237632Z → 2026-11-15T22:03:05.22928384Z |
| Jobs | 16 |
| Git | `97a417fcb3885033faa120d208fd15b0dd39db32` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| evaluation-wake-safety | evaluation_wake.qnt | wake_full | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 25.9 | 0.0 | 25.9 |  |
| evaluation-wake-progress-by-pass | evaluation_wake.qnt | wake_full | temporal `ProgressByPass` | tlc | exhaustive | holds | holds | PASS | 1 | 63.9 | 0.0 | 63.9 |  |
| evaluation-wake-retention-schedules-backed-off-key | evaluation_wake.qnt | wake_retention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | holds | holds | PASS | 1 | 18.3 | 0.0 | 18.3 |  |
| evaluation-wake-hazard-no-pass | hazard/evaluation_wake_hazards.qnt | wake_noPass | temporal `ProgressByPass` | tlc | exhaustive | violation | violation | PASS | 1 | 12.7 | 0.0 | 12.7 | [traces/evaluation-wake-hazard-no-pass.txt](traces/evaluation-wake-hazard-no-pass.txt) [traces/evaluation-wake-hazard-no-pass.tlc.txt](traces/evaluation-wake-hazard-no-pass.tlc.txt) |
| evaluation-wake-hazard-no-retention | hazard/evaluation_wake_hazards.qnt | wake_noRetention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | violation | violation | PASS | 1 | 12.7 | 0.0 | 12.7 | [traces/evaluation-wake-hazard-no-retention.txt](traces/evaluation-wake-hazard-no-retention.txt) [traces/evaluation-wake-hazard-no-retention.tlc.txt](traces/evaluation-wake-hazard-no-retention.tlc.txt) |
| evaluation-wake-run-healthy-edge-converges | evaluation_wake.qnt | wake_conf | run `healthy_edge_converges` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| evaluation-wake-run-hydrate-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `hydrate_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-persist-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `persist_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-crash-between-fsync-and-dispatch-recovers-by-boot-pass | evaluation_wake.qnt | wake_conf | run `crash_between_fsync_and_dispatch_recovers_by_boot_pass` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-failed-dataplane-inside-backoff-recovers-without-row-write | evaluation_wake.qnt | wake_conf | run `failed_dataplane_inside_backoff_recovers_without_row_write` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-fresh-deploy-without-row-is-evaluated | evaluation_wake.qnt | wake_conf | run `fresh_deploy_without_row_is_evaluated` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| evaluation-wake-run-enumeration-failure-of-one-reconciler-does-not-starve-others | evaluation_wake.qnt | wake_conf | run `enumeration_failure_of_one_reconciler_does_not_starve_others` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-edge-wake-during-burst-is-admitted-after-burst | evaluation_wake.qnt | wake_conf | run `edge_wake_during_burst_is_admitted_after_burst` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-pass-submit-merges-retained-not-before-to-immediate | evaluation_wake.qnt | wake_conf | run `pass_submit_merges_retained_not_before_to_immediate` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| evaluation-wake-run-world-changes-between-drain-and-finish | evaluation_wake.qnt | wake_conf | run `world_changes_between_drain_and_finish` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |

## Inputs (sha256)

- `checks.toml` `f482196bdf9f1e3937e4c106e28503e74bbb8f70719fba7573c9aef3ae4b956b`
- `evaluation_wake.qnt` `401886c2533daf744773cc1bed1ea19b84858bcbc445b6a9f46b228afbe4d487`
- `hazard/evaluation_wake_hazards.qnt` `1c11a02650f850bce57f1fca46ed61b480868cfe6f81d0904cfb53e17fd78071`
