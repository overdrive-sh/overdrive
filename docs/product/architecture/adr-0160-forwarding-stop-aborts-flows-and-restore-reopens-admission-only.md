# ADR-0160 — Stopping forwarding aborts flows with a reset; half-close waits for the kernel drain signal; restore reopens admission only

## Status

**Proposed — decision D20 approved by user 2026-10-05, with its correction —
`SO_LINGER{1,0}` extended to the host's remote-side sockets and leg-F's
remote socket (independent DESIGN review finding H-5) — confirmed by user
2026-10-06; pending independent DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. It replaces ADR-0124's
containment primitive (TAP quiescence); ADR-0124's cadence and fail-stop are
unchanged.

The user ruled on 2026-10-05 that a guest-owner crash must surface to the
application as an error, using `SO_LINGER{1,0}`, and that this is a named
validation item (V-13).

## Context

ADR-0124 contained damage by bringing TAPs down and restoring them after a
clean audit; a frame dropped meanwhile was recovered end to end by TCP.

With SockHash forwarding, a TCP socket has already acknowledged any byte the
verdict receives. If the verdict then drops it, both ends see a healthy stream
that has silently lost data. TCP cannot recover it.

The guest-capture spike established the half-close mechanics:

- a FIN-only skb redirected to egress disables the target's TX with `EPIPE`,
  so verdicts drop zero-length skbs and the owner propagates FIN;
- the owner can tell, without reading payload, that a hop has drained:
  per-socket forwarded bytes (verdict counter) equal the bytes received
  (`TCP_INFO.bytes_received` less the FIN), and the target's sent count from
  `fexit(skb_send_sock)` has caught up. 120,008 half-closes ordered this way
  had zero truncations;
- the counters must be exact: LRU eviction stalled one half-close;
- killing the guest owner gave the in-flight application a clean EOF with 0
  bytes, because the owner's sockets closed normally. The same holds on the
  host: a remote destination connected by the host owner sees a clean FIN on a
  truncated stream when the owner exits, unless that socket carries
  `SO_LINGER{1,0}`.

## Decision

- **Stop aborts the whole flow.** Quiescence, teardown, removal after an error,
  a `Refused` or `Abort` control message, a drain deadline, and owner loss
  each abort the affected flow: both sides close all of the flow's sockets,
  and TCP sockets close with reset semantics so both ends observe failure.
- **No partial drops.** A pair is never left installed with its routes
  removed.
- **Half-close.** On peer EOF the owner calls `shutdown(SHUT_WR)` on the
  partner only after the drain signal shows every hop toward that partner has
  drained. If the drain signal does not converge within the pinned drain
  deadline, the flow is aborted, never truncated.
- **Owner-crash visibility, both ends.** Every TCP socket an owner holds whose
  far end is outside that owner carries `SO_LINGER{1,0}` from creation, so an
  owner exit resets it rather than ending it with a clean FIN:
  - guest: intake children and sockets connected to the application;
  - host: intake children, and the socket the owner connects toward a
    non-mesh destination;
  - leg-F: for a forwarded mesh flow, its socket toward the remote peer.

  Before a clean close the owner clears linger. Sockets whose far end is
  inside the same process (the owner's socket to leg-F) need no linger: both
  ends die together.
- **Restore reopens admission only.** Pairs a stop aborted are not reinstated.
  Who may restore, and when a restore reopens forwarding, is ADR-0169.

## Alternatives considered

- **Remove routes and keep the sockets, then restore routes.** Bytes are lost
  silently mid-stream. Rejected.
- **Hold redirected data in the kernel during quiescence.** SK_SKB has no
  holding primitive. Rejected.
- **Half-close on EOF without a drain check.** FIN overtakes queued bytes and
  truncates the tail. Rejected.
- **Half-close after a fixed delay.** A timing heuristic, not a signal.
  Rejected.

## Consequences

- During recovery live guest flows fail with resets instead of stalling.
- After a stop aborts a datagram association, the guest closes its legs; a
  datagram sent while the association is down may be lost, as UDP permits.
- The drain signal needs the host and guest `fexit(skb_send_sock)` program and
  exact counters (ADR-0151).
- An owner exit now resets in-flight connections on both sides — the guest
  application and the remote peer — instead of ending them cleanly. That is
  the intended surface; it is untested (V-13).
- Remote clients of an inbound flow are connected to leg-C, not to the owner;
  on an `overdrive serve` exit they observe the TLS session ending without
  `close_notify`, which TLS reports as truncation. Leg-C's own linger policy is
  not changed by this ADR.
