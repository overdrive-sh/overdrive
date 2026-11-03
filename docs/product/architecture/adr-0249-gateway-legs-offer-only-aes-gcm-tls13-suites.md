# ADR-0249 — Both gateway legs offer only the AES-128-GCM and AES-256-GCM TLS 1.3 suites

## Status

Proposed — pending independent review. User ruling P-12 (R-8, 2026-11-02).

## Context

[ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md)
arms kernel TLS on both gateway legs after a userspace handshake. rustls's default
TLS 1.3 set adds ChaCha20-Poly1305. The spike
([findings](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md))
exercised kTLS arming, rekey and splice only with the two AES-GCM suites.

## Decision

Both legs offer exactly the AES-128-GCM and AES-256-GCM TLS 1.3 suites. A client
that offers no AES-GCM suite is refused before arming.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Offer rustls's default TLS 1.3 suites | Adds ChaCha20-Poly1305, whose kTLS arming and rekey behaviour the spike never exercised. Rejected by ruling P-12. |
| Offer ChaCha20-Poly1305 after a new spike | A new validation for a suite no known client needs exclusively; adds scope without a reachable failure. |

## Consequences

- TLS 1.3 clients offering no AES-GCM suite cannot connect (none known).
- The suite set is the one set the spike and the Tier-3 relay checks exercise.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), [ADR-0226](adr-0226-public-listener-tls13-only.md), [ADR-0234](adr-0234-exact-peer-gateway-client-mtls.md)
