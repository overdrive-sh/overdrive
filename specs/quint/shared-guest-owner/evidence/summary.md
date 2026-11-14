# Quint evidence — `shared-guest-owner`

Written by `cargo xtask quint check --subsystem shared-guest-owner --record`; do not edit. `cargo xtask quint verify-evidence --subsystem shared-guest-owner` checks it against the specs.

| | |
|---|---|
| Result | 22 of 22 check(s) matched `expect` |
| Run | `203327-1794621518.386`, 2026-11-14T01:58:38.38626816Z → 2026-11-14T01:59:47.612084736Z |
| Jobs | 16 |
| Git | `ea5d508aa03246f51fa675711af855a21940292d` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| guest-safety-tlc | shared_guest.qnt | guest_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 17.4 | 0.0 | 17.4 |  |
| guest-every-client-terminates | shared_guest.qnt | guest_ok | temporal `EveryClientTerminates` | tlc | exhaustive | holds | holds | PASS | 1 | 68.9 | 0.0 | 68.9 |  |
| guest-hazard-owner | hazard/guest_hazards.qnt | guest_bugNoLock | invariant `I_OWNER_UNIQUE` | tlc | exhaustive | violation | violation | PASS | 1 | 15.6 | 0.0 | 15.6 | [traces/guest-hazard-owner.txt](traces/guest-hazard-owner.txt) [traces/guest-hazard-owner.tlc.txt](traces/guest-hazard-owner.tlc.txt) |
| guest-hazard-serial | hazard/guest_hazards.qnt | guest_bugNoSerial | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 13.8 | 0.0 | 13.8 | [traces/guest-hazard-serial.txt](traces/guest-hazard-serial.txt) [traces/guest-hazard-serial.tlc.txt](traces/guest-hazard-serial.tlc.txt) |
| guest-hazard-taint | hazard/guest_hazards.qnt | guest_bugNoTaint | invariant `I_NO_REUSE` | tlc | exhaustive | violation | violation | PASS | 1 | 14.3 | 0.0 | 14.3 | [traces/guest-hazard-taint.txt](traces/guest-hazard-taint.txt) [traces/guest-hazard-taint.tlc.txt](traces/guest-hazard-taint.tlc.txt) |
| guest-hazard-orphan | hazard/guest_hazards.qnt | guest_bugOrphan | invariant `I_NO_ORPHAN` | tlc | exhaustive | violation | violation | PASS | 1 | 14.6 | 0.0 | 14.6 | [traces/guest-hazard-orphan.txt](traces/guest-hazard-orphan.txt) [traces/guest-hazard-orphan.tlc.txt](traces/guest-hazard-orphan.tlc.txt) |
| guest-hazard-detach | hazard/guest_hazards.qnt | guest_bugCancelOnDisconnect | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 13.5 | 0.0 | 13.5 | [traces/guest-hazard-detach.txt](traces/guest-hazard-detach.txt) [traces/guest-hazard-detach.tlc.txt](traces/guest-hazard-detach.tlc.txt) |
| guest-hazard-bound | hazard/guest_hazards.qnt | guest_bugNoBound | temporal `EveryClientTerminates` | tlc | exhaustive | violation | violation | PASS | 1 | 16.6 | 0.0 | 16.6 | [traces/guest-hazard-bound.txt](traces/guest-hazard-bound.txt) [traces/guest-hazard-bound.tlc.txt](traces/guest-hazard-bound.tlc.txt) |
| guest-hazard-idle | hazard/guest_hazards.qnt | guest_bugIdleAnyTime | invariant `I_IDLE_NO_LOSS` | tlc | exhaustive | violation | violation | PASS | 1 | 16.4 | 0.0 | 16.4 | [traces/guest-hazard-idle.txt](traces/guest-hazard-idle.txt) [traces/guest-hazard-idle.tlc.txt](traces/guest-hazard-idle.tlc.txt) |
| guest-run-happy-path-three-clients-share-one-guest | shared_guest.qnt | guest_conf | run `happy_path_three_clients_share_one_guest` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| guest-run-failure-then-replacement-guest | shared_guest.qnt | guest_conf | run `failure_then_replacement_guest` | evaluator | run | holds | holds | PASS | 1 | 3.4 | 0.0 | 3.4 |  |
| guest-run-guest-lost-mid-exec-then-replacement | shared_guest.qnt | guest_conf | run `guest_lost_mid_exec_then_replacement` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| guest-run-disconnect-while-running-next-waits | shared_guest.qnt | guest_conf | run `disconnect_while_running_next_waits` | evaluator | run | holds | holds | PASS | 1 | 3.4 | 0.0 | 3.4 |  |
| guest-run-disconnect-while-queued-is-removed | shared_guest.qnt | guest_conf | run `disconnect_while_queued_is_removed` | evaluator | run | holds | holds | PASS | 1 | 2.0 | 0.0 | 2.0 |  |
| guest-run-boot-failure-then-retry-boot-on-next-attach | shared_guest.qnt | guest_conf | run `boot_failure_then_retry_boot_on_next_attach` | evaluator | run | holds | holds | PASS | 1 | 2.9 | 0.0 | 2.9 |  |
| guest-run-exec-bound-discards-guest-and-replies-bound | shared_guest.qnt | guest_conf | run `exec_bound_discards_guest_and_replies_bound` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| guest-run-idle-exit-only-when-empty | shared_guest.qnt | guest_conf | run `idle_exit_only_when_empty` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| guest-run-concurrent-first-arrival-one-owner | shared_guest.qnt | guest_env | run `concurrent_first_arrival_one_owner` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| guest-run-owner-crash-then-new-owner-elected | shared_guest.qnt | guest_env | run `owner_crash_then_new_owner_elected` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| guest-run-elector-killed-before-owner-starts | shared_guest.qnt | guest_env | run `elector_killed_before_owner_starts` | evaluator | run | holds | holds | PASS | 1 | 3.1 | 0.0 | 3.1 |  |
| guest-run-owner-start-fails-then-next-elects | shared_guest.qnt | guest_env | run `owner_start_fails_then_next_elects` | evaluator | run | holds | holds | PASS | 1 | 2.8 | 0.0 | 2.8 |  |
| guest-run-run-teardown-removes-leftovers | shared_guest.qnt | guest_env | run `run_teardown_removes_leftovers` | evaluator | run | holds | holds | PASS | 1 | 3.4 | 0.0 | 3.4 |  |

## Inputs (sha256)

- `checks.toml` `42115b03c1b61de2c6afcfc5dcc0a58efb12226e26ce486f77a74cd8ae37d04e`
- `hazard/guest_hazards.qnt` `210a59c98d04f93370fdced804ea1dc2f1c92706ab19828f94771d5d33f85ff1`
- `shared_guest.qnt` `18105724c667d218469875038591124bd25128601592cd6156fb505b28bb4e1b`
