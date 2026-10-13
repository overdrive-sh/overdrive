# ADR-0152 — Only the guest gateway is host-local in the guest prefix

## Status

**Accepted — GW-LOCAL approved by user 2026-10-13.** GH #295.
Recorded in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] vsock Attachment
Replacement DESIGN* (decision index; § *[REF] Owner and provisioner —
`overdrive-control-plane::guest_network`* and § *[REF] Composition —
production entry points* carry the contract).

Related decisions, each its own ADR: the absence of any host-local route for
the rest of the prefix and how host platform clients reach a VM port
(ADR-0173, INB-1, accepted), the prefix holding no node address and no VIP
(ADR-0185, accepted), the constant rules that refuse guest-prefix traffic not
diverted to leg-C (ADR-0172), the same-node leg-B dial that needs no route
for the prefix (ADR-0190), and guest DNS (ADR-0154), which depends on this
decision.

## Context

Under ADR-0145 the guest has no NIC, and no address of the guest prefix is
local to the host (ADR-0173). One address of the prefix still has to be
reachable on the host: the guest gateway. Guest DNS keeps today's
nameserver, the gateway, port 53 (ADR-0154). The owner pairs a guest DNS
association with a host UDP socket connected to `gateway:53`, and the
`DnsResponder` binds `0.0.0.0:53` (ADR-0116). The exchange works only if
the gateway is a local address of the host, so that the query arrives
addressed to the gateway and the reply leaves from it.

A destination is local when the route lookup returns a `local` route: output
then goes through the loopback device (`net/ipv4/route.c:2848-2867 @ v7.0`)
and input is delivered locally (`net/ipv4/route.c:2373-2379 @ v7.0`). An unbound
socket takes a `local` route's preferred source, or with none the
destination, as its source (`net/ipv4/route.c:2848-2853 @ v7.0`): a socket
connected to `gateway:53` sends from the gateway, the `DnsResponder` receives
the query addressed to the gateway, and its reply leaves from the gateway,
the source the connected socket accepts. A spike measured this path with the
gateway route as the only route in the prefix: an unbound UDP socket
connected to `100.95.0.1:53` was answered from `100.95.0.1`, while other
prefix addresses failed `ENETUNREACH` (Lima `7.0.0-38-generic`;
[`findings-prefix-route-lo.md` § "Case 6 — rules restored, route removed, GW-LOCAL kept"](../../feature/netns-density-295/spike/findings-prefix-route-lo.md#case-6--rules-restored-route-removed-gw-local-kept)).
With ADR-0172's constant rules loaded and the gateway excluded from them, a
datagram to `100.95.0.1:53` was delivered to a host socket while every other
prefix destination was rejected (Lima `7.0.0-38-generic` and metal
`7.0.0-29-generic`;
[`findings-v33-legb-mark.md`](../../feature/netns-density-295/spike/findings-v33-legb-mark.md)
S3). Overdrive installs no other route
in the guest prefix (ADR-0190); without the gateway's `local` route, a datagram to the
gateway follows whatever route the node has — none (`ENETUNREACH`) or a node
default route out of the host.

## Decision

1. **The gateway is the one host-local address of the prefix.** The guest
   gateway is local to the host through one `local <gateway>/32` route on
   `lo`, tagged with Overdrive's route protocol identifier. No other address
   of the guest prefix becomes local by this decision.
2. **Written only by boot.** Boot keeps an identical tagged gateway route,
   replaces a tagged gateway route whose attributes differ, or adds a missing
   one. Nothing writes the route at runtime. A crash at any boot step leaves
   the earlier state or the new one, and the next boot converges it. The
   gateway's `/32` sits in the local table, which is consulted before the
   main table (`net/ipv4/fib_rules.c:491`, `:494 @ v7.0`), so a node route in
   the main table does not cover the gateway.
3. **Never removed.** `serve` never removes the route at shutdown, graceful or
   not. While `serve` is down the gateway stays local; no guest flow exists
   then, because guest flows exist only through the running owner.

No steering decides delivery to the gateway: a host-local or
guest-originated connection to the gateway reaches node sockets exactly as a
connection to a node address does, and ADR-0167 refuses guest flows to it
except `gateway:53`. Under this decision the gateway is excluded from the
constant guest-prefix rules (ADR-0172).

## Alternatives considered

- **Assign the gateway as an address on `lo`.** The kernel would add the same
  `local` route (`net/ipv4/fib_frontend.c:1156 @ v7.0`), but the gateway would
  then be a node interface address inside the prefix, which ADR-0185 refuses
  for every configured address; carving an exception into that refusal for
  Overdrive's own address costs more than one tagged route. Rejected.
- **Move the DNS address out of the prefix** (for example to a loopback
  address such as `127.0.0.53`). Changes the guest resolver's configured
  address; ADR-0154 rejects it under D15. Rejected.
- **Pair DNS associations with a socket connected to a node address.** Needs
  a destination rewrite in the owner for one service, and the responder's
  gateway fallback (ADR-0116) would no longer match the address the guest
  queries; ADR-0154 rejects it. Rejected.
- **Audit and repair the gateway route at runtime.** On the appliance nothing
  but boot writes it; a runtime audit could detect only a change made by
  software that does not exist on the appliance. Boot convergence covers
  Overdrive's own crashes. Rejected.
- **Remove the gateway route at graceful shutdown.** A crash skips that step
  anyway, so boot convergence must already handle a left-behind route;
  removing it makes graceful and crash restarts differ for no gain. Rejected.

## Consequences

- Guest DNS datagrams paired to `gateway:53` reach the `DnsResponder` through
  `lo`, as ADR-0154 requires.
- A host-local connect to a guest-prefix address other than the gateway is
  not delivered on the host: it fails `ENETUNREACH` when no node route covers
  it, and otherwise reaches the constant output reject of ADR-0172.
- The gateway route is the only route Overdrive installs inside the guest
  prefix (ADR-0190).
- Only Overdrive writes the gateway route on the appliance; the image
  (ADR-0068) carries no network manager or daemon that removes routes it did
  not create. That is an assumption of the design, discharged by the image,
  not by a runtime check.
