# ADR-0172 — Constant rules refuse guest-prefix traffic that is not diverted to leg-C

## Status

**Accepted — the constant guest-prefix output reject and prerouting drop
approved by user 2026-10-06 (D8a); their scope without a host-local prefix
route follows INB-1, approved by user 2026-10-11.** GH #295. Recorded in the
#295 feature delta, `docs/feature/netns-density-295/feature-delta.md`
§ *[REF] vsock Attachment Replacement DESIGN* (decision index, § *[REF]
leg-S over vsock (temporary bridge until #303) and the mTLS
forwarded-outbound port*, § *Lifecycle Gate Ownership*).

The same-node leg-B dial these rules refuse when its tuple is not registered
is ADR-0190 (LEGB-DIAL, approved by user 2026-10-13).

Amends ADR-0125 (operative): rules 5 and 8 match the constant guest prefix
instead of `managed_guest_ips`; `managed_guest_ips` and its element lifecycle
are removed. ADR-0125 carries the dated amendment note.

## Context

Under ADR-0145 the guest has no NIC, and the guest prefix is not host-local
(ADR-0173). Traffic to a `workload_addr` reaches the host in two ways:

- a remote or same-node mesh client's connection, which the intercept's
  TPROXY rule assigns to leg-C (`inbound_destinations`, ADR-0125); a
  same-node one arrives from leg-B marked and bound to `127.0.0.1`
  (ADR-0190);
- a host-local socket that connects to a guest-prefix address. No platform
  client does this: leg-S and the probe runner reach a VM port through an
  owner-opened vsock flow (ADR-0173).

Overdrive installs no route covering the guest prefix except the gateway's
`/32` (ADR-0152, ADR-0190). An unmarked host connect to a guest-prefix
address therefore follows the node's own routing: with no node route covering
the prefix, `connect()` fails `ENETUNREACH`; with one (a node default route),
it takes that route's source, a node address, and a non-diverted packet
follows the route off the host. A marked connect — leg-B's same-node dial
(ADR-0190) — matches the intercept's `fwmark 0x1 → table 100` rule and its
`local default dev lo` route, which sends the packet through the loopback
device (`net/ipv4/route.c:2848-2858 @ v7.0`) and delivers it locally on input
(`net/ipv4/route.c:2373-2379 @ v7.0`). A marked packet to a tuple that is not
registered would therefore reach any host socket that matches it — a host
wildcard UDP socket, a host TCP listener — and a remote packet to the prefix
that is not diverted would be left to the forwarding path. None of these is
acceptable: the first sends guest-addressed traffic off the host, the second
exposes host services under a guest address, the third sends
guest-addressed traffic onward.

No host socket sends from a guest-prefix address to a host destination that
answers into the prefix:

- Leg-B, the agent's socket toward a peer's leg-C, binds `127.0.0.1` and
  sets the intercept mark for a peer inside the prefix (ADR-0190), and sets
  neither for any other peer, whose source then comes from the node's
  routing. Neither source lies in the prefix (ADR-0185).
- Leg-C's accepted sockets carry the dialled `workload_addr` as their local
  address, but they never connect; their replies go to leg-B's source,
  outside the prefix.
- The owner's host sockets bind `127.0.0.1` (toward leg-F) or take a source
  from host routing (non-mesh egress, ADR-0162).

A reply addressed into the prefix would arise only from a socket that
connected with a guest-prefix source, which for an unbound socket needs a
`local` route for the destination at connect time (`net/ipv4/route.c:2848-2853
@ v7.0`: with no preferred source the source becomes the destination). No
`local` route of the main or local table covers a `workload_addr` (ADR-0173).
The table-100 `local` route a marked lookup reaches would make the
destination the source, but that source is not a host address, so the
connect fails `ENETUNREACH` (`net/ipv4/route.c:2729`, `:2766-2769 @ v7.0`;
V-LEGB-MARK S4 c). No such socket exists.

## Decision

- **Constant rules keep each packet on its path.** In output and prerouting,
  every packet to the guest prefix that is not diverted to leg-C is refused:
  the output rule rejects (TCP reset, ICMP port-unreachable otherwise), so a
  host-local TCP client sees `ECONNREFUSED` and a host-local UDP sender sees
  `EPERM` at `send()`, and the packet reaches no host socket; the
  prerouting rule drops, as today for remote traffic. A marked,
  `127.0.0.1`-bound leg-B dial (ADR-0190) to a tuple that is not registered
  is refused the same way — TCP `ECONNREFUSED`, UDP `EPERM`, with host decoy
  sockets on the dialled ports silent (V-LEGB-MARK S3 on Linux 7.0,
  `docs/feature/netns-density-295/spike/findings-v33-legb-mark.md`). Each
  rule follows the leg-C divert, so every packet of a diverted connection is diverted before the rule sees it. A
  packet to a guest-prefix address therefore never leaves the host by another
  route and never reaches a host socket other than leg-C.
- **The gateway is excluded** from the rules' guest-prefix match: it is the
  one host-local address of the prefix (GW-LOCAL, ADR-0152) and the DNS
  address (ADR-0154), and guest flows to it are decided by ADR-0167. Leg-B
  never dials it.
- **The leg-S mark exemption does not concern the guest prefix.** Leg-S
  dials a VM backend through an owner-opened flow (ADR-0173), so neither rule
  carries a guest-prefix exception for it.
- **No per-allocation membership.** The constant prefix replaces the
  per-allocation `managed_guest_ips` set. ADR-0125's rules 1, 4, 6 and 7
  stay; rules 5 and 8 match the constant guest prefix. (Rules 2–3 and
  `outbound_sources` are removed by ADR-0153 and ADR-0162.)

## Alternatives considered

- **No rules: let unmatched guest-prefix traffic follow the routing table.**
  A marked leg-B packet to an unregistered tuple would be delivered by
  table 100 to a matching host socket; an unmarked host-local packet would
  follow a node covering route off the host; and a remote packet to an
  unassigned guest address would be routed onward instead of dropped. That
  a TCP connect may also fail without the rules — the listener's SYN-ACK from a non-local source has no route
  (`net/ipv4/route.c:2766-2769 @ v7.0`) — is incidental: the SYN still
  reaches the listener, UDP is delivered outright, and nothing guarantees
  the failure. Rejected.
- **Keep `managed_guest_ips` and refuse only managed addresses.** Unassigned
  and released guest-prefix addresses would still reach host sockets or be
  routed onward. Rejected.
- **Precede each rule with `ct state established,related accept`.** It
  admits replies addressed into the prefix, which arise only for a host
  socket that connected from a prefix address. No such socket exists
  (Context). Rejected.
- **A per-allocation host netdevice per guest address.** Reintroduces per-VM
  netdevices. Rejected (ADR-0145).

## Consequences

- A connection to `workload_addr:port` reaches leg-C or a refusal — never a
  host service, never another network.
- The rules are load-bearing, not a diagnostic. They keep a marked leg-B
  dial to an unregistered tuple away from host sockets — the table-100 route
  supplies only the route `connect()` needs (ADR-0190) — and they keep
  non-diverted guest-prefix traffic from leaving the host by a node covering
  route; neither decision duplicates the other.
- Two constant rules match the guest prefix; `managed_guest_ips` and its
  element lifecycle are deleted.
- The rules decide every packet to the prefix that is not diverted. A future
  host socket that connects from a guest-prefix address would have its
  replies refused; adding one is a change to this decision.
- A failed write of the shared firewall table at boot refuses boot
  (ADR-0184); runtime damage to it is repaired under ADR-0124's recovery and
  ADR-0169's counted hold.
- The rules and their placement are validated in the production composition
  by the Tier-3 scenarios named in the feature delta's § *Validation plan*.
