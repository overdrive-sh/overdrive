# ADR-0147 — One kernel-forwarded socket pair per transport flow; flows are never multiplexed over one vsock connection

## Status

**Accepted.** Decision D3 (one forwarded socket set per transport flow),
approved by the user 2026-10-05. GH #295. Recorded in the #295 feature
delta, § *[REF] vsock Attachment Replacement DESIGN*. Not yet implemented.

Related decisions, each separately reversible: the node and per-allocation
bounds on concurrent flows (ADR-0208); when the host releases a flow's
places (ADR-0209); how the pair's bytes are moved (ADR-0197); how a pair is
paired (ADR-0158) and attached (ADR-0212), stopped (ADR-0160) and how its half-close completes
(ADR-0183); how an end is classified (ADR-0180) and reported (ADR-0202); a
datagram association's carrier (ADR-0201); the host end of a dialled flow
(ADR-0173, ADR-0214), its watch (ADR-0216) and its temporary leg-S relay
(ADR-0186).

## Context

The forwarder (ADR-0197) joins the two sockets of one pair and moves bytes
between them. Nothing in the kernel demultiplexes one vsock connection into
several sockets without reading payload.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, WORKS on
7.0.0-29) carried every flow on its own vsock connection: 60,004 TCP flows
and 12,003 UDP associations in the final build.

Host platform clients — leg-S delivering a mesh connection to a VM backend,
and the VM probe runner — reach a VM port through a flow the owner opens on
their behalf, whose host end the client holds (ADR-0173). V-28
(`spike/findings-v28-legs-vsock.md`) measured that path correct and costing
about 0.5 ms per connection and 12–37 % of client-to-guest bulk throughput
against loopback TCP.

## Decision

- Each transport flow gets its own vsock connection and its own host-facing
  socket. The per-flow socket sets are:
  - **guest-opened TCP connection:** guest intake child and guest vsock
    STREAM; host vsock STREAM and the host socket connected to the
    destination (or to leg-F for a mesh destination);
  - **host-opened TCP connection (a dial, ADR-0173):** host vsock STREAM, held
    by the dialer and watched, not forwarded, on the host; guest vsock
    STREAM and the guest socket connected to the application;
  - **UDP association** (one guest socket to one destination): one vsock
    STREAM (ADR-0201); on the host one connected UDP socket; in the guest the
    permanent per-slot UDP slot socket.
- A vsock connection never carries two flows.
- No payload crosses Overdrive userspace, with one exception: leg-S relays a
  dialled flow between kTLS and the host vsock until #303 moves mTLS into the
  guest kernel and deletes the relay (ADR-0186).

## Alternatives considered

- **One vsock connection per VM, multiplexed.** Needs a userspace
  demultiplexer that reads payload. Rejected by constraint.
- **One vsock connection per (VM, service port).** Same demultiplexer problem.
  Rejected.
- **Peer addressing inside datagram frames, so one association serves many
  peers.** It is the shape inbound UDP service would need (#310). Rejected
  for this feature.

## Consequences

- **Setup cost per flow:** one vsock connect, one or two socket creations per
  side, one forwarder attach on each side, and one control exchange.
  Per-flow setup latency through production is measured in V-9.
- **Sockets scale with flows, not VMs**, so the node bounds concurrent flows
  (ADR-0208).
- **Restart closes flows.** The owner process holds every host forwarder
  context, so an owner restart closes every live flow (ADR-0159, ADR-0199;
  survival is #312).
- The socket sets are the host's view of every guest: the host side is
  guest-neutral (ADR-0207); a guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321).
