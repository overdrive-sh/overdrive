# ADR-0250 — The gateway never originates a TLS KeyUpdate

## Status

Proposed — pending independent review. User ruling P-17 (R-13, 2026-11-02).

## Context

RFC 8446 §5.5 bounds a traffic key at about 2^24.5 full-size records. The
[ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md) ceilings (100 responses
of at most 16 MiB per public connection; one request per upstream leg) stay about
two orders of magnitude below that bound for full-size records. A transmit
KeyUpdate needs a record counter, a lock-taking path and a leg-C behaviour on a
received KeyUpdate that no test exercised.

## Decision

The gateway never originates a KeyUpdate on either leg. Only a peer-initiated
KeyUpdate is handled, as [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md)
states.

## Alternatives considered

| Alternative | Why it lost |
|---|---|
| Originate a KeyUpdate on each transmit direction before 2^23 records | Needs a record counter, a lock-taking path and an untested leg-C behaviour on a received KeyUpdate, to defend only a workload that streams very many tiny events on one public connection (the relay passes no `SPLICE_F_MORE`, so each small source piece is one TX record). Rejected by ruling P-17. |
| Rely on the ADR-0238 ceilings alone and handle no peer-initiated KeyUpdate either | Removes the receive-side handling too. Rejected: a peer (leg-C, or a public client) may legitimately send a KeyUpdate at any time, and the kernel pauses the receive direction with `-EKEYEXPIRED` until the receive key is re-armed, so ignoring it would stall the exchange. Handling a peer-initiated KeyUpdate stays; only origination is dropped. |

## Consequences

- A workload streaming one-octet events on one connection could in principle
  exceed the key's record bound and is not defended.
- No gateway code path takes the transmit lock to re-key on its own initiative.

## Links

- [Kernel relay contract](../../feature/public-ingress-gateway/feature-delta.md#wave-design--ref-kernel-relay-data-path-and-driven-ports-g1)
- [ADR-0244](adr-0244-gateway-legs-use-kernel-tls-after-a-userspace-rustls-handshake.md), [ADR-0238](adr-0238-fixed-finite-public-runtime-limits.md)
