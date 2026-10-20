# ADR-0212 — Each side attaches a flow's pair to the forwarder at pairing, and early bytes wait in the source socket's own queue

## Status

**Accepted — EVMOD-PAIR, approved by the user 2026-10-17** (for Linux
guests), with the spike corrections of EV-PINS approved the same day
(attach accepts a TCP half in `CLOSE_WAIT` and classifies the attach-time
state). GH #295. Not yet implemented. Contract: the #295 feature delta,
§ *[REF] vsock data path — the in-kernel event-driven forwarder (EVMOD)*,
"Forwarding, per direction" and "States attach and watch accept", and
§ *[REF] Per-kind total orders*.

Related decisions, each separately reversible: pairing is out of band and
the request is read first (D18, ADR-0158); a half that has already ended at
its attach is refused and its flow aborted (EVMOD-ATTACH-END, ADR-0210); how
an end present at attach is classified (ADR-0180); the host end of a
host-opened flow is held by its dialer, not attached (ADR-0214), and
watched (ADR-0216).

## Context

The forwarder of ADR-0197 joins two connected sockets at any time: bytes
that arrived before the attach wait in the source socket's own receive
queue, whose TCP window or vsock credit holds the early sender, and the
attach queues one run of each direction, which forwards them. Its
`recvmsg`-based run reads the queue from the start; nothing is consumed
before the attach.

Spike V-EV-PAIR (`spike/findings-ev-pair.md`, P-71, Linux 7.0) measured
attach at pairing over the shapes that had stranded bytes before (P-22,
P-23): 961,995 of 962,000 trials byte-exact (the five failures were the
harness's own `connect()` before any attach), early data waiting only in
the source's queue. Two rules proved load-bearing: accepting a TCP half in
`CLOSE_WAIT` (an `ESTABLISHED`-only rule refused 414 of 500 write-and-FIN
flows, hazard `h_strict`) and classifying the attach-time state (hazard
`h_noseed`).

A read by the owner that consumes a TCP half's pending error makes a later
read return end-of-stream (`net/ipv4/tcp.c:2794-2806 @ v7.0`), turning a
reset into a clean end (P-71 § 9.6).

## Decision

- **Each side attaches its pair at pairing:**
  - host TcpConnect and datagram flows: after the destination's connect (or
    the connected UDP socket's `connect()`) succeeded, before `Paired` is
    sent;
  - guest TcpConnect: on `Paired`, the intake child to the flow vsock;
  - guest TcpAccept: after the connect to the application succeeded, before
    `Paired` is sent;
  - guest datagram association: on `Paired`, the UDP slot socket to the flow
    vsock.
- **Early bytes and an early end wait in the source's own queue.** The
  attach-time run forwards them; an end present at attach is classified once
  from the attach-time state (ADR-0180).
- **Attach accepts a still-connected half whose peer already ended its
  direction** — a TCP half in `ESTABLISHED` or `CLOSE_WAIT` with no pending
  error, a vsock half in `ESTABLISHED` with any peer shutdown bits and no
  pending error — and installs its callbacks and reads each half's state in
  one section under the half's socket lock, `sk_callback_lock` and, for
  vsock, `vsock_table_lock`, so no transition falls between the two. A half
  that has ended is not attached (ADR-0210).
- **Owners do not touch attached sockets.** An owner never shuts or writes a
  socket it will attach after reading its request, and never reads, writes or
  shuts an attached socket before the pair's end record (ADR-0202).

## Alternatives considered

- **Hook every intake child at establishment and park its bytes in a kernel
  cell until `Paired`.** Needed only by a forwarder that loses bytes arriving
  before its socket is hooked (the sockmap redirect, P-23); it brings parking
  cells, a re-arm and a `sock_ops` program per side. Rejected.
- **Attach at accept, before the target exists.** No such pair: the
  forwarder joins two connected halves. Rejected.
- **Attach only `ESTABLISHED` TCP halves.** Refused 414 of 500 write-and-FIN
  flows (hazard `h_strict`). Rejected.

## Consequences

- Neither side needs parking cells or a `sock_ops` program for forwarding;
  the guest keeps only its capture programs (ADR-0150).
- Leg-F needs no write gate: the host socket connected to leg-F is attached
  before `Paired` (ADR-0153).
- The per-kind total orders carry no install, arm, route or parking step.
- The host side attaches the same way whatever guest opened the flow
  (ADR-0207); the guest-side attach points are those of a Linux guest. A
  guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).
- Checked in `specs/quint/guest-flow-owner` (module `fwd_pair`, the named
  runs `Paired` and `DgramPaired`; feature delta, § *Formal protocol model*).
