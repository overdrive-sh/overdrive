# ADR-0160 — Stopping forwarding aborts flows with a reset; owners' sockets linger to reset on exit; restore reopens admission only

## Status

**Accepted — decision D20 approved by user 2026-10-05, with its correction —
`SO_LINGER{1,0}` extended to the host's remote-side sockets and leg-F's
remote socket — confirmed by user 2026-10-06; leg-F's linger clearing
approved by user directive 2026-10-09; the abort triggers of R2-END and
R2-DRAIN approved by user 2026-10-11.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*. It replaces
ADR-0124's containment primitive (TAP quiescence); ADR-0124's cadence and
fail-stop are unchanged. How a clean half-close completes is ADR-0183; how a
flow's end is classified is ADR-0180.

The user ruled on 2026-10-05 that an owner crash must surface to the
application as an error, using `SO_LINGER{1,0}`.

## Context

With SockHash forwarding, a TCP socket has already acknowledged any byte the
verdict receives. If the verdict then drops it, both ends see a healthy stream
that has silently lost data, and TCP cannot recover it. Containment by
bringing an interface down and restoring it later, which TCP recovers end to
end, is therefore not available.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) showed that
closing an owner's sockets normally gives the in-flight application, or a
remote destination the owner connected, a clean EOF on a truncated stream. A
socket that carries `SO_LINGER{1,0}` is reset instead.

The two owners end differently:

- **Host owner.** The guest-flow owner runs in `overdrive serve`; when that
  process exits, its files close, so a linger set on its sockets resets their
  far ends. The guest then receives `0x3` for every flow and aborts them
  (ADR-0180).
- **Guest owner.** The guest owner runs in `overdrive-init`, PID 1. When the
  last thread of the global init exits, the guest kernel panics
  (`kernel/exit.c:929-930 @ v7.0`) before the exiting task's files are
  closed (`kernel/exit.c:971 @ v7.0`), so no guest socket closes and no
  guest linger fires on that path. The VM ends (ADR-0182), and the host's
  flows of that guest are reset by the device-release orphan sweep
  (ADR-0146). On the guest, linger acts on the abort path only: when the
  guest owner aborts a flow, closing its sockets resets the application's
  connection.

## Decision

- **Stop aborts the whole flow.** Quiescence, teardown, removal after an
  error, a `Refused` or `Abort` control message, a classified abort
  (ADR-0180), a refused redirect or a lost ring record (ADR-0183), and owner
  loss each abort the affected flow: both sides close all of the flow's
  sockets, and TCP sockets close with reset semantics so both ends observe
  failure.
- **No partial drops.** A pair is never left installed with its routes
  removed.
- **Failure visibility, both ends.** Every TCP socket an owner holds whose
  far end is outside that owner carries `SO_LINGER{1,0}` from creation, so an
  abort, or on the host the owner's exit, resets it rather than ending it
  with a clean FIN:
  - guest: intake children and sockets connected to the application (the
    abort path);
  - host: the socket the owner connects toward a non-mesh destination;
  - leg-F: for a forwarded mesh flow, its socket toward the remote peer,
    whose linger leg-F clears only on a clean close of both directions — an
    error or reset on its loopback leg closes it with linger set.

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
- **Let an owner's sockets close normally on exit.** The far end sees a clean
  EOF on a truncated stream. Rejected.

## Consequences

- During recovery live guest flows fail with resets instead of stalling.
- After a stop aborts a datagram association, the guest closes its legs; a
  datagram sent while the association is down may be lost, as UDP permits.
- A host owner exit resets in-flight connections on both sides — the remote
  peer through linger and the guest application through the guest's abort
  on `0x3` — instead of ending them cleanly. A guest owner exit ends the VM. The mesh
  remote peer through leg-F was not measured with production leg-F.
- Remote clients of an inbound flow are connected to leg-C, not to the owner;
  on an `overdrive serve` exit they observe the TLS session ending without
  `close_notify`, which TLS reports as truncation. Leg-C's own linger policy is
  not changed by this ADR.
