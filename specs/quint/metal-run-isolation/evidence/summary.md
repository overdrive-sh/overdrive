# Quint evidence — `metal-run-isolation`

Written by `cargo xtask quint check --subsystem metal-run-isolation --record`; do not edit. `cargo xtask quint verify-evidence --subsystem metal-run-isolation` checks it against the specs.

| | |
|---|---|
| Result | 18 of 18 check(s) matched `expect` |
| Run | `3144650-1794516386.722`, 2026-11-12T20:46:26.72239872Z → 2026-11-12T20:47:29.529517824Z |
| Jobs | 16 |
| Git | `7e3d84f5caba1b56275ce51c06228463218e1855` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| runner-safety | runner_protocol.qnt | runner_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 32.4 | 0.0 | 32.4 |  |
| runner-owners-progress | runner_protocol.qnt | runner_ok | temporal `OwnersEventuallyEnd` | tlc | exhaustive | holds | holds | PASS | 1 | 62.5 | 0.0 | 62.5 |  |
| runner-maintenance-progress | runner_protocol.qnt | runner_ok | temporal `MaintenanceEventuallyResolves` | tlc | exhaustive | holds | holds | PASS | 1 | 50.6 | 0.0 | 50.6 |  |
| runner-hazard-exclusive-ordinary | hazard/runner_hazards.qnt | runner_bugOrdinaryExclusive | invariant `OrdinaryDoesNotBlockOrdinary` | tlc | exhaustive | violation | violation | PASS | 1 | 8.3 | 0.0 | 8.3 | [traces/runner-hazard-exclusive-ordinary.txt](traces/runner-hazard-exclusive-ordinary.txt) [traces/runner-hazard-exclusive-ordinary.tlc.txt](traces/runner-hazard-exclusive-ordinary.tlc.txt) |
| runner-hazard-collect-at-terminal | hazard/runner_hazards.qnt | runner_bugCollectAtTerminal | invariant `NoPrematureCommandCompletion` | tlc | exhaustive | violation | violation | PASS | 1 | 8.3 | 0.0 | 8.3 | [traces/runner-hazard-collect-at-terminal.txt](traces/runner-hazard-collect-at-terminal.txt) [traces/runner-hazard-collect-at-terminal.tlc.txt](traces/runner-hazard-collect-at-terminal.tlc.txt) |
| runner-run-independent-worktrees | runner_protocol.qnt | runner_conf | run `independent_worktrees` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-same-checkout-excludes-sync | runner_protocol.qnt | runner_conf | run `same_checkout_excludes_sync` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-controller-eof-before-start | runner_protocol.qnt | runner_conf | run `controller_eof_before_start` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-controller-loss-after-start | runner_protocol.qnt | runner_conf | run `controller_loss_after_start` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-supervisor-loss-is-not-a-result | runner_protocol.qnt | runner_conf | run `supervisor_loss_is_not_a_result` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-cleanup-failure-keeps-command | runner_protocol.qnt | runner_conf | run `cleanup_failure_keeps_command` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-missing-completion-refuses | runner_protocol.qnt | runner_conf | run `missing_completion_refuses` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-maintenance-waits-for-stop | runner_protocol.qnt | runner_conf | run `maintenance_waits_for_stop` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-no-sync-mismatch-refuses | runner_protocol.qnt | runner_conf | run `no_sync_mismatch_refuses` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| runner-run-repeat-collection-agrees | runner_protocol.qnt | runner_conf | run `repeat_collection_agrees` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-admission-timeout | runner_protocol.qnt | runner_conf | run `admission_timeout` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-preflight-aborts-before-start | runner_protocol.qnt | runner_conf | run `preflight_aborts_before_start` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| runner-run-missing-rsync-aborts-before-start | runner_protocol.qnt | runner_conf | run `missing_rsync_aborts_before_start` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |

## Inputs (sha256)

- `checks.toml` `aca65dde0e79ac0dd4a19f0f5b95ebce7725e418f841f3e53dff5eef6b452b16`
- `hazard/runner_hazards.qnt` `907af15a80388e355818f249abc69fbd3b339b20867266b3d9174fa15daeb2f7`
- `runner_protocol.qnt` `11689b4a6970570d72eaf477304db6cd145228984b81daa5039eb0de3a387416`
