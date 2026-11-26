# ADR-0360 — The node DNS responder also serves TCP/53, bound to the guest gateway

## Status

**Accepted** (GH #382, DESIGN decision GPD-5; approved by the user 2026-11-25).
Interface contract and numbers: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (responder lifecycle and load). Evidence: `docs/feature/guest-public-dns-resolution/spike/findings-design-validation.md` (V-5).

## Context

The responder today binds UDP only. ADR-0167 already admits guest TCP to `gateway:53`, but no socket listens there, so a guest whose UDP reply was truncated (TC) and which retries over TCP, as glibc and musl do, is refused and the lookup fails with the same symptom this feature removes. Large answers arise from many addresses, long CNAME chains and DNSSEC data.

## Decision

- A TCP listener binds `gateway:53` only, never the wildcard, so it cannot serve any other node address. The gateway is a local-table route that `run_server` converges before the responder is probed; that order is a contract, and no bind option is used to avoid it.
- Each connection carries one query, read under a deadline. It is classified and answered or forwarded exactly like a UDP one, a forwarded TCP query goes upstream over TCP, and the connection closes after the reply.
- Each accepted connection takes an in-flight permit at accept and holds it until the connection closes, so the connection and any upstream exchange run under one permit; with no permit the connection is closed unread ([ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md)). There is no other connection cap.
- The probe binds the listener; a bind failure refuses boot with the existing bind reason.
- The accept loop stops through the cancellation token of [ADR-0356](adr-0356-the-node-dns-responder-forwards-gateway-addressed-non-mesh-queries-to-the-host-resolvers.md).

## Alternatives considered

- **UDP only, truncation passed through.** Smallest, but oversize answers fail with a refused TCP connect, and the gap would need a tracking issue. Rejected for the complete service.
- **Retry the truncated answer upstream over TCP and answer the guest over UDP.** The guest cannot receive more than it advertised, so it must still be told to retry over TCP. Rejected.
- **Bind the TCP listener to the wildcard.** Makes the node a TCP open resolver on every address. Rejected.
- **Bind with a non-local-address option so the bind does not depend on the route.** Not needed: the route converges first, and the option would hide a boot-order regression instead of refusing it. Rejected.
- **Several queries per connection.** Adds idle-connection state and a per-connection pipeline the stub resolvers in the guest do not need; each of their TCP retries opens its own connection. Rejected.

## Consequences

- Positive: a complete DNS service for the guest; no reliance on any guest resolver's truncation behaviour.
- Negative: a second socket in the probe and shutdown; framing and a read deadline the UDP path never had; a client that pipelines several queries on one connection gets one answer and a close; because the permit is taken at accept, before the query class is known, a TCP connection of any class (a mesh name included) is closed when every permit is held. Mesh answers over UDP are unaffected.
