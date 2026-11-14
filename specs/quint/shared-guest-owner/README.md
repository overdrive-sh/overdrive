# Shared-guest owner protocol

Feature `metal-test-placement-attribute`; decisions are numbered in the feature delta as DDD-8 (owner uniqueness and lifetime), DDD-9 (serving order, no cancel, discard after failure), DDD-16 (exec bound, D-BOUND), DDD-17 (idle exit, D-IDLE), DDD-18 (a client that goes away while its test runs taints the guest, part of D-TAINT). DDD-10 there is `inside_guest()` and is unrelated. D-BOUND, D-IDLE and the detach clause of D-TAINT are approved user decisions N-1, N-2 and N-3 (2026-11-14).

`shared_guest.qnt` models ONE guest key, three clients (nextest test processes) and one run. Module `shared_guest` is the state machine (`init`, `step`). The owner is reactive like `OwnerCore::handle(Event) -> Vec<Effect>`: each owner event is one action and ends with `drive` (no exec in flight and a non-empty queue: Ready guest -> begin the exec of the queue head, NoGuest -> begin a boot). `hazard/guest_hazards.qnt` weakens one decision per instance.

## Instances (const `ENV`, `REC`, `OFF`)

| Instance | ENV | REC | Use |
|---|---|---|---|
| `guest_ok` | true | false | exhaustive checks (`Safety`, `EveryClientTerminates`); hazards use the same shape with one `OFF` rule |
| `guest_env` | true | true | environment named runs (evaluator; NOT pinned by conformance) |
| `guest_conf` | false | true | owner-driven named runs: the `#[quint_test]` pins |

With `ENV = false` every environment action is disabled, `init` has the owner already started (`owners = 1`, lock free, clients `IDLE`), and `owners` is 1 or 0 and equals `not(OwnerObservation.stopped)`. `REC = true` makes `conf_step` record the action taken (variant named after the action, parameters by name).

The `guest_env` runs and the environment actions `Arrive`, `ElectBegin`, `OwnerStarted`, `OwnerStartFails` and `OwnerCrash` are outside `OwnerCore` conformance replay, under an exemption approved by the user (design decision N-4, 2026-11-14): the OS or the client shell performs them (an owner crash leaves no process to receive an event). Each invariant they touch is discharged as follows: `I_OWNER_UNIQUE` by the Tier-3 scenario of eight simultaneous first arrivals electing exactly one owner (V-5); `I_NO_ORPHAN` by the Tier-3 scenario in which SIGKILL of the owner during a guest exec leaves no VMM (V-4); an owner-start failure (missing owner binary, `LaneNotPrepared`) by a Tier-3 scenario. Every other invariant is constrained by the actions `guest_conf` replays.

## Validation budget

20 checks; wall time and per-check seconds are in `evidence/summary.md`; the two `ci = false` checks are `guest-every-client-terminates` and `guest-hazard-bound`.

## Actions

Owner events, each one `OwnerCore` event (the whole `step` of `guest_conf`): `Attached{c}`, `ClientGone{c}`, `BootFinished{ok}`, `ExecFinished{pass}`, `GuestLost`, `ExecBound`, `IdleElapsed`.

Environment actions (`ENV` only; not an `OwnerCore` event): `Arrive{c}`, `ElectBegin{c}`, `OwnerStarted{c}` (also enqueues the elector as `Attached` does, then drives), `OwnerStartFails{c}` (the electing client holding the lock fails with the typed `LaneNotPrepared` outcome, phase `STARTFAIL`; lock released, no owner, nothing left running), `OwnerCrash`.

## Decisions and their checks

| Decision | Rule modelled | Invariant / property | Hazard (`OFF`) |
|---|---|---|---|
| D-OWNER | first arrival takes the exclusive lock, starts the owner, releases the lock; later arrivals attach to the live owner | `I_OWNER_UNIQUE` | `noLock` |
| D-LIFE | owner death kills the guest (A-1) | `I_NO_ORPHAN` | `orphan` (a crash leaves the guest) |
| D-SERIAL | queue appended at the tail and consumed from the head; `drive` begins an exec only with none in flight | `I_SERIAL` | `noSerial` |
| D-DETACH | disconnect while queued removes the client; while running the client is detached, the exec runs on, the result is discarded, the slot frees only when the exec ends | `I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED` | `cancelOnDisconnect` (checked against `I_SERIAL`) |
| D-TAINT | a failed, lost or detached-from exec taints its generation: the guest is discarded when that exec ends; the next attach boots generation + 1; a boot failure answers the head client `BootFailed` | `I_NO_REUSE` = Ready with no exec in flight implies the generation is not bad | `noTaint` (failure and detach taint off) |
| D-BOUND | `ExecBound` ends a hung exec: guest discarded, client `BOUND` (`GONE` if detached), the replacement boots if clients wait | `EveryClientTerminates` (fairness set `Progress` has `ExecBound` and `BootFinished`, not `ExecFinished`) | `noBound` (expects `EveryClientTerminates` violated) |
| D-IDLE | `IdleElapsed` only with no client queued, running or detached-running; owner and guest end | `I_IDLE_NO_LOSS` | `idleAnyTime` |

`Safety` is the conjunction of every invariant above. `EveryClientTerminates` (TLC, weak fairness of `Progress`, fault budget `FAULTS = 2` shared by `OwnerCrash` and `GuestLost`) says every attached client reaches a terminal phase. Time is abstracted: `IdleElapsed` may fire at any moment its guard holds; a too-early idle exit only costs a reboot of the guest for the next client, never a wrong result.

## Named runs

`guest_conf` (the `#[quint_test]` pins):

| Scenario | Run |
|---|---|
| happy path, 3 clients share one generation | `happy_path_three_clients_share_one_guest` |
| failure then replacement guest | `failure_then_replacement_guest` |
| guest lost mid exec then replacement | `guest_lost_mid_exec_then_replacement` |
| disconnect while running, next waits, detached exec taints, replacement is generation 2 | `disconnect_while_running_next_waits` |
| disconnect while queued | `disconnect_while_queued_is_removed` |
| boot failure then boot again on next attach | `boot_failure_then_retry_boot_on_next_attach` |
| hung exec ended by the bound | `exec_bound_discards_guest_and_replies_bound` |
| idle exit only when empty | `idle_exit_only_when_empty` |

`guest_env` (evaluator only):

| Scenario | Run |
|---|---|
| concurrent first arrival, one owner | `concurrent_first_arrival_one_owner` |
| owner crash then a new arrival elects a new owner | `owner_crash_then_new_owner_elected` |
| owner start fails (electing client gets `LaneNotPrepared`, lock released), next client elects | `owner_start_fails_then_next_elects` |

## Projection for the conformance driver

Projectable from `OwnerCore::observe()`: `guest` (NoGuest/Booting/Ready), `gen` (`generation`), `inflight` (`in_flight`), `queue`, and in the `ENV = false` instance `owners` via `stopped`. Model-only (the driver does not project them from its own bookkeeping): `owners` for election (`I_OWNER_UNIQUE`; Tier-3 proves the real lock), `lock`, `bad`, `execgen`, `execs`, `faults`, `idleLoss`, `conf_step`, and the per-client outcome phases (the driver reads them from the `Reply` effect). Note: a discarded guest is replaced inside the same event when clients wait, so `NoGuest` is observable after an event only with an empty queue.

## Assumptions

| Assumption | Discharged by |
|---|---|
| A-1 the owner is init of its PID namespace, so the kernel kills the VMM when the owner dies (rule of `OwnerCrash` and `IdleElapsed`) | Discharged by Tier-3 validation V-4 (HOLDS on Metal kernel 7.0.0-38: no VMM survives the owner, gone within 38.6 ms; `docs/feature/metal-test-placement-attribute/spike/findings-owner-lifetime.md`, kernel basis `kernel/pid_namespace.c:225-274 @ v7.0`); an assumption of the model, not proven by it |
| A-3 exec pass/fail, boot ok/fail and guest loss are environment choices | `ExecFinished`, `BootFinished`, `GuestLost` are nondeterministic actions |
| A-4 a nextest retry is a new client id | modelled as a fresh client arriving |
| A-5 owner crashes and guest losses are finite (budget 2); without it a perpetual crash defeats any progress property | fairness precondition of `EveryClientTerminates` |
| A-6 election, owner start and attach are atomic w.r.t. the owner's own state (`OwnerStarted` registers the owner and enqueues the electing client in one step) | design pin: `OwnerCore::observe()` shows the owner only once it serves |
| A-8 a client connecting at the instant the owner exits idle sees the connection fail and re-arrives (stays `ARRIVING`); the model makes `Attached` and `IdleElapsed` atomic | discharged by the client rule "a connection that ends before the owner acknowledged the request has run nothing, so the client re-attaches once through the election" (feature delta DDD-17; kernel basis `net/unix/af_unix.c:721-723` and `708-709 @ v7.0`: closing a listening unix socket resets queued unaccepted connections) and by a deterministic client-side test: a unix listener that accepts and closes before it acknowledges produces exactly one re-attach through the election, and a second cut is a `GuestExecution` failure at stage `owner` (design DDD-17); the model keeps `Attached` and `IdleElapsed` atomic |

Commands: `cargo xtask metal run -- cargo xtask quint check --subsystem shared-guest-owner --record`, then `verify-evidence`.
