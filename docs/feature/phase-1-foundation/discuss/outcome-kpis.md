# Outcome KPIs — phase-1-foundation

## Feature: phase-1-foundation

### Objective

Overdrive platform engineers can run `cargo xtask dst` on a clean clone in under a minute, trust that every named invariant actually catches real bugs, and reproduce any failure bit-for-bit from the printed seed — by the end of the first walking-skeleton release.

### Outcome KPIs (feature level)

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Overdrive platform engineer | Runs `cargo xtask dst` on a clean clone and sees green invariants | 100% of clean-clone runs green within 60s wall-clock on an M-class laptop | N/A (greenfield) | xtask reports wall-clock per run; CI job duration for the DST step | Leading — secondary |
| K2 | CI | Blocks any PR that introduces `Instant::now()` / `rand::random()` / `tokio::net::*` in a core crate | 100% of smuggling attempts blocked; 0% false positives on wiring crates | N/A (greenfield) | Deliberate regression PR seeded weekly; inspection of CI run results | Leading — secondary |
| K3 | Overdrive platform engineer | Reproduces a red run bit-for-bit from the printed seed | 100% of red runs reproduce on the same git SHA and toolchain; same invariant, same tick | N/A (greenfield) | DST self-test runs the harness twice per seed and diff-compares | Leading — secondary |
| K4 | Operator running LocalStore (future) | Observes a control plane starting within the whitepaper-claimed envelope | Cold start < 50ms; RSS < 30MB under empty-store conditions | Whitepaper claim "~30MB RAM" | Micro-benchmark in the LocalStore crate; values asserted in a test | Leading — primary |
| K5 | Overdrive platform engineer | Reads every identifier type in `overdrive-core` and sees a complete newtype (FromStr, Display, serde, validating constructor) | 100% of identifiers listed in whitepaper §4 + §8 + §11 are newtypes; 0 `String`-as-identifier on the public API | N/A (greenfield) | Static inspection via a test macro or clippy lint scanning the public API | Leading — secondary |
| K6 | Overdrive platform engineer | Round-trips a snapshot through `LocalStore` and reads the same bytes back | 100% of snapshots are bit-identical after `export → bootstrap_from → export` | N/A (greenfield) | proptest in overdrive-core/tests asserting byte equality | Leading — secondary |

### Metric Hierarchy

- **North Star**: a clean-clone `cargo xtask dst` run is green, under 60s, with a printed seed that reproduces (K1 ∧ K3).
- **Leading Indicators**: K2 (lint gate effectiveness), K5 (newtype completeness), K6 (snapshot round-trip).
- **Guardrail Metrics**: K4 (LocalStore density — must not regress; the commercial density argument depends on it). DST harness wall-clock — must not exceed 60s without explicit scope change. Lint gate false-positive rate — must stay at 0 (false positives train engineers to bypass the gate).

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| K1 | xtask DST summary output | CI job log; local xtask output | Every PR; every dev run | platform engineer + CI |
| K2 | CI lint step result | CI job log + deliberate regression test | Every PR | CI |
| K3 | xtask DST self-test | Automated twin-run diff in the harness | Every PR | CI |
| K4 | `criterion` bench + runtime RSS probe | Micro-benchmark in tests, failing on regression | Every PR touching overdrive-core store code | CI |
| K5 | Static test asserting newtype completeness | Test in overdrive-core | Every PR touching overdrive-core | CI |
| K6 | proptest in overdrive-core/tests | Property-based test | Every PR touching overdrive-core | CI |

### Hypothesis

We believe that shipping `IntentStore` + `ObservationStore` + the six nondeterminism traits + a turmoil-based DST harness + a CI lint gate as a single walking-skeleton release will achieve a foundation that every subsequent Overdrive feature can build on with confidence.

We will know this is true when **a Overdrive platform engineer can run `cargo xtask dst` on a clean clone, see green invariants within 60s, and — on the first red run — reproduce the failure bit-for-bit from the printed seed**. If any of those three properties fails, the whitepaper §21 claim is not real, and every later phase is shipping on sand.

### Smell Tests

| Check | Status | Note |
|---|---|---|
| Measurable today? | Yes | Every KPI has an automated measurement path in CI or in a test. |
| Rate not total? | K1/K2/K3 are rates (percentage of runs/PRs); K4/K5/K6 are binary per-PR signals (pass/fail). Acceptable for a greenfield walking skeleton — rates become meaningful after accumulation. |
| Outcome not output? | K1, K2, K3 target engineer behaviour under the harness; K4 targets the operator experience the control-plane density commercial claim depends on. Not feature-delivery checkboxes. |
| Has baseline? | Greenfield — no prior implementation. Whitepaper claims serve as the baseline target where one exists (K4). |
| Team can influence? | Yes — every KPI is a direct consequence of code the platform team writes. |
| Has guardrails? | Yes — DST wall-clock ceiling, LocalStore RSS ceiling, lint-gate false-positive rate. |

## Handoff to DEVOPS

The platform-architect needs these from this document to plan instrumentation:

1. **Data collection requirements**: CI job logs capturing seed, wall-clock, pass/fail per invariant; micro-benchmark outputs for LocalStore cold start and RSS.
2. **Dashboard/monitoring needs**: CI dashboards tracking DST wall-clock trend, lint-gate fire rate, and snapshot round-trip bytes-identical pass rate.
3. **Alerting thresholds**: DST wall-clock > 60s on main branch triggers a platform-team alert; any lint-gate false positive triggers an immediate investigation.
4. **Baseline measurement**: LocalStore cold-start and RSS benches run on a reference VM at release tagging time to establish the published baseline.

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial KPIs for phase-1-foundation DISCUSS wave. |
