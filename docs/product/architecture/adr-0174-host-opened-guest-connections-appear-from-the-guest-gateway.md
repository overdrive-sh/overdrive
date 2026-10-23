# ADR-0174 — Host-opened guest connections appear from the guest gateway address

## Status

**Accepted — the `TcpAccept` peer of every host-opened flow approved by user
2026-10-11 as part of INB-1 (ADR-0173); follows ruling D15 (approved
2026-10-05): guests keep today's network behaviour wherever the kernel allows
it.** GH #295. The request layout lives in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] Wire contracts —
flow request, control message, datagram frame, flow event record*.

**GUEST-TRANSPARENT-ROUTE accepted (2026-10-24).** Native STEP 11-06
invalidated the former `fwmark -> local table` implementation premise, not
this ADR's accepted peer identity. Independent DESIGN review iteration 2
approved the bounded `SO_MARK=0x295` / table-295 local-default contract, typed
setup failures and cleanup recorded in ADR-0150 and the feature delta, with no
open finding or further user decision required.

## Context

Under the bridge topology every inbound connection a guest application
accepted came from a host platform client — leg-S after mTLS termination, or
a probe — and reached the guest from the bridge gateway address. Remote mesh
clients were never the guest's TCP peer; leg-S was.

Under ADR-0173 the guest-flow owner opens a vsock flow for each dial and the
guest connects to the application on the dialer's behalf. Unless the
`TcpAccept` request names a client address, the guest connects from its own
address, and the application sees `workload_addr` as its peer (V-28 Q3:
`10.77.0.2:34784`).

The guest binds the connecting socket with `IP_TRANSPARENT` (ADR-0150), so it
may bind a non-local address (`include/net/inet_sock.h:433-449 @ v7.0`,
checked in `net/ipv4/af_inet.c:510-513 @ v7.0`). Binding with port 0 makes the
kernel choose the port at bind time (`net/ipv4/af_inet.c:542-544 @ v7.0`).

## Decision

- **Peer = the guest gateway, port 0.** Every host-opened flow's `TcpAccept`
  request carries the guest gateway address with port 0 as the peer. The
  gateway is a per-lease constant available to every dialer. The guest binds
  its connecting socket transparently to that address with port 0 and the
  guest kernel picks the port, so `getpeername` in the application returns
  the gateway address and a kernel-chosen port.
- **Every inbound connection appears from the gateway.** leg-S dials on behalf
  of remote mesh clients and the probe runner dials for itself; both present
  the gateway, as under the bridge topology.

## Alternatives considered

- **Carry the leg-C TLS peer's address and port.** Works (V-28 Q3: the
  application saw the remote client exactly, 40 of 40 concurrent flows from
  one source with distinct ports), but every remote mesh client would appear
  as itself instead of as the gateway, a change the guest never saw under the
  bridge. Rejected under D15.
- **Carry no peer and let the guest connect from its own address.** The
  application would see `workload_addr` as its client. Rejected.
- **Carry a fixed gateway port.** Concurrent dials would collide on one
  four-tuple. Rejected; the guest kernel picks the port.

## Consequences

- A guest application sees the gateway as the client of every inbound
  connection, for remote mesh clients and host probes alike, as today.
- The transparent bind needs the guest network setup of ADR-0150; without it
  the transparent bind fails (V-28 Q3).
- Under GUEST-TRANSPARENT-ROUTE, the guest's reset toward the
  gateway is delivered at once through its dedicated `SO_MARK=0x295` rule and
  table-295 local-default route: `tcp_fwmark_accept=1` marks the accepted
  application child, and `fwmark_reflect=1` marks the kernel-generated reset
  (ADR-0150). The peer remains the gateway under either configuration.
- No policy or safety rule keys on the guest-side source address.
