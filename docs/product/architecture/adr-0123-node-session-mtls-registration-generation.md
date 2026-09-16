# ADR-0123 — Mint transparent-mTLS registration generations inside the node listener owner

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records GEN-295-A.

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
