# STEP 12-01 approved DESIGN contract — buffered `ControlOpen` followed by queued reset

## Status and decision boundary

**Status: CONTROLOPENRESET1 APPROVED by the user 2026-10-26; bounded formal
alignment and replacement evidence remain pending.** Independent DESIGN/formal
review found zero open technical findings. Exact authority: “Approve
CONTROLOPENRESET1”. The decision changes only the stable post-state of the
existing formal `HostReadOpen(c)` action when the connection was queued at VMM
death, its valid buffered `ControlOpen` is delivered, and the already-queued
reset follows immediately. It adds no product behavior, production gate,
public API, wire value, runtime mechanism, formal state, action, run, scenario,
hazard, check, driver filter, or test seam.

Roadmap validation remains `pending`. Approval authorizes only the exact
existing-action/README alignment below; it does not approve replacement
evidence or roadmap validation.

## Reproduced contradiction

Metal run `0ca90d92-a15a-468a-b49b-7a22d03b1e8c` replayed
`flows_conf_randomized_traces` with `QUINT_SEED=0x6539f0dd`. Trace 33 is:

```text
InitWith("g")
  -> Provision
  -> VmStart
  -> GuestOpenSession(2)
  -> ActivatePending
  -> VmStop
  -> Quiesce
  -> HostAcceptSession(2)
  -> ActivateQuiesced
  -> HostReadOpen(2)
```

The raw log is
`.context/netns-density-295/12-01/owner-flows-6539f0dd-verbose-current.log`.
Trace 33 begins at line 24681. The connection remains queued with its launch-1
`ControlOpen` through `VmStop`, is accepted as `reading`, and forwarding is
still quiesced at lines 24866-25008. `HostReadOpen(2)` at lines 25010-25045
produces the only divergence:

```text
model:          claim = 2, sess(2) = dead
implementation: claim = 0, sess(2) = closed
```

Allocation, generation, quiescence, flows, dials, activation result, and all
other session slots agree. The mismatch is not attribution, activation,
projection identity, or an omitted production cleanup.

## Complete production and Sim path

The failure is reachable through the current production owner path:

1. `GuestOpenSession(2)` queues one connection with a complete current-launch
   `ControlOpen` while the VMM is alive.
2. `VmStop` invokes the real K-A2-prime transport semantics. For a connection
   still in the listener queue, `SimGuestFlowIo::vm_stop_before_callbacks`
   preserves the buffered first record and attaches `Reset` to the queued
   child (`flow_io/sim.rs:1315-1323`). This matches the accepted Linux fact:
   a connected child stays in the accept queue with buffered bytes and is
   reset without being dequeued.
3. `HostAcceptSession(2)` releases the real owner accept. The Sim adapter moves
   the queued child's buffered record and reset into one accepted record socket
   (`flow_io/sim.rs:2055-2119`). The owner records it as `unread` and starts the
   existing control task (`owner_runtime.rs:185-235`).
4. `HostReadOpen(2)` calls the existing private
   `release_read(key, None)`. It releases the whole buffered `ControlOpen` and,
   because the buffer is now empty and a reset is already queued, makes that
   end readable in the same release (`flow_io/sim.rs:1219-1231`). No second
   driver effect exists between those facts.
5. The production task's first fixed read returns the complete opening record:
   `read_record` gives ready bytes before the queued end
   (`flow_io/sim.rs:2124-2145`). `control_session` decodes it, verifies the
   current launch and free claim, and briefly installs the session
   (`owner_runtime.rs:242-305`).
6. The task immediately enters its existing control read loop. With no bytes
   remaining, the already-readable reset returns an error; the loop exits
   (`owner_runtime.rs:308-368`). Because the just-claimed session is active,
   `session_lost` clears the claim and applies the existing session-loss
   cleanup before the socket closes and the task finishes
   (`owner_runtime.rs:369-380`, `:1420-1475`).
7. The conformance driver's existing `HostReadOpen` handler releases that one
   buffered record and calls the normal settle boundary
   (`quint_owner_flows.rs:537-541`). The first read, brief claim, immediately
   ready reset, session loss, close, and task finish therefore have one stable
   post-state: no claim and a closed session.

No production wait, race injection, or test-only scheduling choice is needed
to reach the result. A barrier after the brief claim would manufacture a
different conformance boundary rather than describe the current owner action.

## Existing contracts preserved

| Contract | Required outcome | Evidence |
|---|---|---|
| K-A2-prime queued child | VMM death does not dequeue a connected listener child or discard its buffered first record; the connection is accepted later and the established transport then fails. | Feature delta K-A2-prime (`feature-delta.md:14230`) and `owner_flows` `VmStop` action mapping (`:10451`). |
| Control-open parsing and claim | Decode/read failure closes. After a complete valid open, token comparison and free-claim check are one per-CID step. | Feature delta control-session Accept contract (`feature-delta.md:3224-3244`). |
| Session loss | A claimed session's EOF/reset clears its claim and aborts every flow/dial of that session. | Existing `sessionLoss(c)`, `HostSessionLoss`, and `HostDeadSessionLoss`; feature delta action map (`feature-delta.md:10460`). |
| Stable QPROJ state | `dead` means the owner still holds the session claim and the peer-gone fact; `closed` means the owner closed it. | QPROJ-3 session row (`feature-delta.md:10436`). |
| Live-session VMM death | A session already live when `VmStop` occurs remains `dead` until the separately driven `HostDeadSessionLoss` action. | `VmDeathMakesLiveSessionDeadThenLossIsProcessed` (`owner_flows.qnt:1268-1272`). |
| Accepted-unread VMM death | A connection already accepted in `reading` when `VmStop` occurs closes without a claim in the `VmStop` action. | Existing `VmStop` branch (`owner_flows.qnt:365-380`). |
| Fail-closed launch isolation | A dead or stale connection never holds a claim against a later live launch. | ATTR-LIVE-CLAIM, `StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim`. |

CONTROLOPENRESET1 leaves the live-session and already-accepted `VmStop` branches
unchanged. It addresses only a child that was still queued at `VmStop` and
whose record plus terminal reset become readable through the existing
`HostReadOpen` driver effect.

## Explicit accepted contradiction

The current formal source does not already have this stable result.
`HostReadOpen(c)` calculates `upSt = dead` when the connection's writer is no
longer up and, after a valid token/free-claim check, sets `sess(c) = dead` and
`claim = c` (`owner_flows.qnt:533-555`). It expects a later
`HostDeadSessionLoss(c)` to close the session and clear the claim.

That two-action abstraction is playable for a session that was live before
`VmStop`: its accepted socket receives a separately releasable peer-gone fact.
It is not playable for this queued child. The queued reset is attached before
accept, and releasing the buffered `ControlOpen` also releases that end; the
real control task processes both before the existing driver settles.

Because changing the existing `HostReadOpen` post-state changes an accepted
internal action contract, the correction is material even though all required
product outcomes already point to the fail-closed result. It therefore
required independent DESIGN review and explicit user approval; technical
review found zero open findings and the user approved the exact contract on
2026-10-26.

## CONTROLOPENRESET1 — exact approved contract

For existing `HostReadOpen(c)`, preserve the current order:

1. require `sess(c).st = reading`;
2. compare the buffered open's launch with the current live-token rule;
3. if the token is invalid, close with no claim exactly as today;
4. if another claim is held, close with no new claim exactly as today; and
5. if the token is valid and the claim is free, distinguish the existing
   writer-liveness fact:
   - while `writerUp(sess(c).gen)` is true, keep the current result:
     `sess(c) = live`, `claim = c`;
   - while `writerUp(sess(c).gen)` is false, compose the immediately pending
     reset and existing session-loss effects into this same formal action:
     `sess(c) = closed`, `claim` remains `0`, registered flows/dials are ended
     by the existing `NoSession` session-loss mapping, and the existing
     in-flight message/result cleanup is preserved.

The writer-down branch represents the stable owner result after the buffered
open and already-queued reset, not a rule that rejects a valid opening record
before parsing it. The production task may briefly install and clear the
claim internally; the existing conformance action has no QPROJ comparison
boundary inside that transient.

`HostDeadSessionLoss(c)` remains unchanged and enabled for a session that is
already `dead`, including a session that was live when `VmStop` occurred. The
existing `VmStop` branch for a connection already in `reading` also remains
unchanged. No state or action is removed.

The existing action/run/check inventory remains unchanged. Exact seed
`0x6539f0dd` supplies the reproduced transition; no new named run, scenario,
hazard, witness, trace filter, or driver state is added.

## Lifecycle Gate Ownership

Not applicable. CONTROLOPENRESET1 changes no production admission, token,
claim, activation, session-loss, or transport gate. The host owner still owns
the same token-and-claim decision and the same session-loss cleanup. The
formal action now ends at the stable result of the existing inseparable
buffer-plus-reset effect instead of exposing a transient claim that the
current production action mapping cannot observe.

## Rejected alternatives

| Alternative | Why it loses |
|---|---|
| Keep `dead`/claimed and ignore the stable production state | Leaves a real randomized conformance divergence and represents a claim the stable QPROJ observation does not hold. |
| Add a barrier after the brief production claim | Adds scheduling/test machinery solely to expose an internal transient and changes the effect-completion boundary. |
| Split `release_read` into forced bytes and a later reset release | Adds a new private seam operation and fabricates two driver effects where the current queued child has one buffered-record release. |
| Project `dead` from driver action history | Compares the model with driver bookkeeping and hides the real cleared owner claim. |
| Change production to delay the pending reset or retain the dead claim | Adds a runtime gate and prolongs a dead session solely to fit the model. |
| Close the queued child at `VmStop` | Contradicts K-A2-prime: the listener child and buffered `ControlOpen` survive VMM death until accept/read. |
| Add a `QueuedResetAfterOpen` formal action or a new named scenario | Unnecessary state/action/test growth; the existing `HostReadOpen` production effect already reaches the terminal stable state and the randomized seed proves it. |
| Change every writer-down/session-dead path | Broadens the correction and would destroy the separately playable live-session `VmStop -> HostDeadSessionLoss` contract. |

## Authorized alignment, cost, and validation

- **Runtime/API/wire/storage cost:** zero. Production already has the required
  fail-closed behavior.
- **Formal cost:** one existing-action branch changes; zero new state,
  actions, runs, scenarios, hazards, witnesses, or checks.
- **Authorized implementation scope:** `owner_flows.qnt` and its README
  wording only. The production owner, Sim adapter, driver, tests, and public
  surfaces do not change.
- **Required evidence:** exact replay of seed `0x6539f0dd`, every unchanged
  named and randomized `owner_flows` target, the complete guest-flow-owner
  record with exactly three jobs, `verify-evidence`, and independent
  source/proof review. Roadmap validation remains pending.

## Approval record and downstream gates

Independent DESIGN/formal review approved CONTROLOPENRESET1 for presentation
with zero open technical findings. On 2026-10-26 the user explicitly said
“Approve CONTROLOPENRESET1”. This authorizes only the exact existing-action
and README alignment above. It authorizes no barrier, forced-byte split,
projection shadow, production gate, public seam, new state/action/scenario, or
broader session semantic change.

DISTILL may align the existing `HostReadOpen` writer-down branch and exact
README/contract wording. Completion still requires exact replay of seed
`0x6539f0dd`, including Trace 33; every unchanged named and randomized
`owner_flows` target; the complete three-job guest-flow-owner record;
`verify-evidence`; and independent source/proof review. Roadmap validation
stays pending until those gates pass.
