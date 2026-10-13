# ADR-0190 — Leg-B marks and loopback-binds same-node VM dials

## Status

**Accepted — LEGB-DIAL approved by user 2026-10-13** (user ruling "Leg-B dial
contract replaces PREFIX-ROUTE-LO / INB-1-READ"). GH #295. Recorded in the
#295 feature delta, `docs/feature/netns-density-295/feature-delta.md`
§ *[REF] vsock Attachment Replacement DESIGN* (decision index); the interface
contract — the dial's owner, its required guest-prefix input and the
fail-closed table — is § *[REF] Component — host mTLS enforcement leg-B dial
(LEGB-DIAL)*.

Related decisions, each its own ADR: the guest prefix is not host-local
(ADR-0173, INB-1), the gateway is its one host-local address (ADR-0152), the
prefix holds no node address and no VIP (ADR-0185), the constant rules that
refuse guest-prefix traffic not diverted to leg-C (ADR-0172), and the
node-shared leg-C listener (ADR-0120). Cross-node VM addressing is
[#317](https://github.com/overdrive-sh/overdrive/issues/317).

## Context

Leg-B is the agent's own socket toward a peer's leg-C. For a mesh dial to a
VM backend, the peer is the VM's `workload_addr`, an address inside the guest
prefix. When that VM runs on the same node, the connection must reach the
node's leg-C listener through the intercept's TPROXY rule for the registered
`inbound_destinations` tuple (ADR-0120, ADR-0125). Today leg-B sets only
`TCP_NODELAY` before `connect()`
(`crates/overdrive-dataplane/src/mtls/mod.rs:647-652`, called from
`outbound.rs:83`).

No address of the guest prefix is local to the host and no route covers the
prefix except the gateway's `/32` (ADR-0173, ADR-0152). An unmarked leg-B
dial to a same-node VM address therefore has no route: with no node default
route `connect()` fails `ENETUNREACH`, and with one it leaves through the
default route's device and source, and reaches leg-C only because the output
divert re-routes it (V-LEGB-MARK S4 a, b2). The node default route is not a
route Overdrive owns, so the same-node mesh path cannot depend on it.

The existing intercept already holds a route for marked packets: the
production client installs `fwmark 0x1 lookup 100` ahead of the main table
(pref 32765: a rule added without a priority takes the second rule's
preference minus one, `net/core/fib_rules.c:83-98`, applied at `:745 @ v7.0`;
V-LEGB-MARK S7) and `local default dev lo table 100`. A connect-time lookup
carries the socket's mark (`include/net/route.h:319-335 @ v7.0`, the mark at
`:332`) and, for an unbound socket, runs twice: once to choose the source,
again with that source (`include/net/route.h:337-356 @ v7.0`). Measured on
Linux 7.0 (V-LEGB-MARK, Lima `7.0.0-38-generic` aarch64 and metal
`7.0.0-29-generic` x86_64):

- **The mark alone fails.** The marked lookup returns the table-100 `local`
  route; with no source set, a `local` result takes the route's preferred
  source or else the destination as source
  (`net/ipv4/route.c:2848-2853 @ v7.0`). The destination is a VM address,
  not a host address, so the second lookup fails the local-source check and
  returns `ENETUNREACH` (`net/ipv4/route.c:2729`, `:2766-2769 @ v7.0`;
  S4 c). That check (`__ip_dev_find`, `net/ipv4/devinet.c:143-170 @ v7.0`)
  looks for an interface address and otherwise consults only the local
  table (`:158-162`), where no prefix address but the gateway has a `local`
  route; the table-100 `local default` route does not count.
- **The loopback bind alone fails.** Unmarked, a `127.0.0.1` source has no
  route to the prefix: the main-table lookup finds none and fails
  `ENETUNREACH` (`net/ipv4/route.c:2814`, `:2844 @ v7.0`; `fib_lookup`
  maps `-ESRCH` to `-ENETUNREACH`, `net/ipv4/fib_rules.c:104-105 @ v7.0`;
  S4 d1), or, with a node default route,
  is refused as a loopback source on a non-loopback device while
  `route_localnet` is 0 (`EINVAL`, `net/ipv4/route.c:2578-2582 @ v7.0`;
  S4 d2). Production never sets `route_localnet`.
- **Both together work.** The bound `127.0.0.1` passes the local-source
  check (it is an address of the loopback device, found by
  `inet_lookup_ifaddr_rcu`, `net/ipv4/devinet.c:149 @ v7.0`), the `local`
  result sends the packet through the loopback device
  (`net/ipv4/route.c:2856-2858 @ v7.0`), and the existing TPROXY rule
  assigns it to leg-C, which sees `peer=127.0.0.1:<ephemeral>`,
  `local=<workload_addr>:<port>` — with and without a node default route,
  and with no packet on the uplink (S1, S2). A real Cloud Hypervisor microVM
  behind a leg-C stand-in answered over vsock (S8-equivalent).
- **The mark must stay conditional.** A mark-1 lookup of a peer outside the
  prefix also resolves to `local … dev lo table 100` (S5, S7), so marking a
  remote peer would loop it to the host.

Evidence: `docs/feature/netns-density-295/spike/findings-v33-legb-mark.md`;
research `docs/research/networking/guest-prefix-enetunreach-alternatives-research.md`.

## Decision

**Leg-B marks and loopback-binds a dial whose peer is inside the guest
prefix, and does neither for any other peer.**

Before `connect()`, when the peer address lies inside the guest prefix, leg-B
sets the socket mark to the intercept mark `MTLS_TPROXY_MARK` (`0x1`,
`crates/overdrive-core/src/dataplane/mtls_mark.rs:27`), allows a bind without
a port (`IP_BIND_ADDRESS_NO_PORT`), and binds `127.0.0.1:0`; otherwise it
sets none of the three. The bind-without-port option is what keeps
concurrent dials from exhausting `127.0.0.1`'s ephemeral ports. Without it,
`bind(127.0.0.1:0)` reserves a local port per socket at bind time
(`net/ipv4/af_inet.c:542-544 @ v7.0` calls `get_port`; `inet_csk_get_port`
→ `inet_csk_find_open_port`, `net/ipv4/inet_connection_sock.c:316-410 @
v7.0`, takes only a port with no bind conflict), so each concurrent
same-node dial would hold one port of the range. With it (the bit set at
`net/ipv4/ip_sockglue.c:1023-1025 @ v7.0`, which makes `bind()` skip
`get_port`), `connect()` picks the port in `__inet_hash_connect`
(`net/ipv4/inet_hashtables.c:1035-1226 @ v7.0`), which reuses a port
whenever the 4-tuple is unique (`:1099-1104`, `:1118-1124`). V-LEGB-MARK
did not compare the option on and off (findings § "Not run, and limits");
V-LEGB-SERVE does (DELIVER, not contract-shaping). The
host mTLS enforcement adapter (`HostMtlsEnforcement`) owns the dial and
receives the guest prefix as a required construction input; production
passes the node's guest prefix. No rule, route, netdevice, BPF program or
node state is added. The signature is in the feature delta.

## Alternatives considered

- **A non-local route on `lo` covering the whole prefix** (PREFIX-ROUTE-LO).
  Makes every prefix address a local-delivery destination for
  host-originated packets, leaving ADR-0172's constant rules as the only
  barrier between non-diverted guest-prefix traffic and host sockets. Needs a
  boot convergence step, a route protocol tag for the prefix and a
  route-conflict refusal. The leg-B dial needs a route for itself only, and
  the existing mark rule provides one. Rejected.
- **A non-loopback dummy device as the output of a prefix route.** One more
  converged netdevice and a dependency on its source address. Rejected.
- **`src 127.0.0.1` on the table-100 route instead of the bind.** Works
  (S4 c'), but needs a new netlink route attribute and changes a route shared
  by every `fwmark 0x1` lookup. Rejected.
- **`SO_BINDTODEVICE=lo`.** Works only together with the loopback bind (S6),
  through the kernel's on-link fallback after a failed lookup with a bound
  device (`net/ipv4/route.c:2818-2843 @ v7.0`); without the bind the source
  is a node address and the handshake times out. No advantage over the bind.
  Rejected.
- **Rely on a node default route.** A route Overdrive does not own; the dial
  would fail on a node without one. Rejected.

## Consequences

- A same-node mesh peer appears to leg-C as `127.0.0.1:<ephemeral>` on every
  host; today, with a node default route, it appears as the uplink address.
  Leg-C resolves a connection by its recovered original destination
  (ADR-0120) and identity is the peer SVID, so nothing in leg-C keys on the
  client source address.
- No route covers the guest prefix except the gateway's `/32` `local` route
  (ADR-0152). INB-1 (ADR-0173) holds at the route table: no prefix address
  other than the gateway is delivered to a host socket by a route.
- Fail-closed shapes (S3, S4): a marked dial to a tuple not registered in
  `inbound_destinations` is refused by ADR-0172's constant output rule — TCP
  `ECONNREFUSED`, UDP `EPERM` — and reaches no host socket; a dial missing
  either half fails `ENETUNREACH` (or `EINVAL` with a node default route)
  and never reaches a host socket. The gateway is the one host-local prefix
  address, and leg-B never dials it.
- "A peer inside the guest prefix is a same-node VM" holds while Overdrive is
  single-node: every node uses the same guest prefix
  (`crates/overdrive-core/src/guest_transport/mod.rs:382`). How a client
  addresses a VM on another node, and whether the condition stays "peer
  inside the guest prefix", is
  [#317](https://github.com/overdrive-sh/overdrive/issues/317).
- Test hosts can omit a node default route; same-node mesh tests no longer
  rely on one.
- V-LEGB-MARK is the contract-shaping validation and is held. V-LEGB-SERVE —
  a real `overdrive serve` + `overdrive deploy` run of a same-node mesh dial
  to a VM backend through the production leg-B, leg-C, owner and
  `workload_addr` — remains a DELIVER validation item (feature delta
  § *Validation plan*); a failure is a DELIVER defect, not a contract change.
