# ADR-0154 — Guest DNS reaches the node DNS responder at the guest gateway address as an ordinary datagram association; there is no guest resolver stub

## Status

**Proposed — decision D10 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

## Context

Dial-by-name (ADR-0072, ADR-0116) has the guest resolver send UDP to the node
DNS responder at the guest gateway address, port 53 (`overdrive.net=…,dns=`
names the gateway today). ADR-0116's resolver semantics do not depend on the
transport: one in-agent `DnsResponder` over the `NameIndex`, no DNS in eBPF.

A resolver typically sends each query from a fresh unconnected socket. Under
ADR-0147 each one is a new datagram association, which must be established
before its first datagram without that datagram being read in userspace.

The guest-capture spike showed this works: glibc `getent hosts` and
`getent ahostsv4`, bind9 `dig` and busybox `nslookup` all answered on their
first datagram; 200/200 repeated `getent`; 10,000/10,000 first datagrams from
fresh unconnected sockets and 2,000/2,000 from connected ones; neither owner
made a payload syscall. The first datagram parks in a kernel cell until the
association is paired (ADR-0150, ADR-0158).

The gateway lies inside the guest prefix, and the prefix is host-local
(ADR-0152); without an explicit exception the host-internal deny set
(ADR-0167) and the UDP guest-prefix refusal (#310) would both refuse it.

## Decision

- The guest resolver keeps today's nameserver: the node's guest gateway
  address.
- The guest adaptation captures DNS like any other UDP: the gateway is not a
  local guest address, so `sendmsg4` captures it and the flow becomes an
  ordinary datagram association.
- The host owner pairs that association with a UDP socket connected to
  `gateway:53`. The gateway is host-local through the guest prefix's local
  route (ADR-0152), so the datagram leaves through `lo`, with its frame
  removed as ADR-0165 decides (in the host verdict, or for an empty datagram
  on `lo` egress), and is delivered to the `DnsResponder`. `gateway:53` is the
  one host-local destination the host-internal deny set admits (ADR-0167);
  the gateway is excluded from the guest-prefix steering rules (ADR-0152).
- The responder's resolution semantics and bind rule are unchanged
  (ADR-0116). The gateway address it falls back to is the same address; it is
  now local through the `lo` route instead of on the bridge.

## Alternatives considered

- **A guest resolver stub in `overdrive-init`** answering over a typed control
  RPC. Guest userspace would read every query, and the spike showed it is
  unnecessary. Rejected.
- **A distinct guest-local DNS address** (`127.0.0.53`). Changes the
  resolver's configured address without need, and requires capturing a
  loopback destination as an exception. Rejected under D15.
- **Force DNS over TCP.** libc-dependent. Rejected.
- **Resolve names in eBPF.** ADR-0116 already rejects this.

## Consequences

- Every DNS query 4-tuple costs one datagram association. A resolver that
  opens a fresh socket per query pays setup on every query, measured in V-9.
- An association lasts at most as long as the guest socket that created it, or
  until the host aborts it (ADR-0160). A host abort leaves the guest's
  connected UDP socket in place; whether the next datagram on that same
  socket re-associates was not shown (V-5(c)).
- There is no time-based expiry, and an association is never shared across
  sockets.
