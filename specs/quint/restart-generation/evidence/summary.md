# Quint evidence — `restart-generation`

Written by `cargo xtask quint check --subsystem restart-generation --record`; do not edit. `cargo xtask quint verify-evidence --subsystem restart-generation` checks it against the specs.

| | |
|---|---|
| Result | 16 of 16 check(s) matched `expect` |
| Run | `147604-1795091551.959`, 2026-11-19T12:32:31.958882304Z → 2026-11-19T12:33:36.738002688Z |
| Jobs | 16 |
| Git | `07a0d8a5bc6a0270a4ab4618d451b2e5943da4b5` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| rg-safety-tlc | restart_generation.qnt | rg_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 64.4 | 0.0 | 64.4 |  |
| rg-live-replacement | restart_generation.qnt | rg_live_replacement | temporal `RunIntentProgress` | tlc | exhaustive | holds | holds | PASS | 1 | 29.1 | 0.0 | 29.1 |  |
| rg-live-prefail | restart_generation.qnt | rg_live_prefail | temporal `PreFailExhausts` | tlc | exhaustive | holds | holds | PASS | 1 | 9.6 | 0.0 | 9.6 |  |
| rg-hazard-stamp-on-emit | hazard/restart_generation_hazards.qnt | rg_bugStampOnEmit | temporal `RunIntentProgress` | tlc | exhaustive | violation | violation | PASS | 1 | 12.4 | 0.0 | 12.4 | [traces/rg-hazard-stamp-on-emit.txt](traces/rg-hazard-stamp-on-emit.txt) [traces/rg-hazard-stamp-on-emit.tlc.txt](traces/rg-hazard-stamp-on-emit.tlc.txt) |
| rg-hazard-prefail-no-row | hazard/restart_generation_hazards.qnt | rg_bugPrefailNoRow | temporal `PreFailExhausts` | tlc | exhaustive | violation | violation | PASS | 1 | 11.9 | 0.0 | 11.9 | [traces/rg-hazard-prefail-no-row.txt](traces/rg-hazard-prefail-no-row.txt) [traces/rg-hazard-prefail-no-row.tlc.txt](traces/rg-hazard-prefail-no-row.tlc.txt) |
| rg-hazard-refusal-charged | hazard/restart_generation_hazards.qnt | rg_bugRefusalCharged | invariant `I_CAPACITY_WAIT_NEVER_EXHAUSTS` | tlc | exhaustive | violation | violation | PASS | 1 | 10.6 | 0.0 | 10.6 | [traces/rg-hazard-refusal-charged.txt](traces/rg-hazard-refusal-charged.txt) [traces/rg-hazard-refusal-charged.tlc.txt](traces/rg-hazard-refusal-charged.tlc.txt) |
| rg-run-control-plane-crash-during-restart-launch-relaunches-once | restart_generation.qnt | rg_conf | run `control_plane_crash_during_restart_launch_relaunches_once` | evaluator | run | holds | holds | PASS | 1 | 7.6 | 0.0 | 7.6 |  |
| rg-run-second-restart-loses-launch-then-one-placement | restart_generation.qnt | rg_conf | run `second_restart_loses_launch_then_one_placement` | evaluator | run | holds | holds | PASS | 1 | 9.4 | 0.0 | 9.4 |  |
| rg-run-resume-while-running-stops-and-replaces | restart_generation.qnt | rg_conf | run `resume_while_running_stops_and_replaces` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| rg-run-restart-of-exhausted-workload-relaunches-uncharged | restart_generation.qnt | rg_conf | run `restart_of_exhausted_workload_relaunches_uncharged` | evaluator | run | holds | holds | PASS | 1 | 14.2 | 0.0 | 14.2 |  |
| rg-run-generation-replacement-forgives-predecessor-failure | restart_generation.qnt | rg_conf | run `generation_replacement_forgives_predecessor_failure` | evaluator | run | holds | holds | PASS | 1 | 14.9 | 0.0 | 14.9 |  |
| rg-run-persistent-prelaunch-failure-ends-in-backoff-exhausted | restart_generation.qnt | rg_conf | run `persistent_prelaunch_failure_ends_in_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| rg-run-transient-prelaunch-failure-recovers | restart_generation.qnt | rg_conf | run `transient_prelaunch_failure_recovers` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| rg-run-admission-refusal-leaves-no-row-and-waits-for-capacity | restart_generation.qnt | rg_conf | run `admission_refusal_leaves_no_row_and_waits_for_capacity` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| rg-run-restart-of-completed-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `restart_of_completed_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 8.1 | 0.0 | 8.1 |  |
| rg-run-resume-of-stopped-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `resume_of_stopped_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |

## Inputs (sha256)

- `checks.toml` `cf4b5f037b4d5047a12795955853ecfec6dc763ed4891ac4d97f4b83a735dc96`
- `hazard/restart_generation_hazards.qnt` `d0d6b4419b7c592579363a44623a5a3eb8ccac9fd9841672eb7d513be82313d2`
- `restart_generation.qnt` `28b499761045ee2e935cb9fc86fc00df9659e537ef29ff4955ecfd815e6b8b8d`
