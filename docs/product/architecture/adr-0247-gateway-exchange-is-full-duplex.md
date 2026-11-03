# ADR-0247 — The gateway exchange is full-duplex

## Status

Proposed — pending independent review. User ruling P-13b (R-10, 2026-11-02). The
lingering close after an early response is a separate decision
([ADR-0254](adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md),
R-14).

## Context

An origin may answer before it has read the request body (401, 413,
`Expect: 100-continue`), and a streaming echo needs both directions at once. A
half-duplex relay (request fully sent, then response read) deadlocks on full
buffers in those cases.

## Decision

The exchange is full-duplex: the response half starts once the upstream request
head is written and may finish before the request body does.

The relay port operations and signatures are in the feature delta; body
framing is [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
and the single request per upstream leg is
[ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Half-duplex exchange (relay the whole request, then the response) | An origin that answers before reading the body (401, 413, `Expect: 100-continue`) deadlocks or is read only after the request is fully sent, and bidirectional streaming cannot work. It would save two pipe descriptors per exchange. Rejected by ruling P-13b (R-10). |

## Consequences

- An origin that answers early, and a streaming echo, cannot deadlock on full
  buffers.
- An active exchange holds two pipes per direction pair (up to four pipe
  descriptors), counted in the descriptor budget.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md), [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [ADR-0254](adr-0254-gateway-ends-an-early-response-with-a-lingering-close.md)
