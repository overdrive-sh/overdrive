# ADR-0239 — Stream public proxy bodies with backpressure

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

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
- Retry/replay remains independently governed by ADR-0240.

## Links

- [Streaming contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 HTTP stream](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md)
