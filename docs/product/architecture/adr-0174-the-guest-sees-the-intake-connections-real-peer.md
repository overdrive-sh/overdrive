# ADR-0174 — The guest sees the intake connection's real peer; host-local platform clients appear from the guest gateway address

## Status

**Proposed — decision D8 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. Follows ruling D15
(APPROVED 2026-10-05): guests keep today's network behaviour wherever the
kernel allows it.

## Context

Under the bridge topology a guest application saw the real client address of
every inbound connection, and a platform client on the host (leg-S, a probe)
reached the guest from the bridge gateway address.

Under ADR-0173 the host terminates inbound TCP at an intake listener and opens
a vsock flow to the guest. Unless the flow carries the client address, the
guest application would see a vsock-side or loopback peer.

For a destination covered by a `local` route, the kernel selects the route's
preferred source as the source of a host-local connect, if it has one, and
otherwise the destination address itself (kernel source; not yet observed,
V-12).

## Decision

- **Real peer.** The `TcpAccept` request carries the intake connection's
  kernel-reported peer address; the guest connects to the application from
  that address with `IP_TRANSPARENT` (ADR-0150), so `getpeername` in the guest
  returns it.
- **Platform clients appear from the gateway.** The shared `local` route
  (ADR-0152) carries the guest gateway address as its preferred source, so a
  host-local platform client (leg-S, a probe) presents the gateway address, as
  under the bridge topology. Remote clients present whatever leg-S presents
  today, unchanged.

## Alternatives considered

- **No preferred source on the local route.** Platform clients would present
  `workload_addr` itself as the client address. Rejected.
- **Present a fixed host address for every inbound connection.** Loses the
  real client (a D15 regression). Rejected.

## Consequences

- The guest application logs the real client for remote and host-local
  connections (spike: `sshd` logged the host's real client).
- The guest's reset toward a transparent client address is delivered at once
  because the guest sets `net.ipv4.fwmark_reflect=1` (ADR-0150).
- No steering, policy or safety rule keys on the source; a falsified V-12
  changes only the address a platform client presents, surfaced to the user.
