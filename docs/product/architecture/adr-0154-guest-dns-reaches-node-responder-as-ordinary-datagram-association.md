# ADR-0154 — Guest DNS reaches the node DNS responder at the guest gateway address as an ordinary datagram association; there is no guest resolver stub

## Status

**Accepted — decision D10 approved by user 2026-10-05.** GH #295. Recorded in
the #295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*.

Depends on GW-LOCAL (ADR-0152, accepted 2026-10-13): the guest gateway is
the one host-local address of the guest prefix.

## Context

Dial-by-name (ADR-0072, ADR-0116) has the guest resolver send UDP to the node
DNS responder at the guest gateway address, port 53 (`overdrive.net=…,dns=`
names the gateway today). ADR-0116's resolver semantics do not depend on the
transport: one in-agent `DnsResponder` over the `NameIndex`, no DNS in eBPF.
The responder binds the wildcard address on port 53 (ADR-0116).

A resolver typically sends each query from a fresh unconnected socket. Under
ADR-0147 each one is a new datagram association, which must be established
before its first datagram without that datagram being read in userspace.

The guest-capture spike showed this works: glibc `getent hosts` and
`getent ahostsv4`, bind9 `dig` and busybox `nslookup` all answered on their
first datagram; 200/200 repeated `getent`; 10,000/10,000 first datagrams from
fresh unconnected sockets and 2,000/2,000 from connected ones; neither owner
made a payload syscall. The first datagram parks in a kernel cell until the
association is paired (ADR-0150, ADR-0158).

The guest prefix is not host-local (ADR-0173). A host socket connected to an
address that is not local follows the host's routes and does not reach a host
socket. The gateway therefore reaches the wildcard-bound responder only if it
alone is made host-local, which is what GW-LOCAL (ADR-0152) decides.

## Decision

- The guest resolver keeps today's nameserver: the node's guest gateway
  address.
- The guest adaptation captures DNS like any other UDP: the gateway is not a
  local guest address, so `sendmsg4` captures it and the flow becomes an
  ordinary datagram association.
- The host owner pairs that association with a UDP socket connected to
  `gateway:53`. The gateway is host-local through its own boot-converged
  `local` route on `lo` (ADR-0152), so the datagram is delivered through `lo`
  to the `DnsResponder` addressed to the gateway, and the reply leaves from
  the gateway, the source the connected socket accepts. The host verdict removes the datagram
  frame, and an empty datagram is sent by the owner (ADR-0165).
- `gateway:53` is the one host-local destination the host-internal deny set
  admits (ADR-0167), and the gateway is excluded from the constant
  guest-prefix rules (ADR-0172).
- The responder's resolution semantics and bind rule are unchanged
  (ADR-0116).

## Alternatives considered

- **A guest resolver stub in `overdrive-init`** answering over a typed control
  RPC. Guest userspace would read every query, and the spike showed it is
  unnecessary. Rejected.
- **A distinct guest-local DNS address** (`127.0.0.53`). Changes the
  resolver's configured address without need, and requires capturing a
  loopback destination as an exception. Rejected under D15.
- **Pair DNS associations with a socket connected to a node address instead
  of the gateway.** Needs a special destination rewrite in the owner for one
  service, and the responder's gateway fallback (ADR-0116) would no longer
  match the address the guest queries. Rejected.
- **Force DNS over TCP.** libc-dependent. Rejected.
- **Resolve names in eBPF.** ADR-0116 already rejects this.

## Consequences

- Every DNS query 4-tuple costs one datagram association. A resolver that
  opens a fresh socket per query pays setup on every query, measured in V-9.
- An association lasts at most as long as the guest socket that created it, or
  until the host aborts it (ADR-0160). After a host abort the guest's next
  datagram on the same connected socket starts a new association
  (CELL-RECREATE, ADR-0150).
- There is no time-based expiry beyond the guest's idle slot release
  (ADR-0150), and an association is never shared across sockets.
