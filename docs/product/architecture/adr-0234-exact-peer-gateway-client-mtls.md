# ADR-0234 — Require the receipt-selected workload identity in gateway-client mTLS

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13; re-pinned to the leg-C path by D3 (user-approved 2026-11-01, see [feature-delta § Reconstruction: user decisions](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-reconstruction-user-decisions-and-proposals)). Remediation r1 (2026-11-02) merged the former ADR-0234 (separate public TLS from workload mTLS) into this ADR as the trust-domain consequence; the decision is unchanged.

## Context

External users authenticate a public Web-PKI origin and present no SPIFFE
identity. After public TLS terminates, the platform must authenticate both ends
of the internal hop and ensure the workload is the backend BPF actually
selected, so the two legs use different credentials and trust semantics.

The gateway has a dedicated internal SVID and the Dataplane receipt identifies
the selected backend's applied workload SPIFFE identity. On main, the gateway's
rewritten dial is diverted to leg-C, which presents the SVID of the allocation
that owns the dialed guest address and verifies only that the client SVID chains
to the trust bundle; it does not select or pin the client
(`crates/overdrive-dataplane/src/mtls/tls_config.rs`,
[ADR-0186](adr-0186-leg-s-reaches-vm-backends-through-a-temporary-vsock-relay-until-303.md)).
Trust-bundle validity alone on the gateway side would therefore accept a
different valid workload after a routing, receipt or identity-association error.

## Decision

Terminate public Web-PKI TLS at the embedded gateway, then originate a distinct
internal mTLS connection. Have the gateway-client rustls connection, run over
the prepared, marked and loopback-bound upstream socket
([ADR-0190](adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md),
[ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md)),
present the current Gateway SVID and complete only when the peer certificate
chains to the trust bundle and its sole SPIFFE URI SAN equals the exact
workload identity resolved from the receipt-selected BackendId. Any absence,
expiry, malformed peer identity or mismatch fails closed with no plaintext or
alternate-backend fallback. The peer is the node-shared leg-C enforcement point
presenting the selected allocation's SVID; the equality check is what binds the
authenticated peer to BPF's actual selection.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Verify only that the peer chains to the internal trust bundle | A valid but unintended workload identity could be accepted. |
| Reuse the transparent workload interception API | Its intercepted-connection and Allocation-shaped semantics do not represent this explicit L7 client. |
| Fall back to plaintext on handshake failure, or a plaintext gateway-to-workload hop | It defeats the internal trust boundary and loses confidentiality and workload authenticity after public TLS termination. |
| Reuse the public origin certificate internally | Gives a Web-PKI server credential the wrong client identity and trust domain. |

## Consequences

- Backend choice and peer authentication are bound to one actual connect.
- Public clients never need or receive SPIFFE credentials. Public Web-PKI,
  operator/control-plane HTTPS and internal SPIFFE credentials remain three
  non-substitutable domains.
- The gateway identity is not an Allocation identity and cannot use a fabricated
  `AllocationId`. The gateway SVID holder remains inaccessible to request
  handlers and other consumers; its lifecycle is owned by ADR-0225.
- Leg-C remains authentication-only toward its client; authorization of the
  selected peer is the gateway's, by this equality check.
- Wrong-valid-peer, missing gateway material or internal handshake failure fails
  closed without plaintext or alternate-backend fallback.

## Links

- [Gateway upstream path into leg-C](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-upstream-path-into-leg-c-d3-d5-d)
- [Gateway-client mTLS contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Canonical C4 trust transition](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0225](adr-0225-dedicated-gateway-svid-identity-lifecycle.md), [ADR-0232](adr-0232-gateway-connect-selected-backend-receipt.md), [ADR-0241](adr-0241-dataplane-applied-backend-identity-association.md), [ADR-0186](adr-0186-leg-s-reaches-vm-backends-through-a-temporary-vsock-relay-until-303.md)
