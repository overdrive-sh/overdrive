# ADR-0239 — Move public proxy bodies kernel-side between the two kernel-TLS sockets

## Status

Proposed — pending independent review (2026-11-02). Streamed bodies with
backpressure ratified by the user 2026-09-13; the body mechanism is set by user
ruling G1 (2026-11-02).

## Context

Request and response bodies may approach the finite body ceilings
([ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md)). Buffering a whole
body multiplies per-request memory and delays the first forwarded octet. The
accepted L7 feature set needs bodies streamed with backpressure, preserved
status and raw body octets. The user ruling G1 (2026-11-02) keeps body octets out
of gateway userspace: both legs are kernel-TLS sockets
([ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md))
and the gateway parses only heads and framing lines
([ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md)).

The spike
([findings (c)](../../feature/public-ingress-gateway/spike/findings-ktls-splice-relay.md))
found exact-length and per-chunk splicing, keep-alive and backpressure lossless with
bounded memory and no read past the body's end, and splicing not cheaper than a
userspace copy.

## Decision

Move request and response body octets between the two kernel-TLS sockets with
`splice(2)` through a private pipe, never into gateway userspace memory.
Splice exactly the octets the framing decision names: the declared length for a
`Content-Length` body, one chunk's declared size per chunk for a chunked body, and
to end of stream for a close-delimited response. The gateway writes every other
octet itself: heads, chunk-size lines, the line terminators and the final chunk.
A body is bounded by its ceiling; no body is buffered whole, spooled to disk or
exposed outside the exchange owner. Backpressure is the destination socket's
send buffer. Preserve the response status and the raw body octets.

The pipe contract is [ADR-0246](adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md).
How the two halves of an exchange run together and end is
[ADR-0247](adr-0247-gateway-exchange-is-full-duplex-and-ends-an-early-response-with-a-lingering-close.md).

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Userspace `hyper` body streaming | Every body octet passes through gateway memory; G1 moved it into the kernel. |
| Userspace read and write loop over the kernel-TLS sockets | No CPU gain over splice and the same bounded memory, but body octets pass through gateway memory. |
| Buffer each complete body | Memory grows with concurrent body ceilings and forwarding starts late. |
| Spool bodies to disk | Adds filesystem ownership, cleanup and secret-at-rest concerns. |

## Consequences

- The claim is bounded memory and no userspace payload copy. No CPU or throughput
  gain is claimed; splice calls per MiB scale with the peer's record size.
- The relay never passes `SPLICE_F_MORE`, so each splice-out ends its TLS record
  and no record is held open waiting for octets that are not coming; a body that
  trickles is sent as small records (feature delta, "Pipe and splice").
- Slow clients and workloads constrain the opposite leg; finite no-progress and
  whole-request deadlines bound stalled streams.
- Retry and replay remain governed by ADR-0240: a request body is never replayed.
- A control record inside a body (KeyUpdate, NewSessionTicket, close_notify) is
  handled by the leg and never forwarded.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md), [ADR-0240](adr-0240-single-upstream-attempt-per-public-request.md), [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), [ADR-0245](adr-0245-gateway-parses-http11-heads-and-owns-message-framing.md), [ADR-0246](adr-0246-relay-splices-into-an-empty-private-pipe-of-at-least-64-kib.md), [ADR-0247](adr-0247-gateway-exchange-is-full-duplex-and-ends-an-early-response-with-a-lingering-close.md)
