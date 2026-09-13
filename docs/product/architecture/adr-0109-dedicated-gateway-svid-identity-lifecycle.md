# ADR-0109 — Use a dedicated Gateway SVID identity lifecycle

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The public client never uses SPIFFE, but the gateway needs an internal client
identity for its workload hop. Existing SVID holders and lifecycle targets are
Allocation-shaped, while the gateway is a node-owned, non-allocation
application role.

## Decision

Give each enabled node one volatile Gateway Identity Slot and one node-targeted
`GatewaySvidLifecycle` for
`spiffe://overdrive.local/gateway/<node-id>`. Reuse the existing internal
issue-and-audit, renewal and retry foundation without fabricating an
AllocationId. The slot is reissued after restart and drops only after gateway
users drain.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Fabricate an AllocationId and reuse `IdentityMgr` | It corrupts Allocation identity and lifecycle meaning. |
| Reuse the public Web-PKI origin key | It crosses public and internal trust domains. |
| Run the gateway without an internal identity | The workload hop could not authenticate the gateway through the existing SPIFFE trust domain. |

## Consequences

- Allocation SVID ownership and transparent workload mTLS remain unchanged.
- A checked desired epoch prevents a stale issue completion from re-holding
  identity after disable.
- Exact workload-peer authentication is independently decided in ADR-0126.

## Links

- [Exact Gateway SVID lifecycle contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Canonical C4 identity path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Exact-peer mTLS ADR-0126](adr-0126-exact-peer-gateway-client-mtls.md)
- [Domain identity ADR-0113](adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md)
- [System trust boundary ADR-0120](adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md)
- ADR-0063 and ADR-0067
