# ADR-0346 — Gateway-client mTLS requires the selected backend's workload identity

## Status

Accepted — approved by the user 2026-11-21 (D3). Supersedes [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md)
(exact-allocation pin). ADR-0234 keeps its text.

## Context

ADR-0234 requires the peer certificate's sole SPIFFE URI SAN to equal the exact
identity resolved from the receipt-selected `BackendId`, so a different valid
workload answering after a routing, receipt or association error is rejected. Its
identity was per allocation, so "exact" meant exact instance. That made a
replacement allocation of the same workload at the same endpoint a mismatch, the
502 of the RCA, although the replacement is precisely the party the gateway
intends to reach ([ADR-0344](adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md)).

The threat ADR-0234 names is a different *workload* answering. The instance of a
workload is not part of that threat: every allocation of the workload is
authorised to serve it.

## Decision

After the rustls handshake and before the connection is armed as kernel TLS, the
gateway requires the peer certificate to chain to the trust bundle and its sole
SPIFFE URI SAN to equal the **workload identity** resolved from the
receipt-selected `BackendId`
([ADR-0347](adr-0347-dataplane-backend-identity-association-carries-the-workload-identity.md)).
A replacement allocation of the same workload passes; a different workload,
absence, expiry, a malformed identity or a mismatch fails closed with no plaintext
or alternate-backend fallback. The remainder of ADR-0234 (distinct internal mTLS
after Web-PKI termination, the gateway SVID, no fabricated `AllocationId`,
leg-C authenticating only) is unchanged.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep the exact-allocation pin with per-allocation IDs | The identity itself was the defect; every replacement fails the pin. |
| Verify only that the peer chains to the trust bundle | Accepts any valid workload; defeats the check ADR-0234 exists for. |
| Pin the workload by comparing a parsed path prefix of a per-allocation ID | Rejected with ADR-0344: couples the verifier to a path convention. |

## Consequences

- The gateway cannot tell which allocation of the workload answered; that is not
  part of its trust decision. Allocation attribution of a connect, if wanted, comes
  from the dataplane, not the peer certificate.

## Links

- [feature-delta](../../feature/workload-spiffe-identity/feature-delta.md)
- [ADR-0344](adr-0344-a-workload-svid-names-the-workload-not-the-allocation.md), [ADR-0347](adr-0347-dataplane-backend-identity-association-carries-the-workload-identity.md), [ADR-0243](adr-0243-gateway-upstream-sockets-are-prepared-by-the-dataplane-adapter.md), [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md)
