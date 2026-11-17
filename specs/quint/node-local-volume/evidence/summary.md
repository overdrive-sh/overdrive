# Quint evidence — `node-local-volume`

Written by `cargo xtask quint check --subsystem node-local-volume --record`; do not edit. `cargo xtask quint verify-evidence --subsystem node-local-volume` checks it against the specs.

| | |
|---|---|
| Result | 14 of 14 check(s) matched `expect` |
| Run | `719589-1794930005.904`, 2026-11-17T15:40:05.9041984Z → 2026-11-17T15:44:46.433732608Z |
| Jobs | 16 |
| Git | `80a94c0616881a92dc4cf1a5d3d6224f603e38ff` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| vol-safety-tlc | node_volume.qnt | vol_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 280.2 | 0.0 | 280.2 |  |
| vol-live-start | node_volume.qnt | vol_live_start | temporal `EveryStartLiveness` | tlc | exhaustive | holds | holds | PASS | 1 | 10.4 | 0.0 | 10.4 |  |
| vol-hazard-wait | hazard/volume_hazards.qnt | vol_bugNoWait | invariant `I_NO_DRAIN_FAILURE_CHARGES_BUDGET` | tlc | exhaustive | violation | violation | PASS | 1 | 11.9 | 0.0 | 11.9 | [traces/vol-hazard-wait.txt](traces/vol-hazard-wait.txt) [traces/vol-hazard-wait.tlc.txt](traces/vol-hazard-wait.tlc.txt) |
| vol-hazard-ensure | hazard/volume_hazards.qnt | vol_bugRecreate | invariant `I_VD9` | tlc | exhaustive | violation | violation | PASS | 1 | 11.9 | 0.0 | 11.9 | [traces/vol-hazard-ensure.txt](traces/vol-hazard-ensure.txt) [traces/vol-hazard-ensure.tlc.txt](traces/vol-hazard-ensure.tlc.txt) |
| vol-hazard-stop | hazard/volume_hazards.qnt | vol_bugStopDeletes | invariant `I_STOP_KEEPS_VOLUME` | tlc | exhaustive | violation | violation | PASS | 1 | 15.1 | 0.0 | 15.1 | [traces/vol-hazard-stop.txt](traces/vol-hazard-stop.txt) [traces/vol-hazard-stop.tlc.txt](traces/vol-hazard-stop.tlc.txt) |
| vol-run-happy-path-deploy-to-running | node_volume.qnt | vol_conf | run `happy_path_deploy_to_running` | evaluator | run | holds | holds | PASS | 1 | 6.2 | 0.0 | 6.2 |  |
| vol-run-restart-waits-for-drain-then-attaches | node_volume.qnt | vol_conf | run `restart_waits_for_drain_then_attaches` | evaluator | run | holds | holds | PASS | 1 | 5.7 | 0.0 | 5.7 |  |
| vol-run-stop-keeps-volume-and-restart-reattaches | node_volume.qnt | vol_conf | run `stop_keeps_volume_and_restart_reattaches` | evaluator | run | holds | holds | PASS | 1 | 5.7 | 0.0 | 5.7 |  |
| vol-run-crash-after-create-before-attach-reuses-volume | node_volume.qnt | vol_conf | run `crash_after_create_before_attach_reuses_volume` | evaluator | run | holds | holds | PASS | 1 | 5.7 | 0.0 | 5.7 |  |
| vol-run-node-reboot-reactivates-and-data-survives | node_volume.qnt | vol_conf | run `node_reboot_reactivates_and_data_survives` | evaluator | run | holds | holds | PASS | 1 | 4.9 | 0.0 | 4.9 |  |
| vol-run-launch-bound-exceeded-fails-typed-and-charges-budget | node_volume.qnt | vol_conf | run `launch_bound_exceeded_fails_typed_and_charges_budget` | evaluator | run | holds | holds | PASS | 1 | 5.7 | 0.0 | 5.7 |  |
| vol-run-no-space-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `no_space_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 5.2 | 0.0 | 5.2 |  |
| vol-run-store-absent-fails-typed-and-never-attaches | node_volume.qnt | vol_conf | run `store_absent_fails_typed_and_never_attaches` | evaluator | run | holds | holds | PASS | 1 | 6.2 | 0.0 | 6.2 |  |
| vol-run-superseded-launch-result-discarded | node_volume.qnt | vol_conf | run `superseded_launch_result_discarded` | evaluator | run | holds | holds | PASS | 1 | 3.3 | 0.0 | 3.3 |  |

## Inputs (sha256)

- `checks.toml` `be583682e9788dd23ef1ad502f71965345c5867211d250743b5ad696e5b08ffc`
- `hazard/volume_hazards.qnt` `fa90a34977559267500f5888ca57de0da6f45f9d7d1e1a135d36edafe59e6fb8`
- `node_volume.qnt` `c726c97d181d397eae498da1cee15d4567d09cde4edd40a98ca37f54a5ed68ae`
