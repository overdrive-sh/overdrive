# Quint evidence — `node-local-volume`

Written by `cargo xtask quint check --subsystem node-local-volume --record`; do not edit. `cargo xtask quint verify-evidence --subsystem node-local-volume` checks it against the specs.

| | |
|---|---|
| Result | 19 of 19 check(s) matched `expect` |
| Run | `50128-1795487077.095`, 2026-11-24T02:24:37.09510656Z → 2026-11-24T02:48:18.598071808Z |
| Jobs | 16 |
| Git | `1d7fb3fc607cb2276bb015a0ddda8c7fae3f0883` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| vol-safety-tlc | node_volume.qnt | vol_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 1378.4 | 0.0 | 1378.4 |  |
| vol-live-start | node_volume.qnt | vol_live_start | temporal `EveryStartLiveness` | tlc | exhaustive | holds | holds | PASS | 1 | 31.1 | 0.0 | 31.1 |  |
| vol-hazard-wait | hazard/volume_hazards.qnt | vol_bugNoWait | invariant `I_NO_DRAIN_FAILURE_CHARGES_BUDGET` | tlc | exhaustive | violation | violation | PASS | 1 | 18.5 | 0.0 | 18.5 | [traces/vol-hazard-wait.txt](traces/vol-hazard-wait.txt) [traces/vol-hazard-wait.tlc.txt](traces/vol-hazard-wait.tlc.txt) |
| vol-hazard-ensure | hazard/volume_hazards.qnt | vol_bugRecreate | invariant `I_VD9` | tlc | exhaustive | violation | violation | PASS | 1 | 15.9 | 0.0 | 15.9 | [traces/vol-hazard-ensure.txt](traces/vol-hazard-ensure.txt) [traces/vol-hazard-ensure.tlc.txt](traces/vol-hazard-ensure.tlc.txt) |
| vol-hazard-stop | hazard/volume_hazards.qnt | vol_bugStopDeletes | invariant `I_STOP_KEEPS_VOLUME` | tlc | exhaustive | violation | violation | PASS | 1 | 18.2 | 0.0 | 18.2 | [traces/vol-hazard-stop.txt](traces/vol-hazard-stop.txt) [traces/vol-hazard-stop.tlc.txt](traces/vol-hazard-stop.tlc.txt) |
| vol-hazard-interrupt | hazard/volume_hazards.qnt | vol_bugNoInterrupt | invariant `I_SUPERSEDE_ABANDONS_WAITS` | tlc | exhaustive | violation | violation | PASS | 1 | 15.4 | 0.0 | 15.4 | [traces/vol-hazard-interrupt.txt](traces/vol-hazard-interrupt.txt) [traces/vol-hazard-interrupt.tlc.txt](traces/vol-hazard-interrupt.tlc.txt) |
| vol-run-happy-path-deploy-to-running | node_volume.qnt | vol_conf | run `happy_path_deploy_to_running` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| vol-run-restart-waits-for-drain-then-attaches | node_volume.qnt | vol_conf | run `restart_waits_for_drain_then_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| vol-run-stop-keeps-volume-and-restart-reattaches | node_volume.qnt | vol_conf | run `stop_keeps_volume_and_restart_reattaches` | evaluator | run | holds | holds | PASS | 1 | 3.5 | 0.0 | 3.5 |  |
| vol-run-crash-after-create-before-attach-reuses-volume | node_volume.qnt | vol_conf | run `crash_after_create_before_attach_reuses_volume` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| vol-run-node-reboot-reactivates-and-data-survives | node_volume.qnt | vol_conf | run `node_reboot_reactivates_and_data_survives` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| vol-run-launch-bound-exceeded-fails-typed-and-charges-budget | node_volume.qnt | vol_conf | run `launch_bound_exceeded_fails_typed_and_charges_budget` | evaluator | run | holds | holds | PASS | 1 | 3.5 | 0.0 | 3.5 |  |
| vol-run-restart-of-failed-row-forgives-the-failure | node_volume.qnt | vol_conf | run `restart_of_failed_row_forgives_the_failure` | evaluator | run | holds | holds | PASS | 1 | 42.7 | 0.0 | 42.7 |  |
| vol-run-no-space-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `no_space_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| vol-run-store-absent-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `store_absent_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| vol-run-superseded-launch-result-discarded | node_volume.qnt | vol_conf | run `superseded_launch_result_discarded` | evaluator | run | holds | holds | PASS | 1 | 37.7 | 0.0 | 37.7 |  |
| vol-run-stop-during-boot-wait-terminates-vmm-and-keeps-volume | node_volume.qnt | vol_conf | run `stop_during_boot_wait_terminates_vmm_and_keeps_volume` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| vol-run-stop-during-ensure-lets-ensure-finish-then-abandons | node_volume.qnt | vol_conf | run `stop_during_ensure_lets_ensure_finish_then_abandons` | evaluator | run | holds | holds | PASS | 1 | 3.0 | 0.0 | 3.0 |  |
| vol-run-shutdown-abandons-every-launch-including-one-admitted-late | node_volume.qnt | vol_conf | run `shutdown_abandons_every_launch_including_one_admitted_late` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |

## Inputs (sha256)

- `../restart-generation/restart_budget.qnt` `ca46472ee87c59142884f0b62b6d89ec826683dfa75adb3b4482fe7203e30918`
- `checks.toml` `05ec961f101c94124e654baf48bbf0f3331fd987ad7c22601e5a065855a60688`
- `hazard/volume_hazards.qnt` `38fa3890ef20d0e8690529bcd7ce827d2393ec9f49ad503a52d0e351d69506f6`
- `node_volume.qnt` `bc8d41bab47c15c7c7320c1512117500b0c7688a201a4af193168d3bb65cceca`
