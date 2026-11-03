# ADR-0253 — The gateway supports close-delimited responses and HTTP/1.0 responses with an explicit length

## Status

Proposed — pending independent review. User ruling P-13d (R-12, 2026-11-02).

## Context

A legitimate HTTP/1.1 origin may delimit a response body by closing the
connection. [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
makes the gateway the owner of each message's end, so it must say how a response
without length ends.

## Decision

A response without length is close-delimited: the origin's close_notify ends the
message cleanly (any other end of stream is a truncated response), and the gateway
forwards it to the client with `Connection: close`. An `HTTP/1.0` response is
accepted only with an explicit length.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Reject a response without length (502) | A legitimate HTTP/1.1 origin that closes to delimit its body would get 502. Rejected by ruling P-13d (R-12). |
| Accept an `HTTP/1.0` response without length | An `HTTP/1.0` close-delimited body cannot be told from a truncated one in a way the gateway can report to the client; the explicit length is the only checkable form. |

## Consequences

- A close-delimited response ends the client connection after the response.
- An origin that ends the stream without close_notify produces a truncated-response
  outcome, not a clean end.

## Links

- [Message-framing contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md)
