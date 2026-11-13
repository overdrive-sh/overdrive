# Shared-guest owner protocol

Feature `metal-test-placement-attribute`; decisions DDD-8 (owner uniqueness and lifetime), DDD-9 (serving order, detach), DDD-10 (no reuse after failure). DESIGN proposal; no user approval is implied.

`shared_guest.qnt` models ONE guest key, three clients (nextest test processes), one anchor (the nextest process) and one run. Module `shared_guest` is the state machine (`init`, `step`); `guest_ok` is the exhaustive instance, `guest_conf` the conformance instance (`REC = true`, `conf_step` records the action taken as a variant named after the action, parameters by name). `hazard/guest_hazards.qnt` weakens one decision per instance.

## Decisions and their checks

| Decision | Rule modelled | Invariant | Hazard (`OFF`) |
|---|---|---|---|
| D-OWNER | first arrival takes the exclusive lock, starts the owner, releases the lock; later arrivals connect to the live owner | `I_OWNER_UNIQUE` | `noLock` |
| D-LIFE | owner death (crash or anchor gone) kills the guest (A-1); the owner exits when the anchor is gone; run teardown removes every owner and guest | `I_NO_ORPHAN`, `I_TEARDOWN_CLEAN` | `orphan` (a crash leaves the guest) |
| D-SERIAL | FIFO queue; `BeginExec` only with no exec in flight | `I_SERIAL`, `I_FIFO` | `noSerial` |
| D-DETACH | disconnect while queued removes the client; while running the client is detached, the exec runs on, the result is discarded, the slot frees only when the exec ends | `I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED` | `cancelOnDisconnect` (checked against `I_SERIAL`) |
| D-TAINT | a failed or lost guest is discarded; the next attach boots generation + 1; a boot failure answers the head client `BootFailed` and the owner stays | `I_NO_REUSE` | `noTaint` |

`Safety` is the conjunction. `EveryClientTerminates` (TLC, weak fairness of `Progress`, anchor alive forever, fault budget `FAULTS = 2` shared by `OwnerCrash` and `GuestLost`) says every attached client reaches a terminal phase (outcome or disconnected).

## Named runs (the `#[quint_test]` pins; all `main = guest_conf`)

| Scenario | Run |
|---|---|
| happy path, 3 clients share one generation | `happy_path_three_clients_share_one_guest` |
| concurrent first arrival, one owner | `concurrent_first_arrival_one_owner` |
| failure then replacement guest | `failure_then_replacement_guest` |
| guest lost mid exec then replacement | `guest_lost_mid_exec_then_replacement` |
| owner crash then a new arrival elects a new owner | `owner_crash_then_new_owner_elected` |
| disconnect while running, next client waits | `disconnect_while_running_next_waits` |
| disconnect while queued | `disconnect_while_queued_is_removed` |
| boot failure then boot again on next attach | `boot_failure_then_retry_boot_on_next_attach` |
| anchor gone ends owner and guest | `anchor_gone_ends_owner_and_guest` |
| teardown backstop removes leftovers | `run_teardown_removes_leftovers` |
| retry of a failed client gets a fresh guest | `retry_of_failed_client_gets_fresh_guest` |
| electing client killed before the owner exists | `elector_killed_before_owner_starts` |

## Assumptions

| Assumption | Discharged by |
|---|---|
| A-1 the owner is init of its PID namespace, so the kernel kills the VMM when the owner dies (rule of `OwnerCrash` and `OwnerExit`) | Tier-3 validation V-4; NOT proven by the model |
| A-2 client processes are children of the anchor and take no action once it is gone | nextest process-group kill semantics; Tier-3 run teardown scenario |
| A-3 exec pass/fail, boot ok/fail and guest loss are environment choices | `ExecDonePass`/`ExecDoneFail`, `BootOk`/`BootFail`, `GuestLost` are nondeterministic actions |
| A-4 a nextest retry is a new client id | modelled as a fresh client arriving |
| A-5 owner crashes and guest losses are finite (budget 2); without it a perpetual crash defeats any progress property | fairness precondition of `EveryClientTerminates` |
| A-6 election, owner start and connect are atomic w.r.t. the owner's own state (`OwnerStarted` registers the owner and enqueues the electing client in one step) | design pin: `OwnerCore::observe()` shows the owner only once it serves |

## Model-only variables

`anchor`, `torn`, `lock`, `bad`, `execgen`, `execs`, `seq`, `nseq`, `lastb`, `fifobad`, `faults`, `conf_step`, and the per-client `phase` outcomes. The driver must not project them from its own bookkeeping.

Commands: `cargo xtask metal run -- cargo xtask quint check --subsystem shared-guest-owner --record`, then `verify-evidence`.
