# ADR-0125 — Use constant node-global nft rules with shared intercept element sets

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records PORT-295-C and amends C1's storage
mechanism. The user-authorized solution-review F-03 remediation adds only the
node-owner converge/audit operations needed to create and read back this ADR's
shared rules/sets at boot/runtime; allocation install methods remain unchanged.

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

**Fresh-process target recovery amendment, 2026-09-16 (user-authorized
S2-F01).** Retained constant rules are adopted only for typed identity after
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

**Rollback error amendment, 2026-09-16 (user-authorized I3-F01).** A failed
rollback write or rollback read carries the real typed netlink source and an
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

Listener recovery never rewrites constant-rule targets. It must rebind the
exact previously recorded port; failure to do so reaches ADR-0124 fail-stop.

## Alternatives considered

### Keep linear rules with a one-port measurement profile

Rejected. It makes the density claim conditional while leaving valid production
input unbounded.

### Restrict Services to one TCP listener

Rejected. It changes valid product semantics to accommodate an implementation
cardinality problem.

## Consequences

Positive: rule count is constant and valid multi-port Services remain supported;
dynamic state scales as N managed-IP + N source + M destination-port elements,
while unmatched intercepted or managed-destination TCP drops fail closed.
Negative: the shared sets and constant-rule targets become node-global runtime
dependencies governed by ADR-0124, and element memory/churn still requires
measurement at the observed port distribution.
