# ADR-0122 — Preserve the last usable Public Certified Key during replacement

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Public Certified-Key Custody may receive a manual replacement while an older
generation is serving traffic. Candidate validation, protection, persistence,
resolver construction and status publication can fail at different points. An
unusable candidate must not destroy the last usable generation.

## Decision

Make replacement a preserve-old transaction. Prove the complete candidate
usable and construct its resolver before changing durable or live current
state; then seal and persist the complete generation, atomically publish that
already-built generation as Current/Usable, and write redacted status last.
Any pre-publication failure retains the prior durable/live generation. Restart
adopts a complete durable generation and repairs status rather than reverting
to an older one.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Overwrite durable current before validation/resolver construction | A crash or candidate defect can replace a working generation with unusable state. |
| Clear current whenever refresh fails | A bad replacement would cause avoidable public outage despite a still-usable prior generation. |
| Publish status before live current | Status could promise a generation that new connections cannot consume. |

## Consequences

- Invalid, not-yet-valid, mismatched or unprotectable replacements preserve the
  prior generation while it remains time-usable.
- A crash after durable persistence is recovered by adopting that complete
  generation; a post-publication status failure is retried without rollback.
- Custody ownership itself remains ADR-0107.

## Links

- [Exact replacement and crash-order contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-persistence-custody-and-redaction)
- [Canonical C4 custody transaction](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Custody ownership ADR-0107](adr-0107-producer-neutral-public-certified-key-custody.md)
- [Domain custody ADR-0112](adr-0112-public-certified-key-custody-domain-boundary.md)
- [System acquisition boundary ADR-0117](adr-0117-separate-public-certificate-acquisition-from-runtime-consumption.md)
