# Upstream Issues — phase-1-control-plane-core (DISTILL)

**Wave**: DISTILL (acceptance-designer)
**Date**: 2026-04-23

---

## Summary

**No upstream issues surfaced.** Reconciliation between DISCUSS and
DESIGN waves passed with zero contradictions (`wave-decisions.md`
§Reconciliation). All user-story acceptance criteria mapped to
scenarios without AC edits. All DESIGN ADR decisions (0008–0015)
compose into `test-scenarios.md` without forcing a DISCUSS rewrite.

## Soft observations (non-blocking)

The two items below do not block handoff. They are recorded so
downstream waves can close them when the artifacts materialise.

1. **Product KPI contracts file absent** —
   `docs/product/kpi-contracts.yaml` does not exist. Feature-level
   KPIs K1–K7 from `discuss/outcome-kpis.md` drive scenario tagging.
   When a product-level KPI contracts file lands (likely post-Phase 1),
   Sentinel (acceptance-designer-reviewer) may re-audit and propose
   `@kpi-contract` tag additions. No change to DISCUSS or DESIGN is
   required now.

2. **DEVOPS wave not yet run** — `docs/feature/phase-1-control-plane-core/
   devops/` does not exist. DISTILL applied the default environment
   matrix (`clean`, `with-pre-commit`, `with-stale-config`). When the
   platform-architect runs the DEVOPS wave, any environment refinement
   is additive and cannot invalidate scenarios here — the walking
   skeletons use `tempfile::TempDir` scratch directories and synthesise
   their own clean environment.

## Decisions that were candidates for upstream change but weren't

For the record, the following were considered and rejected as upstream
changes:

- **`JobSpec` placeholder in `observation_store.rs`** — DISCUSS
  Key Decision 6 + ADR-0011 resolved this as an intra-feature cleanup
  (delete the vestigial struct or rename). No DISCUSS-AC change needed;
  the crafter handles at implementation. No upstream ticket filed.
- **Slice 4 "whole vs split"** — ADR-0013 §7 confirmed ship-whole as
  the default, with split available as a crafter-time escape hatch.
  DISCUSS Key Decision 7 already framed this possibility. No upstream
  change.
- **ObservationStore impl choice** — DISCUSS Key Decision 8 named the
  three options; ADR-0012 picked `SimObservationStore` reuse. DISCUSS
  AC does not specify the impl — the decision fits inside DISCUSS's
  stated optionality. No upstream change.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial upstream-issues record for phase-1-control-plane-core DISTILL. Placeholder — no issues to raise. |
