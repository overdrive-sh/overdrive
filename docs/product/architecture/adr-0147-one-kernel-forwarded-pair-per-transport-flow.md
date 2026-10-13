# ADR-0147 — One kernel-forwarded socket pair per transport flow; flows are never multiplexed over one vsock connection

## Status

**Accepted — decision D3 approved by user 2026-10-05; the quota's count of
draining flows (R2-DRAIN) and the host end of a dialled flow (INB-1, with its
temporary leg-S relay) approved by user 2026-10-11.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*. How a pair
is established is ADR-0158; how it is stopped is ADR-0160; how a half-close
drains is ADR-0183.

## Context

SK_SKB redirect moves whole skbs between the sockets of one pair. Nothing in
the kernel demultiplexes one vsock connection into several sockets without
reading payload.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, WORKS on
7.0.0-29) carried every flow on its own vsock connection(s): 60,004 TCP flows
and 12,003 UDP associations in the final build. Pooled kernel cells were
reused heavily (256-cell pools cycled 78 to 156 times per cell).

Host platform clients — leg-S delivering a mesh connection to a VM backend,
and the VM probe runner — reach a VM port through a flow the owner opens on
their behalf, whose host end the client holds (ADR-0173). V-28
(`spike/findings-v28-legs-vsock.md`) measured that path correct and costing
about 0.5 ms per connection and 12–37 % of client-to-guest bulk throughput
against loopback TCP.

## Decision

- Each transport flow gets its own vsock connection(s) and its own
  host-facing socket. The per-flow socket sets are:
  - **guest-opened TCP connection:** guest intake child and guest vsock
    STREAM; host vsock STREAM and the host socket connected to the
    destination (or to leg-F for a mesh destination); plus a pooled guest
    parking cell (two loopback TCP sockets);
  - **host-opened TCP connection (a dial, ADR-0173):** host vsock STREAM, held
    by the dialer and not installed in the host forwarding maps; guest vsock
    STREAM and the guest socket connected to the application;
  - **UDP association** (one guest socket to one destination): one vsock
    STREAM toward the guest and one vsock SEQPACKET from the guest
    (ADR-0148); on the host one connected UDP socket; in the guest a
    permanent per-slot UDP intake and its framing and reassembly cells.
- A vsock connection never carries two flows.
- No payload crosses Overdrive userspace, with one exception: leg-S relays a
  dialled flow between kTLS and the host vsock until #303 moves mTLS into the
  guest kernel and deletes the relay (INB-1).
- Kernel parking and reassembly cells are pooled per owner and are not part of
  a flow's identity.
- **Two bounds.** Each node bounds concurrent flows twice: a global pair
  capacity sized from the attachment target (ADR-0155), and a per-allocation
  quota of 4,096. A flow holds its place in both from admission until it
  ends — pending pairing, open, or draining a half-close (half-closed, not
  yet completed, ADR-0183). Dialled flows count like any other, except that
  a VM probe dial (PROBE-NOSAMPLE, ADR-0187) holds a
  place in the global capacity only, not in its allocation's quota (at most
  one per declared probe). Exhausting
  either bound refuses that flow with a typed refusal; it never fails the
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
- **Release a flow's quota place at half-close.** A guest that withholds
  credit would then hold node flow capacity beyond its quota. Rejected
  (ADR-0183).

## Consequences

- **Setup cost per flow:** one vsock connect (two for UDP), one or two socket
  creations per side, map updates on each side, and one control exchange.
  Per-flow setup latency through production is measured in V-9.
- **Sockets scale with flows, not VMs.** The host fd budget is checked at
  startup against the pinned pair capacity (feature delta, § *[REF] Owner and
  provisioner*).
- **Quota is a bulkhead, not a fair share.** `capacity / quota` allocations can
  exhaust the global pool. Fair-share flow accounting is not part of this
  feature.
- **A slow reader holds quota.** A draining flow keeps its place until its
  target accepts the last byte, so a guest that withholds credit holds at
  most its quota of node flow capacity — what it can already hold with open
  flows (ADR-0183).
- **Restart closes flows.** The owner process holds every host pair socket, so
  an owner restart closes every live flow (ADR-0159; survival is #312).
