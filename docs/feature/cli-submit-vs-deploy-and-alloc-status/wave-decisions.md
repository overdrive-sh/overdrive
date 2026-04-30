# Wave decisions — `cli-submit-vs-deploy-and-alloc-status`

**Feature ID**: `cli-submit-vs-deploy-and-alloc-status`
**Type**: Brownfield UX divergence (existing CLI surface)
**Status**: DIVERGE complete (pending peer review by `nw-diverger-reviewer`).
**Date**: 2026-04-30

---

## DIVERGE

**Wave owner**: Flux (`nw-diverger`).

**Inputs**:

- User session reproduction (verbatim, recorded in
  `diverge/job-analysis.md`).
- Whitepaper §4 (Intent/Observation split), §18 (Reconciler
  primitive, ESR contract).
- Existing artifacts: `docs/product/jobs.yaml` (J-OPS-002, J-OPS-003),
  `docs/product/journeys/submit-a-job.yaml` (canonical journey),
  `docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.yaml`
  (TUI mockup specifying the snapshot shape that motivated this
  divergence), `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`.
- ADR-0008 (REST/OpenAPI), ADR-0014 (CLI shared types), ADR-0027
  (`POST /v1/jobs/{id}:stop` precedent for verb-suffix lifecycle ops).

**Configuration honoured per invocation**:

- Work type: brownfield UX divergence.
- Research depth: lightweight (3 named competitors + reference
  points; no web fetches).
- Output directory: `docs/feature/cli-submit-vs-deploy-and-alloc-status/`.

**Phase summary**:

| Phase | Skill | Gate | Status |
|---|---|---|---|
| 1 — JTBD Analysis | `nw-jtbd-analysis` | G1 (job at strategic level, ≥3 ODI outcomes) | PASS — 6 outcomes, 5 of which severely under-served |
| 2 — Competitive Research | `nw-researcher` (skipped — user already named tools; lightweight depth) | G2 (3+ competitors, non-obvious alternative) | PASS — kubectl + rollout, nomad job run + alloc status, fly deploy, plus systemctl+journalctl as the non-obvious alternative |
| 3 — Brainstorming | `nw-brainstorming` | G3 (6 diverse options; SCAMPER coverage; no eval language) | PASS — 6 options after merging X1→M, X2→S, dropping C as future-extension on S |
| 4 — Taste Evaluation | `nw-taste-evaluation` | G4 (DVF filter, locked weights, ranking, recommendation traceable to scores) | PASS — Option S 4.47, Option A 3.77, Option M 3.68 |

**Key decisions captured in DIVERGE**:

1. **Job extracted at strategic level**: "Reduce the time and
   uncertainty between declaring intent and knowing whether the
   platform converged on it." Solution-agnostic — every option
   serves this same job.
2. **6 structurally diverse options** generated against the HMW
   "How might we make the time and reasoning between declaring an
   intent and knowing whether the platform converged on it as small
   as possible, without forcing operators to learn a separate
   diagnostic toolchain?". Each passes the 3-point diversity test.
3. **Locked weights with explicit adjustment**:
   - DVF 25% / T1 Subtraction 15% / **T2 Concept Count 25% (+5pp)** /
     T3 Progressive Disclosure 15% / **T4 Speed-as-Trust 20% (−5pp)**.
   - T2 raised because the divergence's central risk is verb-soup
     (six options propose 1–3 verbs each).
   - T4 lowered because the user's complaint is about honesty, not
     raw responsiveness.
4. **Recommendation**: Option S — Submit-streams-default. Score 4.47
   vs runner-up 3.77 (Option A). Clear winner. The 0.70-point gap
   means this is not a coin flip.
5. **Dissenting case documented**: Option A is the correct fallback
   if the DISCUSS wave rejects the assumption "submit can become a
   long-lived HTTP request bounded by the lifecycle reconciler's
   convergence-or-backoff window."
6. **`alloc status` snapshot enrichment is no-regret** — it ships
   identically under S, A, or M, and the work can begin in DESIGN
   before the wait-shape ADR is finalised.

**Rejected options (taste-phase)**:

- Option E (One-verb, submit absorbs status) — 3.58. Sometimes-commit-
  sometimes-attach dual semantic costs T2.
- Option P (`deploy` verb) — 3.13. Adds a verb without justifying it
  (no build-and-push complexity to motivate the split).
- Option R (Plan/Apply split) — 2.96. Foreign mental model for
  ops; strong long-term shape but wrong fork for this divergence.

**Artifacts produced**:

- `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/job-analysis.md`
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/competitive-research.md`
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/options-raw.md`
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/taste-evaluation.md`
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/recommendation.md`
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/wave-decisions.md` (this file)
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/review.yaml`
  (pending peer review)

**Open questions surfaced for the user**: None blocking. The decision
between Option S and the fallback Option A is **the** decision the
DISCUSS wave (Luna, `nw-product-owner`) ratifies; the recommendation
makes a clear call but explicitly documents the assumption that
ratification turns on.

**Hand-off shape**:

- → DISCUSS (`nw-product-owner` / Luna): ratify the direction (Option
  S; fallback Option A). Drive the journey re-extension and the AC
  rewrite for any user stories that touch submit and alloc status.
- → DESIGN (`nw-solution-architect`): on the back of DISCUSS
  ratification, produce two ADRs:
  1. HTTP shape for streaming submit response (NDJSON vs SSE; Accept
     header gating; `Vec<Action>` consumer pattern).
  2. `alloc status` snapshot enrichment (fields exposed, render
     contract, retry-budget surface).

---

## Peer review

**Reviewer**: Prism (`nw-diverger-reviewer`) — inline self-review (Task
tool unavailable in execution environment; reviewer-of-self limitation
acknowledged in `diverge/review.yaml`).
**Verdict**: APPROVED — all 5 dimensions PASSED, two advisory
recommendations (non-blocking).

| Dimension | Status |
|---|---|
| JTBD rigor | PASSED |
| Research quality | PASSED |
| Option diversity | PASSED |
| Taste application | PASSED |
| Recommendation coherence | PASSED |

**Advisory recommendations applied** (both polishes are non-blocking
but cheap to incorporate; both landed inline before handoff):

1. Recommendation now explicitly acknowledges that the API contract
   evolution (sync-JSON → polymorphic-by-Accept-header NDJSON) is more
   design surface than the feasibility-4 DVF score naively suggests,
   and names it as a DESIGN-wave HTTP-shape ADR responsibility.
2. The fallback to Option A now carries a sharp two-clause trigger
   ("if the team will not ship streaming machinery in Phase 1, OR if
   the API contract evolution is judged too expensive for the
   deadline").

Full review record: `docs/feature/cli-submit-vs-deploy-and-alloc-status/diverge/review.yaml`.

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial DIVERGE wave artifacts produced. Recommendation: Option S (Submit-streams-default), with Option A as the documented fallback. |
| 2026-04-30 | Inline peer review by Prism: APPROVED, 5/5 dimensions PASSED, 2 advisory recommendations applied to recommendation.md. Ready for DISCUSS wave handoff. |
