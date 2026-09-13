# ADR-0110 — Serve public ingress as a bounded TLS 1.3 HTTP/1.1 proxy

## Status

Proposed — 2026-09-13.

## Context

Real application users need browser/tool-compatible public ingress while the
first slice must remain auditable and resource-bounded. TLS passthrough cannot
perform Host/path routing, and adding HTTP/2 introduces multiplexing, flow
control and drain semantics not required by the first Route model.

## Decision

Terminate operator-supplied public Web-PKI TLS 1.3 with rustls and serve strict
Hyper HTTP/1.1 on the `serve`-owned IPv4 TCP/443 listener. Match exact SNI,
Host and raw exact/segment-prefix path; sanitize proxy headers; stream request
and response bodies with backpressure; enforce one private, fixed first-slice
limit policy; and perform no retry, replay, hedge or userspace backend choice.

This is one decision about the public wire protocol and resource policy.
Operator lifecycle/status reporting is independently reversible and recorded
in ADR-0111.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| TLS passthrough | It cannot terminate the public certified key or perform the required Host/path L7 Route. |
| HTTP/2 in the first slice | It adds stream concurrency, flow-control and GOAWAY lifecycle without a user requirement. |
| External proxy process | It adds another deployment/secret/lifecycle boundary and breaks the selected in-process BPF access path. |

## Consequences

- Public clients use ordinary Web PKI and never SPIFFE.
- Finite connection, parsing, body and deadline ceilings bound unauthenticated
  resource exposure; the values are product policy rather than operator knobs.
- Route miss, no backend and internal upstream failure remain distinct public
  outcomes; post-header failure closes rather than fabricating a response.
- HTTP/2, WebSocket, gRPC, TLS passthrough and middleware remain absent from
  this decision.

## Links

- [Exact HTTP/TLS/limit contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-gateway-application-status-and-http-contract)
- [Public driving surface](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-exact-driving-and-driven-ports)
- [Canonical C4 public runtime](c4-diagrams.md#public-ingress-gateway-canonical-c4)
- [System placement ADR-0104](adr-0104-embed-single-node-public-ingress-in-overdrive-serve.md)
- [System listener-gate ADR-0121](adr-0121-gate-public-listener-without-redefining-workload-service-readiness.md)
- [GitHub #54](https://github.com/overdrive-sh/overdrive/issues/54)
