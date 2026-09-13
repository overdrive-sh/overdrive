# ADR-0107 — Protect public certified keys in producer-neutral custody

## Status

Proposed — 2026-09-13.

## Context

Public TLS consumes an operator-supplied Web-PKI certificate chain and matching
private key in the first slice. Manual filesystem paths are acquisition inputs,
not runtime authority, and later ACME issue #57 must be able to produce the same
logical credential without redesigning the listener. A bad or not-yet-usable
replacement must not destroy the last time-usable generation.

## Decision

Use one producer-neutral `PublicCertifiedKeyCustody` owner that validates and
builds a complete usable generation before sealing and persisting it, then
atomically publishes an opaque resolver snapshot and only then writes redacted
status. Replacement is preserve-old:
an invalid or currently unusable candidate never overwrites the durable or live
last usable generation. Manual input is the only first-slice producer; #57 may
later add a Workflow producer at the same install boundary.

This is one decision about public credential ownership and replacement
atomicity. Route ownership, internal SPIFFE identity and HTTP behavior remain
separate decisions.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Let the listener read PEM paths directly | It couples consumption to manual acquisition, leaks path/secret concerns into request handling and cannot accept a later Workflow producer cleanly. |
| Reuse the operator/control-plane CA or internal SPIFFE CA | Those credential domains do not provide public Web-PKI server identity and must remain non-substitutable. |
| Persist a replacement before proving profile, time usability and rustls construction | A crash or restart could replace a working generation with one the listener cannot use. |

## Consequences

- Plaintext key bytes and manual paths stop at the source/custody boundary;
  durable state contains only a protected key and non-secret metadata.
- Persistence, live publication and status have one explicit order and
  crash-recovery adoption rule.
- Invalid refresh preserves service until the prior generation itself becomes
  unusable; expiry still closes new admission fail-closed.
- ACME remains excluded from this slice and can be added without a second TLS
  store, manager or resolver.

## Links

- [Exact custody, replacement and persistence contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Lifecycle gates](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-identity-and-lifecycle-integration)
- [Canonical C4 custody boundary](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0112](adr-0112-public-certified-key-custody-domain-boundary.md)
- [System acquisition/consumption ADR-0117](adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
- [GitHub #57](https://github.com/overdrive-sh/overdrive/issues/57)
