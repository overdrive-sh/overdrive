# Roadmap Review — phase-2-xdp-service-map

| Field | Value |
|---|---|
| Review ID | `roadmap-rev-2026-05-05-phase2.2-xdp-service-map` |
| Reviewer | Atlas (`nw-solution-architect-reviewer`, Haiku 4.5 inherited) |
| Date | 2026-05-05 |
| Artifact | `docs/feature/phase-2-xdp-service-map/deliver/roadmap.json` |
| Verdict | **APPROVED** — handoff-ready for DELIVER |

## Verdict summary

```
Blocking issues:           0
Suggestions (non-blocking): 4
Praise items:              7 (exceptional AC specificity, locked-decision
                              compliance, risk-proportional mitigation,
                              effort credibility, scenario coverage,
                              mutation discipline, tier mapping coherence)
Compliance:                13/13 review dimensions PASS
```

## Dimensions checked (all PASS)

1. **Slice→phase fidelity** — every phase mirrors `slices/slice-N-*.md` 1:1; dependency + parallel-with metadata matches DISCUSS Decision 4.
2. **Scenario→step coverage** — all 30 `S-2.2-NN` scenarios referenced exactly once across the 30 steps; tier distribution 8/8/9/5 matches `distill/test-scenarios.md`.
3. **AC quality** — every step's `criteria` is behavioral, measurable, and artifact-grounded (test paths, invariant names, kill-rate thresholds, veristat deltas, ADR-NNNN citations). No vague AC.
4. **`implementation_scope` accuracy** — every path is either an existing scaffold (verified against commit `5e9ca73`) or a documented RED-scaffold target. No fabricated paths.
5. **ADR + decision traceability** — every step touching a locked surface cites its governing ADR (0038 substrate, 0040 three-map+sanity, 0041 Maglev+REVERSE_NAT+endianness, 0042 hydrator+Action+`service_hydration_results`).
6. **Locked-decision compliance** — Q1..Q7, Q-Sig, Q-Action, Drifts 1/2/3, `MaglevTableSize=u32` all propagate without alternative invention.
7. **Project-rules propagation** — mutation policy per-step (28 with crate targets, 2 justified skips); `integration-tests` gating; macOS-Lima discipline acknowledged in 01-03 / 02-04 / 05-04 etc.; RED-scaffold flip narrative in every step description.
8. **Phase dependency + parallelism graph** — `01 → 02 → 03 → 04 → {05 ∥ 06} → 07; 08 ∥ {03..06}, 08 deps on 02` matches DISCUSS Decision 4. Phase 6 correctly depends on Phase 4 (verifier-budget baseline).
9. **Cross-cutting deliverables** — `Action::DataplaneUpdateService` + `service_hydration_results` table + `action_shim.rs` directory-modulisation in 08-01; `SimDataplane` HashMap→BTreeMap migration in 02-01; endianness conversion site in 05-03; `MaglevTableSize` newtype in 04-01.
10. **Effort budgeting honesty** — 112h overrun (vs 65–75h target) explained and credible (Tier 3/4 infrastructure complexity); per-step hours sampled and realistic.
11. **Risk traceability** — all 8 DISCUSS risks (R1..R8) grounded in roadmap step `risk` fields with mitigation narratives.
12. **Tier mapping consistency** — newtype/enum steps classified Tier 1, programs Tier 2, real-kernel Tier 3, gates Tier 4 — internally coherent. (Documentation suggestion below.)
13. **Step decomposition ratio** — 30 steps / ~35 production files ≈ 0.86 (well under 2.5 cap).

## Suggestions (non-blocking)

| # | Concern | Recommendation |
|---|---|---|
| S1 | Tier 2/3 integration-tests gating is implicit (lives in `test-scenarios.md`), not explicit in every step's first AC | Add "Integration test gated `integration-tests` feature" as AC #1 of every Tier 2/3 step for uniform clarity |
| S2 | `cargo xtask bpf-build` prerequisite for Tier 3 phases (2..6) is implicit in xtask infrastructure | Note it once in the Phase 02 overview for pedagogical clarity |
| S3 | `BackendSetFingerprint` alias + `fingerprint(...)` rkyv-archived helper definition isn't traced to a step explicitly (referenced in 08-02 AC, but defined where?) | Add a one-line implementation_scope entry on 08-01 (`crates/overdrive-core/src/dataplane/fingerprint.rs`) calling out the alias + helper definition |
| S4 | `tier` field semantics: newtype steps are Tier 1 even though they unblock Tier 2/3 scenarios | Add a one-line note: "`tier` reflects the step's own test harness, not the scenario's final tier" |

All four are documentation refinements — they do not affect roadmap validity or execution.

## Praise (verbatim)

> "Every criterion is behavioral, measurable, and artifact-grounded.
>  Scenario-level test names, invariant names, and verifier thresholds
>  are load-bearing context."

> "Locked-decision compliance: all seven DISCUSS constraints (Q1–Q7,
>  Q-Action, Q-Sig, Drifts 1/2/3) propagate cleanly through the
>  roadmap with ADR citations."

> "112h overrun is explained and justified (Tier 3/4 infrastructure
>  complexity); per-step hours are realistic."

## Handoff confidence

**HIGH.** The architect and crafter teams have the specificity needed to execute. Per `.nwave/des-config.json` rigor (`mutation_enabled=true`), the per-step mutation gate fires on each PR; the per-PR final gate at phase boundary (`cargo xtask mutants --diff origin/main --features integration-tests --package <crate>`) is implicit and runs without further roadmap support.

Starting step: `01-01` (per `S-2.2-01` — the only non-`@pending` scenario in `test-scenarios.md`).

---

*This review file complements `roadmap.json` (strict JSON; cannot
carry an embedded review block). Pair this file with the JSON for
the full DELIVER-wave handoff package.*
