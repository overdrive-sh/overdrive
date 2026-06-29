# DISTILL Wave Review - backend-instance-replacement

review_id: `accept_rev_2026-06-30T02:42:00+07:00`
reviewer: `acceptance-designer (review mode)`
approval_status: `approved`

## Scope

Reviewed the current rev3 DISTILL package:

- `docs/feature/backend-instance-replacement/feature-delta.md`
- `docs/feature/backend-instance-replacement/design/wave-decisions.md`
- `docs/feature/backend-instance-replacement/distill/test-scenarios.md`
- `docs/feature/backend-instance-replacement/distill/red-classification.md`
- `docs/architecture/atdd-infrastructure-policy.md`

## Findings

No blocking findings.

### Low - DISTILL changelog rows are out of revision order

Dimension: Internal consistency.

The package is internally consistent on the current scenario count and coverage
ratio, but the changelog lists `rev3` before `rev2`. This can make the review
history harder to scan, especially because both rows describe post-review
remediation.

Evidence:

- `feature-delta.md:1065`: rev1 row.
- `feature-delta.md:1066`: rev3 row.
- `feature-delta.md:1067`: rev2 row.

Recommendation:

Optionally reorder the changelog rows as initial DISTILL, rev1, rev2, rev3.
This is documentation polish only; it does not affect DELIVER readiness.

## Closed Prior Findings

- The hidden two-tick `S-BIR-COALESCE` scenario is now split into
  `S-BIR-COALESCE-PLACE` and `S-BIR-COALESCE-NO-REPLAY`, with one
  `reconcile()` action per scenario (`test-scenarios.md:362-404`).
- The prior traceability labels are corrected: `S-BIR-TXN-01..04` now map to
  `US-BIR-1 AC4 / DDD-9`, while coalescing and sequential restart map to
  `DDD-10 / K-BIR-1`; the not-found scenarios remain on AC5
  (`test-scenarios.md:127-147`, `feature-delta.md:823-842`).
- The stale scenario count and ratio are corrected to 24 scenarios and
  14/24 error, edge, or regression coverage in the current DISTILL summary
  (`feature-delta.md:810-812`, `feature-delta.md:1024-1030`).
- The project infrastructure policy rows for backend-instance-replacement are
  present in the actual policy file (`atdd-infrastructure-policy.md:33-35`,
  `atdd-infrastructure-policy.md:43`, `atdd-infrastructure-policy.md:49`).
- The prior multi-When examples in `S-BIR-SEQUENTIAL`,
  `S-BIR-REGRESSION-STOPPED`, and `S-BIR-REGRESSION-RUNNING` are reframed so
  crash / second-restart state is Given context and `reconcile()` is the single
  driving action (`test-scenarios.md:406-474`).
- The adapter coverage table now distinguishes real-I/O proof from focused
  in-process handler coverage (`test-scenarios.md:742-772`).

## Strengths

- Coverage is broad and traceable: store atomicity, reconciler generation and
  cardinality behavior, stale-veto regression protection, handler behavior, CLI
  dispatch, and all three existing Tier-3 oracle ATs are represented.
- Error, edge, and regression coverage is strong at 14/24 scenarios, exceeding
  the 40 percent target.
- The RED-classification plan cleanly distinguishes new
  `MISSING_FUNCTIONALITY` scaffolds from existing ignored oracle ATs that should
  become GREEN after un-ignore and verb swap.
- Walking-skeleton boundary proof is credible: no new skeleton is invented, and
  the reused dial-by-name Tier-3 oracle remains the real production-path proof.
- The `Scaffold MANIFEST` is consistent with the Rust workspace precedent used
  by prior complex features: DISTILL records the executable specification and
  pinned RED shape; DELIVER materialises compile-affecting Rust scaffolds.

## Gate Checks

| Gate | Result | Evidence |
|---|---|---|
| Happy-path bias | PASS | 14/24 error, edge, or regression scenarios. |
| GWT format | PASS | Rev3 split the remaining hidden trajectory; every current scenario has one driving action. |
| Business/domain language | PASS with project-policy exceptions | User-facing scenarios use operator/domain language; mechanism-level Rust tier scenarios use platform port terms required to specify this infrastructure feature. |
| Coverage completeness | PASS | US-BIR-1 and US-BIR-2 are covered; DDD-9/10/13 design-contract cases are mapped. |
| Observable assertions | PASS | Assertions are through returned action/view tuples, store reads, handler/CLI results, and Tier-3 name/connect behavior. |
| Traceability coverage | PASS | Story, design-decision, environment, scaffold, adapter, and driving-port mappings are present. |
| Walking skeleton / real-I/O boundary | PASS | Policy rows are appended; real-I/O proofs are explicit; handler doubles are not over-counted. |
| RED scaffold convention | PASS | `red-classification.md` defines expected RED reasons and separates existing oracle ATs from new scaffolds. |

## Decision

`approved`.

The DISTILL package is ready for DELIVER. The only residual note is the
non-blocking changelog ordering cleanup described above.
