# ADR-0135 — Release allocation intercept elements through one awaited, convergent, retry-retaining port operation

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R10. Proposed 2026-09-23; reviewed by independent DESIGN review
rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. Makes ADR-0125's accepted cleanup intent
expressible at the intercept port. That intent is to delete all `2 + P`
elements in one batch before disarming, with `Drop` as fallback only. This ADR
also replaces the exact-delta refusal on already-absent members with
convergent removal. Exact signatures live only in the #295 feature
delta.

## Context

ADR-0125 requires normal allocation stop to remove the allocation's shared IP
elements in one batch with read-back, and to keep retirement ownership when
that fails.

The accepted port returns each element guard as an erased marker trait with no
method. The worker can therefore only drop the guards. A failed delete is
logged in `Drop`, the registry record is removed, and `stop_alloc` returns
`Ok`. The action shim then tears down the TAP, marks the row Terminated, and
releases the address. A same-address successor is then admitted and adopts the
leftover elements. The proof `shared_element_cleanup_failure` reproduced all of
this for both deletion rejection and read-back failure.

Separately, the accepted grouped delete refuses when a requested member is
already absent. A retry after an external deletion therefore can never
converge, and the allocation would hold its lease indefinitely.

Prior art settles both halves. The CNI specification requires DEL to succeed
when "the interface in question, or any modifications added, are missing", and
Cilium's DEL treats not-found as success because the kubelet retries deletion.
Kubernetes finalizers keep an object, and its ownership, until the cleanup
controller succeeds. Rust `Drop` cannot await; libraries such as tokio's
`fs::File` expose an explicit async operation to run before drop, and the
language roadmap's "sync `Drop` plus a spawned task" workaround is forbidden
here for completion-bearing effects. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 6.1, 6.2 and 6.3.)

## Decision

The intercept port gains one grouped removal operation. The worker calls it
after it has drained the allocation's handles, passing the allocation's source
address and complete destination set:

- **On success:** the process-local element tokens for those members are
  retired, so their guards' `Drop` performs no effect. The worker then completes
  the retirement.
- **On failure:** the typed source is returned, and tokens, guards, and the
  Retiring registry record stay in place. `stop_alloc` returns a typed
  element-removal error. Every owner above it keeps cleanup ownership: the
  action shim tears down no TAP, releases no lease, and writes no terminal row.
  A later stop re-runs the removal.

Removal converges. It observes first, deletes exactly the requested members
that are still present in one atomic batch, and then proves every requested
member absent. Every other member, the constant program, and the foreign
complement must be unchanged. Members already absent before the call are
reported in the returned observation and are not an error.

The existing rules for batch rejection, post-commit mismatch with one inverse
restoration, and retained dual sources are unchanged. `Drop` remains the
best-effort unwind or crash fallback only.

## Alternatives considered

### A fallible release method on each element guard

Rejected. It needs `P + 1` batches instead of one. It exposes intermediate
partial states and reverses the accepted rule that `InterceptGuard` carries no
method.

### Replace erased guards with a concrete, typed element-group token

Rejected. It is a larger change across the host and sim port implementations,
with the same effect.

### Keep exact-delta refusal on already-absent members

Rejected. It makes cleanup non-convergent after external drift, and the lease
would never be released.

## Consequences

Positive: cleanup failure is visible, typed, and retryable, and address reuse
cannot adopt residue. Cleanup also converges after drift.

Negative: the intercept port grows by one method. The worker's stop error
becomes a typed enum, replacing its stringified failure list. Element effects
must serialize with the runtime member audit (feature delta, decision R15).
