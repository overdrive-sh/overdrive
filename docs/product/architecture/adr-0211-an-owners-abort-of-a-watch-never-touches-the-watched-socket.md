# ADR-0211 — An owner's abort of a watch never touches the watched socket; only the context's release shuts it

## Status

**Accepted — WATCH-ABORT, the user's ruling of 2026-10-17** (the
recommended option of `spike/findings-ev-ctl.md` § 4.3). GH #295. Not yet
implemented. Contract: the #295 feature delta, § *[REF] vsock data path —
the in-kernel event-driven forwarder (EVMOD)*, "End classification" ("A
watch ended by the owner", "Only the context's release shuts watched
sockets") and "Control interface" (`OVD_FWD_ABORT`).

Related decisions, each separately reversible: the watch of a dialled vsock
(ADR-0216); the context's lifetime and its release (ADR-0199); the end
record (ADR-0202); how a `GuestStream`
resolves its end (EVMOD-STREAM-END) and how an owner abort reaches the
stream and the guest (EVMOD-STREAM-ABORT), both in the feature delta's
§ *[REF] Driving port — guest stream dial*.

## Context

A host platform client's dial is a flow whose host vsock, `V_h`, is held and
read by the client (leg-S or the VM probe runner) as a `GuestStream`, and
watched, not forwarded, by the host forwarder (ADR-0214, ADR-0216). The owner
aborts a watch on quiescence's first hold, teardown, control-session loss or
the `Forwarder` recovery.

If the forwarder shut `V_h` itself, it would change the holder's socket
under it: after the forwarder's `SHUT_RDWR`, `vsock_shutdown` clears
`SOCK_DONE` (`net/vmw_vsock/af_vsock.c:1220 @ v7.0`), and once the socket
has left `ESTABLISHED` without it `recvmsg` returns `-ENOTCONN` rather than 0
(`:2522-2531`, 0 only while `ESTABLISHED`, `:2545-2548 @ v7.0`). Measured:
`ENOTCONN` in 10 of 400, 4 of 400 and 1,916 of 4,096 reads, the rest 0
(P-70 § 3.5).

At an owner abort the owner is present: it can end every call on the stream
itself and tell the guest on the control session. At the context's release
no owner remains to do either (ADR-0199).

## Decision

- **An owner's `abort` or `abort_all` of a watch queues the watch's record,
  removes the forwarder's callbacks and drops the forwarder's reference, and
  never touches the holder's socket**: no shutdown, no state change, nothing
  sent on the vsock.
- **Only the context's release shuts watched sockets**, with
  `shutdown(SHUT_RDWR)`, which the guest classifies as an abort.
- **The holder never takes its outcome from its own read**: after its read
  ends — EOF or an error — the outcome comes from the owner's own end of the
  watch or the watch's record (EVMOD-STREAM-END).

## Alternatives considered

- **The forwarder shuts `V_h` on an owner abort** (EVMOD-WATCH-SHUT). It
  changes the holder's socket under it, whose `read` then returns 0 or
  `ENOTCONN`, while the owner can end the stream and tell the guest itself.
  Rejected.
- **The release also leaves watched sockets untouched.** With no owner left,
  nothing would end the guest application's connection until the holder
  closed its descriptor. Rejected.

## Consequences

- After an owner abort the holder's socket is unchanged; the owner's stream
  calls return `ConnectionAborted`, and the guest ends its pair on the
  owner's `Abort { flow }` or on the holder's close of `V_h`
  (EVMOD-STREAM-ABORT).
- After a release the holder's `read` may return 0 or fail `ENOTCONN`; in
  production a release either ends the process that holds the streams or,
  in the `Forwarder` recovery, follows the owner's abort of every stream.
- The Quint module `fwd_pair` checks the rule (third iteration; runs
  `OwnerAbortOfWatchLeavesVhAndStreamReportsAborted` and
  `ContextReleaseShutsWatchedVhAndHolderReadsNotConnected`).
