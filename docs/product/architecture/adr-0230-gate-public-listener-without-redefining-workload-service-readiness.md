# ADR-0230 — Gate the public listener without redefining workload or Service readiness

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13; its demanded-frontend gate is retired by D1/D2 (user-approved 2026-11-01, see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)).

## Context

TCP/443 must not accept a connection before the gateway can complete the public
and internal trust path. Those gateway prerequisites do not own Allocation
`Running`, Service `Stable`, Backend Eligibility or BPF Hydrated semantics.
Backend availability is request-time: the typed `NoBackend` outcome of the
gateway's consuming port ([ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)),
not a listener prerequisite.

## Decision

Let the `overdrive serve` gateway gate task bind and admit on public TCP/443
only after, in order: a usable current certified key, a held current Gateway
SVID, a passing registered-connect probe (the sentinel connect proves the
dataplane adapter, hook and receipt path honour their contract in the real
kernel), an actual-frontend probe whose `NoBackend` result is bind-valid and
whose `Selected` result completes exact-peer mTLS, and promotion of a complete
current Route/certified-key generation. Loss or withdrawal of a required
identity or key closes new public admission before retiring retained
generations. A failed gate never refuses the node's boot and never requests
serve shutdown: TCP/443 stays unbound and the cause is reported through the
operator status path. Gateway gates never delay, advance, revoke or reinterpret
Allocation `Running`, Service `Stable`, Backend Eligibility, BPF Hydrated or
operator HTTPS readiness.

This ADR decides only public-listener gate ownership and its relationship to
existing lifecycle states. Gate order, failure projection and boundary
scenarios are in the feature delta; status shapes are ADR-0227.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Bind a degraded listener before prerequisites | Creates partial public admission that cannot satisfy the required trust/forwarding path. |
| Gate Allocation `Running` or Service `Stable` on gateway readiness | Transfers a node-ingress concern into unrelated workload/Service state owners. |
| Keep accepting after a required identity/key generation is lost | Admits work that cannot complete safely and encourages fallback behaviour. |
| Require a healthy backend before binding | Couples node ingress to workload readiness; backend absence is a per-request 503. |

## Consequences

- Gateway unavailability is visible through the separate redacted operator
  status path, not by changing workload or Service state meanings.
- Existing admitted connections drain under their retained generation; no HA or
  cross-node failover is claimed.
- `NoBackend` remains a request-time result while an otherwise healthy listener
  stays bound; it does not redefine Backend Eligibility.

## Links

- [Lifecycle Gate Ownership](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-lifecycle-gate-ownership)
- [Composition-root slot, boot and shutdown](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-composition-root-gateway-slot-boot-and-shutdown-d5)
- [ADR-0222](adr-0222-public-ingress-route-and-gateway-application-owner.md), [ADR-0227](adr-0227-redacted-operator-gateway-status.md), [ADR-0242](adr-0242-backend-unavailability-is-a-typed-outcome-at-the-gateway-consuming-port.md)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
