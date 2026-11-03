# ADR-0254 — The gateway ends an early or gateway-authored response with a lingering close

## Status

Proposed — pending independent review. User ruling P-18 (R-14, 2026-11-02).

## Context

When the gateway has delivered a final response while the client is still sending,
closing the socket with unread received data makes the kernel send a reset that can
destroy the response the client has not yet read. This arises after an early origin
response and after a gateway-authored rejection, the cases the full-duplex exchange
([ADR-0247](adr-0247-gateway-exchange-is-full-duplex.md))
admits.

## Decision

After the gateway has delivered a complete final response while request octets
remain unread (an early origin response, or a gateway-authored 400, 404, 413, 421,
431, 502, 503 or 505; not 408), it sends `close_write` (close_notify, FIN) and
then reads and drops the remaining request octets until the peer's close_notify,
until `max_request_body_bytes`, or until the existing no-progress, whole-request
or drain deadline, and closes the socket on every outcome. The discard adds no
new limit.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| `close_write` only after an early response, accepting a reset | No discard cost, but the client may see a reset instead of the 413 or 401. A short fixed discard bound would add a constant and still reset a large rejected upload. Rejected by ruling P-18 (R-14). |

## Consequences

- Per early-answered or rejected connection, up to `max_request_body_bytes`
  (16 MiB) of client data is read and dropped at kernel speed, and the
  connection slot and two descriptors are held until the peer closes or a
  deadline fires; a client can already hold a slot as long with a slow upload.
- The relay port gains one bounded read-and-drop operation beyond the body moves; its exact signature is pinned in the feature-delta kernel relay contract.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0247](adr-0247-gateway-exchange-is-full-duplex.md), [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md)
