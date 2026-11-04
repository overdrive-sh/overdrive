# ADR-0235 — Serve HTTP/1.1 only on public ingress

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13; the framing authority is set
by user ruling G1 (2026-11-02).

## Context

The first Public Route model needs ordinary request/response proxying without
HTTP/2 multiplexing, GOAWAY or per-stream drain semantics. Allowing an implicit
protocol set would make connection lifecycle and framing dependency-sensitive.

## Decision

Negotiate and serve HTTP/1.1 only on the public TLS connection (ALPN `http/1.1`).
Do not enable HTTP/2 or HTTP/3 in this first slice. Message framing is the
gateway's own ([ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Enable HTTP/1.1 and HTTP/2 | Multiplexing and GOAWAY add lifecycle semantics absent from the Route/application model, and a multiplexed stream cannot be spliced as one message. |
| Use an HTTP/2-only gateway | It excludes ordinary HTTP/1.1 clients without a product requirement. |
| Forward raw TCP after TLS | It cannot apply the selected HTTP Route and proxy policy. |

## Consequences

- One connection uses HTTP/1.1 request/response framing.
- The gateway is the sole framing authority on both legs.
- Route selection, headers, limits, streaming and upstream-attempt policy are
  independently decided in ADR-0236–ADR-0240.
- HTTP/2 and gRPC on the public listener are deferred to
  [#341](https://github.com/overdrive-sh/overdrive/issues/341); until then a
  client that offers only `h2` is refused at the handshake.

## Links

- [HTTP protocol contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 public runtime](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0226](adr-0226-public-listener-tls13-only.md), [ADR-0236](adr-0236-canonical-public-route-match-key.md), [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)
- [GitHub #54](https://github.com/overdrive-sh/overdrive/issues/54)
- [GitHub #341](https://github.com/overdrive-sh/overdrive/issues/341)
