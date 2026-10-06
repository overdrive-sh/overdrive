# ADR-0147 — One kernel-forwarded socket pair per transport flow; flows are never multiplexed over one vsock connection

## Status

**Proposed — decision D3 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. How a pair is
established is ADR-0158; how it is stopped is ADR-0160.

## Context

SK_SKB redirect moves whole skbs between the sockets of one pair. Nothing in
the kernel demultiplexes one vsock connection into several sockets without
reading payload.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, WORKS on
7.0.0-29) carried every flow on its own vsock connection(s). It ran 60,004
TCP flows and 12,003 UDP associations in the final build, and established the
real per-flow socket set:

- **TCP egress:** guest intake child + guest vsock STREAM; host vsock STREAM +
  host destination socket; plus a pooled guest parking cell (two loopback TCP
  sockets).
- **TCP inbound:** host intake child + host vsock STREAM; guest vsock STREAM +
  guest socket connected to the application; plus a pooled host parking cell.
- **UDP association:** guest vsock STREAM + SEQPACKET; host vsock STREAM +
  SEQPACKET + host UDP socket; plus, in the guest, a permanent per-slot UDP
  intake and four cell sockets (framing and reassembly).

Cells are pooled and reused (256-cell pools cycled 78 to 156 times per cell).

## Decision

- Each transport flow gets its own vsock connection(s) and its own host-facing
  socket:
  - **a TCP connection:** one vsock STREAM, paired on each side with one
    `AF_INET` TCP socket;
  - **a UDP association** (one guest socket to one destination): one vsock
    STREAM toward the guest and one vsock SEQPACKET from the guest
    (ADR-0148), paired on the host with one connected UDP socket.
- A vsock connection never carries two flows. No payload crosses userspace.
- Kernel parking and reassembly cells are pooled per owner and are not part of
  a flow's identity.
- Each node bounds concurrent flows twice: a global pair capacity sized from
  the attachment target (ADR-0155), and a per-allocation quota. Exhausting
  either refuses that flow with a typed refusal; it never fails the
  allocation.

## Alternatives considered

- **One vsock connection per VM, multiplexed.** Needs a userspace
  demultiplexer that reads payload. Rejected by constraint.
- **One vsock connection per (VM, service port).** Same demultiplexer problem.
  Rejected.
- **Peer addressing inside datagram frames, so one association serves many
  peers.** An SK_SKB verdict on an unconnected UDP socket sees payload only,
  not the sender, so this needs TC-level classification. It is the shape
  inbound UDP service would need (#310). Rejected for this feature.

## Consequences

- **Setup cost per flow:** one vsock connect (two for UDP), one or two socket
  creations per side, map updates on each side, and one control exchange.
  Per-flow setup latency through production is measured in V-9.
- **Sockets scale with flows, not VMs.** The host fd budget is checked at
  startup against the pinned pair capacity (feature delta, § *Owner*).
- **Quota is a bulkhead, not a fair share.** `capacity / quota` allocations can
  exhaust the global pool. Fair-share flow accounting is not part of this
  feature.
- **Restart closes flows.** The owner process holds every host pair socket, so
  an owner restart closes every live flow (ADR-0159; survival is #312).
