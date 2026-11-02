# ADR-0247 — The gateway exchange is full-duplex and ends an early response with a lingering close

## Status

Proposed — pending independent review (2026-11-02). User rulings 2026-11-02:
full-duplex exchange (P-13b, R-10); after an early or gateway-authored response,
`close_write` then bounded `discard_remaining` (P-18, R-14).

## Context

An origin may answer before it has read the request body (401, 413,
`Expect: 100-continue`), and a streaming echo needs both directions at once. A
half-duplex relay (request fully sent, then response read) deadlocks on full
buffers in those cases. When the gateway has delivered a final response while the
client is still sending, closing the socket with unread received data makes the
kernel send a reset that can destroy the response the client has not yet read.

## Decision

The exchange is full-duplex: the response half starts once the upstream request
head is written and may finish before the request body does.

After the gateway has delivered a complete final response while request octets
remain unread (an early origin response, or a gateway-authored 400, 404, 413, 421,
431, 502, 503 or 505; not 408), it sends `close_write` (close_notify, FIN) and
then reads and drops the remaining request octets until the peer's close_notify,
until `max_request_body_bytes`, or until the existing no-progress, whole-request
or drain deadline, and closes the socket on every outcome. The discard adds no
new limit.

The relay port operations and signatures are in the feature delta; body
framing is [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
and the single request per upstream leg is
[ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Half-duplex exchange (relay the whole request, then the response) | An origin that answers before reading the body (401, 413, `Expect: 100-continue`) deadlocks or is read only after the request is fully sent, and bidirectional streaming cannot work. It would save two pipe descriptors per exchange. Rejected by ruling P-13b (R-10). |
| `close_write` only after an early response, accepting a reset | No discard cost, but the client may see a reset instead of the 413 or 401. A short fixed discard bound would add a constant and still reset a large rejected upload. Rejected by ruling P-18 (R-14). |

## Consequences

- An origin that answers early, and a streaming echo, cannot deadlock on full
  buffers.
- Per early-answered or rejected connection, up to `max_request_body_bytes`
  (16 MiB) of client data is read and dropped at kernel speed, and the
  connection slot and two descriptors are held until the peer closes or a
  deadline fires; a client can already hold a slot as long with a slow upload.
- The relay port has one operation beyond the body moves: `discard_remaining`.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md), [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
