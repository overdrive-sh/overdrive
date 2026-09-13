# ADR-0120 — Separate public TLS from gateway-SVID workload mTLS

## Status

Proposed — 2026-09-13.

## Context

External users authenticate a public Web-PKI origin and present no SPIFFE
identity. After public TLS terminates, the platform must authenticate both ends
of the internal hop and ensure the workload is the backend BPF actually selected.
The two legs therefore have different credentials and trust semantics.

## Decision

Terminate public Web-PKI TLS at the embedded gateway, then originate a distinct
internal mTLS connection using the node's dedicated gateway SPIFFE SVID. Require
the workload peer SPIFFE identity to equal the Dataplane receipt's exact selected
backend identity before application bytes cross the internal trust boundary.
Public Web-PKI, operator/control-plane HTTPS and internal SPIFFE credentials
remain three non-substitutable domains.

This ADR decides only the System trust-boundary transition. ADR-0109 owns the
dedicated holder/lifecycle and exact-peer Application decision.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Plaintext gateway-to-workload hop | Loses confidentiality and workload authenticity after public TLS termination. |
| Reuse the public origin certificate internally | Gives a Web-PKI server credential the wrong client identity and trust domain. |
| Verify only that the workload chains to the internal CA | Accepts a wrong-but-valid workload and fails to bind authentication to BPF's actual selection. |

## Consequences

- Public clients never need or receive SPIFFE credentials.
- The gateway identity is not an Allocation identity and cannot use a fabricated
  `AllocationId`.
- Wrong-valid-peer, missing gateway material or internal handshake failure fails
  closed without plaintext or alternate-backend fallback.

## Links

- [ADR-0109](adr-0109-dedicated-gateway-svid-lifecycle-and-exact-peer-mtls.md)
- [ADR-0113](adr-0113-dedicated-gateway-identity-slot-and-lifecycle.md)
- [ADR-0114](adr-0114-dataplane-selection-receipt-and-applied-backend-identity.md)
- [Exact Application identity contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
