# STEP 12-01 `fwd_pair` abort/handoff reconciliation

## Status

**APPROVED THROUGH REVIEW ITERATION 4 — canonical fold and bounded formal/
conformance implementation eligible; replacement evidence pending.** The
Trace-14 recovery and Trace-47 `TakeEnd` cleanup branches have zero open
DESIGN findings. This approval is not formal proof, roadmap approval or commit
authority; roadmap validation remains `pending` until every proof gate below
passes.

The proposal preserves the two already-approved outcomes:

- an `AwaitingReply` dial ended by quiescence returns `Quiesced`, aborts its
  watch, closes the still-owner-held `V_h`, releases its places, and sends
  `Abort { flow }` while the control session is live (R5-53); and
- a `Paired` dial held by a `GuestStream` is ended under
  EVMOD-STREAM-ABORT: the watch is aborted, the holder's `V_h` is untouched,
  pending and later stream calls return `ConnectionAborted`, and
  `Abort { flow }` reaches the guest (R5-63(a), WATCH-ABORT).

No accepted `zero`, `never`, `must`, exact typed outcome, ownership boundary,
or lifecycle invariant is weakened.

## Scope

This is a formal-abstraction correction for `fwd_pair` and the existing
private `quint_fwd_pair` conformance projection/action mappings. It proposes
no production behavior or public API change, no new model action, named run,
scenario, lane, seam, or production mechanism. `owner_flows` remains the
model that owns the dial result, late-control-message discard, and K-A4 guest
teardown behavior.

## Reproduced counterexample and production reachability

The randomized conformance replay at seed `0xea0ef1a4`, Trace 11, reaches:

```text
InitWith("watch") -> Abort("h")
```

The retained trace finding is recorded in
`deliver/distill-12-01-alignment.md:1045-1068`. The failure is reachable
through the real production owner path:

1. `InitWith("watch")` starts `GuestStreamDial::dial` and settles until the
   watch and request exist, but retains only the dial future; `stream` is
   `None` (`quint_fwd_pair.rs:560-621`).
2. Production `run_dial` owns `V_h`, installs its watch, writes the request,
   and waits for `Paired` (`guest_network/dial.rs:210-272`). `V_h` moves into
   a `GuestStream` only in the `Paired` branch
   (`guest_network/dial.rs:273-311`). The public observation names these
   states `DialPhase::AwaitingReply` and `DialPhase::Paired`, the latter
   explicitly held by its `GuestStream` (`flow_owner.rs:495-513`).
3. `Abort("h")` calls the real `owner.quiesce()`
   (`quint_fwd_pair.rs:649-655`). On its first hold, `quiesce` records
   `Quiesced` for every `AwaitingReply` dial, cancels it, clears the dial
   tables, and calls `abort_all` (`guest_network.rs:1270-1326`).
4. The cancelled `run_dial` takes its error cleanup: it sends
   `Abort { flow }` after a written request, aborts the watch, closes its
   still-owned `V_h`, and removes the dial (`guest_network/dial.rs:328-354`).

The actual post-state is therefore a queued `Detached` watch record, sent
SHUTDOWN `{R,S}` on `V_h`, a `Quiesced` dial result, and no `GuestStream` to
mark owner-aborted.

The current model instead initializes the watch as live while
`strmAbort = false` (`fwd_pair.qnt:478-494`), enables `Abort(h)` from watch
liveness alone, and unconditionally sets `strmAbort = true` while preserving
`V_h` (`fwd_pair.qnt:572-585`). Its action therefore applies the held-stream
contract before the stream handoff. The accepted production contract is not
falsified; the model premise that every live watch already has a holder is.

### Trace 14 — pending terminal record before dial cleanup

Metal run `07069ac6-5be7-47a5-9f05-26b0cc412ff8` replayed
`QUINT_SEED=0xee26ec37`; the raw trace is
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ee26ec37-verbose1.log`.
Trace 14 reaches:

```text
InitWith("watch")
  -> TcpRst("g", false)
  -> VmmExit
  -> FwdErr("h")
  -> CtxRelease("h")
```

Immediately before `CtxRelease(h)`, the host watch is ended with one queued
`SocketError` record (`pr(h).ph = ended`, `rec = queued`, `nrec = 1`),
`strmHeld = false`, `strmAbort = false`, `hres = none`, the old context is
open, `V_h` has `err = true` but has sent no shutdown, and `vmUp = false`
(raw log lines 2066-2091). The queued record has ended the forwarder watch,
but the owner end-record task has not consumed it and the real dial remains
`AwaitingReply` with its owner-held `V_h`.

`CtxRelease(h)` selects Forwarder recovery because no pair/watch remains
live. Its required `owner.quiesce()` observes that `AwaitingReply` dial,
records `Quiesced`, cancels it and lets `run_dial` finish its existing error
cleanup. That cleanup closes owner-held `V_h`; because the VM/control session
is gone it sends no `Abort { flow }`. The exact observed delta is only
`vs(h).sent: {} -> {R,S}` and `vs(h).sshut: false -> true`; the queued
`SocketError` record/cause/count, `strmHeld = false`, `strmAbort = false`,
`hres = none` and `guestAbortSent = false` remain unchanged (raw log lines
2093-2162).

This falsifies the iteration-2 premise that every holder-free ended watch has
already completed typed-error cleanup before recovery. It does not falsify
the ownership cut, held-stream recovery, the queued record, the typed dial
outcome, or production behavior.

### Trace 47 — record consumption triggers the same pending-dial cleanup

Metal run `76bf4c31-9881-4fd7-aea7-c7d60e5686ea` replayed
`QUINT_SEED=0xea0ef1a4`; the raw trace is
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ea0ef1a4-attempt2-verbose1.log`.
Trace 47 reaches:

```text
InitWith("watch")
  -> CloseUnattached("g")
  -> CtxRelease("g")
  -> VmmExit
  -> FwdErr("h")
  -> VCtl("gh")
  -> TakeEnd("h")
```

Immediately before `TakeEnd(h)`, the holder-free host watch has one queued
`SocketError` record, `strmHeld = false`, `strmAbort = false`, `hres = none`,
`ep(h) = alive`, `V_h.sent = {}` / `sshut = false`, and `vmUp = false`; the
owner still holds an `AwaitingReply` dial and `V_h` (raw log lines 9497-9549).
The model and driver agree on every projected field.

The real `TakeEnd` releases that record to `process_end`, which marks the
record consumed, fixes the pending dial result to
`Unreachable { errno: ECONNRESET }`, and cancels its task. `run_dial` then
performs the mandatory error cleanup: stale abort of the already-ended watch,
dial removal and close of owner-held `V_h`. No guest `Abort` is sent because
the VM/session is gone. The post-state keeps the same `SocketError`, now with
`rec = read`, `nrec = 1`, `strmHeld = false`, `strmAbort = false`,
`hres = none`, and `guestAbortSent = false`; only `sent`/`sshut` differ from
the current model (raw log lines 9551-9629).

This falsifies the current watch-mode `TakeEnd` assumption that a watch's
`V_h` is always holder-owned. It does not change record classification,
consumption order, the held-stream path, or production behavior.

## Exact proposed formal delta

### 1. Represent the existing ownership cut directly

Add one `fwd_pair` state variable:

```quint
var strmHeld: bool
```

Its meaning is exact: the dial future returned `Ok(GuestStream)` and that
move-only stream is still held by the conformance driver. It is not a new
product state. On the host driver it is projected from the already-existing
real holder fact `self.stream.is_some()`; the driver must not add a shadow
boolean or infer it from owner-table absence. It is a top-level host-driver
projection beside `hres`, compared even after the owner projection is absent;
owner death does not destroy the independently held `GuestStream`. It is
model-only for the guest driver, which does not hold the host stream.

State transitions are:

- `InitWith("watch")`: `strmHeld = false`;
- the existing successful `Attach("g")` step, which includes delivery of
  `Paired` and completion of the dial future: `strmHeld = true` only when that
  actual future returns `Ok(GuestStream)`;
- a late `Attach("g")` after the host ended the pending dial: the existing
  late `Paired` is discarded, the actual future returns its already-fixed
  error, and `strmHeld` remains false even if the guest pair attached locally;
- `HolderDrop` of an actual stream: `strmHeld = false` and `strmAbort = false`
  after the existing watch-abort/`V_h`-close effect; and
- every other action preserves `strmHeld`.

This variable is required rather than a derived `attRes("g")` guard. A late
`Paired` can change guest-side attach facts after the host already ended the
dial, while a handed-out stream remains held after its watch later ends.
Those histories can have the same current attach/watch values but different
ownership. `strmHeld` records the production ownership handoff itself and is
projected from the real object, so it is neither a trace filter nor invented
harness state.

The model-side handoff predicate is exact and uses existing pre/post-state:

```quint
mode == "watch"
  and s == "g"
  and live(cur, "h")
  and next.attRes.get("g") == "ok"
```

Here `next` is the existing `attachEff` result. A refused guest attach fails
the final clause; a late `Paired` after host cancellation fails the live-watch
clause. The host driver cross-checks that predicate against the actual dial
future result and projects only the real `stream` object's presence.

### 2. Keep `Abort(h)` enabled and branch on the ownership cut

The existing `Abort(h)` action remains enabled in the Trace-11 pre-handoff
state. It gains two effects selected by pre-state `strmHeld`:

| Pre-state | Required post-state |
|---|---|
| `strmHeld = false` | Abort the watch and queue its one `Detached` record; close the owner-held `V_h`, so `vs(h).sent = {R,S}`, `vs(h).sshut = true`, and `ctl(hg)` carries `{R,S}`; keep `strmAbort = false` and `hres = none`; set `guestAbortSent = vmUp` for the real control write; release the dial through its already-modelled owner path. |
| `strmHeld = true` | Preserve the existing EVMOD-STREAM-ABORT effect unchanged: abort the watch and queue its one `Detached` record; leave `V_h` and its queues untouched; set `strmAbort = true`; keep `strmHeld = true` until the holder drops the object; send `Abort { flow }`. |

`gh.vhTouched` continues to mean that an owner abort changed a
**holder-owned** `V_h`. Closing `V_h` while `strmHeld = false` does not set
that ghost because no holder socket exists at that cut. The concrete socket
facts still record the close and remain compared through QPROJ-8.

### 3. Close every existing holder action over the ownership cut

Every action that represents a call or observation through the host
`GuestStream` uses `strmHeld` as an enabledness premise. The correction is
limited to the dialer branch of each existing action; ordinary stream-pair and
guest-side behavior is unchanged.

| Existing action or guard | Exact watch-mode contract |
|---|---|
| dialer `Write("hg")` | Enabled only with `strmHeld`, in addition to its existing direction, capacity, error and shutdown guards. |
| dialer `Fin("hg")` | Enabled only with `strmHeld`, in addition to its existing write-open guards. |
| dialer `Read("gh")` | Enabled only with `strmHeld`, in addition to its existing queued-unit and read-open guards. |
| `HolderEnd(k)` / `holderReadable` | Enabled only with `strmHeld`; the host driver also requires `self.stream.is_some()` before stimulating the held stream's existing Sim socket. |
| dialer `ReadEof("gh")` | Enabled only with `strmHeld`, in addition to its existing owner-end or resolved-record guard. |

This closes the production-impossible successor
`Abort("h") -> Attach("g") -> HolderEnd -> ReadEof`: the late attach may make
the guest pair live, but it cannot create a host holder after the dial already
returned `Quiesced`. The holder-only actions are disabled because
`strmHeld = false`; the existing guest and vsock actions remain enabled and
carry the live guest pair to its accepted end.

### 4. Preserve late attach and late control-message orders

No `Paired` or discard action is added.

- If the existing `Attach("g")` step follows pre-handoff `Abort("h")`, its
  control reply is played through the real owner and discarded. The guest
  pair may still attach locally if the host close has not reached it; the
  actual dial future returns `Quiesced`, so the driver leaves `stream = None`
  and the model leaves `strmHeld = false`.
- `guestAbortSent` continues to record the real `Abort { flow }` write. The
  existing `GuestAbort("g")` action is enabled for delivery while the VM and
  guest context live, including after the guest pair has already ended. Its
  effect is selected by the actual guest pair phase, never by `strmHeld`:
  - in watch mode, `pr(g).ph = live` aborts that guest pair whether
    `strmHeld` is true or false;
  - in watch mode, `pr(g).ph = unattached`, `ended`, or `refused` discards
    the message and changes no guest pair; and
  - outside watch mode, the existing live-pair abort and unattached-flow
    close remain, while an already ended or refused flow discards the late
    message.
  Every branch clears `guestAbortSent`. This preserves both R5-28 orders:
  `Abort` before the guest holds a flow is discarded, while `Attach("g")`
  before delivery makes a live guest pair that the same `Abort` ends even
  though the host never received a stream. If the already-closed `V_h`
  reaches and ends the guest pair first, the later `Abort` is the ordinary
  U-5 closed-flow discard.

The corresponding model guard is only
`guestAbortSent && vmUp && ctx(g) == "open"`; the effect then inspects
`pr(g).ph`. `fwd_pair` has no accepted-but-unattached guest-owner state:
`Attach("g")` is its atomic guest attach/`Paired` action. The separately
accepted-unattached V_g branch is owned and retained by `owner_flows`, where
`GuestRecvAbort` closes `gf = acc`. No `fwd_pair` phase is invented to copy
that other model's state.

The private host driver maps these branches from facts it already owns:
`stream`, `dial`, the production dial result, and the production owner's
control outbox. It must not inject `Paired`, fabricate `strmAbort`, suppress
the socket shutdown projection, or maintain a second phase flag.

### 5. Pin recovery and `HolderDrop` terminal semantics

`CtxRelease("h")` retains its existing state-selected meanings. A live host
watch still selects owner death. When no watch is live and the action selects
the `Forwarder` recovery close, `recoveryClose` branches as follows:

- `strmHeld = true`, the watch already ended, and the stream was not already
  owner-aborted: set `strmAbort = true`, keep `strmHeld = true`, preserve an
  already reported `hres`, and record `Abort { flow }` while the VM/session
  is live (U-R1);
- `strmHeld = false` and the watch is ended: preserve its existing record,
  cause and count, and apply `shutAll` to owner-held `V_h`. This ensures
  `vs(h).sent = {R,S}` and `sshut = true` when typed-error cleanup is still
  pending, and is idempotent when an earlier cleanup already closed the
  socket. Keep `strmAbort = false` and `hres = none`. Set a pending guest
  abort only when the cleanup is newly required while `vmUp` is true; preserve
  the existing message state after an already-closed socket. In the reproduced
  VM-gone state `guestAbortSent` remains false. No new watch record is created
  and the existing queued `SocketError` is neither consumed nor reclassified.

The holder-free branch needs no new state. Its exact message/close selector is
the existing socket fact:

```quint
val cleanupPending = t.vs.get("h").sent != both
val closed = shutAll(t, "h")
{ ...closed,
  strmAbort: false,
  hres: "none",
  guestAbortSent: t.guestAbortSent or (cleanupPending and t.vmUp) }
```

`shutAll` is idempotent. In Trace 14 `cleanupPending` is true and `vmUp` is
false, so only the concrete close facts change. If the same record window is
reached while the model's session/VM is live, this is the already-accepted
pending-dial cancellation `Abort { flow }`; if cleanup already closed `V_h`,
the branch preserves the existing message state and adds nothing.

The same selector is the one accepted pending-dial error cleanup when the
owner consumes the terminal watch record. `TakeEnd(s)` first performs its
existing `rec: queued -> read` update. It then applies the selector above only
when all of these existing facts hold:

```quint
s == "h"
  and watchSide(t, "h")
  and not(t.strmHeld)
  and t.ep.get("h") == "alive"
```

It runs the selector over the post-consumption state, so `rec = read`, the
original outcome/cause and `nrec` are preserved. A held stream keeps the
current watch-mode no-close result. A dead dialer endpoint keeps its completed
cleanup result. In the reproduced VM-gone state only `V_h`'s sent/own-shutdown
facts change.

`HolderDrop` keeps its one existing action and has exactly two enabled
pre-states:

| Pre-state | Required effect |
|---|---|
| `strmHeld = true` | Drop the actual stream: abort a still-live watch, close `V_h`, set the existing `gh.dropped` ghost, then clear both `strmHeld` and `strmAbort`. If the owner already ended the watch, only `V_h` closes. |
| `strmHeld = false` and the host watch is still live | Cancel the pending dial future: abort the watch, close the owner-held `V_h`, leave `strmAbort = false` and `gh.dropped = false`, and retain the existing pending-dial `Abort { flow }` outcome. |

With `strmHeld = false` and the host watch already ended, `HolderDrop` is
disabled. Dropping or joining an already-completed dial future has no protocol
effect; it is not a third drop transition. The private driver may retain that
completed handle until a late `Attach("g")` consumes its fixed error or until
the driver is dropped. This removes no observable outcome and avoids a model
stutter action whose only effect would be test bookkeeping.

Its exact corrected guard is the existing mode/endpoint/fault guard plus
`strmHeld || live(cur, "h")`. The two disjuncts select the two table rows
above; no third phase or fallback is inferred.

### 6. Tighten existing properties without adding a scenario

The existing held-stream rules retain their exact strength:

- `WatchAbortLeavesHolderSocket` applies to the `strmHeld` branch and still
  forbids any holder-socket mutation; its existing property also carries
  `strmAbort => strmHeld`, with both facts cleared together on holder drop;
- `NoHolderReadAfterWatchAbort` still follows from `strmAbort`, and every
  holder read/call now separately requires `strmHeld`;
- `GuestPairEndsAfterOwnerAbort` remains the held-stream liveness property;
- `RecoveryEndsHeldStream` uses `strmHeld` as its ownership premise and
  constrains the `recoveryClose` transition itself, rather than only its
  post-state invariant; and
- `NoLiveWatchAfterDrop` remains the actual-stream-drop rule.

Pending-dial cancellation remains owned by R5-53 and the existing
`owner_flows` runs (`QuiesceAbortsDialAndSendsAbort`, late `Paired` discard,
and K-A4 host-close teardown). The Trace-11 replay supplies the existing
`fwd_pair` conformance path. No new named run, scenario, hazard, or check is
proposed.

## Iteration-1 finding disposition: reachability and necessity

The independent review's F1 `SPECIFICATION_AMBIGUITY` is resolved by the
guards and effects above. Each added clause is tied to a current reachable
trace and real owner behavior:

| Required clause | Reachability and production evidence | Why it is necessary |
|---|---|---|
| Holder-only guards | Existing `canAttach(g)` permits an ended host watch, so `InitWith("watch") -> Abort("h") -> Attach("g")` can leave `attRes(g) = ok` and a live guest pair. Current `holderReadable`, dialer `Write`/`Fin`/`Read`, and `ReadEof` use attach/socket facts without an actual stream (`fwd_pair.qnt:865-959`); the host driver's `ReadEof` fails when `self.stream` is `None` (`quint_fwd_pair.rs:908-921`). | Without the `strmHeld` guards the corrected model still manufactures holder calls after the real dial returned `Quiesced`. |
| Late attach with no handoff | R5-28 permits the guest to attach and send `Paired` after the host ended the dial; the host discards it. `owner_flows` reaches this in `LatePairedForClosedHostFlowIsDiscarded`, and production `run_dial`'s biased cancellation re-check fixes the error before any late reply can hand out `V_h` (`dial.rs:261-300`). | Guest-pair ownership and host-stream ownership must remain independent; filtering `Attach(g)` would remove an accepted ordering. |
| Holder-free recovery | Two existing states reach it. After a completed pre-handoff abort, `shutAll` is idempotent. In Trace 14, `FwdErr(h)` has queued the watch's `SocketError` but the end-record task has not consumed it, so the dial remains `AwaitingReply` with owner-held `V_h`; recovery quiescence cancels it and the real cleanup closes that socket. | The transition branches on `strmHeld` and preserves `strmAbort = false`; its holder-free branch must also close owner-held `V_h` without replacing/consuming the existing record. Assuming cleanup already completed is false. |
| Holder-free `TakeEnd` | Trace 47 consumes the queued watch record while the dial is still `AwaitingReply`; `process_end` fixes `Unreachable { ECONNRESET }`, cancels `run_dial`, and its error cleanup closes owner-held `V_h`. | After the existing `rec = read` update, the watch/no-holder/alive-endpoint branch must reuse the same close/message selector without changing the record, holder facts or result class. |
| Guest `Abort` before and after local attach | Production discards an `Abort` for an unknown/already-ended flow, aborts an accepted-unattached V_g/C, and aborts a paired flow according to the actual guest owner tables (`overdrive-init/src/guest_flow/owner.rs:610-629`). `owner_flows` reaches the unknown-flow discard and the late-paired live-flow abort in `DialAbortedBeforeGuestAcceptsEndsByHostClose` and `LatePairedForClosedHostFlowIsDiscarded`. | `strmHeld` is a host-holder fact and cannot decide whether the guest owns a pair. `GuestAbort(g)` must branch on `pr(g).ph`; otherwise one of R5-28's two orders is lost. |
| Post-end `HolderDrop` disabledness | The corrected pre-handoff abort leaves `strmHeld = false` and the watch ended, while the private driver may still contain a completed join handle. Current `HolderDrop` is enabled by `mode`, `ep(h)`, and the fault budget alone (`fwd_pair.qnt:986-997`). Dropping an already-completed future changes no production protocol state; the pending cancellation already happened. | Treating it as a stream drop would falsely set `gh.dropped` and close `V_h` again. A new inert action/state would model only driver bookkeeping, so the existing action is disabled in this terminal combination. |
| Held-stream recovery and drop | A real handed-out stream remains independently held after its watch ended; the existing named run `RecoveryEndsHeldStreamWhoseWatchEnded` reaches that state. Production recovery ends it through the owner while preserving an already reported watch result; later object drop closes only `V_h`. | `recoveryClose` must set `strmAbort` only with `strmHeld`, and actual `HolderDrop` must clear both current-holder facts while preserving `hres`. |

The review's requested surface expansions are rejected where no additional
mechanism is needed:

- no new dial-phase enum or driver flag: the actual `GuestStream` object's
  presence is the authoritative host-holder fact;
- no new late-`Paired`, discard, terminal-drop, or recovery action: the
  existing actions already represent every observable transition;
- no new named run or scenario: the reproduced randomized trace covers the
  pre-handoff cut, the existing held-stream runs cover the other cut, and the
  existing `owner_flows` runs cover both R5-28 message orders; and
- no production correction: the real owner already implements the accepted
  outcomes. Only the formal abstraction and its private conformance mapping
  are wrong.

The bounded transition audit for this ownership premise is complete:

- `Abort(h)` with no holder and `HolderDrop` with a live pending watch already
  initiate the accepted cancellation cleanup directly;
- refused watch-mode `Attach(g)` invokes the same dial-error cleanup through
  its real `Refused` result (F44-01);
- holder-free recovery invokes it through quiescence (Trace 14);
- holder-free `TakeEnd(h)` invokes it through `process_end` (Trace 47);
- `FwdErr(h)` only queues the terminal record, and `VCtl`, `VmmExit`,
  held-stream `TakeEnd`, non-watch `TakeEnd` and `OwnerClose` do not invoke
  this pending-dial cleanup class.

No further transition is added or changed by this audit.

No new user decision is required because every branch above is an exact
restatement of R5-28, R5-53, R5-63, U-5, K-A4, WATCH-ABORT, and U-R1. Any
alternative that changes one of those outcomes remains outside this proposal
and requires explicit user approval.

## Contract and surface audit

| Boundary | Disposition |
|---|---|
| `GuestStreamDial`, `GuestStream`, `GuestStreamIo` | Unchanged. |
| `DialPhase::{AwaitingReply, Paired}` | Unchanged; these existing states are the production fact the model now represents. |
| `GuestDialError::Quiesced` | Unchanged. |
| EVMOD-STREAM-ABORT / WATCH-ABORT | Unchanged for a held stream; `V_h` remains untouched. |
| R5-53 pending dial cancellation | Unchanged; owner-held `V_h` is closed and the dial returns `Quiesced`. |
| Quint actions and named runs | Same action set and same run/check inventory. |
| Conformance support | One top-level private state projection from the existing `GuestStream` field and bounded guards/effects for existing actions; no public or production seam or shadow phase. |
| Production code and tests | No change authorized by this proposal. |

## Validation invalidation and resumption gate

The reproduced falsifications invalidate the current `fwd_pair` live-watch ⇒
held-stream premise, iteration 2's holder-free-ended-watch ⇒ cleanup-complete
premise, the watch-`TakeEnd` ⇒ holder-owned premise, their affected
action/projection text, and the recorded
guest-flow-owner evidence as authority for those premises. They do not
invalidate R5-53, R5-63, EVMOD-STREAM-ABORT, WATCH-ABORT, the production
owner implementation, or the other Quint modules.

Roadmap validation remains `pending` until all of these occur in order:

1. a bounded model/conformance crafter implements only the approved spec,
   README/feature-delta mapping, and private driver projection/action changes;
2. exact seeds `0xea0ef1a4` and `0xee26ec37` replay through the real owner
   without a fabricated state or filtered action;
3. the complete formal input change is re-recorded with the required
   three-job command:

   ```text
   cargo xtask metal run -- cargo xtask quint check --subsystem guest-flow-owner --jobs 3 --record
   cargo xtask metal run -- cargo xtask quint verify-evidence --subsystem guest-flow-owner
   ```

4. the existing `pair_conf` named runs and randomized host/guest conformance
   drivers pass, including the unchanged held-stream owner-abort runs; and
5. an independent review verifies the new record, the exact existing-action
   guards/effects above, both ownership branches, recovery, both late-message
   orders, post-end `HolderDrop` disabledness, and the absence of public API,
   new scenario, trace filtering, or shadow harness state before roadmap
   validation is approved.

No model input or recorded evidence is changed in this proposal phase.
