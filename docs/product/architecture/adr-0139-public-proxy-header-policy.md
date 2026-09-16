# ADR-0139 — Apply one explicit public proxy-header policy

## Status

Accepted. 2026-09-13. User explicitly approved the complete `public-ingress-gateway` DESIGN.

## Context

Forwarding hop-by-hop fields or trusting client-supplied forwarding provenance
can create request smuggling and false client attribution. The gateway must
state which connection-specific fields are removed and which provenance it
authors.

## Decision

Remove `Connection`-nominated and fixed hop-by-hop fields on both directions.
On requests, discard incoming `Forwarded` and `X-Forwarded-*`, retain the
validated public Host, author one `Forwarded` value from the accepted IPv4
client, and append `Via: 1.1 overdrive`. On responses, append the same Via
product token after hop-by-hop removal. Consume rather than forward trailers.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Forward all received headers unchanged | Hop-by-hop fields can corrupt framing across proxy legs. |
| Extend client-supplied `Forwarded`/`X-Forwarded-*` chains | The first-slice gateway has no trusted-proxy allowlist for those claims. |
| Emit only `X-Forwarded-*` | It introduces several non-standard provenance fields where one standardized `Forwarded` value suffices. |

## Consequences

- Hyper regenerates legal framing after transformation.
- The gateway is the sole authority for first-slice forwarding provenance.
- Body streaming and resource policy remain separate decisions.

## Links

- [Exact header transformation contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Canonical C4 HTTP runtime](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [HTTP/1.1 ADR-0137](adr-0137-public-http11-only.md)
- [Streaming ADR-0141](adr-0141-stream-public-proxy-bodies-with-backpressure.md)
