# DISCUSS Wave Decisions — phase-1-foundation

**Wave**: DISCUSS (product-owner)
**Owner**: Luna
**Date**: 2026-04-21
**Status**: COMPLETE — handoff-ready for DESIGN (solution-architect)

---

## Wizard decisions honoured

- **Feature type**: Infrastructure. Consumer = Overdrive platform engineers running `cargo xtask dst`. No end-user UX.
- **Walking skeleton**: yes — this feature IS the project walking skeleton.
- **UX research depth**: lightweight. Engineer → CLI → CI gate. No web/desktop UX skills loaded beyond baseline; TUI patterns applied to lint-gate and DST output design.
- **JTBD analysis**: skipped. Motivation is explicit in whitepaper §21 and distilled into `docs/product/jobs.yaml` entries J-PLAT-001, J-PLAT-002, J-PLAT-003.

## Artifacts produced

### Product SSOT (bootstrapped in this wave)

- `docs/product/vision.md` — distilled from whitepaper §2 + commercial.md. Establishes design principles, target tiers, and what Phase 1 must prove.
- `docs/product/jobs.yaml` — five job statements (three active for Phase 1, two deferred with traceability). Jobs grounded in whitepaper sections and `commercial.md`, not freshly derived.
- `docs/product/journeys/trust-the-sim.yaml` — the canonical product-level journey for the Phase 1 engineer experience.

### Feature artifacts

- `docs/feature/phase-1-foundation/discuss/journey-trust-the-sim-visual.md` — ASCII journey + TUI mockups + emotional arc.
- `docs/feature/phase-1-foundation/discuss/journey-trust-the-sim.yaml` — structured journey with embedded Gherkin per step (NO `.feature` file — project rule).
- `docs/feature/phase-1-foundation/discuss/journey-trust-the-sim-scenarios.md` — journey-level scenarios as markdown-fenced Gherkin. Complements `user-stories.md` which holds per-story scenarios.
- `docs/feature/phase-1-foundation/discuss/shared-artifacts-registry.md` — seven shared artifacts tracked, each with SSOT + consumers + integration risk + validation.
- `docs/feature/phase-1-foundation/discuss/story-map.md` — 5-activity backbone, walking-skeleton identified, 6 carpaccio slices, priority rationale, scope assessment = PASS.
- `docs/feature/phase-1-foundation/discuss/user-stories.md` — six LeanUX stories (US-01 through US-06) with a System Constraints header and embedded Example Mapping.
- `docs/feature/phase-1-foundation/discuss/outcome-kpis.md` — six feature-level KPIs with measurement plans.
- `docs/feature/phase-1-foundation/discuss/dor-validation.md` — 9-item DoR PASS for all 6 stories.
- `docs/feature/phase-1-foundation/slices/slice-{1..6}-*.md` — one brief per carpaccio slice.
- `docs/feature/phase-1-foundation/discuss/wave-decisions.md` (this file).

## Key decisions

### 1. No DIVERGE artifacts present — grounded directly in whitepaper + commercial.md

No `docs/feature/phase-1-foundation/diverge/recommendation.md` or `job-analysis.md` existed. Per the skill's "If absent" branch, full discovery was not run — motivation was explicitly signed off as explicit in whitepaper §21 (wizard decision "JTBD analysis: No"). Job statements were distilled into `docs/product/jobs.yaml` with clear pointers back to whitepaper sections as their source. **Risk**: job statements are not interview-validated. **Mitigation**: they trace to published platform design; DIVERGE can be retrofitted if needed.

### 2. No `.feature` files — project rule enforced

Per `.claude/rules/testing.md` and the wizard prompt: all Gherkin lives as markdown blocks or YAML embedded. `journey-trust-the-sim-scenarios.md` and per-story scenarios in `user-stories.md` hold the scenarios; the journey YAML holds step-level Gherkin inline. The crafter will translate to Rust `#[test]` / `#[tokio::test]` in `crates/{crate}/tests/`.

### 3. Walking skeleton = all 6 slices in Release 1

Because this feature IS the walking skeleton, there is no Release 2. All six slices must ship before Phase 2 can proceed. Slices are ordered by dependency; Slices 1+2 and 3+4 can run in parallel.

### 4. `single_leader` invariant in Phase 1 is a stubbed-topology test

`RaftStore` is out of scope (deferred to the convergence-engine feature). The `single_leader` invariant in Slice 6 operates against a stubbed leader-election topology, primarily to prove the invariant machinery works. Retired in Phase 2 when real consensus lands. Documented explicitly in US-06 technical notes so DESIGN does not design around a Raft that isn't there.

### 5. Outcome KPIs anchored to commercial claims where they exist

K4 (LocalStore cold start < 50ms, RSS < 30MB) directly encodes the `commercial.md` "Control Plane Density" claim. This is a guardrail metric: if a future PR regresses it, the commercial density argument starts to decay.

### 6. System Constraints section in `user-stories.md`

Six cross-cutting constraints (no `.feature` files, Result alias, newtypes strict, deterministic hashing, banned-API list, IntentStore/ObservationStore separation) are declared once at the top of `user-stories.md` rather than repeated per story. Stories explicitly reference these where relevant.

### 7. Scenario titles are business outcomes, not implementation

Every scenario title describes what the user observes (e.g. "LocalStore snapshot round-trip is bit-identical", "Lint gate blocks a core crate that uses Instant::now()"). None name internal types, methods, or protocols as the subject of the scenario. This is Luna's contract with DISTILL.

## Scope assessment result

- **Stories**: 6 (under 10-ceiling).
- **Bounded contexts**: 2 existing (`overdrive-core`, `xtask`) + 1 new (`overdrive-sim`). Under 3-ceiling.
- **Walking-skeleton integration points**: 5 (one per activity; harness is the integration point). Under 5-signal threshold; the single integration point is intentional.
- **Estimated effort**: 4–6 focused days.
- **Multiple independent user outcomes worth shipping separately**: no — the walking skeleton is indivisible.
- **Verdict**: **RIGHT-SIZED.** No split required.

## Risks surfaced

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Partial scaffolding in `crates/overdrive-core/src/traits/` diverges from the requirements here | Medium | Medium | DESIGN wave reconciles partial scaffolding against these AC; treat existing code as a starting point, not as locked-in. |
| `RaftStore` out-of-scope means `single_leader` invariant is stubbed in Phase 1 | Low | Low | Documented in US-06 technical notes; retired in Phase 2. |
| Turmoil's published stability story is strong but version drift could impact bit-identical reproduction guarantee | Low | High | Pin turmoil version in workspace; twin-run identity self-test is a continuous guard. |
| No DIVERGE-run job validation — jobs are whitepaper-distilled, not interview-validated | Low | Low | Jobs trace to published design; can retrofit DIVERGE if a job turns out wrong. |
| The DST wall-clock < 60s target may be challenged by real invariant complexity as Phase 2 grows | Medium | Medium | Guardrail metric; if regression exceeds, either parallelise runs or relax with explicit scope change. |

## What DESIGN wave should focus on

1. **Reconcile partial scaffolding**: `crates/overdrive-core/src/traits/{clock,transport,entropy,intent_store,observation_store,dataplane,driver,llm}.rs` already exist. DESIGN must decide whether to complete them in place or refactor. These requirements are the target contract; the existing code is not canonical.
2. **Canonicalisation for `SchematicId`**: decide rkyv-archived vs RFC 8785 JCS. Either is defensible; inconsistency is not.
3. **Core-crate labelling mechanism**: `package.metadata.overdrive.crate_class = "core"` is suggested; DESIGN picks and documents.
4. **`overdrive-sim` crate layout**: whether Sim* traits live in `overdrive-sim` or in a sibling crate; whether the harness and invariants are separate crates.
5. **Test distribution**: per-crate integration tests vs top-level `crates/*/tests/acceptance/*.rs` style.
6. **CI wiring**: `cargo xtask dst` + `cargo xtask dst-lint` as required checks; artifact upload on failure.

## What is NOT being decided in this wave (deferred to DESIGN)

- Which Rust modules hold which types.
- Error variant taxonomy beyond "use `thiserror` + embed via `#[from]`."
- Trait method signatures in detail (only semantics are specified here).
- Whether `export_snapshot` is `async` or sync.
- Concrete redb schema layout.
- Exact rkyv derive attribute choices.
- Whether `Sim*` impls are one crate or several.

## Handoff package for DESIGN (solution-architect)

- `docs/product/vision.md` — platform vision distilled
- `docs/product/jobs.yaml` — validated job register
- `docs/product/journeys/trust-the-sim.yaml` — canonical engineer journey
- `docs/feature/phase-1-foundation/discuss/journey-trust-the-sim-visual.md` + `.yaml` + `-scenarios.md` — journey artifacts
- `docs/feature/phase-1-foundation/discuss/shared-artifacts-registry.md` — integration points
- `docs/feature/phase-1-foundation/discuss/story-map.md` — carpaccio slices + priority
- `docs/feature/phase-1-foundation/discuss/user-stories.md` — LeanUX stories with AC and per-story Gherkin
- `docs/feature/phase-1-foundation/discuss/outcome-kpis.md` — measurable KPIs + guardrails
- `docs/feature/phase-1-foundation/discuss/dor-validation.md` — 9-item DoR PASS for all 6 stories
- `docs/feature/phase-1-foundation/slices/slice-{1..6}-*.md` — slice briefs
- Reference: `docs/whitepaper.md` §2, §4, §17, §18, §21, §22
- Reference: `docs/commercial.md` "Control Plane Density", "Billing Infrastructure"
- Reference: `.claude/rules/testing.md`, `.claude/rules/development.md`, `CLAUDE.md` (Result alias)

## Open questions surfaced for user

None blocking handoff. DESIGN wave can proceed with the artifact set above.

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial DISCUSS wave decisions for phase-1-foundation. |
