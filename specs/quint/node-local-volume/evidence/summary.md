# Quint evidence — `node-local-volume`

Written by `cargo xtask quint check --subsystem node-local-volume --record`; do not edit. `cargo xtask quint verify-evidence --subsystem node-local-volume` checks it against the specs.

| | |
|---|---|
| Result | 18 of 18 check(s) matched `expect` |
| Run | `3309178-1795136452.219`, 2026-11-20T01:00:52.218757888Z → 2026-11-20T01:05:24.605366784Z |
| Jobs | 16 |
| Git | `cd09d45d8304c33dec1444d1df4b415b87f24cea` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| vol-safety-tlc | node_volume.qnt | vol_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 272.1 | 0.0 | 272.1 |  |
| vol-live-start | node_volume.qnt | vol_live_start | temporal `EveryStartLiveness` | tlc | exhaustive | holds | holds | PASS | 1 | 13.7 | 0.0 | 13.7 |  |
| vol-hazard-wait | hazard/volume_hazards.qnt | vol_bugNoWait | invariant `I_NO_DRAIN_FAILURE_CHARGES_BUDGET` | tlc | exhaustive | violation | violation | PASS | 1 | 13.9 | 0.0 | 13.9 | [traces/vol-hazard-wait.txt](traces/vol-hazard-wait.txt) [traces/vol-hazard-wait.tlc.txt](traces/vol-hazard-wait.tlc.txt) |
| vol-hazard-ensure | hazard/volume_hazards.qnt | vol_bugRecreate | invariant `I_VD9` | tlc | exhaustive | violation | violation | PASS | 1 | 13.7 | 0.0 | 13.7 | [traces/vol-hazard-ensure.txt](traces/vol-hazard-ensure.txt) [traces/vol-hazard-ensure.tlc.txt](traces/vol-hazard-ensure.tlc.txt) |
| vol-hazard-stop | hazard/volume_hazards.qnt | vol_bugStopDeletes | invariant `I_STOP_KEEPS_VOLUME` | tlc | exhaustive | violation | violation | PASS | 1 | 13.9 | 0.0 | 13.9 | [traces/vol-hazard-stop.txt](traces/vol-hazard-stop.txt) [traces/vol-hazard-stop.tlc.txt](traces/vol-hazard-stop.tlc.txt) |
| vol-hazard-interrupt | hazard/volume_hazards.qnt | vol_bugNoInterrupt | invariant `I_SUPERSEDE_ABANDONS_WAITS` | tlc | exhaustive | violation | violation | PASS | 1 | 16.2 | 0.0 | 16.2 | [traces/vol-hazard-interrupt.txt](traces/vol-hazard-interrupt.txt) [traces/vol-hazard-interrupt.tlc.txt](traces/vol-hazard-interrupt.tlc.txt) |
| vol-run-happy-path-deploy-to-running | node_volume.qnt | vol_conf | run `happy_path_deploy_to_running` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| vol-run-restart-waits-for-drain-then-attaches | node_volume.qnt | vol_conf | run `restart_waits_for_drain_then_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| vol-run-stop-keeps-volume-and-restart-reattaches | node_volume.qnt | vol_conf | run `stop_keeps_volume_and_restart_reattaches` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| vol-run-crash-after-create-before-attach-reuses-volume | node_volume.qnt | vol_conf | run `crash_after_create_before_attach_reuses_volume` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| vol-run-node-reboot-reactivates-and-data-survives | node_volume.qnt | vol_conf | run `node_reboot_reactivates_and_data_survives` | evaluator | run | holds | holds | PASS | 1 | 3.6 | 0.0 | 3.6 |  |
| vol-run-launch-bound-exceeded-fails-typed-and-charges-budget | node_volume.qnt | vol_conf | run `launch_bound_exceeded_fails_typed_and_charges_budget` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |
| vol-run-no-space-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `no_space_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.5 | 0.0 | 3.5 |  |
| vol-run-store-absent-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `store_absent_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| vol-run-superseded-launch-result-discarded | node_volume.qnt | vol_conf | run `superseded_launch_result_discarded` | evaluator | run | holds | holds | PASS | 1 | 8.1 | 0.0 | 8.1 |  |
| vol-run-stop-during-boot-wait-terminates-vmm-and-keeps-volume | node_volume.qnt | vol_conf | run `stop_during_boot_wait_terminates_vmm_and_keeps_volume` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |
| vol-run-stop-during-ensure-lets-ensure-finish-then-abandons | node_volume.qnt | vol_conf | run `stop_during_ensure_lets_ensure_finish_then_abandons` | evaluator | run | holds | holds | PASS | 1 | 3.5 | 0.0 | 3.5 |  |
| vol-run-shutdown-abandons-every-launch-including-one-admitted-late | node_volume.qnt | vol_conf | run `shutdown_abandons_every_launch_including_one_admitted_late` | evaluator | run | holds | holds | PASS | 1 | 3.8 | 0.0 | 3.8 |  |

## Inputs (sha256)

- `checks.toml` `0ce435f04d962c2622e7d61c594de1d09b22010672e026e508f9c4f06994d2c6`
- `hazard/volume_hazards.qnt` `f4242d6f367af98b1c1c5b3056b534034b2a7be6e07a05a11e1d400b4fd1bd15`
- `node_volume.qnt` `ea7eedcd9c07b37ebf028c20b4a673426a85526d16fda15613faf8078d42936f`
