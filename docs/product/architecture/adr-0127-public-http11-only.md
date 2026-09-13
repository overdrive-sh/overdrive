# ADR-0127 — Serve HTTP/1.1 only on public ingress

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

The first Public Route model needs ordinary request/response proxying without
HTTP/2 multiplexing, GOAWAY or per-stream drain semantics. Allowing an implicit
protocol set would make connection lifecycle and framing dependency-sensitive.

## Decision

Negotiate and serve Hyper HTTP/1.1 only on the public TLS connection. Do not
enable HTTP/2 or HTTP/3 in this first slice.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Enable HTTP/1.1 and HTTP/2 | Multiplexing and GOAWAY add lifecycle semantics absent from the Route/application model. |
| Use an HTTP/2-only gateway | It excludes ordinary HTTP/1.1 clients without a product requirement. |
| Forward raw TCP after TLS | It cannot apply the selected HTTP Route and proxy policy. |

## Consequences

- One connection uses HTTP/1.1 request/response framing.
- Hyper remains the sole framing parser.
- Route selection, headers, limits, streaming and upstream-attempt policy are
  independently decided in ADR-0128–ADR-0132.

## Links

- [Exact HTTP protocol contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 public runtime](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [TLS 1.3 ADR-0110](adr-0110-public-listener-tls13-only.md)
- [Route selection ADR-0128](adr-0128-canonical-public-route-match-key.md)
- [GitHub #54](https://github.com/overdrive-sh/overdrive/issues/54)
