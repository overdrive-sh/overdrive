# ADR-0166 — Flow control messages travel on one dedicated per-VM vsock control session owned by the guest-flow owner, not on the beacon

## Status

**Accepted — carrier sub-decision D18a approved by user 2026-10-05; the
handling of a control message for an unknown or closed flow (U-5) pinned at
the user's direction and its exact rule (discard and count, no reply)
confirmed by user 2026-10-06, with its assumption K-A4 and validation item
V-24; ATTR-TOKEN and ATTR-LIVE-CLAIM approved by user directive 2026-10-09;
the pairing replies of host-opened flows (INB-1), the
flow-end classification that leaves no half-close message on the session
(R2-END) and NO-SURVIVE approved by user 2026-10-11.** GH #295. The
out-of-band pairing order (D18) is ADR-0158. Recorded in the #295 feature delta, § *[REF] Control session*.
Which launch a session belongs to is ADR-0177. K-A4 was validated by V-24
(feature delta P-38).

## Context

ADR-0158 moves `Paired`, `Refused` and `Abort` off the flow connections. They
need a per-VM carrier whose sender is attributable to the VM. The VM already
has one host session: the beacon (ADR-0157), owned by the VM driver, which
carries READY / EXEC / EXIT authority with its own lifecycle.

## Decision

- The guest opens one dedicated control session per VM, a vsock STREAM to a
  host port owned by the guest-flow owner, and keeps it for the VM's life,
  reconnecting after loss. Within one VM's life the host side of the session
  is lost only inside one `serve` process: the owner's control listener task
  exits and is re-bound by in-process recovery (ADR-0124), and the guest
  reconnects. A `serve` restart ends the VM (ADR-0159), so no session
  reconnects across `serve` processes.
- The host attributes a session by the kernel-reported peer CID and the
  launch token of the session's opening record (ADR-0177), exactly as the
  beacon is attributed: the token must equal the CID's live launch token,
  which exists only while the CID is Provisioned or Active.
- **At most one live session per CID.** Accepting a session is an atomic claim
  on the CID, taken in one per-CID step after the opening record is read,
  which requires both that its token equals the live launch token and that
  the claim is free (ATTR-LIVE-CLAIM, ADR-0177); a second session for a CID with
  a live claim is closed. Teardown ends the live token in the step that
  closes the session and releases its claim, so a connection read after
  teardown takes no claim. Releasing the claim happens when the session
  ends.
- Records on the session are fixed-size, written and read in userspace only:
  one opening record, then control messages. The messages are `Paired`,
  `Refused`, `TimedOut` and `Abort`. `Refused` and `TimedOut` carry the
  guest's errno when they answer a host-opened flow's request (ADR-0173). No
  message carries a flow's half-close or end: the kernel's vsock `SHUTDOWN`
  flags classify it (ADR-0180).
- Loss of the session aborts every flow of that VM, because no further
  `Paired` can be delivered.
- A host-side flow request from a CID with no live session is not paired: the
  host closes that flow connection without a control message. A host-opened
  dial to a VM with no live session is refused before any vsock is opened
  (ADR-0173).
- **A control message for a flow the receiver does not hold is discarded
  (U-5).** A `Paired`, `Refused`, `TimedOut` or `Abort` naming a flow id the
  receiver never opened or has already closed is dropped and counted, with no
  reply. The one exception is the guest's datagram slot rule — a late
  `Paired` for a released slot is answered with `Abort` (ADR-0150). No reply
  is needed because the side that closes a flow before `Paired` sends `Abort`
  while the session is live, session loss aborts every flow on both sides,
  and, for a host-opened flow whose guest vsock the guest has not yet
  accepted (so the guest holds no entry for it), the host's close of its own
  vsock reaches the guest and ends the guest side when the guest accepts it —
  on the same or a reconnected session (assumption K-A4).

## Alternatives considered

- **Control messages on the beacon session.** Couples two owners and two
  lifecycles on one socket. Rejected.
- **One control message stream per flow (in-band).** Falsified by ADR-0158's
  evidence. Rejected.
- **Answer a control message for an unknown flow with `Abort`.** Adds traffic
  the closing side's own `Abort` and socket close already make redundant, and
  does not reach a guest whose session was lost. Rejected.
- **A guest tombstone for an `Abort` of a host-opened flow it does not yet
  hold.** Covers an abort before the guest accepts, but not a session loss
  before the guest accepts: the guest's tombstone dies with the session. The
  model shows the second path remains (`owner-flows-alt-tombstone-misses-close`).
  Rejected in favour of relying on, and validating, K-A4.

## Consequences

- One more vsock listener on the host (the control port, default 1243;
  node configuration, ADR-0218) and one session per VM.
- Activation of a VM's forwarding requires its live session (G-V3 in the
  feature delta).
- A guest that cannot hold its control session has no working flows; READY is
  unaffected.
- Attribution by peer CID alone across allocations is unsafe: a session a
  dead VM left in the host's accept queue is accepted with the CID's next
  holder as peer and delivers its buffered bytes (V-22, feature delta P-37).
  The launch token (ADR-0177) makes the attribution exact.
- Discarding unknown-flow messages without reply is safe under K-A4:
  closing the host's socket of an aborted or session-lost host-opened flow
  reaches the guest and ends the guest-side connection, including one still
  in the guest's accept queue — after its request is read, the guest vsock
  reads EOF, reports `POLLRDHUP` and fails its first write (validated by
  V-24, feature delta P-38). The guest owner checks for that end after the
  request read. No byte of such a flow reaches the application because the
  host writes nothing after the request before `Paired` (ADR-0158, H-1).
- Even under K-A4, a guest application can accept a connection for a client
  the host already aborted and then see it reset at once.
- The session protocol, including these rules, is covered by the feature
  delta's § *Formal protocol model*.
