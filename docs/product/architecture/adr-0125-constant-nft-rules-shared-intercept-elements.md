# ADR-0125 — Use constant node-global nft rules with shared intercept element sets

## Status

**Accepted — the current #295 contract is user-approved and independently
approved.** This records PORT-295-C's current storage and
ownership decision. Node-owner converge/audit creates and reads back shared
rules/sets; allocation install methods remain unchanged. Exact signatures live
only in the #295 feature delta.

## Context

Valid Services carry an unbounded listener vector. The current intercept appends
one outbound rule and two inbound rules per projected port, producing
allocation-and-port-linear rule traversal and cleanup. It also projects UDP
declarations into a TCP-only interception path.

The node-shared listeners already provide one leg-F and one leg-C destination,
so per-allocation rule programs are unnecessary. Dynamic ownership is endpoint
and destination membership, not rule code.

## Decision

The worker intercept owner exclusively maintains three typed nft sets: managed
guest IPv4s, active outbound source IPv4s, and destination IPv4 plus TCP port. It
owns exactly eight constant IP rules: two leg-S exemptions; outbound TPROXY;
unhandled-intercept drop; inbound prerouting TPROXY; prerouting managed-
destination fallback drop; output-route divert; and output managed-destination
fallback drop. Together with the three bridge proof-mark rules, #295 nft rule
cardinality is eleven regardless of allocations or listener count.

The existing outbound and inbound install methods remain. Their guards own and
reference-count only set elements: outbound owns both managed-guest and source
elements; inbound owns one destination tuple. They never own rules, sets,
chains, or the shared table. Listener-port arguments must match the node rule target. The
userspace destination registry is keyed by guest IPv4 and holds the immutable
capability plus an allowed set of distinct TCP ports. UDP listeners remain valid
but create no TCP intercept membership.

One distinct node-scoped guard, returned by the existing intercept adapter's
shared converge operation, owns the constant rules and set objects after exact
target read-back; the worker never places it on an allocation. The sibling
audit operation is read-only. Exact signatures stay in the feature delta.

For fresh-process target recovery, retained constant rules are adopted only for typed identity after
the EXEC gate is BootClosed and stale attachment recovery proves zero managed
TAPs. Fresh F/C listeners may bind ephemeral ports only then. One nft atomic
transaction replaces every owned occurrence of the two TPROXY target ports
while preserving rule order, normalized non-target expressions, userdata,
sets, and foreign objects. Transaction rejection leaves the prior program
intact. A post-commit read-back mismatch triggers one atomic rollback to the
captured prior owned program and another full read-back; startup refuses whether
rollback succeeds or fails, preserving typed primary/rollback evidence. A
foreign, duplicate, malformed, or schema-conflicting object refuses before
mutation. No port is persisted or fixed.

At runtime the recorded F/C ports are immutable. Missing owned rules may be
recreated with those same targets; a present wrong-target rule is never
rewritten and reaches ADR-0124's bounded fail-stop path.

The captured prior is `None` when no owned shared program exists and `Some`
only for a complete owned identity. If a first-boot create commits but full
read-back mismatches, rollback targets absence and success requires the next
observation to be `None`. Every successful-rollback, rollback-I/O-failure, and
rollback-postcondition-mismatch outcome therefore carries the optional prior;
an empty fabricated program is not a representation of absence.

`HostMtlsIntercept` retains the replacement/read-back/rollback algorithm above
one module-private effect seam with exactly two responsibilities: observe the
optional normalized shared program and atomically replace an expected optional
current program with an optional desired program. Production construction
privately supplies real netlink I/O. Only an in-module `cfg(test)` constructor
may supply a scripted implementation; the seam is not public, not available to
integration consumers, and not a compatibility adapter. This keeps public
five-method `MtlsIntercept`, worker ownership, and allocation element methods
unchanged while making source-honest rollback branches deterministic.

A failed rollback write or rollback read carries the real typed netlink source and an
operation discriminator. A rollback write/read that succeeds but observes the
wrong prior identity is a distinct source-less semantic postcondition failure.
Exact successful rollback is a third source-less restored-prior disposition.
All retain structured prior/requested/replacement/rollback identities as
available, leave EXEC BootClosed, and refuse startup. No fabricated source or
formatted error string is permitted; exact variants live only in the feature
delta.

Allocation activation is externally atomic: publish Pending capability state,
acquire all required elements, then publish Active under the registry lock. A
partial install is never Active. Stop retires registry visibility first,
removes all owned elements in one batch with read-back, then drops guards. Boot
reclamation clears stale elements while retaining and verifying constant rules.
Exact set names, keys, error variants, method signatures, and ordering live in
the feature delta.

The existing general nft exports cannot implement that element ownership: they
operate on rules and expose neither typed dynamic-set observation nor a
semantic multi-set transaction. The accepted lower correction is therefore
one doc-hidden, IP-intercept-specific extension in `overdrive-netlink::nft`.
It adds one opaque semantic `SharedIpInterceptState` (the unchanged
handle-free `SharedIpInterceptIdentity` plus sorted typed members), one
generation-bracketed state observer, and exactly four named effects: atomic
outbound two-element insert, atomic inbound one-element insert, grouped
allocation delete, and boot clear. Every effect requires the expected constant
program identity, returns its mandatory semantic read-back, preserves the
untouched-member/constant-program/foreign complement, and retains exact typed
netlink sources. Exact signatures are authoritative only in the feature delta.

No set/key/mutation enum or generic additions/removals function crosses the
crate boundary. Family and runtime-mode selectors, set IDs, rule handles,
ruleset generations, raw keys/attributes, and raw builders remain private.
IPv4 keys are four octets; the destination key is IPv4 plus big-endian TCP port
plus two zero ABI-alignment bytes (`key_len = 8`). `HostMtlsIntercept`, not the
netlink adapter, owns clone-shared process refcounts and group tokens:
identical installs adopt a token without a second write, first/final ownership
performs the semantic insert/delete, normal stop deletes all `2 + P` elements
in one batch before disarming guards, and guard Drop is only the best-effort
fallback. Restart adopts no tokens; post-reclamation boot clears and reads back
all three sets empty before listener-target convergence.

The accepted public port remains exactly the feature delta's five methods.
In particular `install_outbound(Ipv4Addr, u16)` replaces the live pre-cut
`&str` signature as required compiler fallout. A textual overload, parse/fallback
branch, compatibility method, or return to the per-interface rule installer is
not permitted.

Listener recovery never rewrites constant-rule targets. It must rebind the
exact previously recorded port; failure to do so reaches ADR-0124 fail-stop.

## Alternatives considered

### Keep linear rules with a one-port measurement profile

Rejected. It makes the density claim conditional while leaving valid production
input unbounded.

### Restrict Services to one TCP listener

Rejected. It changes valid product semantics to accommodate an implementation
cardinality problem.

### Expose a generic set/key additions-and-removals API

Rejected. It lets callers compose invalid set/key pairs and unrelated member
mutations, cannot make the outbound two-set group structural, and leaks nft
policy below the sole `HostMtlsIntercept` consumer. Named group effects plus an
opaque semantic state projection are smaller and keep raw ABI private.

## Consequences

Positive: rule count is constant and valid multi-port Services remain supported;
dynamic state scales as N managed-IP + N source + M destination-port elements,
while unmatched intercepted or managed-destination TCP drops fail closed.
Negative: the shared sets and constant-rule targets become node-global runtime
dependencies governed by ADR-0124, and element memory/churn still requires
measurement at the observed port distribution. The host adapter adds one
private effect interface for staged rollback evidence, but no public port or
second ownership path.
