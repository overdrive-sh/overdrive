# Outcome KPIs — service-health-check-probes

## Feature

A health-check primitive (HTTP / TCP / Exec probes; startup / readiness / liveness roles) that closes RCA root cause A (kernel-accepted exec is NOT operator-meaningful liveness) for the Service workload kind. Per GH #170.

## Objective

Within one release cycle, every operator submitting a Service-kind workload receives a wire signal that reflects operator-meaningful liveness — never the kernel's bare-fork acceptance — so the platform earns operator trust on the most common control-surface interaction (`overdrive job submit`).

## Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Operator submitting a Service whose workload either crashes within startup deadline or whose startup probe never passes | Sees `ServiceSubmitEvent::Failed { reason: StartupProbeFailed \| EarlyExit }` (NOT `Stable` and NOT bare `Running` claim) within `startup_deadline + 1 tick` | ≥99 of 100 such submissions (≥99%) | 0% (Phase 1 always reports Stable-equivalent for the kernel-accepted window — the RCA-A failure mode) | Integration test reshapes `examples/coinflip.toml` as Service with never-passing startup probe and parses CLI output | Leading (Outcome) |
| K2 | Service-kind backend whose readiness probe transitions Pass → Fail | Has `Backend.healthy = false` reflected in the dataplane fingerprint within 1 reconciler tick (≤ tick_period_ms after the ProbeResultRow lands) | ≥99% within 1 tick; 100% within 2 ticks | N/A (readiness probe doesn't exist yet; current Backend.healthy is always true at Service kind level) | Acceptance test: assert `compute_fingerprint(vip, backends)` value changes between pre-fail tick and post-fail tick | Leading (Outcome) |
| K3 | Service-kind alloc whose liveness probe fails consecutively past threshold | Is restarted (gets a new alloc with `restart_count` incremented) within 1 reconciler tick | ≥99% within 1 tick | N/A | Acceptance test: stand up Service with HTTP liveness probe, kill the binary's HTTP listener, assert `AllocStatusRow.restart_count` increments | Leading (Outcome) |
| K4 | Operator running `overdrive alloc status --job <service-id>` against a Service with probes | Sees a "Probes:" section listing every declared/inferred probe with its current status and last-fail reason | 100% of Service allocs with probes; 0% of Job/Schedule allocs | N/A (Probes section doesn't exist) | Snapshot tests on render output | Leading (Outcome) |
| K5 | Operator who declares `[[health_check.*]]` under `[job]` or `[schedule]` in TOML | Gets a parse-time error naming the kind AND naming the right primitive ("Job has no readiness question; on completion is enough.") | 100% of misshapen specs | 0% (TOML deserialiser silently accepts unknown fields or errors with generic "unknown field") | Acceptance test against fixture specs | Guardrail |

## Guardrail metrics (must NOT degrade)

| Metric | Threshold | Source | Rationale |
|---|---|---|---|
| Phase-1 baseline Service submit latency (no probes declared, default inferred TCP probe, listener already bound) | Stable wire event arrives within 1.5 × current "first Running row" latency. Current baseline: ~p99 50ms for `/bin/sleep 3600` style fixture; new p99 budget ≤ 75ms with the inferred default-TCP probe. | streaming_submit happy_path integration test latency histogram | The fix must not turn the common case into a slow case. |
| ProbeRunner CPU overhead per alloc | ≤ 0.5% of one core sustained per Service alloc with 3 declared probes at default intervals (2s) | Worker process CPU profile under sustained 10-alloc fixture | Probe runner is per-alloc per-machine; runaway cost defeats Phase 1's 30MB-tenant claim. |
| `cargo nextest run` wall-clock | Total suite time grows by ≤ 10% over baseline | CI nextest summary | The 4 tiers of tests this feature adds (parser, runner, reconciler, render) shouldn't double the test suite. |
| ObservationStore row count per alloc | LWW per `(alloc_id, probe_idx)`; total ProbeResultRows = N_probes per alloc, NOT N_probes × N_ticks | redb size measurement on a 1-hour soak test | Append-mode rows would be unbounded; LWW MUST be the structural invariant per `.claude/rules/development.md` § "Persist inputs, not derived state". |

## Metric Hierarchy

- **North Star**: K1 — Service-submit honesty rate. This is the metric the RCA exists to move from 0% to ≥99%.
- **Leading Indicators**: K2 (readiness → dataplane convergence), K3 (liveness → restart effectiveness), K4 (operator visibility coverage).
- **Guardrail**: K5 (misshapen-spec named-error rate) — prevents the secondary failure mode of "operator declared probes on a kind that doesn't support them" from silently no-op'ing.

## Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Integration test `crates/overdrive-cli/tests/integration/service_honest_stable.rs` (NEW) reshapes coinflip.toml as a Service with never-passing startup probe; asserts CLI prints `Failed (StartupProbeFailed | EarlyExit)` for ≥99 of 100 deterministic seeds | Per-PR CI green/red | Every PR | acceptance-designer (DISTILL wave) |
| K2 | Acceptance test asserts `compute_fingerprint(vip, [backend with .healthy = false])` differs from the same call with `.healthy = true` AND the reconciler flips the bit within 1 tick of a Fail ProbeResultRow | Per-PR CI green/red | Every PR | acceptance-designer |
| K3 | Acceptance test against a Service whose liveness probe HTTP endpoint can be made to return 500 on demand; assert `restart_count` increments within 1 tick of N consecutive fails | Per-PR CI green/red | Every PR | acceptance-designer |
| K4 | Snapshot tests on CLI render output for: Service with probes (expect section), Job (expect no section), Schedule (expect no section) | Per-PR CI green/red | Every PR | acceptance-designer |
| K5 | Per-fixture parse error tests: `[job] + [[health_check.startup]]` and `[schedule] + [[health_check.startup]]` fixtures both produce the named error string | Per-PR CI green/red | Every PR | acceptance-designer |

## Hypothesis

We believe that adding declarative HTTP / TCP / Exec health-check probes scoped per-role (startup, readiness, liveness) for the Service workload kind will achieve K1 ≥99% honest-submit rate (vs current 0%). We will know this is true when operators submitting Service workloads with non-trivially-failing entrypoints see `Failed` (with actionable reason) rather than the kernel's accidental `Running` claim, within `startup_deadline + 1 tick` of submit.

## Out-of-band considerations

- **Probe runner overhead distribution.** K1 / K2 / K3 are pass/fail per submission; the platform-architect (DEVOPS) wave will need a histogram for the operational p99 of probe runner work-per-tick. Not in this DISCUSS scope but should appear in instrumentation requirements at DEVOPS.
- **Time-to-Stable distribution.** Stable's `settled_in` Duration is a per-submission measurement and could be aggregated into a per-Service histogram (P50 / P95 startup time). Out of scope for K1 — K1 is the binary honesty rate. A future operator analytics feature would consume `settled_in` as a leading indicator of slow-warming Services.
- **The 60s `streaming_cap` is unchanged by this feature.** Operators with Services that legitimately take >60s to settle will see `ConvergedFailed { Timeout }` on the wire and must inspect `alloc status` for the probe state. Surface as a known limitation in DESIGN wave; potential ADR amendment to make cap configurable per-spec is deferred.
