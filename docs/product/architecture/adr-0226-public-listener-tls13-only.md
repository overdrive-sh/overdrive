# ADR-0226 — Require TLS 1.3 on the public listener

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

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
  independently reversible decisions in ADR-0235–ADR-0240.
- Internal gateway-SVID workload mTLS remains a separate trust domain.

## Links

- [Public TLS contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 public listener](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0235](adr-0235-public-http11-only.md), [ADR-0236](adr-0236-canonical-public-route-match-key.md), [ADR-0237](adr-0237-public-proxy-header-policy.md), [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md)
- [ADR-0220](adr-0220-embed-single-node-public-ingress-in-overdrive-serve.md)
- [ADR-0230](adr-0230-gate-public-listener-without-redefining-workload-service-readiness.md)
- [GitHub #54](https://github.com/overdrive-sh/overdrive/issues/54)
