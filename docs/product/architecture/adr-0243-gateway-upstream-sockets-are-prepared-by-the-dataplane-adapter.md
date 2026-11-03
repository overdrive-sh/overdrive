# ADR-0243 — Gateway upstream sockets are prepared by the dataplane adapter

## Status

Proposed — pending independent review (2026-11-02). Decision D5 approved by the
user 2026-11-01 (see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).
User ruling DG-11 (R-5, 2026-11-02): ledger slot reserved before the preparation operation.

## Context

A gateway upstream connect reaches a VM backend deterministically only if the
socket is created in the node's attach cgroup, carries `SO_MARK` and a
`127.0.0.1` source
([ADR-0190](adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md);
mark alone fails `ENETUNREACH` and bind alone fails `EINVAL`, observed on 7.0.0-38
by V-GW-1). The preparation defends against the node default route and the nft
re-route path: an unprepared dial to a registered tuple also reaches leg-C on a
host with a default route, but through the default-route source address and the
output-chain mark rule, which depends on host routing state. The socket also needs
its cookie registered as a gateway intent before `connect(2)`; registration, not
the marking, is what keeps an unregistered socket out of the selection arm. If the gateway crate performs these steps, a socket
can exist unmarked, unbound or unregistered, and the gateway crate would own
kernel-level knowledge (mark value, bind rule, intent maps) that belongs to the
dataplane. The cookie read before `connect(2)` equals the cookie the hook sees
(`net/core/sock.c:2109-2113`, `net/core/filter.c:5151-5154 @ v7.0`), and the hook
runs against the creating task's cgroup (`kernel/bpf/cgroup.c:1694 @ v7.0`).

## Decision

The dataplane adapter owns upstream-socket preparation. It creates the socket in
the calling `serve` task, applies the mark and the loopback bind, reads the
cookie and registers the intent, and hands the prepared socket to the gateway's
connector. The gateway-facing dataplane port has no separate registration
operation: a registered socket can be obtained only prepared. The only intended
caller of the preparation operation is the gateway upstream connector. It holds
the dataplane port in a private field of its module; that module privacy and
review are the enforcement, and no lint scans for it. The dataplane adapter's own boot probe makes
its sentinel connect on a socket it prepares itself.

Preparation is one atomic operation: socket creation, mark, bind, cookie read and
intent registration complete or fail together, so the connector cannot interpose
between the cookie read and the registration. The connector's bounded cleanup
ledger therefore reserves one slot before it asks for a prepared socket and binds
the returned registered intent to that slot afterwards. A full ledger refuses
before any socket exists or any intent is registered; a slot reserved for a
preparation that fails is released; and no registered intent is ever without a
ledger slot for its cleanup.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| The gateway creates and marks the socket and calls a separate register operation | Lets an unmarked, unbound or unregistered socket exist; spreads the mark and bind contract into the gateway crate. |
| Use a gateway-specific cgroup, netns or interface for the socket | The `serve` process already lies in the attach subtree (validated by V-GW-2), so it adds lifecycle for no gain. |
| A shared helper function the gateway calls | Same unrepresentable-state gap as the first row, enforced only by convention. |
| Split preparation so the connector reserves its ledger slot between the cookie read and the registration | Reintroduces a socket that exists with a cookie but no registration, and makes the port two operations again. Rejected by ruling DG-11 (R-5). |

## Consequences

- The bug class "gateway socket not marked, bound and registered" is not
  representable through the gateway-facing port.
- The dataplane crate depends on the gateway crate's port definitions, not the
  reverse for implementations.
- The new public surface is the confirmed D5 list in the feature delta (R-18).
- The ledger order is a rule of one sequential connector task. It is covered by
  the seeded `overdrive-sim` invariants (listed in the feature delta), not by a
  Quint hazard (no concurrent protocol is modelled for it).

## Links

- [Gateway upstream path into leg-C](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-upstream-path-into-leg-c-d3-d5-d)
- [New public surface (D5)](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5)
- [ADR-0190](adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md), [ADR-0219](adr-0219-run-server-receives-its-driven-adapters-from-the-composition-root.md), [ADR-0224](adr-0224-existing-cgroup-bpf-gateway-backend-selection.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md)
