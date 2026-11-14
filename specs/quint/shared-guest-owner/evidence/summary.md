# Quint evidence — `shared-guest-owner`

Written by `cargo xtask quint check --subsystem shared-guest-owner --record`; do not edit. `cargo xtask quint verify-evidence --subsystem shared-guest-owner` checks it against the specs.

| | |
|---|---|
| Result | 19 of 19 check(s) matched `expect` |
| Run | `559831-1794660033.602`, 2026-11-14T12:40:33.602347008Z → 2026-11-14T12:42:43.102461952Z |
| Jobs | 16 |
| Git | `939dd3f53de929a6d3ec947fefcdaec450944216` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| guest-safety-tlc | shared_guest.qnt | guest_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 45.3 | 0.0 | 45.3 |  |
| guest-every-client-terminates | shared_guest.qnt | guest_ok | temporal `EveryClientTerminates` | tlc | exhaustive | holds | holds | PASS | 1 | 78.8 | 0.0 | 78.8 |  |
| guest-hazard-owner | hazard/guest_hazards.qnt | guest_bugNoLock | invariant `I_OWNER_UNIQUE` | tlc | exhaustive | violation | violation | PASS | 1 | 29.0 | 0.0 | 29.0 | [traces/guest-hazard-owner.txt](traces/guest-hazard-owner.txt) [traces/guest-hazard-owner.tlc.txt](traces/guest-hazard-owner.tlc.txt) |
| guest-hazard-serial | hazard/guest_hazards.qnt | guest_bugNoSerial | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 48.2 | 0.0 | 48.2 | [traces/guest-hazard-serial.txt](traces/guest-hazard-serial.txt) [traces/guest-hazard-serial.tlc.txt](traces/guest-hazard-serial.tlc.txt) |
| guest-hazard-taint | hazard/guest_hazards.qnt | guest_bugNoTaint | invariant `I_NO_REUSE` | tlc | exhaustive | violation | violation | PASS | 1 | 58.1 | 0.0 | 58.1 | [traces/guest-hazard-taint.txt](traces/guest-hazard-taint.txt) [traces/guest-hazard-taint.tlc.txt](traces/guest-hazard-taint.tlc.txt) |
| guest-hazard-orphan | hazard/guest_hazards.qnt | guest_bugOrphan | invariant `I_NO_ORPHAN` | tlc | exhaustive | violation | violation | PASS | 1 | 49.6 | 0.0 | 49.6 | [traces/guest-hazard-orphan.txt](traces/guest-hazard-orphan.txt) [traces/guest-hazard-orphan.tlc.txt](traces/guest-hazard-orphan.tlc.txt) |
| guest-hazard-detach | hazard/guest_hazards.qnt | guest_bugCancelOnDisconnect | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 52.9 | 0.0 | 52.9 | [traces/guest-hazard-detach.txt](traces/guest-hazard-detach.txt) [traces/guest-hazard-detach.tlc.txt](traces/guest-hazard-detach.tlc.txt) |
| guest-hazard-bound | hazard/guest_hazards.qnt | guest_bugNoBound | temporal `EveryClientTerminates` | tlc | exhaustive | violation | violation | PASS | 1 | 80.5 | 0.0 | 80.5 | [traces/guest-hazard-bound.txt](traces/guest-hazard-bound.txt) [traces/guest-hazard-bound.tlc.txt](traces/guest-hazard-bound.tlc.txt) |
| guest-hazard-idle | hazard/guest_hazards.qnt | guest_bugIdleAnyTime | invariant `I_IDLE_NO_LOSS` | tlc | exhaustive | violation | violation | PASS | 1 | 26.1 | 0.0 | 26.1 | [traces/guest-hazard-idle.txt](traces/guest-hazard-idle.txt) [traces/guest-hazard-idle.tlc.txt](traces/guest-hazard-idle.tlc.txt) |
| guest-run-happy-path-three-clients-share-one-guest | shared_guest.qnt | guest_conf | run `happy_path_three_clients_share_one_guest` | evaluator | run | holds | holds | PASS | 1 | 13.8 | 0.0 | 13.8 |  |
| guest-run-failure-then-replacement-guest | shared_guest.qnt | guest_conf | run `failure_then_replacement_guest` | evaluator | run | holds | holds | PASS | 1 | 8.5 | 0.0 | 8.5 |  |
| guest-run-guest-lost-mid-exec-then-replacement | shared_guest.qnt | guest_conf | run `guest_lost_mid_exec_then_replacement` | evaluator | run | holds | holds | PASS | 1 | 10.9 | 0.0 | 10.9 |  |
| guest-run-disconnect-while-running-next-waits | shared_guest.qnt | guest_conf | run `disconnect_while_running_next_waits` | evaluator | run | holds | holds | PASS | 1 | 7.5 | 0.0 | 7.5 |  |
| guest-run-disconnect-while-queued-is-removed | shared_guest.qnt | guest_conf | run `disconnect_while_queued_is_removed` | evaluator | run | holds | holds | PASS | 1 | 11.6 | 0.0 | 11.6 |  |
| guest-run-boot-failure-then-retry-boot-on-next-attach | shared_guest.qnt | guest_conf | run `boot_failure_then_retry_boot_on_next_attach` | evaluator | run | holds | holds | PASS | 1 | 12.3 | 0.0 | 12.3 |  |
| guest-run-exec-bound-discards-guest-and-replies-bound | shared_guest.qnt | guest_conf | run `exec_bound_discards_guest_and_replies_bound` | evaluator | run | holds | holds | PASS | 1 | 11.9 | 0.0 | 11.9 |  |
| guest-run-idle-exit-only-when-empty | shared_guest.qnt | guest_conf | run `idle_exit_only_when_empty` | evaluator | run | holds | holds | PASS | 1 | 7.8 | 0.0 | 7.8 |  |
| guest-run-concurrent-first-arrival-one-owner | shared_guest.qnt | guest_ok | run `concurrent_first_arrival_one_owner` | evaluator | run | holds | holds | PASS | 1 | 10.0 | 0.0 | 10.0 |  |
| guest-run-owner-crash-then-new-owner-elected | shared_guest.qnt | guest_ok | run `owner_crash_then_new_owner_elected` | evaluator | run | holds | holds | PASS | 1 | 9.3 | 0.0 | 9.3 |  |

## Inputs (sha256)

- `checks.toml` `507285322c21ec40214619fc99a76c9066578c900a55d13d50f34bb3dae9e250`
- `hazard/guest_hazards.qnt` `210a59c98d04f93370fdced804ea1dc2f1c92706ab19828f94771d5d33f85ff1`
- `shared_guest.qnt` `267ab487a022503887f8556e5532169bfe0afda17c66a59fb76e444a7832a248`
