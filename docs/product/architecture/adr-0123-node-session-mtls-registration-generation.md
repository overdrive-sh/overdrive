# ADR-0123 — Mint transparent-mTLS registration generations inside the node listener owner

## Status

**Accepted — user-approved and approved by system design review iteration 5 on
2026-09-16; D-295-DISTILL-7's complete private registry contract was
independently approved at review iteration 9 on 2026-09-17.** This records
GEN-295-A.

## Context

One node-shared leg-F/leg-C listener pair needs to distinguish a retired
allocation capability from a successor that reuses its guest address. Part D
proved immutable selection and stale rejection with manually supplied
generations, but did not identify the production mint owner or exercise
retirement during an in-flight enforcement call.

The boot contract reclaims VMMs and starts with no adopted allocation
capabilities, so generation uniqueness is needed only within one live node
listener-owner session.

## Decision

The node-shared listener owner exclusively owns a checked, monotonically
increasing non-zero registration generation. It lives under the same lock as
the source/destination indexes, capability state, in-flight claims, and
published handles. The owner initializes it only after boot reclamation and
empty-registry verification; it is neither persisted nor adopted.

Registration checks that the next value can advance before publishing any
effect. Exhaustion refuses registration with a typed intercept-install error and
leaves indexes and nft elements unchanged. The value is internal and is not
added to `GuestNetworkAssignment` or `AllocationSpec`.

Pending registration reserves allocation/source/destination keys under the
same lock but is not claimable. A conflict with Pending, Active, or Retiring
state refuses before element acquisition. Failed Pending work removes only its
reservations and never reuses its consumed generation. Retirement removes
claimable indexes immediately but retains reservations until exact handles and
elements have drained and their absence is confirmed, so an address-reuse
successor cannot overlap the predecessor.

If stop or owner shutdown races a Pending registration, retirement atomically
takes that exact generation without releasing its reservations. The retirement
wait completes only after the Pending owner activates into Retiring or rolls
back/relinquishes every partial effect and signals its waiter. A cancelled
Pending owner removes reservations only when the same generation is still
Pending and retirement has not taken ownership; it can never remove a
successor's reservation.

Activation after retirement is a typed install failure, never success. It names
the existing allocation identity rather than exposing the private generation.
The Pending owner transfers its guards to Retiring, wakes the waiter, and
returns the failure; retirement retains reservations until all effects drain.
Consequently the production action owner cannot release guest EXEC or reuse the
address on this branch.

Accept captures one immutable capability and increments its exact in-flight
claim before enforcement. Publish either records the handle while that
capability remains Active or tears it down after retirement. Stop removes index
visibility, waits for all claims, drains published handles, and only then allows
TPROXY/TCX/TAP teardown and address release. Owner shutdown closes accept
admission, retires every capability, waits for claims, drains handles, and
returns only after completion.

## Alternatives considered

### Per-allocation session counter

Rejected. Tuple uniqueness would work, but retaining a counter map after
retirement adds state without strengthening the single-owner-session invariant.

### Reuse workload desired generation

Rejected. It couples interception lifetime to reconciler intent and requires a
new public transient field despite allocation identity already being distinct.

## Consequences

Positive: address reuse and late enforcement have one internal linearization
authority without widening the approved handoff. Negative: the owner gains an
in-flight waiter and checked-exhaustion path. The retirement-during-enforcement
publish/teardown race remains required evidence because Part D did not execute
it.
