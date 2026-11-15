# Quint evidence — `reconciler-evaluation-wake`

Written by `cargo xtask quint check --subsystem reconciler-evaluation-wake --record`; do not edit. `cargo xtask quint verify-evidence --subsystem reconciler-evaluation-wake` checks it against the specs.

| | |
|---|---|
| Result | 15 of 15 check(s) matched `expect` |
| Run | `1210968-1794748185.169`, 2026-11-15T13:09:45.168973056Z → 2026-11-15T13:09:56.872643584Z |
| Jobs | 16 |
| Carried over | 13 check(s) were not re-run by this run: their inputs are unchanged since the run named in their `run_id` in summary.json |
| Git | `fb71b400fa42cbf2c0252e0e2697d6fac93f69db` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| evaluation-wake-safety | evaluation_wake.qnt | wake_full | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 38.9 | 0.0 | 38.9 |  |
| evaluation-wake-progress-by-pass | evaluation_wake.qnt | wake_full | temporal `ProgressByPass` | tlc | exhaustive | holds | holds | PASS | 1 | 120.9 | 0.0 | 120.9 |  |
| evaluation-wake-pending-is-admitted | evaluation_wake.qnt | wake_full | temporal `PendingIsAdmitted` | tlc | exhaustive | holds | holds | PASS | 1 | 232.4 | 0.0 | 232.4 |  |
| evaluation-wake-retention-schedules-backed-off-key | evaluation_wake.qnt | wake_retention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | holds | holds | PASS | 1 | 26.9 | 0.0 | 26.9 |  |
| evaluation-wake-hazard-no-pass | hazard/evaluation_wake_hazards.qnt | wake_noPass | temporal `ProgressByPass` | tlc | exhaustive | violation | violation | PASS | 1 | 11.4 | 0.0 | 11.4 | [traces/evaluation-wake-hazard-no-pass.txt](traces/evaluation-wake-hazard-no-pass.txt) [traces/evaluation-wake-hazard-no-pass.tlc.txt](traces/evaluation-wake-hazard-no-pass.tlc.txt) |
| evaluation-wake-hazard-no-retention | hazard/evaluation_wake_hazards.qnt | wake_noRetention | invariant `RetentionSchedulesBackedOffKey` | tlc | exhaustive | violation | violation | PASS | 1 | 9.1 | 0.0 | 9.1 | [traces/evaluation-wake-hazard-no-retention.txt](traces/evaluation-wake-hazard-no-retention.txt) [traces/evaluation-wake-hazard-no-retention.tlc.txt](traces/evaluation-wake-hazard-no-retention.tlc.txt) |
| evaluation-wake-run-healthy-edge-converges | evaluation_wake.qnt | wake_conf | run `healthy_edge_converges` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| evaluation-wake-run-hydrate-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `hydrate_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| evaluation-wake-run-persist-error-recovers-by-pass | evaluation_wake.qnt | wake_conf | run `persist_error_recovers_by_pass` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| evaluation-wake-run-crash-between-fsync-and-dispatch-recovers-by-boot-pass | evaluation_wake.qnt | wake_conf | run `crash_between_fsync_and_dispatch_recovers_by_boot_pass` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| evaluation-wake-run-failed-dataplane-inside-backoff-recovers-without-row-write | evaluation_wake.qnt | wake_conf | run `failed_dataplane_inside_backoff_recovers_without_row_write` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| evaluation-wake-run-fresh-deploy-without-row-is-evaluated | evaluation_wake.qnt | wake_conf | run `fresh_deploy_without_row_is_evaluated` | evaluator | run | holds | holds | PASS | 1 | 3.9 | 0.0 | 3.9 |  |
| evaluation-wake-run-enumeration-failure-of-one-reconciler-does-not-starve-others | evaluation_wake.qnt | wake_conf | run `enumeration_failure_of_one_reconciler_does_not_starve_others` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| evaluation-wake-run-edge-wake-during-burst-is-admitted-after-burst | evaluation_wake.qnt | wake_conf | run `edge_wake_during_burst_is_admitted_after_burst` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| evaluation-wake-run-pass-submit-merges-retained-not-before-to-immediate | evaluation_wake.qnt | wake_conf | run `pass_submit_merges_retained_not_before_to_immediate` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |

## Inputs (sha256)

- `checks.toml` `53b386766d06c2da9dfa7f02460636825dd756faed9d68256d25a34bfa813f09`
- `evaluation_wake.qnt` `ed0a98c7063a496882202b6f83af4c9f4c65fc45ef7b04785f2a952b87e32add`
- `hazard/evaluation_wake_hazards.qnt` `1c11a02650f850bce57f1fca46ed61b480868cfe6f81d0904cfb53e17fd78071`
