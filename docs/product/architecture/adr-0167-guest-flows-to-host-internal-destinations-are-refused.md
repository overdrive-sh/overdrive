# ADR-0167 — Guest flows to host-internal destinations are refused with a typed refusal, checked by the owner and enforced atomically in the host kernel

## Status

**Proposed — decision D26 (raised by independent DESIGN review finding B-1)
approved by user 2026-10-06; pending independent DESIGN review.**
GH #295. Recorded in the #295 feature delta, § *[REF] Per-flow policy and the
host-internal deny set*.

Amends on acceptance: ADR-0162 (non-mesh egress is forwarded to its
destination — except the destinations below).

## Context

Under ADR-0162 the guest-flow owner opens the host-side socket for every
non-mesh guest flow from inside the `overdrive serve` process. Any guest
process can request any IPv4 destination. Without a deny set, a guest could
make the host connect to:

- host loopback services (`127.0.0.0/8`) — admin APIs, local databases,
  debugging endpoints bound to loopback because loopback is trusted;
- link-local addresses (`169.254.0.0/16`), including cloud instance metadata
  at `169.254.169.254`, which hands out host credentials;
- any address local to the node: the host's interface addresses, the guest
  gateway, and every `workload_addr` (local through the shared route,
  ADR-0152), where a wildcard-bound host service (`sshd`, the operator API)
  would answer.

Today's bridge topology (#295 code before this replacement) is not a safe
baseline either: guest TCP to any destination is TPROXYed to leg-F, and
leg-F's non-mesh pass-through dials `orig_dst` from the host with no mark and
no deny list (`spawn_cleartext_passthrough`), so guest TCP to `127.0.0.1` or
`169.254.169.254` already reached host services. Guest UDP to `127.0.0.0/8`
was dropped only by the kernel's implicit martian handling. No deny set
existed by design. The vsock replacement makes every guest flow an owner
decision, so the deny set must be explicit.

Service VIPs (ADR-0053, default `10.96.0.0/16`) are platform-authorised: their
backends are chosen by the platform's own service map and may be host-local.

## Decision

- **Deny set.** The owner refuses a guest-opened flow with the typed refusal
  `HostInternal` when its requested IPv4 destination is in:
  1. `127.0.0.0/8` (host loopback);
  2. `169.254.0.0/16` (link-local, including instance metadata);
  3. any address the host kernel would deliver locally — its FIB route type
     for the destination is `local` — which covers the host's interface
     addresses, the guest gateway and the whole guest prefix;

  with exactly two exceptions:
  - the guest gateway on port 53 (UDP and TCP), which reaches the node DNS
    responder (ADR-0154);
  - a TCP destination that the owner's mesh resolution classifies as mesh:
    it is never connected directly; the owner connects to leg-F (ADR-0153).

  The existing `Policy` refusals stay: `0.0.0.0/8`, multicast `224.0.0.0/4`,
  limited broadcast, and UDP to the guest prefix (#310). `240.0.0.0/4`
  (reserved) joins `Policy`.
- **Order.** Static ranges first; then mesh resolution for TCP; then the
  local-delivery check for every flow the owner would connect directly.
- **Atomic kernel enforcement.** Every host socket the owner connects directly
  for a guest flow whose requested destination is not a service VIP carries a
  dedicated socket mark. A constant host firewall rule in the output path
  rejects any packet carrying that mark whose destination is locally
  delivered or in `127.0.0.0/8` or `169.254.0.0/16`, except the gateway on
  port 53. The owner's check gives the typed refusal; the kernel rule closes
  the window between the check and the connect (an address added to the host
  in between). A connect the kernel rule rejects is projected as
  `DestinationUnreachable`.
- **VIP flows.** A flow whose requested destination is a service VIP is left
  unmarked, so ADR-0053's rewrite to a platform-chosen backend is honoured.
  For datagram associations the owner reads the connected socket's kernel peer
  before installing any route; a peer inside the guest prefix is refused
  `Policy` (#310).

## Alternatives considered

- **No deny set (bridge-era behaviour).** Lets any guest process reach
  loopback services, instance metadata and wildcard-bound host services.
  Rejected.
- **Owner check only.** A check-then-connect race lets a just-added host
  address through. Rejected in favour of the check plus a kernel rule.
- **Kernel rule only.** Atomic, but the guest sees a generic failure instead
  of a typed refusal, and the owner cannot count it by cause. Rejected as the
  only mechanism.
- **A fixed range list without the local-delivery check.** Misses the host's
  own public and private addresses and any address an operator adds. Rejected.

## Consequences

- A guest can no longer reach host loopback services, link-local addresses or
  wildcard-bound host services. This is a deliberate change from the
  bridge-era TCP behaviour and is the only D15 deviation this ADR introduces.
- Platform services stay reachable through mesh resolution, service VIPs and
  DNS.
- One more refusal code on the wire, one socket mark, and one constant
  firewall rule.
- Whether the firewall's local-delivery match works in the host output path
  is a validation item (V-21), not an observed fact.
