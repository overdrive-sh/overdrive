# ADR-0110 — Require TLS 1.3 on the public listener

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Real application users need ordinary Web-PKI TLS at the public gateway. The
first slice has no compatibility requirement for older TLS versions, and a
smaller protocol set reduces unauthenticated negotiation surface.

## Decision

Configure rustls on the public TCP/443 listener to accept TLS 1.3 only, using
the current opaque Public Certified Key supplied by custody. Public clients
authenticate that Web-PKI identity and never use SPIFFE.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Allow TLS 1.2 and TLS 1.3 | The first slice has no legacy-client requirement that justifies the additional protocol surface. |
| Use a library-default version set | It would make supported public protocol versions implicit and dependency-version-sensitive. |
| Terminate TLS in another process | It adds a deployment and secret-consumption boundary outside the selected embedded gateway. |

## Consequences

- TLS 1.2-only clients cannot connect.
- HTTP version, routing, headers, limits, streaming and retry policy are
  independently reversible decisions in ADR-0127–ADR-0132.
- Internal gateway-SVID workload mTLS remains a separate trust domain.

## Links

- [Exact public TLS contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 public listener](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [HTTP/1.1 ADR-0127](adr-0127-public-http11-only.md)
- [Route match ADR-0128](adr-0128-canonical-public-route-match-key.md)
- [Proxy-header ADR-0129](adr-0129-public-proxy-header-policy.md)
- [Runtime-limit ADR-0130](adr-0130-fixed-finite-public-runtime-limits.md)
- [Streaming ADR-0131](adr-0131-stream-public-proxy-bodies-with-backpressure.md)
- [Single-attempt ADR-0132](adr-0132-single-upstream-attempt-per-public-request.md)
- [System placement ADR-0104](adr-0104-embed-single-node-public-ingress-in-overdrive-serve.md)
- [System listener-gate ADR-0121](adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md)
- [GitHub #54](https://github.com/overdrive-sh/overdrive/issues/54)
