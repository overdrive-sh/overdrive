# Outcome KPIs — workload-kind-discriminator

## Changed Assumptions

- 2026-05-10 — folded in GH #164 (service listener spec shape). New KPI **K6**
  added below: byte-equality between submit echo and `alloc status` Listeners
  sections across 100 Service submits with pinned VIPs. K1's honesty-rate
  scope is **not** expanded to cover listener-related claims at this stage —
  the dataplane mapping from a kernel BPF entry to a Service listener is a
  Phase 2.2 concern (see `crates/overdrive-bpf/`), and K1 today measures
  Job-kind exit-code honesty over `examples/coinflip.toml`, which is
  orthogonal. Decision recorded here so a future KPI revisit can extend K1
  or add a separate K7 if the architect determines listener-mapping honesty
  needs measurement.

## Feature: workload-kind-discriminator

### Objective

Restore operator trust in `overdrive`'s submit and status surfaces by encoding workload
lifecycle kind in the spec, making the Phase 1 coinflip false-positive structurally
unrepresentable for Job kind, and giving operators kind-aware vocabulary that matches
the workload's actual semantics.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| K1 | Overdrive platform engineers submitting Jobs | receive a CLI verdict that matches the workload's kernel exit code | ≥99% honesty rate over 100 trials of `examples/coinflip.toml` | 0% (current CLI says "running" 100% of the time regardless of kernel exit) | Integration test that submits coinflip 100 times and asserts CLI exit code matches workload exit code | Leading (primary) |
| K2 | Overdrive platform engineers writing TOML specs | receive structured parser errors for invalid kind combinations | 100% of mixed-kind specs (`[service]+[job]`, `[schedule]+[service]`, `[schedule]` without `[job]`, missing `[exec]`) rejected with named guidance within 50ms p95 | n/a (validation does not exist) | Parser unit + integration tests timing the rejection path | Leading (secondary) |
| K3 | Overdrive platform engineers inspecting Failed Jobs | correctly identify the workload's exit code from `alloc status` output | ≥95% of operators in a usability check (5–10 operators) correctly state the exit code from a Failed Job's alloc status output | 0% (kind does not exist; per-attempt exit codes are not surfaced) | Usability check (small sample); stretch — automated parsing-from-fixtures regression test | Leading (secondary) |
| K4 | Overdrive platform engineers maintaining existing Service workflows | continue to use existing Service-shaped tests without behavioural regression | 100% of pre-feature Service integration tests pass after `[service]` migration | 100% pre-feature on legacy shape | CI run | Guardrail |
| K5 | Overdrive platform engineers planning recurring jobs | receive consistent deferral messaging across submit and alloc status | 100% byte-equality between submit echo deferral URL and alloc status deferral URL across all Schedule submits | n/a | Integration test asserting URL string equality | Leading (Schedule sub-feature) |
| K6 | Overdrive platform engineers declaring Service listeners | round-trip declared `(vip, port, protocol)` triples byte-identically through submit echo and `alloc status` | 100% byte-equality between submit echo Listeners section and `alloc status` Listeners section across 100 Service spec submits with pinned VIPs | n/a (listener fields do not exist today) | Integration test that submits 100 Service specs with pinned VIPs and asserts byte-equality of the two Listeners sections; parser unit tests for rejection paths (zero listeners, duplicate triple, unsupported protocol, port=0) | Leading (Listener sub-feature, folded in from #164) |

### Metric Hierarchy

- **North Star (K1 — honesty rate)**: this IS the bug fix; everything else exists to
  make this metric move from 0% to ≥99% AND keep it there.
- **Leading Indicators**: K2 (parser correctness), K3 (alloc status comprehension),
  K6 (listener round-trip byte-equality) predict K1's stability and the broader
  honesty surface — if any degrades, the platform's "what you declared = what we
  show" property weakens and operator trust erodes.
- **Guardrail Metrics**: K4 (no Service regression) — must NOT degrade. If existing
  Service tests fail post-feature, the rename mechanism is wrong and must be fixed
  before merge.

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|-------------|-------------------|-----------|-------|
| K1 | Integration-test assertion on coinflip workload | Automated test in `crates/overdrive-cli/tests/integration/job_submit.rs` (or new file) running coinflip 100×; assert CLI exit code ↔ workload exit code | Per CI run on every PR | DELIVER wave (acceptance-designer) |
| K2 | Parser unit + integration tests | `cargo xtask lima run -- cargo nextest run -p overdrive-cli` with timing assertions | Per CI run on every PR | DELIVER wave |
| K3 | Usability check (small sample); automated fixture parse | Manual check on first release; automated assertion that the rendered Exit column matches the persisted exit_code | Once at feature release; ongoing automated | PO + DELIVER wave |
| K4 | CI test results | `cargo xtask lima run -- cargo nextest run --workspace --features integration-tests` | Per CI run on every PR | CI |
| K5 | Integration-test string-equality assertion | New integration test in `crates/overdrive-cli/tests/integration/job_submit_schedule.rs` | Per CI run on every PR | DELIVER wave |
| K6 | Integration-test byte-equality assertion | New integration test in `crates/overdrive-cli/tests/integration/job_submit_service_listeners.rs` (or similar — architect to confirm) submitting 100 Service specs with pinned VIPs and asserting byte-equality between submit echo and `alloc status` Listeners sections; parser unit tests for the rejection paths | Per CI run on every PR | DELIVER wave |

### Hypothesis

We believe that introducing a `WorkloadKind` enum at the spec-parser boundary AND per-
kind streaming protocols (Service / Job / Schedule) for Overdrive platform engineers
will achieve a ≥99% honesty rate on the coinflip Job workload (vs. 0% today).

We will know this is true when **operators submitting `examples/coinflip.toml` see the
CLI's reported verdict and exit code match the workload's actual kernel exit code on
≥99% of 100 trials**, and when ≥95% of operators inspecting a Failed Job via
`alloc status` correctly identify the exit code from the rendered output.

## Cross-feature alignment

These KPIs feed into J-OPS-002's outcome statement *"the platform is honest about what
it does and does not know — no silent blank outputs, no fabricated placeholder rows"*.

The current state on the coinflip workload (0% honesty, fabricated "running" message)
is a direct violation of this clause. K1 measures the post-feature state; K2 + K3
ensure new code paths inherit the honesty by construction.

## Handoff to platform-architect (DEVOPS)

For instrumentation planning:

- **K1 instrumentation**: integration-test infrastructure — no production telemetry
  needed; the CI run IS the measurement.
- **K2 instrumentation**: parser-side timing — likely standard test framework timing;
  no new instrumentation.
- **K3 instrumentation**: deferred to ongoing usability work; not gating this feature.
- **K4 instrumentation**: existing CI; no change.
- **K5 instrumentation**: integration test string equality; no new infrastructure.
- **K6 instrumentation**: integration test byte equality across submit echo and
  `alloc status` Listeners sections; no new production telemetry.

No new production telemetry / dashboards / alerting is required to land this feature.
The measurement infrastructure is the test suite. (This is consistent with Phase 1's
walking-skeleton scope — operator-facing telemetry arrives in later phases.)
