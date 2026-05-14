# DESIGN Wave Decisions — workload-gc-absent-stale-allocs

**Wave**: DESIGN
**Date**: 2026-05-14
**Architect**: Morgan
**Requirements anchor**: GitHub issue #148 (no DISCUSS wave)
**Interaction mode**: propose (Decision 1 carried from /nw-design)

## Decisions ledger

| # | Decision | Rationale | Alternatives considered | Rejected because |
|---|---|---|---|---|
| D-1 | **Location of GC logic = extend `WorkloadLifecycle::reconcile`'s `None` arm (Option A)** | Strongest reuse — every downstream surface (action shim, terminal field, observation write, lifecycle event echo) is already wired for `Action::StopAllocation`. Cheapest hydration — per-target shape already in place; no full-table scan. Best concurrency — local LWW per evaluation. | Option B: sibling `WorkloadGC` reconciler. Option C: hybrid (collapses to A since obs-row GC is out of scope per #148). | B requires new `AnyReconciler`/`AnyState`/`AnyReconcilerView` variants, new hydrator with cross-target scan, and a new target-resource registration story — no evidence A is impossible, so F-1 default (EXTEND over CREATE NEW) binds. |
| D-2 | **Distinct terminal condition = `StoppedBy::SystemGC` (new variant on existing enum)** | Satisfies #148 AC "distinct TerminalCondition operators can match against". Reuses the `TerminalCondition::Stopped { by }` shape — no new top-level variant. ADR-0037 §5 covers additive `StoppedBy` extensions as SemVer-minor. | New top-level `TerminalCondition::Withdrawn` variant. Reuse `Stopped { by: Reconciler }`. | New top-level variant inflates `TerminalCondition` for a sub-classification that belongs in `StoppedBy`. Reusing `Reconciler` collapses two distinct semantics ("reconciler chose to stop a still-desired alloc, e.g. drift correction" vs. "system withdrew an undesired alloc") — operators cannot distinguish them in audit. |
| D-3 | **`StoppedBy::SystemGC` appended as variant index `3` (after `Process`)** | Preserves the rkyv discriminant pin documented in the existing comment (`Operator=0, Reconciler=1, Process=2`). Existing on-disk archived bytes continue to decode. Schema-evolution golden fixture protects the discriminant order in CI. | Insert `SystemGC` before `Process` per "Process MUST remain last" comment intent. | The "Process last" comment is a discriminant-pin claim, not an aesthetic one. Renumbering `Process` to `3` IS the breaking change the comment forbids. Appending `SystemGC` as `3` and updating the comment honours the underlying intent. |
| D-4 | **ADR action = amend ADR-0037 (not new ADR)** | The architecturally significant decision is the new terminal-condition vocabulary item; this falls exactly inside the ADR-0037 SemVer-additive amendment scope already documented in its §5. No new architectural primitive is being introduced. | New ADR for the GC arm. | The reconciler body change is inside a documented branch (TODO(#148) in source); it is implementation, not architecture. The only architectural surface that moves is the public `StoppedBy` enum — which is exactly what ADR-0037 governs. |
| D-5 | **No grace window / immediate stop on first orphan-observed tick** | Simplest correct cut. Adding a grace window requires View memory (`first_observed_orphan_at` input field), live-policy lookup, additional invariants, and a tunable knob — none of which #148 requires. | Configurable grace window with View input `first_observed_orphan_at: UnixInstant`. | Premature complexity. If a future operator scenario shows immediate GC is wrong, the change lands as a tracked follow-up via the standard deferral protocol (user approval first, then issue creation). The current cut is correct per #148's AC; the grace window is speculative. |
| D-6 | **GC arm is workload-kind agnostic** | Withdrawal of intent is a system action distinct from a kind-specific natural exit. `Job`-kind workloads that exit cleanly mid-orphan still reach `Completed { exit_code: 0 }` via the natural-exit branch — the orphan check is "intent absent AND row still non-terminal". | Per-kind branching inside the GC arm (e.g. `Job`-kind orphans get synthesised `Failed { exit_code: SIGTERM }`). | No #148 AC justifies kind-branching here. The `TerminalCondition::Stopped { by: SystemGC }` claim is semantically correct for every kind — the system *withdrew* the workload, no kind-specific lifecycle conclusion applies. |
| D-7 | **DST scenario shape = two invariants** (`gc.converges` + `gc.terminal_claim`) **plus race-shape scenario** (`resubmit_after_gc`) | #148 AC requires "DST scenario". The convergence + typed-terminal pair are the load-bearing checks. The race-shape scenario protects against future regressions in the per-evaluation LWW resolution. | Single invariant covering only convergence. Property test over arbitrary fault sequences. | Single invariant misses the typed-terminal AC. Property test is overkill for a single fault primitive (`IntentStore::remove`); the two scripted scenarios cover the cases #148 names. |

## Quality attribute ranking (per implicit #148 priorities)

1. **Correctness/convergence** — must converge to terminal in every triggering scenario.
2. **Auditability** — distinct `TerminalCondition` operators can branch on.
3. **Simplicity/reviewability** — smallest in-scope change that satisfies AC.
4. **Performance** — cold path; not a concern.

Option A wins all four. Option B ties on 1-2, loses on 3, ties on 4.

## Architecturally significant decisions deferred to crafter

None. The architecture pins the location (Option A), the vocabulary (`StoppedBy::SystemGC`), the discriminant slot (index 3), and the DST invariants. The crafter resolves three implementation-level questions documented in `architecture.md` § 8 (grace window, kind branching, Pending/Draining filter shape) — none of which the architecture binds.

## Forward pointers requiring user approval

**None.** This design does NOT defer scope to future tickets. The three open questions in `architecture.md` § 8 are scoped "not in scope today, no promise of a future ticket" per CLAUDE.md "no aspirational docs" and "no 'Phase 3+ ticket' language without a real issue number".

## Handoff to next wave

There is no DISTILL wave configured for this feature (per the /nw-design dispatch wording). The architecture document and the ADR-0037 amendment together are sufficient for DELIVER to plan a roadmap. The crafter dispatch should:

1. Read `architecture.md` and the ADR-0037 amendment.
2. Add `StoppedBy::SystemGC` to the enum (append, index 3).
3. Replace the `None` arm body per the specification in `architecture.md` § 4 Option A.
4. Add schema-evolution golden-bytes fixture coverage for the new discriminant.
5. Land the two DST scenarios from `architecture.md` § 7.
6. Verify acceptance: hard-delete, multi-node-drain, crash-recovery scenarios all converge.

## External integrations

None. This feature is entirely intra-control-plane. No contract-test annotation needed for the platform-architect handoff.
