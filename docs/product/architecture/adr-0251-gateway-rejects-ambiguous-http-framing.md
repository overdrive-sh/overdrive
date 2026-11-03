# ADR-0251 — The gateway rejects ambiguous HTTP message framing

## Status

Proposed — pending independent review. User ruling P-13a (R-9, 2026-11-02).

## Context

[ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md) makes
the gateway the sole author of each message's end. A message whose length is
stated twice, repeated, overflowing or by an unsupported coding is read
differently by different parsers: it is the request-smuggling and
desynchronisation defect itself (RFC 9112 §6.3 and §11.2).

## Decision

Ambiguous framing is rejected, never forwarded and never repaired. The rejection
set is the strict set in the feature delta "Message-framing contract" (both
`Content-Length` and `Transfer-Encoding`, repeated or conflicting or overflowing
length, an unsupported transfer coding).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Forward ambiguous framing | It is the request-smuggling defect itself. Rejected by ruling P-13a. |
| Repair ambiguous framing (for example drop `Content-Length` when `Transfer-Encoding` is present) | Makes the gateway's reading differ from the origin's on the same bytes. Rejected with P-13a. |

## Consequences

- The strict rejection set costs parser complexity; it is the accepted price of
  the smuggling defence.
- A client or origin that sends ambiguous framing receives a gateway-authored
  rejection.

## Links

- [Message-framing contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
