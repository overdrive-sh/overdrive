# ADR-0141 — Stream public proxy bodies with backpressure

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Request and response bodies may approach the finite body ceilings. Buffering a
whole body before forwarding multiplies per-request memory and delays the first
upstream/downstream bytes.

## Decision

Stream request and response body octets between Hyper legs with bounded
backpressure. Do not buffer whole bodies, spool them to disk or expose body
chunks outside the connection/request owner. Preserve response status and raw
body octets.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Buffer each complete body in memory | Memory grows with concurrent body ceilings and forwarding starts late. |
| Spool bodies to disk | It adds filesystem ownership, cleanup and secret-at-rest concerns. |
| Aggregate into large application chunks | It weakens backpressure and creates another buffering policy. |

## Consequences

- Slow clients and workloads naturally constrain their opposite proxy leg.
- Finite no-progress and whole-request deadlines still bound stalled streams.
- Retry/replay remains independently governed by ADR-0142.

## Links

- [Exact streaming contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 HTTP stream](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [Runtime limits ADR-0140](adr-0140-fixed-finite-public-runtime-limits.md)
- [Single-attempt ADR-0142](adr-0142-single-upstream-attempt-per-public-request.md)
