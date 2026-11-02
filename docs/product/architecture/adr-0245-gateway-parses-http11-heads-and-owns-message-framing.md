# ADR-0245 — The gateway parses HTTP/1.1 message heads and owns message framing

## Status

Proposed — set by user ruling G1 (2026-11-02); pending independent review. The
strict ambiguous-framing rejection set (feature delta P-13a) is awaiting the user's
ruling.

## Context

Under user ruling G1 request and response bodies move between the two kernel-TLS
sockets without entering gateway userspace
([ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md)). A
userspace HTTP stack that owns the byte stream cannot sit in that path. The
gateway still has to read every request head (Route match, Host equals SNI,
origin-form, header policy, limits) and every response head (header policy,
status mapping), and it now decides where each message ends. A wrong decision is
a request-smuggling or desynchronisation defect on a public edge (RFC 9112 §6.3
and §11.2).

## Decision

Parse only request heads, response heads, chunk-size lines and trailer sections
in the gateway, after a byte-level validator that rejects bare CR, bare LF, NUL
and field values outside RFC 9110 §5.5. Never parse or inspect body octets.

Derive each message's body length in the gateway by RFC 9112 §6.3 and reject
ambiguous framing instead of forwarding or repairing it. The rejection set is in
the feature delta "Message-framing contract". Chunk extensions and trailers are
consumed and not forwarded.

Re-serialise every forwarded head from the parsed, policy-rewritten form
([ADR-0237](adr-0237-public-proxy-header-policy.md)); the gateway alone emits
`Content-Length`, `Transfer-Encoding` and `Connection`.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Keep `hyper` as the HTTP engine over a userspace stream | It consumes and re-emits bodies in userspace, which G1 removed. |
| Hand-write the head parser | A new parser on the public edge for no gain over an established one. |
| Forward heads verbatim after minimal inspection | Header policy, Host/SNI equality and the `Forwarded`/`Via` rewrite cannot be enforced on bytes the gateway has not parsed. |

## Consequences

- The framing decision and the head validator are pure functions and the
  security-critical surface of the public edge.
- Interim `1xx` responses and responses to `HEAD`, `204`, `205` and `304` need
  explicit rules (feature delta, message-framing contract).
- The strict rejection set is awaiting the user's ruling (feature delta P-13a).

## Links

- [Message-framing contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0235](adr-0235-public-http11-only.md), [ADR-0237](adr-0237-public-proxy-header-policy.md), [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0239](adr-0239-stream-public-proxy-bodies-with-backpressure.md)
