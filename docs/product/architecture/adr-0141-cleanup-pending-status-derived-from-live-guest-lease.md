# ADR-0141 — Report network cleanup-pending status from the live guest-attachment lease, not from a persisted field

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R20. The operator behaviour is a **user ruling of 2026-09-24**:
`overdrive workload describe` shows an allocation whose network cleanup has not
completed as cleanup-pending, never as Running, including after a failed stop
and for crash, replacement, and reclaim cleanups. This record decides how that
status is derived and where it lives. Proposed 2026-09-24; the mechanism was
reviewed by independent DESIGN review rounds 4 and 5 and the round-5
verification (`arch_rev_20260924_netns295_r5_verify`); accepted by the user on
2026-09-24 with the replacement DESIGN. Depends on ADR-0132 and ADR-0133. Exact types,
fields, and rendering live only in the #295 feature delta.

## Context

Under ADR-0133 a guest attachment's lease counts against the node until its
allocation's cleanup finishes, and a stuck cleanup visibly holds a slot. The
allocation's row does not show that state:

- A stop whose cleanup fails keeps its row Running, so the stop is retried
  (ADR-0135). The VMM is gone, but the operator sees Running.
- A crashed or replaced allocation's row is Failed or Terminated while its TAP,
  TCX link, bridge-guard member, and intercept elements still exist, until its
  restart, finalize, or reclaim cleanup finishes (ADR-0136).

The authoritative fact that cleanup has not finished is the allocation's lease
in the server's guest-address pool. A lease becomes Retiring when cleanup
begins, and it is released only when cleanup has finished. The pool is
process-local by design: ADR-0118 rejected a durable lease table. Every fresh
boot starts with an empty pool, at exactly the point where VM reclamation and
the stale-attachment sweep have removed every residue a lease could describe.

Two repository rules bear on the choice. *Persist inputs, not derived state*
forbids persisting a value that is recomputable from its inputs. *A convergent
record cannot answer "did it happen"* requires an occurrence to keep a durable
surface of its own. A crash is such an occurrence, and it already has one: the
Failed row, the restart count, and the last-terminated snapshot (ADR-0078).

## Decision

Cleanup-pending is derived at read time from two inputs: the allocation's live
lease and its row state. It is pending when the lease is Retiring, or when the
lease is Admitted and the row is terminal (the VMM has exited and cleanup has
not begun). It is not persisted anywhere.

The control plane's allocation-status read joins each row with the pool's lease
for that allocation and reports the derived status. Running replicas exclude
cleanup-pending rows. The operator view shows cleanup-pending in place of the
state and keeps the lifecycle state beside it, so a crash still reads as a
crash. The wire change is one additive field. No persisted row, envelope, or
schema changes.

The status is exact for the life of the process that holds the residue. After a
restart, the boot sweep has already finished every network cleanup, so no
allocation is cleanup-pending.

## Alternatives considered

### Persist a cleanup-pending field on the allocation status row

Rejected. It stores a derivation of the lease and the row state. It would also
outlive the residue it describes: the boot sweep removes all residue and writes
no rows, so every persisted pending field would be false after a restart. That
is the "marker outlives the effect it records" defect. It would also need a
versioned envelope bump and a golden fixture for no information the live lease
lacks.

### Add a cleanup-pending lifecycle state

Rejected. Allocation lifecycle states have established meanings that
reconcilers and restart accounting depend on. Cleanup is orthogonal to the
lifecycle: a crashed allocation with pending cleanup is still Failed. Replacing
Failed with a cleanup state would hide the crash, and it would ripple through
every consumer of the lifecycle vocabulary.

### Structured events only

Rejected by the user ruling. Events are transient and scattered; the ruling
requires the status in `describe`.

## Consequences

Positive: the operator sees the one fact the lease already knows, without a
second source of truth, a schema change, or a stale state after restart. A crash
stays visible as a crash.

Negative:

- The status is node-local and process-lifetime. This fits #295's single-node
  scope; a cross-node view would need a different surface.
- A leased allocation that has no row cannot appear in `describe`. That is the
  residual case ADR-0136 records, visible only in admission-refusal counts.
- A GC-branch workload's leased residue is likewise invisible in `describe`:
  once the workload intent is deleted, the allocation-status read returns 404
  (the handler resolves the workload aggregate and fails `NotFound` when the
  intent is absent), so an allocation still being reclaimed under a deleted
  workload has no `describe` output at all. Its residue is visible only in the
  retiring count of admission refusals, beside the no-row residual above.
- The row read and the lease read are separate, so the status can be one
  transition stale, like the other fields the read joins.
- Durable history of individual cleanup attempts is not added; failures remain
  visible as typed dispatch errors, and reclaim's attempt count is kept in its
  reconciler memory.
