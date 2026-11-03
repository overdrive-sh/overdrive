# ADR-0252 — The gateway forwards an interim 100 and the Expect header and drops other interim 1xx

## Status

Proposed — pending independent review. User ruling P-13c (R-11, 2026-11-02).

## Context

An origin may send interim `1xx` responses before its final response. A client
that sent `Expect: 100-continue` waits for the `100` before it sends its body.
[ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md) makes
the gateway the owner of message framing, so it must say what happens to each
interim response.

## Decision

An origin `100` is forwarded to the client as a body-less rewritten head. Every
other interim `1xx` except `101` is read and dropped. `101` is a 502. `Expect` is
forwarded to the origin unchanged and the gateway decides nothing about it.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Reject all interim `1xx` (502) and strip `Expect` | `100-continue` clients would stall until their own timeout. Rejected by ruling P-13c. |
| Forward every interim `1xx` | The gateway would relay informational responses it has no policy for, and `101` would switch protocols under a gateway that speaks HTTP/1.1 only ([ADR-0235](adr-0235-public-http11-only.md)). |

## Consequences

- `100-continue` clients work through the gateway.
- Interim responses other than `100` are invisible to the client.

## Links

- [Message-framing contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [ADR-0235](adr-0235-public-http11-only.md), [ADR-0247](adr-0247-gateway-exchange-is-full-duplex.md)
