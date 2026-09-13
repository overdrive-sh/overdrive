# ADR-0109 — Use a dedicated non-allocation gateway SVID for exact-peer workload mTLS

## Status

Proposed — 2026-09-13.

## Context

The external user authenticates the public Web-PKI certificate and never holds
a SPIFFE identity. The gateway still needs an internal client identity for its
workload hop, while BPF-selected BackendId must resolve to the exact workload
SPIFFE peer. Existing identity holders and transparent-mTLS entry points are
Allocation-shaped and cannot represent this role without inventing an
AllocationId.

## Decision

Give each enabled node one dedicated, volatile Gateway Identity Slot and a
node-targeted `GatewaySvidLifecycle` that reuse the existing issue-and-audit,
renewal and retry policy for `spiffe://overdrive.local/gateway/<node-id>`.
Only `HostGatewayClientMtls` may consume held private material; it presents the
gateway leaf plus node intermediate and completes only when the peer's sole
SPIFFE URI SAN equals the Dataplane receipt-selected workload identity.

This is one decision about internal gateway transport identity and peer
authentication. The public certified key remains a different credential
domain, and BPF selection remains a different owner.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Fabricate an AllocationId and reuse `IdentityMgr`/`MtlsEnforcement` | It corrupts Allocation identity/lifecycle meaning and forces the wrong intercepted-connection API. |
| Verify only that the peer chains to the internal trust bundle | A valid but unintended workload identity would be accepted after a selection/routing error. |
| Reuse the public Web-PKI private key as the internal client identity | It crosses trust domains and exposes public-origin material to internal SPIFFE transport. |

## Consequences

- Allocation SVID ownership and the transparent kTLS pump remain unchanged.
- The slot is ephemeral, reissued after restart and dropped only after gateway
  users drain; a checked desired epoch prevents stale post-disable re-hold.
- Exact-peer mismatch, absent identity and expiry fail closed with no plaintext
  or alternate-backend fallback.
- One additional reconciler kind and closed-dispatch variants are required.

## Links

- [Exact lifecycle and mTLS port contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Reconciler/component registration contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-application-component-decomposition)
- [Canonical C4 identity path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0113](adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md)
- [System trust-boundary ADR-0120](adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md)
- ADR-0063, ADR-0067, ADR-0069 and ADR-0071
