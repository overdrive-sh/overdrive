# ADR-0121 — Gate the public listener without redefining workload or Service readiness

## Status

Proposed — 2026-09-13.

## Context

TCP/443 must not accept a connection before the gateway can complete the public
and internal trust path. Those gateway prerequisites do not own Allocation
`Running`, Service `Stable`, Backend Eligibility or BPF Hydrated semantics.

## Decision

Let the `overdrive serve`/Gateway Application owner bind and admit on public
TCP/443 only after a complete current Route/certified-key generation, demanded
frontend application, trusted registered-connect path and current gateway SVID/
exact-peer mTLS capability are available. Loss or withdrawal closes new public
admission before retiring retained generations. Gateway gates never delay,
advance, revoke or reinterpret Allocation `Running`, Service `Stable`, Backend
Eligibility, BPF Hydrated or operator HTTPS readiness.

This ADR decides only public-listener gate ownership and its relationship to
existing lifecycle states. Exact gates and status shapes live in the feature
delta and ADR-0111.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Bind a degraded listener before prerequisites | Creates partial public admission that cannot satisfy the required trust/forwarding path. |
| Gate Allocation `Running` or Service `Stable` on gateway readiness | Transfers a node-ingress concern into unrelated workload/Service state owners. |
| Keep accepting after a required identity/key generation is lost | Admits work that cannot complete safely and encourages fallback behavior. |

## Consequences

- Gateway unavailability is visible through the separate redacted operator
  status path, not by changing workload or Service state meanings.
- Existing admitted connections drain under their retained generation; no HA or
  cross-node failover is claimed.
- `NoBackend` remains a request-time result while an otherwise healthy listener
  may stay bound; it does not redefine Backend Eligibility.

## Links

- [Lifecycle Gate Ownership](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-lifecycle-gate-ownership)
- [ADR-0106](adr-0106-public-ingress-route-and-gateway-application-owner.md)
- [ADR-0111](adr-0111-redacted-operator-gateway-status.md)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
