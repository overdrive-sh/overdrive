# ADR-0166 — Flow control messages travel on one dedicated per-VM vsock control session owned by the guest-flow owner, not on the beacon

## Status

**Proposed — carrier sub-decision D18a approved by user 2026-10-05; the
handling of a control message for an unknown or closed flow (U-5) pinned at
the user's direction on 2026-10-06 and its exact rule (discard and count, no
reply) confirmed by user 2026-10-06 with its assumption K-A4 and validation
item V-24 (second model check, `spike/quint-owner-findings-r2.md`, finding
r2-3); pending independent DESIGN review.** GH #295. Split out of ADR-0158 (out-of-band
pairing order, D18) so that each ADR records one decision. Recorded in the
#295 feature delta, § *[REF] Control session (D18a)*.

## Context

ADR-0158 moves `Paired`, `Refused`, `Abort` and `ListenState` (ADR-0163) off
the flow connections. They need a per-VM carrier whose sender is attributable
to the VM. The VM already has one host session: the beacon (ADR-0157), owned
by the VM driver, which carries READY / EXEC / EXIT authority with its own
lifecycle.

## Decision

- The guest opens one dedicated control session per VM, a vsock STREAM to a
  host port owned by the guest-flow owner, and keeps it for the VM's life,
  reconnecting after loss.
- The host attributes a session by the kernel-reported peer CID, exactly as
  the beacon is attributed, and accepts it only for a CID it holds as
  Provisioned or Active.
- **At most one live session per CID.** Accepting a session is an atomic claim
  on the CID (the claim and its check are one operation); a second session
  for a CID with a live claim is closed. Releasing the claim happens when the
  session ends.
- Messages on the session are fixed-size, written and read in userspace only.
- Loss of the session aborts every flow of that VM, because no further
  `Paired` can be delivered, and closes the VM's intake listeners (ADR-0163).
- A host-side flow request from a CID with no live session is not paired: the
  host closes that flow connection without a control message.
- **A control message for a flow the receiver does not hold is discarded
  (U-5).** A `Paired`, `Refused` or `Abort` naming a flow id the receiver
  never opened or has already closed is dropped and counted, with no reply.
  The one exception is the guest's datagram slot rule — a late `Paired` for a
  released slot is answered with `Abort` (ADR-0150). No reply is needed
  because the side that closes a flow before `Paired` sends `Abort` while the
  session is live, session loss aborts every flow on both sides, and, for a
  host-opened flow whose guest vsock the guest has not yet accepted (so the
  guest holds no entry for it), the host's close of its own vsock reaches the
  guest and ends the guest side when the guest accepts it — on the same or a
  reconnected session (assumption K-A4).

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

- One more vsock listener on the host (port 1243) and one session per VM.
- Activation of a VM's forwarding requires its live session (G-V3 in the
  feature delta).
- A guest that cannot hold its control session has no working flows; READY is
  unaffected.
- Attribution by peer CID across allocations assumes the host kernel resets
  every connection of a VM whose vhost device is released — including
  connections still waiting in a host accept queue — before the CID can be
  leased to the next VM. Otherwise a queued session from a dead VM would be
  accepted as the next allocation's session. The formal model shows the
  attribution is safe only under this assumption
  (`spike/quint-owner-findings.md`, finding 3); validation item V-22 tests it.
- Discarding unknown-flow messages without reply is safe only under K-A4:
  closing the host's socket of an aborted or session-lost host-opened flow
  reliably reaches the guest and tears down the guest-side connection,
  including one still in the guest's accept queue. Without it the guest half
  of such a flow, connected to the application, is never closed
  (`spike/quint-owner-findings-r2.md`, finding r2-3). Validation item V-24
  tests it and blocks DISTILL of the scenarios that depend on it.
- Even under K-A4, a guest application can accept a connection for a client
  the host already aborted and then see it reset at once.
- The session protocol, including these rules, is modelled in the formal
  specification of the guest-flow owner (ADR-0168).
