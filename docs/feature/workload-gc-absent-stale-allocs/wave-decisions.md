# Feature Wave Decisions — workload-gc-absent-stale-allocs

Feature-level rollup of wave decisions. One section per wave.

Requirements anchor: **GitHub issue #148** (no DISCUSS wave).

## DESIGN Decisions

**Architect**: Morgan
**Date**: 2026-05-14
**Mode**: propose
**Detail**: `design/wave-decisions.md`, `design/architecture.md`, `design/c4-component.md`

### Summary

- **Option chosen**: A — extend `WorkloadLifecycle::reconcile`'s `None` arm in place.
- **New vocabulary**: `StoppedBy::SystemGc` variant (appended at index `3`; rkyv-discriminant-safe).
- **ADR action**: amend `adr-0037-reconciler-emits-typed-terminal-condition.md` (Amendment 2026-05-14). No new ADR.
- **DST scenarios**: two scripted scenarios (`orphan_workload_converges_to_terminal_gc`, `resubmit_after_gc_creates_fresh_alloc`) with three invariants (`gc.converges`, `gc.terminal_claim`, `gc.no_fresh_alloc`).
- **Rejected**: Option B (sibling `WorkloadGC` reconciler — adds cross-target hydration, fails F-1 default of EXTEND over CREATE NEW).
- **Deferrals**: none. Three open questions in `design/architecture.md` § 8 are scoped as "not in scope today, no promise of a future ticket" — they are not deferrals.

### Quality attributes (priority order)

1. Correctness/convergence — Option A satisfies AC §1.3 by reusing the Stop-branch convergence pattern.
2. Auditability — `StoppedBy::SystemGc` is the distinct terminal condition operators can match against.
3. Simplicity/reviewability — ~20 LOC reconciler change + 1 enum variant. No new components.
4. Performance — cold path; not a concern.

### Handoff

No platform-architect or contract-test annotation required (this is intra-control-plane). Crafter dispatch should reference `design/architecture.md` § 4 Option A for the implementation specification, § 7 for the DST invariants, and the ADR-0037 Amendment 2026-05-14 for the enum extension.
