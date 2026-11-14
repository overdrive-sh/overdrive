# Quint evidence — `shared-guest-owner`

Written by `cargo xtask quint check --subsystem shared-guest-owner --record`; do not edit. `cargo xtask quint verify-evidence --subsystem shared-guest-owner` checks it against the specs.

| | |
|---|---|
| Result | 20 of 20 check(s) matched `expect` |
| Run | `3755208-1794651215.551`, 2026-11-14T10:13:35.551472128Z → 2026-11-14T10:14:07.663257856Z |
| Jobs | 16 |
| Git | `8b08a7c668013a4f5c28f3cf9605b6cb14e7fd9d` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| guest-safety-tlc | shared_guest.qnt | guest_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 9.9 | 0.0 | 9.9 |  |
| guest-every-client-terminates | shared_guest.qnt | guest_ok | temporal `EveryClientTerminates` | tlc | exhaustive | holds | holds | PASS | 1 | 19.6 | 0.0 | 19.6 |  |
| guest-hazard-owner | hazard/guest_hazards.qnt | guest_bugNoLock | invariant `I_OWNER_UNIQUE` | tlc | exhaustive | violation | violation | PASS | 1 | 10.7 | 0.0 | 10.7 | [traces/guest-hazard-owner.txt](traces/guest-hazard-owner.txt) [traces/guest-hazard-owner.tlc.txt](traces/guest-hazard-owner.tlc.txt) |
| guest-hazard-serial | hazard/guest_hazards.qnt | guest_bugNoSerial | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 10.9 | 0.0 | 10.9 | [traces/guest-hazard-serial.txt](traces/guest-hazard-serial.txt) [traces/guest-hazard-serial.tlc.txt](traces/guest-hazard-serial.tlc.txt) |
| guest-hazard-taint | hazard/guest_hazards.qnt | guest_bugNoTaint | invariant `I_NO_REUSE` | tlc | exhaustive | violation | violation | PASS | 1 | 10.4 | 0.0 | 10.4 | [traces/guest-hazard-taint.txt](traces/guest-hazard-taint.txt) [traces/guest-hazard-taint.tlc.txt](traces/guest-hazard-taint.tlc.txt) |
| guest-hazard-orphan | hazard/guest_hazards.qnt | guest_bugOrphan | invariant `I_NO_ORPHAN` | tlc | exhaustive | violation | violation | PASS | 1 | 10.9 | 0.0 | 10.9 | [traces/guest-hazard-orphan.txt](traces/guest-hazard-orphan.txt) [traces/guest-hazard-orphan.tlc.txt](traces/guest-hazard-orphan.tlc.txt) |
| guest-hazard-detach | hazard/guest_hazards.qnt | guest_bugCancelOnDisconnect | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 10.9 | 0.0 | 10.9 | [traces/guest-hazard-detach.txt](traces/guest-hazard-detach.txt) [traces/guest-hazard-detach.tlc.txt](traces/guest-hazard-detach.tlc.txt) |
| guest-hazard-bound | hazard/guest_hazards.qnt | guest_bugNoBound | temporal `EveryClientTerminates` | tlc | exhaustive | violation | violation | PASS | 1 | 14.0 | 0.0 | 14.0 | [traces/guest-hazard-bound.txt](traces/guest-hazard-bound.txt) [traces/guest-hazard-bound.tlc.txt](traces/guest-hazard-bound.tlc.txt) |
| guest-hazard-idle | hazard/guest_hazards.qnt | guest_bugIdleAnyTime | invariant `I_IDLE_NO_LOSS` | tlc | exhaustive | violation | violation | PASS | 1 | 10.7 | 0.0 | 10.7 | [traces/guest-hazard-idle.txt](traces/guest-hazard-idle.txt) [traces/guest-hazard-idle.tlc.txt](traces/guest-hazard-idle.tlc.txt) |
| guest-run-happy-path-three-clients-share-one-guest | shared_guest.qnt | guest_conf | run `happy_path_three_clients_share_one_guest` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-failure-then-replacement-guest | shared_guest.qnt | guest_conf | run `failure_then_replacement_guest` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-guest-lost-mid-exec-then-replacement | shared_guest.qnt | guest_conf | run `guest_lost_mid_exec_then_replacement` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-disconnect-while-running-next-waits | shared_guest.qnt | guest_conf | run `disconnect_while_running_next_waits` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-disconnect-while-queued-is-removed | shared_guest.qnt | guest_conf | run `disconnect_while_queued_is_removed` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-boot-failure-then-retry-boot-on-next-attach | shared_guest.qnt | guest_conf | run `boot_failure_then_retry_boot_on_next_attach` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-exec-bound-discards-guest-and-replies-bound | shared_guest.qnt | guest_conf | run `exec_bound_discards_guest_and_replies_bound` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-idle-exit-only-when-empty | shared_guest.qnt | guest_conf | run `idle_exit_only_when_empty` | evaluator | run | holds | holds | PASS | 1 | 2.3 | 0.0 | 2.3 |  |
| guest-run-concurrent-first-arrival-one-owner | shared_guest.qnt | guest_env | run `concurrent_first_arrival_one_owner` | evaluator | run | holds | holds | PASS | 1 | 2.1 | 0.0 | 2.1 |  |
| guest-run-owner-crash-then-new-owner-elected | shared_guest.qnt | guest_env | run `owner_crash_then_new_owner_elected` | evaluator | run | holds | holds | PASS | 1 | 2.6 | 0.0 | 2.6 |  |
| guest-run-owner-start-fails-then-next-elects | shared_guest.qnt | guest_env | run `owner_start_fails_then_next_elects` | evaluator | run | holds | holds | PASS | 1 | 2.5 | 0.0 | 2.5 |  |

## Inputs (sha256)

- `checks.toml` `c9e6e7700ffaed69e446a7570a0dec617c51942ffdcf9ef716b7dd91fe5f7d40`
- `hazard/guest_hazards.qnt` `210a59c98d04f93370fdced804ea1dc2f1c92706ab19828f94771d5d33f85ff1`
- `shared_guest.qnt` `2dc53e0cf67ef84ba82d42fa88de262b7e61ecb76a90f5deaf914451f36d0308`
