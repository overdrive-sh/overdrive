# ADR-0360 — The node DNS responder also serves TCP/53, bound to the guest gateway

## Status

**Accepted** (GH #382, DESIGN decision GPD-5; approved by the user 2026-11-25). Amended 2026-11-28 (a connection serves queries until the client closes; approved by the user).
Interface contract and numbers: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (responder lifecycle and load). Evidence: `docs/feature/guest-public-dns-resolution/spike/findings-design-validation.md` (V-5).

## Amendment 2026-11-28 — a connection serves queries until the client closes

**Approved by the user 2026-11-28 (decisions D1 to D5 of the feature delta's amendment).** Evidence: `docs/feature/guest-public-dns-resolution/spike/findings-tcp-one-query-per-connection.md` (metal probe, kernel 7.0.0-38, glibc 2.43).

The decision below was implemented and shipped with one query per connection. glibc, when it uses TCP and asks for both address families, writes the A and the AAAA query back to back on one connection and expects two replies. The responder answered the first and closed, so `getaddrinfo` failed outright (`getent ahosts` exit 2). That covers the truncation retry as well as `use-vc`, for any glibc guest with a global IPv6 address or a caller that does not suppress AAAA. musl opens one connection per query and was never affected. The amendment reverses the rejected alternative "several queries per connection" (below) and changes only the second bullet of the Decision and the consequence that depended on it; everything else stands.

- **D1.** The listener serves the queries of one connection one after another until the client closes it (RFC 7766 section 6.2.1). Each query is classified and answered or forwarded exactly as before.
- **D2.** The connection keeps the one permit it took at accept for its whole life ([ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md) is unchanged). The existing 5 s read deadline bounds the wait for each next query; there is no new idle timeout. A frame that cannot be served closes the connection without a reply to it, and replies already written stand; which cases close silently and which report `Dropped` is in the feature delta's behaviour table (§ *Amendment 2026-11-28 — TCP serves pipelined queries*).
- **D3.** Queries that arrive pipelined are read and answered in arrival order, replies in the same order, with one upstream exchange at a time on the connection.
- **D4.** No cap on the queries per connection. The permit bound and the read deadline are the bounds.
- **D5.** The upstream-side TCP exchange is unchanged: one query per upstream connection.

## Context

The responder today binds UDP only. ADR-0167 already admits guest TCP to `gateway:53`, but no socket listens there, so a guest whose UDP reply was truncated (TC) and which retries over TCP, as glibc and musl do, is refused and the lookup fails with the same symptom this feature removes. Large answers arise from many addresses, long CNAME chains and DNSSEC data.

## Decision

- A TCP listener binds `gateway:53` only, never the wildcard, so it cannot serve any other node address. The gateway is a local-table route that `run_server` converges before the responder is probed; that order is a contract, and no bind option is used to avoid it.
- ~~Each connection carries one query, read under a deadline. It is classified and answered or forwarded exactly like a UDP one, a forwarded TCP query goes upstream over TCP, and the connection closes after the reply.~~ **Superseded 2026-11-28 (D1 to D3, see the amendment above):** each query is read under a deadline, classified and answered or forwarded exactly like a UDP one, and a forwarded TCP query goes upstream over TCP (one query per upstream connection, D5); the connection serves its queries in order until the client closes it.
- Each accepted connection takes an in-flight permit at accept and holds it until the connection closes, so the connection and any upstream exchange (one at a time) run under one permit; with no permit the connection is closed unread ([ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md)). There is no other connection cap.
- The probe binds the listener; a bind failure refuses boot with the existing bind reason.
- The accept loop stops through the cancellation token of [ADR-0356](adr-0356-the-node-dns-responder-forwards-gateway-addressed-non-mesh-queries-to-the-host-resolvers.md).

## Alternatives considered

- **UDP only, truncation passed through.** Smallest, but oversize answers fail with a refused TCP connect, and the gap would need a tracking issue. Rejected for the complete service.
- **Retry the truncated answer upstream over TCP and answer the guest over UDP.** The guest cannot receive more than it advertised, so it must still be told to retry over TCP. Rejected.
- **Bind the TCP listener to the wildcard.** Makes the node a TCP open resolver on every address. Rejected.
- **Bind with a non-local-address option so the bind does not depend on the route.** Not needed: the route converges first, and the option would hide a boot-order regression instead of refusing it. Rejected.
- **Several queries per connection.** First rejected (2026-11-25) on the claim that the guest stub resolvers need no pipeline and open one connection per TCP retry. **Reversed 2026-11-28**: the claim is false for glibc (see the amendment preamble and its spike findings). Adopted as D1 to D5. The cost it was rejected for, idle-connection state, is bounded by the existing permit and the existing read deadline, so no new bound is added.
- **Keep one query per connection and answer the first only (status quo).** Fails glibc dual-family TCP lookups outright. Rejected on the evidence.
- **Answer pipelined queries concurrently and reorder replies.** Needs per-connection concurrency and a write ordering the stubs do not need, and holds more than one upstream exchange under one permit, which breaks the permit's descriptor accounting (two descriptors per permit). Sequential in-order answering (D3) is correct for both glibc and musl. Rejected.
- **A new idle timeout or a per-connection query cap.** A number with no measured reason; the 5 s read deadline already ends an idle connection and the permit already bounds the connections. Rejected (D2, D4).

## Consequences

- Positive: a complete DNS service for the guest; no reliance on any guest resolver's truncation behaviour; glibc and musl both resolve over TCP, including the truncation fallback.
- Negative: a second socket in the probe and shutdown; framing and a read deadline the UDP path never had; a connection holds its permit for as long as the client keeps sending queries within the read deadline of each other (no cap, D4; one upstream exchange, two descriptors, at a time), and per-guest fairness stays [#385](https://github.com/overdrive-sh/overdrive/issues/385); because the permit is taken at accept, before the query class is known, a TCP connection of any class (a mesh name included) is closed when every permit is held. Mesh answers over UDP are unaffected.
