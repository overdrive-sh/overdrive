# Outcome KPIs — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISCUSS / Phase 3
**Owner**: Luna
**Date**: 2026-04-30

KPIs trace to the validated job and ODI outcomes from
`diverge/job-analysis.md`. Each KPI has a measurement method and a
baseline.

---

## Feature-level KPIs (rolling up to ODI outcomes)

### KPI-01 — Time to first NDJSON event (informational, not a hard SLO)

- **Who**: Ana / inner-loop operator on streaming submit.
- **Does what**: receives the first response line from the CLI.
- **By how much**: ≤ 200 ms p95 on a healthy local control plane.
- **Measured by**: timestamp delta between CLI POST send and CLI
  receipt of the first NDJSON line. Captured in the streaming-submit
  acceptance test.
- **Baseline**: N/A — the streaming surface does not exist today.
- **ODI traceability**: outcome 1 (time to know if spec converged)
  emotional contract floor. If this slips, the operator wonders if
  the CLI hung; the entire trust story is at risk.

### KPI-02 — Submit-with-bad-binary surfaces failure inline (boolean)

- **Who**: Ana / debugging operator on the regression-target case.
- **Does what**: sees the streaming submit close with a
  `ConvergedFailed` event naming the verbatim driver error AND the
  CLI exit code is non-zero.
- **By how much**: TRUE on the regression-target acceptance test.
  Not a percentage; a boolean defended by a named test.
- **Measured by**: integration test that submits a spec with a
  non-existent binary path, captures the CLI exit code and the
  terminal event payload, asserts exit==1 and the driver error is
  present in both stream and snapshot.
- **Baseline**: today, exit 0 on broken-binary submit and `alloc
  status` shows nothing useful. This KPI explicitly inverts that.
- **ODI traceability**: outcomes 2 (silent-accept), 3 (time to
  identify reason), 6 (distinguish "not yet" from "failed"). This
  is the single load-bearing KPI for the feature; its failure means
  the feature has not delivered.

### KPI-03 — `alloc status` snapshot field count

- **Who**: Ana / second-day inspection operator.
- **Does what**: identifies allocation state, last transition,
  failure cause, and reconciler retry posture from one command.
- **By how much**: ≥ 6 actionable fields rendered (state,
  resources, started_at, last_transition.from→to,
  last_transition.reason, last_transition.source). Failed-case
  rendering also includes `error:` and `restart_budget`.
- **Measured by**: visual inspection against the journey TUI
  mockups; AC #1 in US-05 enumerates the field set.
- **Baseline**: today, 1 field (`Allocations: N`).
- **ODI traceability**: outcomes 3, 5 (time to identify reason;
  likelihood of re-deriving state from sparse output).

### KPI-04 — Failure-reason coherence across surfaces (boolean)

- **Who**: Ana / debugging operator using both surfaces.
- **Does what**: never sees two different diagnoses for one event.
- **By how much**: streaming `LifecycleTransition.reason` ==
  snapshot `last_transition.reason` byte-for-byte; streaming
  `ConvergedFailed.error` == snapshot per-row `error`
  byte-for-byte.
- **Measured by**: integration test in Slice 2 that captures both
  outputs for the broken-binary case and asserts string equality.
- **Baseline**: N/A (the streaming surface does not exist today).
- **ODI traceability**: cross-cutting; protects the "told the
  truth" emotional promise.

### KPI-05 — `--detach` exit time

- **Who**: CI / automation operators.
- **Does what**: completes `submit --detach` (or the auto-detached
  pipe equivalent) without waiting on convergence.
- **By how much**: CLI exit ≤ 200 ms p95 on a healthy local control
  plane.
- **Measured by**: regression test that runs `submit --detach` and
  measures wall-clock from invocation to exit.
- **Baseline**: today's submit exits ≤ ~100 ms; this KPI ensures
  the streaming-default introduction doesn't accidentally regress
  `--detach` performance.
- **ODI traceability**: dissenting case from DIVERGE — preserves
  CI/automation use case as first-class.

---

## ODI outcomes → KPI map

| ODI outcome | Severity | KPI(s) addressing it |
|---|---|---|
| 1 — Time to know if spec converged | Severely under-served (17.0) | KPI-01, KPI-02 |
| 2 — Likelihood of silent-accept-while-failing | Severely under-served (16.5) | KPI-02 |
| 3 — Time to identify failure reason | Severely under-served (17.0) | KPI-02, KPI-03 |
| 4 — Effort to observe transitions without external tools | Under-served (13.5) | KPI-01 (streaming is the surface), KPI-03 (snapshot for second-day) |
| 5 — Likelihood of re-deriving state from sparse output | Severely under-served (15.5) | KPI-03 |
| 6 — Time to distinguish "not yet" from "failed" | Severely under-served (16.0) | KPI-02 (exit-code contract makes this binary) |

All six ODI outcomes have at least one KPI mapping.

---

## Notes for DEVOPS

The streaming-submit feature does not introduce new telemetry
infrastructure beyond what the lifecycle reconciler and
ObservationStore already produce. The KPIs above are validated by
acceptance tests at PR time, not by a runtime telemetry pipeline.

If DEVOPS wants to track KPI-01 (200 ms first-event budget) in
production-like CI runs over time, the natural surface is the
streaming-submit acceptance test's timing measurement — emitted as
a structured log line that a future trend tracker can ingest. Phase
1 does not require this; the AC is enough.

KPI-02, KPI-03, KPI-04 are boolean and asserted at PR time.

KPI-05 is regression-target — measured by a single timing
assertion, not a histogram.
