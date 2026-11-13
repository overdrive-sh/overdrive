# Quint evidence — `shared-guest-owner`

Written by `cargo xtask quint check --subsystem shared-guest-owner --record`; do not edit. `cargo xtask quint verify-evidence --subsystem shared-guest-owner` checks it against the specs.

| | |
|---|---|
| Result | 19 of 19 check(s) matched `expect` |
| Run | `2074602-1794607125.650`, 2026-11-13T21:58:45.649596672Z → 2026-11-13T22:00:25.40391296Z |
| Jobs | 16 |
| Git | `af90d87de20109408ca9222243dc7423dc173462` (dirty worktree) |
| Quint | 0.32.0 |
| Apalache | 0.56.1 \| build: 70cdaf4 |
| Java | openjdk version "21.0.12.1" 2026-08-18 LTS |
| Kernel | 7.0.0-38-generic |

| Check | Spec | Main | Property | Backend | Bound | Expect | Outcome | Result | Attempts | Seconds | Retry s | Wall s | Evidence |
|---|---|---|---|---|---|---|---|---|---:|---:|---:|---:|---|
| guest-safety-tlc | shared_guest.qnt | guest_ok | invariant `Safety` | tlc | exhaustive | holds | holds | PASS | 1 | 33.3 | 0.0 | 33.3 |  |
| guest-every-client-terminates | shared_guest.qnt | guest_ok | temporal `EveryClientTerminates` | tlc | exhaustive | holds | holds | PASS | 1 | 99.3 | 0.0 | 99.3 |  |
| guest-hazard-owner | hazard/guest_hazards.qnt | guest_bugNoLock | invariant `I_OWNER_UNIQUE` | tlc | exhaustive | violation | violation | PASS | 1 | 20.4 | 0.0 | 20.4 | [traces/guest-hazard-owner.txt](traces/guest-hazard-owner.txt) [traces/guest-hazard-owner.tlc.txt](traces/guest-hazard-owner.tlc.txt) |
| guest-hazard-serial | hazard/guest_hazards.qnt | guest_bugNoSerial | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 20.2 | 0.0 | 20.2 | [traces/guest-hazard-serial.txt](traces/guest-hazard-serial.txt) [traces/guest-hazard-serial.tlc.txt](traces/guest-hazard-serial.tlc.txt) |
| guest-hazard-taint | hazard/guest_hazards.qnt | guest_bugNoTaint | invariant `I_NO_REUSE` | tlc | exhaustive | violation | violation | PASS | 1 | 18.5 | 0.0 | 18.5 | [traces/guest-hazard-taint.txt](traces/guest-hazard-taint.txt) [traces/guest-hazard-taint.tlc.txt](traces/guest-hazard-taint.tlc.txt) |
| guest-hazard-orphan | hazard/guest_hazards.qnt | guest_bugOrphan | invariant `I_NO_ORPHAN` | tlc | exhaustive | violation | violation | PASS | 1 | 21.0 | 0.0 | 21.0 | [traces/guest-hazard-orphan.txt](traces/guest-hazard-orphan.txt) [traces/guest-hazard-orphan.tlc.txt](traces/guest-hazard-orphan.tlc.txt) |
| guest-hazard-detach | hazard/guest_hazards.qnt | guest_bugCancelOnDisconnect | invariant `I_SERIAL` | tlc | exhaustive | violation | violation | PASS | 1 | 17.4 | 0.0 | 17.4 | [traces/guest-hazard-detach.txt](traces/guest-hazard-detach.txt) [traces/guest-hazard-detach.tlc.txt](traces/guest-hazard-detach.tlc.txt) |
| guest-run-happy-path-three-clients-share-one-guest | shared_guest.qnt | guest_conf | run `happy_path_three_clients_share_one_guest` | evaluator | run | holds | holds | PASS | 1 | 4.9 | 0.0 | 4.9 |  |
| guest-run-concurrent-first-arrival-one-owner | shared_guest.qnt | guest_conf | run `concurrent_first_arrival_one_owner` | evaluator | run | holds | holds | PASS | 1 | 5.7 | 0.0 | 5.7 |  |
| guest-run-failure-then-replacement-guest | shared_guest.qnt | guest_conf | run `failure_then_replacement_guest` | evaluator | run | holds | holds | PASS | 1 | 6.1 | 0.0 | 6.1 |  |
| guest-run-guest-lost-mid-exec-then-replacement | shared_guest.qnt | guest_conf | run `guest_lost_mid_exec_then_replacement` | evaluator | run | holds | holds | PASS | 1 | 4.6 | 0.0 | 4.6 |  |
| guest-run-owner-crash-then-new-owner-elected | shared_guest.qnt | guest_conf | run `owner_crash_then_new_owner_elected` | evaluator | run | holds | holds | PASS | 1 | 6.5 | 0.0 | 6.5 |  |
| guest-run-disconnect-while-running-next-waits | shared_guest.qnt | guest_conf | run `disconnect_while_running_next_waits` | evaluator | run | holds | holds | PASS | 1 | 7.0 | 0.0 | 7.0 |  |
| guest-run-disconnect-while-queued-is-removed | shared_guest.qnt | guest_conf | run `disconnect_while_queued_is_removed` | evaluator | run | holds | holds | PASS | 1 | 6.8 | 0.0 | 6.8 |  |
| guest-run-boot-failure-then-retry-boot-on-next-attach | shared_guest.qnt | guest_conf | run `boot_failure_then_retry_boot_on_next_attach` | evaluator | run | holds | holds | PASS | 1 | 4.8 | 0.0 | 4.8 |  |
| guest-run-anchor-gone-ends-owner-and-guest | shared_guest.qnt | guest_conf | run `anchor_gone_ends_owner_and_guest` | evaluator | run | holds | holds | PASS | 1 | 4.2 | 0.0 | 4.2 |  |
| guest-run-run-teardown-removes-leftovers | shared_guest.qnt | guest_conf | run `run_teardown_removes_leftovers` | evaluator | run | holds | holds | PASS | 1 | 6.4 | 0.0 | 6.4 |  |
| guest-run-retry-of-failed-client-gets-fresh-guest | shared_guest.qnt | guest_conf | run `retry_of_failed_client_gets_fresh_guest` | evaluator | run | holds | holds | PASS | 1 | 4.5 | 0.0 | 4.5 |  |
| guest-run-elector-killed-before-owner-starts | shared_guest.qnt | guest_conf | run `elector_killed_before_owner_starts` | evaluator | run | holds | holds | PASS | 1 | 6.0 | 0.0 | 6.0 |  |

## Inputs (sha256)

- `checks.toml` `b6927ba6cec19a5253bc0e749e57c1578c7c5a8fe653e43877f8694c228205c3`
- `hazard/guest_hazards.qnt` `5f2592dcde82716f8fbde50f600da1604c6b2aa5fed3161c596b8dc165b643a0`
- `shared_guest.qnt` `8678f8207d16444e183d0bee8bca974fe3d8ec8c59476e2eaae87a2058116c63`
