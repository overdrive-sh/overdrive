# ADR-0117 — Use producer-neutral Public Certified-Key Custody

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Public TLS consumes a Web-PKI certificate chain and matching origin private
key. Manual files are the first-slice acquisition source, while GitHub #57 may
later produce the same logical credential through a Workflow. Listener and
request code must not become acquisition-specific secret owners.

## Decision

Use one `PublicCertifiedKeyCustody` owner as the producer-neutral boundary for
protected public-certified-key generations. Manual input is its only
first-slice producer; a later Workflow producer must use the same boundary.
Filesystem paths and plaintext key material stop before runtime consumers,
which receive only an opaque current resolver snapshot.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Let the listener read PEM paths directly | It couples every connection to manual acquisition and spreads path/key handling into runtime code. |
| Give manual and Workflow producers separate managers | Two credential authorities could disagree about the current generation and force listener redesign. |
| Reuse the operator HTTPS CA or internal SPIFFE CA | Neither credential domain represents a public Web-PKI origin identity. |

## Consequences

- Public credential acquisition and runtime consumption remain independently
  evolvable.
- Paths and plaintext private-key material do not enter Route, status or
  request state.
- Replacement ordering is an independent decision recorded in ADR-0132.

## Links

- [Exact custody contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Canonical C4 custody boundary](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Replacement transaction ADR-0132](adr-0132-preserve-last-usable-public-certified-key-replacement.md)
- [Domain custody ADR-0122](adr-0122-public-certified-key-custody-domain-boundary.md)
- [System acquisition boundary ADR-0127](adr-0127-separate-public-certificate-acquisition-from-runtime-consumption.md)
- [GitHub #57](https://github.com/overdrive-sh/overdrive/issues/57)
