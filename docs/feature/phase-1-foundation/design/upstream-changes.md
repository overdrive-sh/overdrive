# Upstream Changes — phase-1-foundation (DESIGN wave)

Back-propagation record for assumptions that changed during the DESIGN
wave. Per the wave protocol, prior-wave documents are preserved verbatim;
deltas are recorded here so the DISCUSS artifacts remain the
as-of-DISCUSS truth while DESIGN records what it actually committed to.

---

## K4 — Control-plane density

### Original DISCUSS text (verbatim from `discuss/outcome-kpis.md`)

Row K4 of the Outcome KPIs table:

> | K4 | Operator running LocalStore (future) | Observes a control plane starting within the whitepaper-claimed envelope | Cold start < 50ms; RSS < 30MB under empty-store conditions | Whitepaper claim "~30MB RAM" | Micro-benchmark in the LocalStore crate; values asserted in a test | Leading — primary |

And the Guardrail line beneath it:

> **Guardrail Metrics**: K4 (LocalStore density — must not regress; the commercial density argument depends on it).

And the Measurement Plan row:

> | K4 | `criterion` bench + runtime RSS probe | Micro-benchmark in tests, failing on regression | Every PR touching overdrive-core store code | CI |

### New assumption (as-of-DESIGN)

**K4 is reframed from a Phase 1 acceptance gate to a Phase 2+ commercial
guardrail.** Phase 1 does not benchmark `LocalStore` cold-start or RSS
and does not gate CI on either figure.

### Rationale

Phase 1 is a walking skeleton that proves the DST harness works
end-to-end. It has no production tenancy context, no control-plane
workload running against it, and no operator surface that can observe a
"cold start." The `<50ms` / `<30MB` targets originate in
`docs/commercial.md` under the "Control Plane Density" claim — that
claim is about the density advantage a tenant-facing control plane
confers when running on the infrastructure layer, which Phase 1
deliberately does not deliver.

Measuring `LocalStore` alone in isolation would produce a number that
is:

1. Not the thing the commercial claim is about (the claim is control
   plane, not a bare embedded KV in a test binary).
2. Sensitive to CI runner class and test-binary link overhead in ways
   that invite flaky gates and bypass culture — exactly what K2 exists
   to avoid.
3. Locking Phase 1 into thresholds that belong to a Phase 2+ scope where
   the actual control-plane process exists to be measured.

Reframing K4 as a Phase 2+ guardrail keeps the commercial argument
honest: the number gets measured when the thing being claimed is the
thing being benchmarked.

### Effect on DESIGN artifacts

- `brief.md` §11 (Quality attributes) — rows that previously mapped to
  K4 now read "Phase 2+ guardrail — `commercial.md` 'Control Plane
  Density'" with a cross-reference to this file.
- `brief.md` handoff annotation to platform-architect — K4 removed from
  the per-PR alerting threshold list; DST wall-clock (K1) and lint-gate
  false-positive rate (K2) remain.
- ADR index is unchanged — no ADR was authored for K4; the reframe is
  an assumption delta, not a new architectural decision.

### Effect on DISCUSS artifacts

**None.** Per the back-propagation rule, `discuss/outcome-kpis.md` is
preserved verbatim. Readers reconciling the two documents should treat
this file as the governing source for the K4 scope question going into
DISTILL and DELIVER.

### Owner

DESIGN wave (Morgan). Will be re-examined in the Phase 2 DISCUSS wave
when the control-plane process that the commercial claim applies to
enters scope.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-22 | K4 reframed from Phase 1 acceptance gate to Phase 2+ commercial guardrail. |
