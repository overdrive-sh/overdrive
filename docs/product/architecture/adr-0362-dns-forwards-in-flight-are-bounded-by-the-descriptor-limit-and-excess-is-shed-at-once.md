# ADR-0362 — DNS forwards in flight are bounded by the descriptor limit; excess is shed at once

## Status

**Accepted** (GH #382, DESIGN decision GPD-8; approved by the user 2026-11-25 as ruling A on GPD-1, recorded as its own decision, split from ADR-0356 in the re-review of the same day).
The descriptor limit itself, its raise and its injection are [ADR-0363](adr-0363-overdrive-serve-raises-its-soft-descriptor-limit-once-at-the-binary-entry-and-injects-the-read-back-value.md).
Interface contract and numbers: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (permits and shed).

## Context

Every in-flight DNS forward holds process descriptors: a UDP forward holds its upstream socket; a TCP forward holds the connection accepted from the guest and its upstream socket. The number of forwards in flight therefore follows the query rate and the upstream round trip, and in a total outage it is the query rate times the forward budget. What stops it is `serve`'s soft descriptor limit, at which allocation fails with `EMFILE` for every other consumer in the process, including guest-flow admission. Every guest query reaches the node with the gateway as source, so the load cannot be attributed to a guest (limitation L-2, [#385](https://github.com/overdrive-sh/overdrive/issues/385)).

## Decision

- There is no fixed in-flight count. The bound is derived from the soft descriptor limit (ADR-0363) minus a reserve for everything else `serve` holds, divided by the descriptors one forward can hold.
- A forward holds one permit for its whole life. A UDP forward takes it when the query is found forwardable. A TCP connection takes it when it is accepted, so the guest's connection and the upstream exchange run under one permit.
- A query or connection that cannot get a permit is shed at once, without waiting. A shed UDP query is answered SERVFAIL; a shed TCP connection is closed unread. A shed query opens no upstream socket and reads no upstream configuration. Mesh answers over UDP never take a permit.
- The permit count and the soft limit it came from are logged once at boot.
- Shedding is visible to the operator as one episode warn: one `dns.forward.shed_started` when shedding starts and one `dns.forward.shed_cleared` when it ends. The episode ends only when the permits have drained to at most half held, so a saturated bound that releases and re-acquires one permit at a time is one episode, not a pair of warns per release. The clearing swap of the flag is itself conditional on that threshold (measured before the acquire): below it the flag is left untouched. A clear is logged on the next qualifying acquire, so a late or absent clear when traffic stops is intended; a race between the check and the swap yields at most one early clear followed by a new start, a log-timing artifact only.

## Alternatives considered

- **A fixed in-flight count, shedding by dropping.** A count chosen without a measured limit is either too low (sheds healthy load) or unrelated to the real limit (descriptors); a dropped query leaves the guest to wait out its timeout, the symptom this feature removes. Rejected.
- **Count a TCP forward as one descriptor, or let TCP take its permit after the query is read.** The accepted connection exists before the query is read, so a flood of connections would hold descriptors outside the bound. Rejected for the permit at accept, counting two descriptors per permit.
- **Answer a shed TCP connection with SERVFAIL.** Needs a read of the framed query under the read deadline and a write, on a connection already refused a permit. Closing it unread costs nothing and the guest's resolver retries or fails, as it does with no listener. Rejected as more expensive for no gain.
- **End the shed episode at the first permit acquired.** At saturation every release is followed by an acquire, so each release would emit a start and a clear: a log line per query pair, against the ruling that the episode warn has no per-query cost. Rejected for the half-drained rule.
- **End the shed episode after a quiet interval.** Needs a time constant with no derivation, and the permit count already says when the pressure is gone. Rejected.

## Consequences

- Positive: no number to tune; the bound follows the limit the kernel enforces; overload gives a definite, immediate UDP answer; `EMFILE` on a non-forward path is kept out of reach by the reserve.
- Negative: a TCP connection of any query class holds a permit while it lives, so under saturation a mesh name asked over TCP is closed like any other (mesh over UDP is unaffected); a UDP forward is counted at the TCP cost, so the bound is half what UDP alone could use; with a high hard limit the worst-case memory held by waiting permits is large (the budget is in the feature-delta); no per-guest fairness (L-2).
- Memory held at the bound is accepted with no second bound (user, 2026-11-26). The worst case and the per-permit costs, which are unmeasured estimates, are in the feature-delta cost budget; flooding is [#385](https://github.com/overdrive-sh/overdrive/issues/385)'s concern.
- Operator visibility of shedding is the episode warn (user, 2026-11-26), decided by atomic flag swaps with no per-query cost; individual shed queries are not logged. A CLI-readable condition is [#386](https://github.com/overdrive-sh/overdrive/issues/386).
