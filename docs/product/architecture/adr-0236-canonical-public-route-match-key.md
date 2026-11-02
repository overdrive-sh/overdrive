# ADR-0236 — Use one canonical Public Route match key

## Status

Proposed — reconstructed on main 549d78fa6, pending independent review (2026-11-01).
Original decision ratified by the user 2026-09-13.

## Context

One public listener may receive requests for the singleton Route's hostname
and path. Percent decoding or ordinary string-prefix matching would make
distinct request targets collide, while accepting Host different from TLS SNI
would route under an authority the TLS connection did not authenticate.

## Decision

Select the Public Route only when canonical Host authority equals the exact TLS
SNI and its original, non-decoded request path satisfies the Route's Exact or
segment-boundary prefix match. Ignore the query for selection while preserving
it upstream. Accept only origin-form requests; reject CONNECT plus authority-,
absolute- and asterisk-form targets before upstream connect. A Route miss
remains 404.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Route by SNI alone | It cannot express the approved path dimension. |
| Route by Host without SNI equality | It permits authority confusion on an already-authenticated TLS connection. |
| Percent-decode then compare ordinary string prefixes | Encoded separators and `/api` versus `/apix` can change Route meaning. |

## Consequences

- Host/path selection is deterministic and independent of backend state.
- Query bytes survive forwarding but do not affect Route identity.
- Header transformation remains independently governed by ADR-0237.

## Links

- [Route matching contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Route aggregate contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 request path](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [ADR-0221](adr-0221-singleton-public-route-set-aggregate.md), [ADR-0235](adr-0235-public-http11-only.md)
