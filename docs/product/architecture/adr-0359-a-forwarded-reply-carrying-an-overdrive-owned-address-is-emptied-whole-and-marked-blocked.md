# ADR-0359 — A forwarded reply that carries an Overdrive-owned address is emptied whole and marked Blocked; the owned ranges are one list

## Status

**Accepted** (GH #382, DESIGN decision GPD-4; approved by the user 2026-11-25, including ruling C: the guarded ranges and the on-hit result).
Interface contract and the range list: `docs/feature/guest-public-dns-resolution/feature-delta.md` § *[REF] Interface contracts* (owned ranges, forwarder step 5). Precedent: `docs/research/dataplane/dns-forwarder-answer-filtering-owned-ranges-research.md`.

## Context

Forwarding makes upstream answers untrusted input to guest addressing (D-4). The node steers a guest connection to an address in its mesh frontend range, in its service VIP pool, or in the guest prefix (where guest workload addresses are allocated and the exact-address mesh match applies), as mesh traffic. A reply that aliased a public name onto one of those would steer the guest's traffic into platform addressing. ADR-0172's guest-prefix refusal covers only traffic not diverted to leg-C, so it does not cover that flow. Real upstreams legitimately serve other private ranges (split-horizon), so a blanket private-address rule is wrong (AC-7); the guest prefix sits inside shared address space that some upstreams legitimately return, an accepted cost.

The mesh workload range needs no entry: no production path steers a guest connection to it as mesh traffic (feature-delta, Grounding findings, 7).

Precedent for neutralising a reply: dnsmasq empties the whole reply and keeps the upstream's response code, adding an Extended DNS Error "Blocked" when the client used EDNS; Unbound strips the matching records; the surveyed systems do not return REFUSED.

## Decision

- The owned set is one list built at the composition root from the values that allocate each range: the mesh frontend range, the configured service VIP pool (reserved addresses included) and the guest prefix.
- The guard inspects the answer section of an accepted upstream reply: A addresses and the IPv4 hints of SVCB and HTTPS records.
- On any hit the whole reply is replaced by the same reply with its answer, authority and additional sections emptied and the upstream's response code kept; when the query carried EDNS, the reply carries an Extended DNS Error with the "Blocked" code (RFC 8914). A debug event per query names the name, address, range kind and upstream; there is no per-query warning. The blocked-outcome count reaches test and benchmark observers only; the production observer records none, and a CLI-readable condition waits on [#386](https://github.com/overdrive-sh/overdrive/issues/386). A reply with no hit is relayed unchanged.
- A mixed answer set is blocked whole, not filtered.
- Accepted non-coverage: AAAA records (a guest has only a link-local IPv6 address) and the authority and additional sections (stub resolvers ignore them). The AAAA non-coverage is revisited when guest IPv6 transport lands ([#308](https://github.com/overdrive-sh/overdrive/issues/308)): a guest that can connect over IPv6 can be steered by an aliased IPv6 answer.

## Alternatives considered

- **Remove only the offending records (Unbound).** Mutates the reply, invalidates DNSSEC signatures, and leaves an upstream that returned an owned address partly trusted. Rejected.
- **Replace the reply with REFUSED.** States a policy refusal, but no surveyed resolver does it, and a stub resolver treats REFUSED as a server fault and may try another nameserver or fail with a different error. Rejected for the dnsmasq shape.
- **NXDOMAIN or SERVFAIL as the on-hit result.** NXDOMAIN lies about existence; SERVFAIL is indistinguishable from an outage. Rejected.
- **Guard only the mesh frontend range.** Leaves the service VIP pool (a guest TCP connection to a VIP resolves as mesh, ADR-0164) reachable by aliasing. Rejected.
- **Also guard the mesh workload range.** No production path steers a guest connection to it as mesh traffic; a guard with no reachable failure is more than correct. Rejected.
- **Leave the guest prefix to ADR-0172.** That refusal covers only traffic not diverted to leg-C; a dial to a guest workload address with a healthy backend is diverted and classified as mesh. Rejected.
- **Block all private ranges.** Breaks split-horizon DNS. Rejected.

## Consequences

- Positive: AC-6 to AC-9; the guest always gets a definite result; a new owned range is one added line in one list.
- Negative: the list is not type-enforced against a new allocator that forgets to join it; a legitimate upstream that serves an address inside an owned range (including shared-address-space answers in the guest prefix) cannot be reached by name. The emptied reply reads to the guest as a name with no addresses, not as a refusal; the guest-observed result under glibc and musl is a DELIVER receipt (feature-delta V-4).
