# Outcome KPIs — phase-1-control-plane-core

## Feature: phase-1-control-plane-core

### Objective

Overdrive platform engineers can run `overdrive job submit <spec>` against a local walking-skeleton control plane, see the spec commit through the real `IntentStore`, observe a reconciler primitive registered with storm-mitigation alive, and see `overdrive alloc status` round-trip honestly — by the end of the first walking-skeleton release for this feature.

### Outcome KPIs (feature level)

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Overdrive platform engineer | Runs `overdrive job submit` → `overdrive alloc status` against a local control plane and sees the spec digest round-trip byte-identical | 100% of round-trips produce matching spec digests on any valid input | N/A (greenfield — the CLI stub does not round-trip today) | Acceptance test in `tests/acceptance/` running CLI subcommands against a real server; proptest variant for N valid inputs | Leading — secondary |
| K2 | CI + Overdrive platform engineer | Submits an invalid spec and sees the server reject it before any IntentStore write | 100% of validating-constructor failures surface as HTTP `400 Bad Request` with a structured JSON error body and no store side effect | N/A (greenfield) | Negative acceptance test asserts no new IntentStore entry on malformed input + HTTP status is `400 Bad Request` | Leading — secondary |
| K3 | Overdrive platform engineer | Observes the LocalStore commit index strictly increase across successive submits | 100% monotonic across any sequence of submits; zero regressions across restarts of the same data directory | N/A (greenfield) | Property test: N successive submits, commit_index strictly increasing | Leading — secondary |
| K4 | Overdrive platform engineer | Writes a reconciler and has the DST harness prove `reconciler_is_pure` + `duplicate_evaluations_collapse` | Both DST invariants pass on every run; `at_least_one_reconciler_registered` always holds at boot | N/A (greenfield — no reconciler primitive exists today) | `overdrive-sim` invariant catalogue adds three new invariants; `cargo xtask dst` gates on them | Leading — primary |
| K5 | Overdrive platform engineer | Reads `overdrive cluster status` and sees the reconciler registry + broker counters | 100% of boots show the noop-heartbeat reconciler registered; broker counters are observable (not stubbed) | N/A (greenfield) | Acceptance test running `cluster status` against a real server asserts the expected section layout + counter values | Leading — secondary |
| K6 | Overdrive platform engineer / operator | Encounters an error path and reads an actionable message | 100% of error paths tested (connection refused, invalid spec, unknown JobId, server internal) render output answering "what / why / how to fix" | N/A (greenfield — CLI stub logs a warning and exits 0) | Review of every error-rendering call site + acceptance test per error class | Leading — secondary |
| K7 | Overdrive platform engineer | Encounters an empty observation (zero nodes, zero allocations) and sees an explicit empty state | 0 silent blank outputs across `node list`, `alloc status`, `cluster status` | N/A (greenfield) | Acceptance test for each command against a fresh cluster asserts empty-state text matches expectations | Leading — secondary |

### Metric Hierarchy

- **North Star**: the round-trip `overdrive job submit <file>` → `overdrive alloc status --job <id>` prints a spec digest byte-identical to what the input file produces locally, AND the reconciler primitive is alive (K1 ∧ K4).
- **Leading Indicators**: K2 (validating-constructor gate), K3 (commit_index monotonicity), K5 (reconciler registry visibility), K6 (actionable errors), K7 (honest empty states).
- **Guardrail Metrics**: none new in Phase 1 — the phase-1-foundation guardrails (DST wall-clock < 60s, lint-gate false-positive rate at 0, snapshot round-trip byte-identical) remain in force and must not regress.

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Acceptance test in `crates/overdrive-cli/tests/acceptance/` (or equivalent location DESIGN picks) | Round-trip CLI subprocess against a real server | Every PR touching the CLI, server, or `overdrive-core` | CI |
| K2 | Negative test in server handlers | Assert no IntentStore entry after rejected submit; assert HTTP status is `400 Bad Request` and the JSON error body names the offending field | Every PR touching the submit path | CI |
| K3 | Property test in `overdrive-store-local` or the server crate | Proptest over N successive submits, assert monotonic commit_index | Every PR touching LocalStore or the submit handler | CI |
| K4 | `cargo xtask dst` invariant catalogue | Three new invariants in `overdrive-sim::invariants` + their evaluators | Every PR | CI |
| K5 | Acceptance test running `cluster status` against a real server | Parse output; assert reconciler section + counters | Every PR touching the reconciler runtime or cluster status | CI |
| K6 | Per-error-class acceptance test | Subprocess CLI against a server in the relevant failure state; assert output pattern | Every PR touching error rendering | CI |
| K7 | Per-empty-state acceptance test | Subprocess CLI against a fresh cluster; assert output does NOT match "blank table" pattern and DOES match expected empty-state text | Every PR touching any CLI handler | CI |

### Hypothesis

We believe that shipping the aggregate structs + the REST + OpenAPI service surface + real API handlers + the reconciler primitive + real CLI handlers as a single walking-skeleton release will achieve a control-plane-core foundation that every subsequent Phase 1 feature and every Phase 2+ reconciler can build on with confidence.

We will know this is true when **a Overdrive platform engineer can run `overdrive job submit` against a local control plane, see the commit index echoed back, see the reconciler primitive registered via `overdrive cluster status`, and see `overdrive alloc status` round-trip the spec digest byte-identical** — AND when the three new DST invariants (`at_least_one_reconciler_registered`, `duplicate_evaluations_collapse`, `reconciler_is_pure`) are green on every PR.

### Smell Tests

| Check | Status | Note |
|---|---|---|
| Measurable today? | Yes | Every KPI has an automated measurement path in CI or in the DST harness. |
| Rate not total? | K1–K3 and K6–K7 are rate-shaped (percentage of rounds / PRs / commands); K4–K5 are binary per-PR signals (pass/fail). Acceptable for a greenfield walking skeleton — rates become meaningful as the invariant catalogue and test matrix accumulate. |
| Outcome not output? | K1, K3, K4 target engineer / operator behaviour against the real walking skeleton; K6 / K7 target operator experience of the CLI. Not feature-delivery checkboxes. |
| Has baseline? | Greenfield — no prior implementation (CLI stub warns and exits; no server exists). Each KPI's "baseline" row is explicit about this. |
| Team can influence? | Yes — every KPI is a direct consequence of code the platform team writes in this feature. |
| Has guardrails? | The phase-1-foundation guardrails remain — must not regress (DST wall-clock, lint-gate false-positive, snapshot round-trip). |

## Handoff to DEVOPS

The platform-architect needs these from this document to plan instrumentation:

1. **Data collection requirements**: CI job logs capturing the three new DST invariants' pass/fail + per-invariant tick; round-trip acceptance test output (the CLI subprocess + server); HTTP error-status table (400/404/409/500) asserted per PR; OpenAPI schema-lint gate pass/fail per PR.
2. **Dashboard/monitoring needs**: CI dashboards tracking invariant pass rate over time; flakiness signal on the round-trip acceptance test (should be 0% flake).
3. **Alerting thresholds**: the three new DST invariants must hold on every merged PR; any regression is a platform-team alert.
4. **Baseline measurement**: none new — the phase-1-foundation baselines (DST wall-clock < 60s on clean clone, snapshot round-trip byte-identical) continue to apply.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial KPIs for phase-1-control-plane-core DISCUSS wave. |
| 2026-04-23 | Transport pivot: K2 measurement reframed around HTTP `400 Bad Request` (was gRPC `InvalidArgument`); hypothesis / handoff text updated to "REST + OpenAPI service surface" and to include schema-lint gate in the CI signals. All KPI targets are otherwise transport-neutral and unchanged. |
