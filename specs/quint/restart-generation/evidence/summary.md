# Quint evidence — `restart-generation`

Written by `cargo xtask quint check --subsystem restart-generation --record`; do not edit. `cargo xtask quint verify-evidence --subsystem restart-generation` checks it against the specs.

| | |
|---|---|
| Result | 17 of 17 check(s) matched `expect` |
| Run | `57183-1795488622.407`, 2026-11-24T02:50:22.406945792Z → 2026-11-24T02:51:58.578530048Z |
| Jobs | 16 |
| Git | `1d7fb3fc607cb2276bb015a0ddda8c7fae3f0883` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| rg-safety-tlc | restart_generation.qnt | rg_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 95.8 | 0.0 | 95.8 |  |
| rg-live-replacement | restart_generation.qnt | rg_live_replacement | temporal `RunIntentProgress` | tlc | exhaustive | holds | holds | PASS | 1 | 52.4 | 0.0 | 52.4 |  |
| rg-live-prefail | restart_generation.qnt | rg_live_prefail | temporal `PreFailExhausts` | tlc | exhaustive | holds | holds | PASS | 1 | 9.8 | 0.0 | 9.8 |  |
| rg-hazard-stamp-on-emit | hazard/restart_generation_hazards.qnt | rg_bugStampOnEmit | temporal `RunIntentProgress` | tlc | exhaustive | violation | violation | PASS | 1 | 11.9 | 0.0 | 11.9 | [traces/rg-hazard-stamp-on-emit.txt](traces/rg-hazard-stamp-on-emit.txt) [traces/rg-hazard-stamp-on-emit.tlc.txt](traces/rg-hazard-stamp-on-emit.tlc.txt) |
| rg-hazard-prefail-no-row | hazard/restart_generation_hazards.qnt | rg_bugPrefailNoRow | temporal `PreFailExhausts` | tlc | exhaustive | violation | violation | PASS | 1 | 12.4 | 0.0 | 12.4 | [traces/rg-hazard-prefail-no-row.txt](traces/rg-hazard-prefail-no-row.txt) [traces/rg-hazard-prefail-no-row.tlc.txt](traces/rg-hazard-prefail-no-row.tlc.txt) |
| rg-hazard-refusal-charged | hazard/restart_generation_hazards.qnt | rg_bugRefusalCharged | invariant `I_CAPACITY_WAIT_NEVER_EXHAUSTS` | tlc | exhaustive | violation | violation | PASS | 1 | 11.1 | 0.0 | 11.1 | [traces/rg-hazard-refusal-charged.txt](traces/rg-hazard-refusal-charged.txt) [traces/rg-hazard-refusal-charged.tlc.txt](traces/rg-hazard-refusal-charged.tlc.txt) |
| rg-hazard-abandon-writes-no-row | hazard/restart_generation_hazards.qnt | rg_bugAbandonWritesNoRow | temporal `RunIntentProgress` | tlc | exhaustive | violation | violation | PASS | 1 | 12.4 | 0.0 | 12.4 | [traces/rg-hazard-abandon-writes-no-row.txt](traces/rg-hazard-abandon-writes-no-row.txt) [traces/rg-hazard-abandon-writes-no-row.tlc.txt](traces/rg-hazard-abandon-writes-no-row.tlc.txt) |
| rg-run-control-plane-crash-during-restart-launch-relaunches-once | restart_generation.qnt | rg_conf | run `control_plane_crash_during_restart_launch_relaunches_once` | evaluator | run | holds | holds | PASS | 1 | 11.1 | 0.0 | 11.1 |  |
| rg-run-second-restart-loses-launch-then-one-placement | restart_generation.qnt | rg_conf | run `second_restart_loses_launch_then_one_placement` | evaluator | run | holds | holds | PASS | 1 | 11.1 | 0.0 | 11.1 |  |
| rg-run-resume-while-running-stops-and-replaces | restart_generation.qnt | rg_conf | run `resume_while_running_stops_and_replaces` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| rg-run-restart-of-exhausted-workload-relaunches-uncharged | restart_generation.qnt | rg_conf | run `restart_of_exhausted_workload_relaunches_uncharged` | evaluator | run | holds | holds | PASS | 1 | 18.7 | 0.0 | 18.7 |  |
| rg-run-generation-replacement-forgives-predecessor-failure | restart_generation.qnt | rg_conf | run `generation_replacement_forgives_predecessor_failure` | evaluator | run | holds | holds | PASS | 1 | 20.5 | 0.0 | 20.5 |  |
| rg-run-persistent-prelaunch-failure-ends-in-backoff-exhausted | restart_generation.qnt | rg_conf | run `persistent_prelaunch_failure_ends_in_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| rg-run-transient-prelaunch-failure-recovers | restart_generation.qnt | rg_conf | run `transient_prelaunch_failure_recovers` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| rg-run-admission-refusal-leaves-no-row-and-waits-for-capacity | restart_generation.qnt | rg_conf | run `admission_refusal_leaves_no_row_and_waits_for_capacity` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| rg-run-restart-of-completed-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `restart_of_completed_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 13.9 | 0.0 | 13.9 |  |
| rg-run-resume-of-stopped-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `resume_of_stopped_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |

## Inputs (sha256)

- `checks.toml` `1b5fb84583c0883a085c92b276a2c81ae831fabbc8b58a878040f5dd8869b081`
- `hazard/restart_generation_hazards.qnt` `7c60afd4d0d08b70892630510d254ff355b98b9df85144759384b22fd876b55e`
- `restart_budget.qnt` `ca46472ee87c59142884f0b62b6d89ec826683dfa75adb3b4482fe7203e30918`
- `restart_generation.qnt` `bf30263668fb26ae091d0d6990608536dae5fe98ac2c3dd48c079b8d2aa126c9`
