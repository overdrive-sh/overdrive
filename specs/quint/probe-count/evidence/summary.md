# Quint evidence — `probe-count`

Written by `cargo xtask quint check --subsystem probe-count --record`; do not edit. `cargo xtask quint verify-evidence --subsystem probe-count` checks it against the specs.

| | |
|---|---|
| Result | 33 of 33 check(s) matched `expect` |
| Run | `252559-1791986812.704`, 2026-10-14T14:06:52.704061184Z → 2026-10-14T14:07:31.00619648Z |
| Jobs | 12 |
| Git | `b71c774889388fbf0174fd786a6dfe537eef6f10` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-34-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| probe-count-no-early-terminate | probe_count.qnt | probe_streak | invariant `NoEarlyTerminate` | tlc | exhaustive | holds | holds | PASS | 1 | 7.9 | 0.0 | 7.9 |  |
| probe-count-no-react-on-same-row | probe_count.qnt | probe_streak | invariant `NoReactOnSameRow` | tlc | exhaustive | holds | holds | PASS | 1 | 7.6 | 0.0 | 7.6 |  |
| probe-count-no-terminate-when-stopped | probe_count.qnt | probe_streak | invariant `NoTerminateWhenStopped` | tlc | exhaustive | holds | holds | PASS | 1 | 7.9 | 0.0 | 7.9 |  |
| probe-count-no-missed-terminate | probe_count.qnt | probe_streak | invariant `NoMissedTerminate` | tlc | exhaustive | holds | holds | PASS | 1 | 6.6 | 0.0 | 6.6 |  |
| probe-count-no-early-ready | probe_count.qnt | probe_streak | invariant `NoEarlyReady` | tlc | exhaustive | holds | holds | PASS | 1 | 6.8 | 0.0 | 6.8 |  |
| probe-count-streak-exact | probe_count.qnt | probe_streak | invariant `StreakExact` | tlc | exhaustive | holds | holds | PASS | 1 | 6.8 | 0.0 | 6.8 |  |
| probe-count-missing-sample-inert | probe_count.qnt | probe_streak | invariant `MissingSampleInert` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| probe-count-store-holds-latest | probe_count.qnt | probe_streak | invariant `StoreHoldsLatest` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| probe-count-no-stale-change | probe_count.qnt | probe_streak | invariant `NoStaleChange` | tlc | exhaustive | holds | holds | PASS | 1 | 7.1 | 0.0 | 7.1 |  |
| probe-count-safety | probe_count.qnt | probe_streak | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 6.6 | 0.0 | 6.6 |  |
| probe-count-witness-terminates | probe_count.qnt | probe_streak | invariant `WitnessNeverTerminates` | tlc | exhaustive | violation | violation | PASS | 1 | 4.8 | 0.0 | 4.8 | [traces/probe-count-witness-terminates.txt](traces/probe-count-witness-terminates.txt) [traces/probe-count-witness-terminates.tlc.txt](traces/probe-count-witness-terminates.tlc.txt) |
| probe-count-witness-ready | probe_count.qnt | probe_streak | invariant `WitnessNeverReady` | tlc | exhaustive | violation | violation | PASS | 1 | 4.6 | 0.0 | 4.6 | [traces/probe-count-witness-ready.txt](traces/probe-count-witness-ready.txt) [traces/probe-count-witness-ready.tlc.txt](traces/probe-count-witness-ready.tlc.txt) |
| probe-count-hazard-reconstruct | hazard/probe_count_hazards.qnt | streak_bugReconstruct | invariant `NoEarlyTerminate` | tlc | exhaustive | violation | violation | PASS | 1 | 5.1 | 0.0 | 5.1 | [traces/probe-count-hazard-reconstruct.txt](traces/probe-count-hazard-reconstruct.txt) [traces/probe-count-hazard-reconstruct.tlc.txt](traces/probe-count-hazard-reconstruct.tlc.txt) |
| probe-count-hazard-no-act-once | hazard/probe_count_hazards.qnt | streak_bugNoActOnce | invariant `NoReactOnSameRow` | tlc | exhaustive | violation | violation | PASS | 1 | 5.3 | 0.0 | 5.3 | [traces/probe-count-hazard-no-act-once.txt](traces/probe-count-hazard-no-act-once.txt) [traces/probe-count-hazard-no-act-once.tlc.txt](traces/probe-count-hazard-no-act-once.tlc.txt) |
| probe-count-hazard-once-per-streak | hazard/probe_count_hazards.qnt | streak_bugOncePerStreak | invariant `NoMissedTerminate` | tlc | exhaustive | violation | violation | PASS | 1 | 5.3 | 0.0 | 5.3 | [traces/probe-count-hazard-once-per-streak.txt](traces/probe-count-hazard-once-per-streak.txt) [traces/probe-count-hazard-once-per-streak.tlc.txt](traces/probe-count-hazard-once-per-streak.tlc.txt) |
| probe-count-hazard-once-ever | hazard/probe_count_hazards.qnt | streak_bugOnceEver | invariant `NoMissedTerminate` | tlc | exhaustive | violation | violation | PASS | 1 | 5.3 | 0.0 | 5.3 | [traces/probe-count-hazard-once-ever.txt](traces/probe-count-hazard-once-ever.txt) [traces/probe-count-hazard-once-ever.tlc.txt](traces/probe-count-hazard-once-ever.tlc.txt) |
| probe-count-hazard-nosample-fail | hazard/probe_count_hazards.qnt | streak_bugNoSampleFail | invariant `MissingSampleInert` | tlc | exhaustive | violation | violation | PASS | 1 | 5.1 | 0.0 | 5.1 | [traces/probe-count-hazard-nosample-fail.txt](traces/probe-count-hazard-nosample-fail.txt) [traces/probe-count-hazard-nosample-fail.tlc.txt](traces/probe-count-hazard-nosample-fail.tlc.txt) |
| probe-count-hazard-lww-observed-at | hazard/probe_count_hazards.qnt | streak_bugLwwObservedAt | invariant `StoreHoldsLatest` | tlc | exhaustive | violation | violation | PASS | 1 | 5.1 | 0.0 | 5.1 | [traces/probe-count-hazard-lww-observed-at.txt](traces/probe-count-hazard-lww-observed-at.txt) [traces/probe-count-hazard-lww-observed-at.tlc.txt](traces/probe-count-hazard-lww-observed-at.tlc.txt) |
| probe-count-run-happy-path-three-fails-terminate-at-threshold | probe_count.qnt | probe_streak_conf | run `HappyPathThreeFailsTerminateAtThreshold` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-coalesced-fails-terminate-on-first-look | probe_count.qnt | probe_streak_conf | run `CoalescedFailsTerminateOnFirstLook` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-missing-sample-changes-nothing | probe_count.qnt | probe_streak_conf | run `MissingSampleChangesNothing` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-re-read-after-failed-stop-does-not-re-terminate | probe_count.qnt | probe_streak_conf | run `ReReadAfterFailedStopDoesNotReTerminate` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-failed-stop-retried-on-next-row | probe_count.qnt | probe_streak_conf | run `FailedStopRetriedOnNextRow` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-new-streak-after-pass-terminates-again | probe_count.qnt | probe_streak_conf | run `NewStreakAfterPassTerminatesAgain` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-flip-between-looks-no-early-terminate | probe_count.qnt | probe_streak_conf | run `FlipBetweenLooksNoEarlyTerminate` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-readiness-reaches-threshold | probe_count.qnt | probe_streak_conf | run `ReadinessReachesThreshold` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-clock-step-back-does-not-lose-row | probe_count.qnt | probe_streak_conf | run `ClockStepBackDoesNotLoseRow` | evaluator | run | holds | holds | PASS | 1 | 1.0 | 0.0 | 1.0 |  |
| probe-count-run-hazard-reconstruct | hazard/probe_count_hazards.qnt | streak_bugReconstruct | run `FlipBetweenLooksNoEarlyTerminate` | evaluator | run | violation | violation | PASS | 1 | 1.3 | 0.0 | 1.3 | [traces/probe-count-run-hazard-reconstruct.txt](traces/probe-count-run-hazard-reconstruct.txt) [traces/probe-count-run-hazard-reconstruct.test.txt](traces/probe-count-run-hazard-reconstruct.test.txt) |
| probe-count-run-hazard-no-act-once | hazard/probe_count_hazards.qnt | streak_bugNoActOnce | run `ReReadAfterFailedStopDoesNotReTerminate` | evaluator | run | violation | violation | PASS | 1 | 1.0 | 0.0 | 1.0 | [traces/probe-count-run-hazard-no-act-once.txt](traces/probe-count-run-hazard-no-act-once.txt) [traces/probe-count-run-hazard-no-act-once.test.txt](traces/probe-count-run-hazard-no-act-once.test.txt) |
| probe-count-run-hazard-nosample-fail | hazard/probe_count_hazards.qnt | streak_bugNoSampleFail | run `MissingSampleChangesNothing` | evaluator | run | violation | violation | PASS | 1 | 1.3 | 0.0 | 1.3 | [traces/probe-count-run-hazard-nosample-fail.txt](traces/probe-count-run-hazard-nosample-fail.txt) [traces/probe-count-run-hazard-nosample-fail.test.txt](traces/probe-count-run-hazard-nosample-fail.test.txt) |
| probe-count-run-hazard-lww-observed-at | hazard/probe_count_hazards.qnt | streak_bugLwwObservedAt | run `ClockStepBackDoesNotLoseRow` | evaluator | run | violation | violation | PASS | 1 | 1.3 | 0.0 | 1.3 | [traces/probe-count-run-hazard-lww-observed-at.txt](traces/probe-count-run-hazard-lww-observed-at.txt) [traces/probe-count-run-hazard-lww-observed-at.test.txt](traces/probe-count-run-hazard-lww-observed-at.test.txt) |
| probe-count-run-hazard-once-per-streak | hazard/probe_count_hazards.qnt | streak_bugOncePerStreak | run `FailedStopRetriedOnNextRow` | evaluator | run | violation | violation | PASS | 1 | 1.3 | 0.0 | 1.3 | [traces/probe-count-run-hazard-once-per-streak.txt](traces/probe-count-run-hazard-once-per-streak.txt) [traces/probe-count-run-hazard-once-per-streak.test.txt](traces/probe-count-run-hazard-once-per-streak.test.txt) |
| probe-count-run-hazard-once-ever | hazard/probe_count_hazards.qnt | streak_bugOnceEver | run `FailedStopRetriedOnNextRow` | evaluator | run | violation | violation | PASS | 1 | 1.0 | 0.0 | 1.0 | [traces/probe-count-run-hazard-once-ever.txt](traces/probe-count-run-hazard-once-ever.txt) [traces/probe-count-run-hazard-once-ever.test.txt](traces/probe-count-run-hazard-once-ever.test.txt) |

## Inputs (sha256)

- `checks.toml` `e9af90d090383c34e7f0b7f8f5bc3e258f74b53bef885eac6c2b4569d8e1ec20`
- `hazard/probe_count_hazards.qnt` `1895dd74d341523f9aad5e6b82b778c573aa354ec37f9564002320175f300a9d`
- `probe_count.qnt` `3adc80a735c1024f38b71924517193d505b2c63562845a10a9c30f794b98673a`
