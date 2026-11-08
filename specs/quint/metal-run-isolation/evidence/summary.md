# Quint evidence — `metal-run-isolation`

Written by `cargo xtask quint check --subsystem metal-run-isolation --record`; do not edit. `cargo xtask quint verify-evidence --subsystem metal-run-isolation` checks it against the specs.

| | |
|---|---|
| Result | 18 of 18 check(s) matched `expect` |
| Run | `47182-1794138494.593`, 2026-11-08T11:48:14.592866048Z → 2026-11-08T11:52:58.767677952Z |
| Jobs | 3 |
| Git | `c74f1e95ce6b3c5acf24fa736bebff99c73c0864` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.11" 2026-04-21 |
| Kernel | 25.1.0 |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| runner-safety | runner_protocol.qnt | runner_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 52.8 | 0.0 | 52.8 |  |
| runner-owners-progress | runner_protocol.qnt | runner_ok | temporal `OwnersEventuallyEnd` | tlc | exhaustive | holds | holds | PASS | 1 | 117.7 | 0.0 | 117.7 |  |
| runner-maintenance-progress | runner_protocol.qnt | runner_ok | temporal `MaintenanceEventuallyResolves` | tlc | exhaustive | holds | holds | PASS | 1 | 83.4 | 0.0 | 83.4 |  |
| runner-hazard-exclusive-ordinary | hazard/runner_hazards.qnt | runner_bugOrdinaryExclusive | invariant `OrdinaryDoesNotBlockOrdinary` | tlc | exhaustive | violation | violation | PASS | 1 | 9.9 | 0.0 | 9.9 | [traces/runner-hazard-exclusive-ordinary.txt](traces/runner-hazard-exclusive-ordinary.txt) [traces/runner-hazard-exclusive-ordinary.tlc.txt](traces/runner-hazard-exclusive-ordinary.tlc.txt) |
| runner-hazard-collect-at-terminal | hazard/runner_hazards.qnt | runner_bugCollectAtTerminal | invariant `NoPrematureCommandCompletion` | tlc | exhaustive | violation | violation | PASS | 1 | 10.2 | 0.0 | 10.2 | [traces/runner-hazard-collect-at-terminal.txt](traces/runner-hazard-collect-at-terminal.txt) [traces/runner-hazard-collect-at-terminal.tlc.txt](traces/runner-hazard-collect-at-terminal.tlc.txt) |
| runner-run-independent-worktrees | runner_protocol.qnt | runner_conf | run `independent_worktrees` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-same-checkout-excludes-sync | runner_protocol.qnt | runner_conf | run `same_checkout_excludes_sync` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-controller-eof-before-start | runner_protocol.qnt | runner_conf | run `controller_eof_before_start` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-controller-loss-after-start | runner_protocol.qnt | runner_conf | run `controller_loss_after_start` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-supervisor-loss-is-not-a-result | runner_protocol.qnt | runner_conf | run `supervisor_loss_is_not_a_result` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| runner-run-cleanup-failure-keeps-command | runner_protocol.qnt | runner_conf | run `cleanup_failure_keeps_command` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-missing-completion-refuses | runner_protocol.qnt | runner_conf | run `missing_completion_refuses` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-maintenance-waits-for-stop | runner_protocol.qnt | runner_conf | run `maintenance_waits_for_stop` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-no-sync-mismatch-refuses | runner_protocol.qnt | runner_conf | run `no_sync_mismatch_refuses` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-repeat-collection-agrees | runner_protocol.qnt | runner_conf | run `repeat_collection_agrees` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-admission-timeout | runner_protocol.qnt | runner_conf | run `admission_timeout` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-preflight-aborts-before-start | runner_protocol.qnt | runner_conf | run `preflight_aborts_before_start` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |
| runner-run-missing-rsync-aborts-before-start | runner_protocol.qnt | runner_conf | run `missing_rsync_aborts_before_start` | evaluator | run | holds | holds | PASS | 1 | 0.8 | 0.0 | 0.8 |  |

## Inputs (sha256)

- `checks.toml` `aca65dde0e79ac0dd4a19f0f5b95ebce7725e418f841f3e53dff5eef6b452b16`
- `hazard/runner_hazards.qnt` `907af15a80388e355818f249abc69fbd3b339b20867266b3d9174fa15daeb2f7`
- `runner_protocol.qnt` `f67ed362807560c981a60be33d127106f47329a290f5148add70fadcbd732b5e`
