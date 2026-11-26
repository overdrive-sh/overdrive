# ADR-0356 — The node DNS responder forwards gateway-addressed non-mesh queries to the host's resolvers, one verbatim exchange per query

## Status

**Accepted** (GH #382, DESIGN decision GPD-1; approved by the user 2026-11-25). The bound on forwards in flight is its own decision: [ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md).
Interface contract and numbers: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts*. Evidence: `docs/feature/guest-public-dns-resolution/spike/findings-design-validation.md` (V-2).

## Context

A VM guest has one nameserver, the guest gateway (ADR-0154). The responder at that address answers mesh names and silently drops everything else, so a guest's public lookup waits out its 5 s resolver timeout (metal probe, `EAI_AGAIN`). The user ruled (D-1) that the existing responder forwards the non-mesh arm and that there is no second resolver.

Forces. The responder's serve loop is a blocking receive per socket, so waiting on an upstream there would stall mesh answers. It binds the wildcard address, so it receives datagrams addressed to any node address and must not become an open resolver. Every guest query arrives with the gateway as source, so guests cannot be told apart. A guest can already reach any external resolver over ADR-0162 egress, so forwarding grants no new ability to reach or flood an upstream.
## Decision

- A query addressed to the gateway whose name is forwardable (ADR-0357) is forwarded; a query addressed to any other node address is never forwarded and keeps today's behaviour.
- Each forward is one independent exchange on a fresh socket with the guest's query bytes unchanged, run off the receive thread. The reply goes back through the existing source-pinned send (ADR-0116).
- Upstreams are tried in order inside a fixed per-query time budget. An upstream answer of any response code is final and relayed verbatim.
- When no upstream answers inside the budget, or none is usable, the guest receives SERVFAIL, a definite reply inside its 5 s timeout.
- There is no cache of forwarded answers. The upstream resolver is the cache; D-5's question is vacuous.
- Each forward runs under an in-flight permit; how many permits exist and what happens to a query that cannot get one is [ADR-0362](adr-0362-dns-forwards-in-flight-are-bounded-by-the-descriptor-limit-and-excess-is-shed-at-once.md).
- Forward tasks and the TCP accept loop stop through one cooperative cancellation token.

## Alternatives considered

- **Forward on the receive thread.** Stalls mesh answers during an upstream outage (AC-11). Rejected.
- **A forwarded-answer cache.** No reachable failure needs it: the first lookup of any name is cold either way, the upstream already caches, and a guest can query upstreams directly at any rate. A cache adds shared state, TTL and negative-caching rules and, with DNSSEC, rewriting hazards. Rejected until a measured failure names one.
- **One shared multiplexed upstream socket with an id table.** Needs id rewriting and an in-flight table, and gives up the per-query random source port. Rejected on correctness, not cost.
- **Forward every query regardless of the address it was sent to.** Makes the node an open resolver on any reachable address. Rejected.
- **A resolver library (`hickory-resolver`) as the forwarder.** Re-synthesises answers, losing verbatim rcodes and DNSSEC bits, and adds its own cache. Rejected.

## Consequences

- Positive: guests resolve public names with default configuration; mesh naming over UDP is isolated from upstream state (a TCP connection is closed at saturation whatever its class, ADR-0360); the forwarder holds no state across queries.
- Negative: one extra upstream round trip per lookup; under a total outage a glibc guest (which retries SERVFAIL on a single nameserver) takes about two budgets before its own failure; the time budgets are internal bounds with no operator setting, which is not a limitation.
- `QueryOutcome` gains forwarded, forward-failed, owned-range-blocked and shed variants so the outcomes appear beside mesh outcomes. They reach test and benchmark observers; the production observer records none of them, so operator visibility today is the forward-failure episode warning and the shed episode warning (ADR-0362), and a CLI-readable condition waits on [#386](https://github.com/overdrive-sh/overdrive/issues/386).
- The gateway-address gate rests on the datagram's `ipi_spec_dst` equalling the destination of a locally delivered datagram; the kernel citations and the measurement are in the feature-delta (Grounding findings, 4) and the findings file above. ADR-0116 binding is unchanged.
