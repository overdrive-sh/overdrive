# STEP 12-01 approved DESIGN contract — pre-Paired guest `Abort` dial error

## Status and boundary

**Status: PABORTERR1 APPROVED by the user 2026-10-26; post-approval fold
review, formal alignment and evidence remain pending.** Exact authority: “i
approve the rest of the decisions.” Technical review found zero open findings.
This approval authorizes only the exact mapping below; STEP 12-01 remains
non-executable until its remaining gates pass.

The bounded gap is one existing transition's typed payload. A guest-side
error on accepted but unattached `V_g` closes that guest flow and sends the
existing payload-free `ControlMessage::Abort { flow }`. When the host receives
that message while the matching dial is `AwaitingReply`, current production
returns `GuestDialError::Unreachable { errno: ECONNRESET }`; the model records
the less specific `Unreachable:io`. Guest and host phases, cleanup, wire
shape, and every other dial result already agree.

PABORTERR1 pins the current production mapping exactly. It adds
no error variant, payload, wire field, API, action, scenario, state,
mechanism, or runtime behavior.

## Reproduced divergence

Metal run `5c554937-a39a-40dc-b2aa-b64121b30f76` failed the existing
`AcceptedVgErrorAbortsWithAbort` conformance run with
`QUINT_SEED=0x3c09cb16`. The raw log is
`.context/netns-density-295/12-01/owner-flows-seed-3c09cb16-verbose1.log`.

The relevant suffix is:

```text
DialOpen(1)
  -> GuestAcceptV(1)
  -> GuestAcceptedError(1)
  -> HostRecvAbort(1)
```

After `GuestAcceptedError(1)`, the model and production both have accepted
guest flow 1 closed, host flow 1 still awaiting a reply, and one guest-to-host
`Abort` pending (raw lines 355-389). After `HostRecvAbort(1)`, both sides have
the host flow closed and the message consumed. The only mismatch is the
dialer's typed result: model `Unreachable:io`, production
`Unreachable:ECONNRESET` (lines 400-479). The exact replay fails at the
existing named run, not at a new or hypothetical state.

## Complete production path

1. The accepted guest-side flow is watched before pairing. A terminal
   `GuestHangup::Error(ErrorKind)` on either its accepted `V_g` or application
   socket enters `GuestFlowOwner::handle_hangup`
   (`crates/overdrive-init/src/guest_flow/owner.rs:662-699`). The owner aborts
   both sockets and writes `ControlMessage::Abort { flow }` while its control
   session is live.
2. `ControlMessage::Abort` has exactly the flow ID. Its canonical encoding
   requires detail and errno bytes to be zero
   (`crates/overdrive-core/src/guest_transport/wire.rs:458-489`, `:497-514`,
   `:555-562`). The guest's local `ErrorKind` is therefore not a host-visible
   errno and cannot be reconstructed at receipt.
3. The host control-session task decodes that message and calls
   `abort_guest_flow(..., FlowCloseCause::AbortReceived)`
   (`crates/overdrive-control-plane/src/guest_network/owner_runtime.rs:258-286`).
4. For the matching `DialPhase::AwaitingReply`, `abort_guest_flow` inserts the
   exact result `GuestDialError::Unreachable { errno:
   KernelErrno::ECONNRESET }` and cancels the dial
   (`owner_runtime.rs:1102-1153`).
5. The biased cancellation branch in `run_dial` calls `cancellation_error`,
   which removes and returns that stored result. Ordinary error cleanup then
   aborts the watch, closes owner-held `V_h`, removes the dial, and releases
   its places before returning (`crates/overdrive-control-plane/src/guest_network/dial.rs:261-271`,
   `:328-350`, `:353-390`).

The existing conformance projection deliberately distinguishes
`Unreachable { ENODEV }`, `Unreachable { ECONNRESET }`, and every other
`Unreachable` errno
(`crates/overdrive-control-plane/src/guest_network/quint_owner_flows.rs:1015-1027`).
The mismatch cannot be closed honestly by relabelling all three as generic
I/O.

## Exact approved contract

### Typed mapping

When all of these preconditions hold:

- the host receives the existing wire value `ControlMessage::Abort { flow }`;
- `flow` names a registered host-opened dial;
- that dial's phase is `DialPhase::AwaitingReply`; and
- no earlier owner transition has fixed the dial's result,

the owner fixes and returns exactly:

```rust
GuestDialError::Unreachable {
    errno: KernelErrno::ECONNRESET,
}
```

`ECONNRESET` is the mapping of the payload-free pre-Paired peer abort. It is
not derived from the guest's private `ErrorKind`, inferred from the action
name by the driver, or copied from a reserved wire byte. The existing
`GuestDialError::Unreachable` variant and `KernelErrno` payload are the exact
interface representation; no new public surface is required.

The public meaning of `Unreachable { errno }` is correspondingly precise: it
covers a host-side connect/watch/write I/O failure before pairing, including
`ENODEV` and the VMM-exit `ECONNRESET`, and the fixed `ECONNRESET` result of a
payload-free guest `Abort` received before pairing. The payload is the exact
host-observed errno where the failing operation supplies one; it is fixed to
`ECONNRESET` for this peer-abort transition because the wire supplies none.

### Existing action semantics

The only formal transition correction is the result assigned by the existing
`HostRecvAbort(h)` action when `hf(h).st == "req"` and `dl.res(h) ==
"pending"`:

```quint
dl' = dlEndOne(h, "Unreachable:ECONNRESET")
```

Every other formal effect remains unchanged: the message leaves
`toHostAbort`, the open host flow becomes `closed`, `toGuestAbort` retains its
current value, and guest state and the U-5 ghosts are preserved. This decision
does not add or remove any wire write. Tighten the final expectation of the
existing `AcceptedVgErrorAbortsWithAbort` run to require
`dl.res.get(1) == "Unreachable:ECONNRESET"`; add no run or scenario.

The correction is deliberately action-specific:

| Existing path | Result retained |
|---|---|
| `DialUnreachable` connect with no VMM device | `Unreachable:ENODEV` |
| `VmStop`/device release with the dial in flight | `Unreachable:ECONNRESET` |
| `HostAbortAccept` from another pre-pair `V_h` I/O failure | `Unreachable:io`, unless that separate action already has an exact modeled errno |
| `HostRecvAbort` while awaiting the guest reply | `Unreachable:ECONNRESET` |
| teardown / quiescence / observed session loss | `NotServing` / `Quiesced` / `NoSession` |
| payload-free `Abort` after the dial already closed | discarded under U-5; the fixed result is unchanged |
| `Abort` after `Paired` | existing held-stream/watch abort; the completed dial result remains `Ok` |

`conf_step = HostRecvAbort` retains the causal action identity even though a
VMM exit can produce the same typed payload. No driver-side action shadow,
normalization, or synthetic state is permitted.

## Contract compatibility

The decision preserves all accepted outcomes:

- **DG-5-E:** an error on accepted, unattached `V_g` closes it and sends one
  `Abort` over the live control session.
- **U-1/U-5:** the closed guest flow cannot later pair; a late message for an
  already-closed host flow is discarded.
- **K-A4:** cleanup closes `V_h`; its guest half tears down.
- **INB-1 typed error family:** the dial returns the existing typed
  `Unreachable` variant with an exact nonzero `KernelErrno` payload.
- **VMM-exit specificity:** `ECONNRESET` remains distinct from `ENODEV` and
  other I/O values in the model and driver. The same payload does not equate
  the separate `VmStop` and `HostRecvAbort` transitions.
- **Wire compatibility:** `Abort` remains type 3 with zero detail/errno bytes;
  no peer or version negotiation changes.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Normalize every `Unreachable` result to `Unreachable:io` in the driver | Erases the accepted `ENODEV` and VMM-exit `ECONNRESET` payloads and compares a weaker abstraction than the public typed error. |
| Keep `HostRecvAbort` as `Unreachable:io` and special-case the driver action | Makes the conformance harness reinterpret a real result from the action name; the owner remains unmatched. |
| Add an errno to `ControlMessage::Abort` | Changes the public wire/API while the accepted Abort contract intentionally has no cause payload; the current owner already defines a total mapping. |
| Reuse the guest's private `GuestHangup::Error(ErrorKind)` | That value does not cross the wire, and `ErrorKind` is not the approved `KernelErrno` representation. |
| Map to `Refused`, `NoSession`, or `PairingDeadline` | The guest sent Abort rather than Refused, the live session delivered it, and a terminal reply arrived before the deadline. |
| Add a new `PeerAborted` error variant | Unnecessary public API and caller fallout for a condition the existing typed family and current production already represent exactly. |

## Approved canonical and downstream changes

User authority is recorded as PABORTERR1 on 2026-10-26 by the exact quote “i
approve the rest of the decisions.” The bounded alignment surface is:

- `specs/quint/guest-flow-owner/owner_flows.qnt`: the `Dl` vocabulary comment,
  only `HostRecvAbort`'s pending result, and the existing named-run
  expectation;
- `specs/quint/guest-flow-owner/README.md`: the action/result map;
- `crates/overdrive-core/src/guest_transport/dial.rs`: documentation of the
  already-existing `Unreachable` payload meaning, with no enum or behavior
  change; and
- generated evidence through the normal formal record workflow.

The production owner and conformance driver require no behavior change. Any
actual Quint input change requires exact replay of seed `0x3c09cb16`, every
unchanged named and randomized `owner_flows` target, the complete
guest-flow-owner re-record with exactly three jobs, `verify-evidence`, and
independent proof/source review. Roadmap validation remains pending until all
STEP 12-01 formal corrections and evidence gates are complete.
