# ADR-0117 — Separate public-certificate acquisition from runtime consumption

## Status

Proposed — 2026-09-13.

## Context

The first production slice uses an operator-supplied Web-PKI certificate and
private key, while GitHub issue #57 may later automate acquisition through ACME.
If the listener or Route reads manual files directly, changing the producer
would also redesign the runtime trust and routing path.

## Decision

Make the public gateway consume one protected, canonical Public Certified Key
through producer-neutral custody. Approved manual operator input is the first
producer. A later ACME workflow may publish through the same custody boundary;
it must not change listener, resolver, Route or Service-forwarding ownership.

This ADR decides only the system boundary between credential acquisition and
runtime consumption. Custody transaction and preserve-old replacement semantics
are owned by ADR-0107.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Listener reads manual PEM paths | Couples every handshake to one acquisition representation and spreads secret/path ownership into runtime code. |
| Require ACME in the first slice | Expands the usable gateway into an unrelated durable acquisition workflow already owned by issue #57. |
| Reuse the operator HTTPS or internal SPIFFE certificate | Neither credential domain supplies the public Web-PKI origin identity external users require. |

## Consequences

- Manual and future ACME producers converge on one logical runtime input.
- TCP/80, challenge state and ACME order/renewal are absent from the first slice.
- Public certificate material remains separate from the operator HTTPS CA and
  internal gateway/workload SPIFFE CA.

## Links

- [ADR-0107](adr-0107-public-certified-key-custody-and-preserve-old-replacement.md)
- [Application custody contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Canonical C4](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [GitHub issue #57](https://github.com/overdrive-sh/overdrive/issues/57)
