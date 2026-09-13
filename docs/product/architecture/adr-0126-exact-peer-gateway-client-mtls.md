# ADR-0126 — Require the receipt-selected workload identity in gateway-client mTLS

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The gateway has a dedicated internal SVID and the Dataplane receipt identifies
the selected backend's applied workload SPIFFE identity. Trust-bundle validity
alone would accept a different valid workload after a routing, receipt or
identity-association error.

## Decision

Have the gateway-client rustls connection present the current Gateway SVID and
complete only when the peer certificate's sole SPIFFE URI SAN equals the exact
workload identity resolved from the receipt-selected BackendId. Any absence,
expiry, malformed peer identity or mismatch fails closed with no plaintext or
alternate-backend fallback.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Verify only that the peer chains to the internal trust bundle | A valid but unintended workload identity could be accepted. |
| Reuse the transparent workload interception API | Its intercepted-connection and Allocation-shaped semantics do not represent this explicit L7 client. |
| Fall back to plaintext on handshake failure | It defeats the internal trust boundary. |

## Consequences

- Backend choice and peer authentication are bound to one actual connect.
- The gateway SVID holder remains inaccessible to request handlers and other
  consumers.
- Gateway SVID lifecycle remains independently owned by ADR-0109.

## Links

- [Exact gateway-client mTLS contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Canonical C4 trust transition](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Gateway SVID ADR-0109](adr-0109-dedicated-gateway-svid-identity-lifecycle.md)
- [Receipt ADR-0124](adr-0124-gateway-connect-selected-backend-receipt.md)
- [System trust ADR-0120](adr-0120-separate-public-tls-from-gateway-svid-workload-mtls.md)
- ADR-0069 and ADR-0071
