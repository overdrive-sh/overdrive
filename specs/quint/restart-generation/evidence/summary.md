# Quint evidence — `restart-generation`

Written by `cargo xtask quint check --subsystem restart-generation --record`; do not edit. `cargo xtask quint verify-evidence --subsystem restart-generation` checks it against the specs.

| | |
|---|---|
| Result | 20 of 20 check(s) matched `expect` |
| Run | `223431-1795824850.493`, 2026-11-28T00:14:10.49334016Z → 2026-11-28T00:15:42.046986496Z |
| Jobs | 16 |
| Git | `4a4e64a17e26f30337a1f8f1b4b10cad109f5b71` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| rg-safety-tlc | restart_generation.qnt | rg_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 91.2 | 0.0 | 91.2 |  |
| rg-live-replacement | restart_generation.qnt | rg_live_replacement | temporal `RunIntentProgress` | tlc | exhaustive | holds | holds | PASS | 1 | 51.2 | 0.0 | 51.2 |  |
| rg-live-prefail | restart_generation.qnt | rg_live_prefail | temporal `PreFailExhausts` | tlc | exhaustive | holds | holds | PASS | 1 | 12.2 | 0.0 | 12.2 |  |
| rg-hazard-stamp-on-emit | hazard/restart_generation_hazards.qnt | rg_bugStampOnEmit | temporal `RunIntentProgress` | tlc | exhaustive | violation | violation | PASS | 1 | 14.4 | 0.0 | 14.4 | [traces/rg-hazard-stamp-on-emit.txt](traces/rg-hazard-stamp-on-emit.txt) [traces/rg-hazard-stamp-on-emit.tlc.txt](traces/rg-hazard-stamp-on-emit.tlc.txt) |
| rg-hazard-prefail-no-row | hazard/restart_generation_hazards.qnt | rg_bugPrefailNoRow | temporal `PreFailExhausts` | tlc | exhaustive | violation | violation | PASS | 1 | 16.5 | 0.0 | 16.5 | [traces/rg-hazard-prefail-no-row.txt](traces/rg-hazard-prefail-no-row.txt) [traces/rg-hazard-prefail-no-row.tlc.txt](traces/rg-hazard-prefail-no-row.tlc.txt) |
| rg-hazard-refusal-charged | hazard/restart_generation_hazards.qnt | rg_bugRefusalCharged | invariant `I_CAPACITY_WAIT_NEVER_EXHAUSTS` | tlc | exhaustive | violation | violation | PASS | 1 | 12.7 | 0.0 | 12.7 | [traces/rg-hazard-refusal-charged.txt](traces/rg-hazard-refusal-charged.txt) [traces/rg-hazard-refusal-charged.tlc.txt](traces/rg-hazard-refusal-charged.tlc.txt) |
| rg-hazard-abandon-writes-no-row | hazard/restart_generation_hazards.qnt | rg_bugAbandonWritesNoRow | temporal `RunIntentProgress` | tlc | exhaustive | violation | violation | PASS | 1 | 12.9 | 0.0 | 12.9 | [traces/rg-hazard-abandon-writes-no-row.txt](traces/rg-hazard-abandon-writes-no-row.txt) [traces/rg-hazard-abandon-writes-no-row.tlc.txt](traces/rg-hazard-abandon-writes-no-row.tlc.txt) |
| rg-hazard-abandoned-row-counts | hazard/restart_generation_hazards.qnt | rg_bugAbandonedRowCounts | invariant `I_ABANDONED_ROWS_DO_NOT_COUNT` | tlc | exhaustive | violation | violation | PASS | 1 | 12.9 | 0.0 | 12.9 | [traces/rg-hazard-abandoned-row-counts.txt](traces/rg-hazard-abandoned-row-counts.txt) [traces/rg-hazard-abandoned-row-counts.tlc.txt](traces/rg-hazard-abandoned-row-counts.tlc.txt) |
| rg-run-control-plane-crash-during-restart-launch-relaunches-once | restart_generation.qnt | rg_conf | run `control_plane_crash_during_restart_launch_relaunches_once` | evaluator | run | holds | holds | PASS | 1 | 16.0 | 0.0 | 16.0 |  |
| rg-run-second-restart-loses-launch-then-one-placement | restart_generation.qnt | rg_conf | run `second_restart_loses_launch_then_one_placement` | evaluator | run | holds | holds | PASS | 1 | 13.7 | 0.0 | 13.7 |  |
| rg-run-resume-while-running-stops-and-replaces | restart_generation.qnt | rg_conf | run `resume_while_running_stops_and_replaces` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| rg-run-restart-of-exhausted-workload-relaunches-uncharged | restart_generation.qnt | rg_conf | run `restart_of_exhausted_workload_relaunches_uncharged` | evaluator | run | holds | holds | PASS | 1 | 21.8 | 0.0 | 21.8 |  |
| rg-run-generation-replacement-forgives-predecessor-failure | restart_generation.qnt | rg_conf | run `generation_replacement_forgives_predecessor_failure` | evaluator | run | holds | holds | PASS | 1 | 25.1 | 0.0 | 25.1 |  |
| rg-run-persistent-prelaunch-failure-ends-in-backoff-exhausted | restart_generation.qnt | rg_conf | run `persistent_prelaunch_failure_ends_in_backoff_exhausted` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| rg-run-transient-prelaunch-failure-recovers | restart_generation.qnt | rg_conf | run `transient_prelaunch_failure_recovers` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| rg-run-admission-refusal-leaves-no-row-and-waits-for-capacity | restart_generation.qnt | rg_conf | run `admission_refusal_leaves_no_row_and_waits_for_capacity` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| rg-run-restart-of-completed-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `restart_of_completed_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 14.2 | 0.0 | 14.2 |  |
| rg-run-resume-of-stopped-job-does-nothing-and-issues-no-id | restart_generation.qnt | rg_conf_job | run `resume_of_stopped_job_does_nothing_and_issues_no_id` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |
| rg-run-crash-restart-abandoned-by-operator-restart-forgives-the-crash | restart_generation.qnt | rg_conf | run `crash_restart_abandoned_by_operator_restart_forgives_the_crash` | evaluator | run | holds | holds | PASS | 1 | 13.4 | 0.0 | 13.4 |  |
| rg-run-consecutive-abandoned-launches-charge-the-crash-once | restart_generation.qnt | rg_conf | run `consecutive_abandoned_launches_charge_the_crash_once` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |

## Inputs (sha256)

- `checks.toml` `ad6cfcf76c63d7afe1ee7a0f565f5d07329608a6a98f9a06ac9c52da02c3a119`
- `hazard/restart_generation_hazards.qnt` `3eca02f4a2597fcc1e66ac2b65344770223aeb4e2792efd161106727abdb7a94`
- `restart_budget.qnt` `1dd686c4e07b3c3509848f98be20023aca4d66eb684fe53a654cf58199302f07`
- `restart_generation.qnt` `0e543c565f22be9229c254385de8d918298b8e74ac039728f02de771a0ce1781`
