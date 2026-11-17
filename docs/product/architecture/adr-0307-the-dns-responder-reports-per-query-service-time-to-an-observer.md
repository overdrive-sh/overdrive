# ADR-0307 — The DNS responder reports per-query service time to an observer

## Status

**Accepted.** Approved by the user 2026-11-17, including this production change; independent design review resolved 2026-11-17. DDD-12 in the feature delta.

## Context

O3 and the DNS requirement: resolver-internal timing separate from client-observed
latency. Production has no per-query timing: the only tracing event is a send
failure, there are no counters, and the injected `Clock` is used once, for the SOA
serial. The pure answer functions can be timed alone, but that leaves out the
receive and send system calls, which are most of a UDP query's service time.

## Decision

The responder takes a required observer, `QueryObserver`, and calls it once per
datagram it handles with the outcome (answered, no data, name error, dropped) and
the service time measured on the injected `Clock` from the instant the receive
returned to the instant the send (or the drop decision) completed. Production
composition passes `NoQueryObserver`, whose call is empty. The budget on the hot
path is two `Clock::now()` reads and one dynamic call per datagram, no allocation
and no lock. The benchmark passes its own recorder. Spike V-2 measured the
instrumentation at 62.7 ns (`NoQueryObserver`) and 75.2 ns (recording sink) per
datagram in a 10-million-iteration microbenchmark on the Metal host; the end-to-end
comparison at 5,000 queries per second was not distinguishable from noise (its A/A
floor of about 700 ns only rules out costs above that), so the microbenchmark is
the cited bound.

## Alternatives

- **Time the pure answer functions in isolation, no production change**: honest
  but covers only decode, lookup and encode, not the query path; rejected.
- **Kernel receive timestamps (`SO_TIMESTAMPNS`)**: needs a second clock domain
  and cmsg parsing in the loop for the same number; rejected.
- **A tracing span per query**: a subscriber lookup and field formatting on every
  datagram; rejected on cost.

## Consequences

This is a production change whose only consumer today is the benchmark. The
`DnsResponder::new` constructor gains one parameter; every construction site
passes `NoQueryObserver` or a recorder. The observer enters through that
constructor only: `GuestDnsDeps` and the factory trait are unchanged, and
`HostGuestDnsFactory` passes `NoQueryObserver`. The profile axes of the family are those that
exist in the responder: query shape, name-table cardinality, client count and
arrival rate. Cache hit or miss, non-UDP transport and response size are not axes
because the responder has none of them; that is a property of the mechanism.

## Links

- [Decision DDD-12 and the interface](../../feature/native-benchmark-program/feature-delta.md#ddd-12--dns-per-query-observer)
