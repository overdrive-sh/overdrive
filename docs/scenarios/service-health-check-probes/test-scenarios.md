<!-- markdownlint-disable MD024 -->

# Test Scenarios — service-health-check-probes

**Feature:** `service-health-check-probes`
**Wave:** DISTILL
**Author:** Quinn (nw-acceptance-designer)
**Date:** 2026-05-24

**Note on format**: per `.claude/rules/testing.md` — "No `.feature` files anywhere. All acceptance and integration tests are written directly in Rust using `#[test]` / `#[tokio::test]` functions." This document is a SPECIFICATION COMPANION for human readers; the Gherkin Given/When/Then blocks below are not parsed or executed. The executable tests live as Rust functions in `crates/{crate}/tests/{acceptance,integration}/*.rs` per the test-placement matrix in `feature-delta.md` § "Wave: DISTILL / [REF] Test placement".

Every Gherkin scenario below maps 1:1 to a Rust test function under the `S-SHCP-*` ID prefix (e.g. `S-SHCP-RECON-01` → `service_lifecycle_stable.rs::given_running_alloc_with_pass_startup_probe_when_reconcile_then_emits_stable_once`).

---

## Slice 01 — Walking Skeleton: Default TCP-connect startup probe end-to-end

**Stories**: US-01, US-08 (partial — establishes `EarlyExit` variant).
**KPI**: K1 (Service-submit honesty rate ≥99%).
**Crates touched**: `overdrive-core` (parser, observation row, reconciler types), `overdrive-control-plane` (`ServiceLifecycleReconciler`), `overdrive-worker` (`ProbeRunner` + `TokioTcpProber`), `overdrive-cli` (Stable/Failed render).

### S-SHCP-INFER-01 — `@walking_skeleton @driving_port @US-01 @real-io @adapter-integration`

Default TCP-connect startup probe inferred when Service spec has listeners and no probes.

```gherkin
Given Ana has authored payments-minimal.toml with:
  - a [service] block
  - one [[listener]] on port 8080
  - zero [[health_check.*]] sections
When the spec is parsed
Then the resulting ServiceSpec carries exactly one inferred ProbeDescriptor with:
  - role: Startup
  - mechanic: ProbeMechanic::Tcp { host: "0.0.0.0", port: 8080 }
  - inferred: true
  - timeout_seconds: 5
  - interval_seconds: 2
  - max_attempts: 30
```

Rust test: `crates/overdrive-core/tests/acceptance/health_check_toml_parse.rs::service_without_probes_with_listener_infers_default_tcp_startup_probe`.

### S-SHCP-INFER-02 — `@US-01 @opt-out`

Explicit empty array opts out of default inference.

```gherkin
Given a TOML containing `[[health_check.startup]] = []` (empty array)
When the spec is parsed
Then no probe descriptor is synthesised
And the Service preserves Phase-1 first-Running semantics
```

Rust test: `crates/overdrive-core/tests/acceptance/health_check_toml_parse.rs::service_with_empty_startup_array_opts_out_of_default_inference`.

### S-SHCP-RECON-01 — `@US-01 @driving_port @kpi K1`

Stable emitted when startup probe Pass + alloc Running.

```gherkin
Given a Service alloc 'alloc-payments-0' has:
  - AllocStatusRow.state = Running (started_at = T0)
  - one ProbeResultRow for probe_idx 0, role Startup, status Pass at T0+1.2s
And ServiceLifecycleView.stable_announced does NOT contain alloc-payments-0
When ServiceLifecycleReconciler::reconcile fires at T0+1.3s
Then exactly one Action::SetTerminalCondition { Stable { settled_in: 1.2s, witness: ProbeWitness { probe_idx: 0, role: Startup, mechanic_summary: "tcp 0.0.0.0:8080", inferred: true } } } is emitted
And the next-View's stable_announced contains alloc-payments-0
```

Rust test: `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs::given_running_alloc_with_pass_startup_probe_when_reconcile_then_emits_stable_once`.

### S-SHCP-RECON-02 — `@US-01 @dedup`

Stable dedup: once announced, no further emission for unchanged inputs.

```gherkin
Given ServiceLifecycleView.stable_announced contains alloc-payments-0
And the same desired/actual inputs as S-SHCP-RECON-01
When ServiceLifecycleReconciler::reconcile fires
Then zero actions are emitted
```

Rust test: `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs::given_stable_already_announced_when_reconcile_then_emits_no_actions`.

### S-SHCP-RECON-03 — `@US-01 @error @kpi K1`

StartupProbeFailed emitted when startup probe exhausts attempts.

```gherkin
Given a Service alloc whose startup probe has had 30 consecutive Fail observations
And startup_deadline (60s) has elapsed since started_at
And no Pass row exists for this probe
When ServiceLifecycleReconciler::reconcile fires
Then Action::SetTerminalCondition { Failed { reason: StartupProbeFailed { probe_idx: 0, last_fail: "connection refused", attempts: 30 } } } is emitted
```

Rust test: `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs::given_startup_probe_exhausts_attempts_when_reconcile_then_emits_failed_startup_probe_failed`.

### S-SHCP-RECON-04 — `@US-08 @error @kpi K1 — closes RCA-A`

EarlyExit emitted when alloc exits before any startup probe Pass.

```gherkin
Given a Service alloc 'alloc-coinflip-0':
  - AllocStatusRow.state = Failed with exit_code = 1
  - elapsed since started_at = 50ms (< startup_deadline)
  - no Pass ProbeResultRow exists for this alloc
When ServiceLifecycleReconciler::reconcile fires
Then Action::SetTerminalCondition { Failed { reason: EarlyExit { exit_code: 1 } } } is emitted
And the wire event is ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code: 1 }, stderr_tail: "..." }
```

Rust test: `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs::given_alloc_exits_within_deadline_no_pass_probe_when_reconcile_then_emits_failed_early_exit`.

### S-SHCP-INT-CLI-01 — `@walking_skeleton @driving_port @real-io @adapter-integration @us-01 @us-08 @kpi K1`

K1 north-star regression: coinflip-as-Service fixture, 100 deterministic seeds.

```gherkin
Given the test fixture coinflip-as-service.toml (a Service whose [exec] exits 1 within 30ms)
When the integration test submits 100 deterministic seeds via overdrive_cli::commands::job::submit
Then at least 99 of 100 submissions emit ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code: 1 } }
And zero submissions emit ServiceSubmitEvent::Stable
And the captured streaming output NEVER contains the literal "(took live)"
```

Rust test: `crates/overdrive-cli/tests/integration/service_honest_stable.rs::given_coinflip_as_service_fixture_when_submit_100_seeds_then_99_emit_failed_early_exit`.

---

## Slice 02 — Explicit HTTP startup probe

**Stories**: US-02. **KPI**: K1. **Dependencies**: Slice 01.

### S-SHCP-PARSE-01 — `@US-02 @parser`

HTTP probe TOML parses with defaults.

```gherkin
Given a [[health_check.startup]] section with type="http", path="/healthz", port=8080
When the spec is parsed
Then the resulting ProbeDescriptor has:
  - mechanic: ProbeMechanic::Http { path: "/healthz", port: 8080, host: None }
  - timeout_seconds: 5
  - interval_seconds: 2
  - max_attempts: 30
```

### S-SHCP-PARSE-02 — `@US-02 @error`

HTTP missing `path` is a named parse error.

```gherkin
Given a [[health_check.startup]] section with type="http", port=8080 (no path)
When the spec is parsed
Then ParseError::HttpProbeMissingPath { probe_idx: 0 } is returned
```

### S-SHCP-PARSE-08 — `@US-02 @error @scope-c6`

`https://` URL scheme is rejected at parse time (Phase 1 plain HTTP only per C6).

### S-SHCP-02-{01,02,03,04} — `@US-02 @in-memory`

`SimHttpProber` behavioural contracts: 200→Pass, 503→Fail with reason "HTTP 503", 302→Fail (no redirect-follow per research § 6.1 Pitfall 5), connection refused→Fail with named reason.

### S-SHCP-INT-02-{01,02,03} — `@US-02 @real-io @adapter-integration`

`HyperHttpProber` against real tokio-spawned HTTP server inside Lima: 200→Pass, 503→Fail with "HTTP 503", 302→Fail (no redirect-follow).

---

## Slice 03 — Explicit Exec startup probe (in-cgroup)

**Stories**: US-03. **KPI**: K1. **Dependencies**: Slice 01.

### S-SHCP-PARSE-03 — `@US-03 @parser`

Exec probe TOML parses with defaults.

### S-SHCP-PARSE-04 — `@US-03 @error`

Exec empty command yields `ParseError::ExecProbeMissingCommand { probe_idx: 0 }`.

### S-SHCP-03-{01..04} — `@US-03 @in-memory`

`SimExecProber` outcome contracts: exit 0 → Pass; exit 1 → Fail with "exit 1"; command-not-found → Fail with "exec: command not found"; timeout → Fail with "timeout after Ns" (SIGKILL delivered at timeout boundary).

### S-SHCP-INT-03-{01,02,03} — `@US-03 @real-io @adapter-integration @linux-only @cgroup`

`CgroupExecProber` against real cgroup scope inside Lima (sudo via `cargo xtask lima run --`): exit 0 → Pass; PID's `/proc/<pid>/cgroup` membership names `alloc-<id>.scope` (NOT worker's scope — the load-bearing prod/sim divergence test per ADR-0059 §2); timeout SIGKILLs via `cgroup.kill`.

---

## Slice 04 — Readiness probe flips Backend.healthy

**Stories**: US-04. **KPI**: K2. **Dependencies**: Slice 01.

### S-SHCP-RECON-07 — `@US-04 @driving_port @kpi K2`

Readiness Pass→Fail flips Backend.healthy within 1 tick.

```gherkin
Given a Service 'payments' with 3 backends, each with readiness probe Pass
When backend 2's readiness probe transitions to Fail between two reconciler ticks
Then within tick_period_ms of the next reconciler tick after the Fail row lands, Backend{2}.healthy = false
And the dataplane fingerprint value (compute_fingerprint(vip, backends)) differs between pre-fail tick and post-fail tick
```

### S-SHCP-RECON-08 — `@US-04 @recovery`

Readiness Fail→Pass restores Backend.healthy within 1 tick.

### S-SHCP-RECON-08b — `@US-04 @default-behaviour`

Service without readiness probes has all backends `healthy = true` post-Stable.

### S-SHCP-RECON-08c — `@US-04 @initial-state`

At alloc spawn, `Backend.healthy = false` until first readiness Pass (avoids the inverse race).

---

## Slice 05 — Liveness probe triggers restart

**Stories**: US-05. **KPI**: K3. **Dependencies**: Slice 04 (proves continuous probe pathway), Slice 01.

### S-SHCP-RECON-09 — `@US-05 @driving_port @kpi K3`

3 consecutive liveness fails → `Action::RestartAllocation { reason: LivenessExhausted { ... } }` emitted within 1 tick.

```gherkin
Given a Service alloc 'alloc-payments-0' with a liveness probe and failure_threshold = 3
And alloc.restart_count = 0
And the liveness probe has Fail rows for the last 3 consecutive ticks
When ServiceLifecycleReconciler::reconcile fires
Then exactly one Action::RestartAllocation { alloc_id: alloc-payments-0, reason: LivenessExhausted { probe_idx, consecutive_failures: 3, threshold: 3 } } is emitted
```

### S-SHCP-RECON-10 — `@US-05 @recovery`

Liveness fail/fail/pass resets the counter; no restart emitted.

### S-SHCP-RECON-11 — `@US-05 @error`

After `RESTART_BACKOFF_CEILING` (5) liveness restarts, next liveness trigger emits `Failed { reason: BackoffExhausted { attempts: 5 } }` (composes with existing JobLifecycle path).

---

## Slice 06 — CLI `alloc status` Probes section

**Stories**: US-06. **KPI**: K4. **Dependencies**: All prior slices (renders all roles + mechanics).

### S-SHCP-CLI-{01..06} — `@US-06 @driving_port @kpi K4`

Render contracts:
- Stable Service with 3 probes (startup+readiness+liveness) renders one row per probe with role, probe_idx, mechanic summary, last status, last observed timestamp.
- Job-kind alloc → NO Probes section.
- Schedule-kind alloc → NO Probes section.
- Fail row renders `last_fail_reason` (e.g. "HTTP 503").
- Probe with no result yet → `last=pending` (not blank).
- Inferred default probe renders `(inferred)` suffix.

---

## Slice 07 — Kind rejection for Job/Schedule

**Stories**: US-07. **KPI**: K5. **Dependencies**: None (parallel to Slice 01).

### S-SHCP-PARSE-05 — `@US-07 @error @kpi K5`

Job + probe → `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance: "Job has no readiness question; on completion is enough." }`.

### S-SHCP-PARSE-06 — `@US-07 @error @kpi K5`

Schedule + probe → `ParseError::ProbesNotAllowedOnKind { kind: "schedule", guidance: "Schedule composes per-fire ..." }`.

### S-SHCP-PARSE-07 — `@US-07 @regression-guard`

Service + probe parses successfully (regression guard).

### S-SHCP-CLI-{12,13,14} — `@US-07 @cli-surface`

Same three cases at the CLI handler boundary: error rendering includes guidance; exit code 1 for reject; accept case is no-op for the regression guard.

---

## Slice 08 — EarlyExit detection within startup deadline

**Stories**: US-08. **KPI**: K1 (closes RCA-A). **Dependencies**: Slice 01 (establishes `EarlyExit` variant).

### S-SHCP-RECON-05 — `@US-08 @post-stable`

Exit AFTER Stable is NOT EarlyExit (falls through to liveness/BackoffExhausted paths).

### S-SHCP-RECON-06 — `@US-08 @edge-case`

Exit 0 within startup_deadline IS still EarlyExit (Service kind expects long-lived).

### S-SHCP-CLI-{07,08,09,10,11} — `@US-08 @cli-render @kpi K1`

CLI render of `Failed { EarlyExit }`: multi-line block (exit_code, elapsed, stderr_tail); exit 0 case includes Service-kind guidance; NEVER `"(took live)"` for any Service-kind alloc (the cross-cutting RCA-A regression guard, applied at every render path).

### S-SHCP-INT-CLI-01 — covered by Slice 01 walking skeleton.

Coinflip-as-Service 100-seed regression test (the K1 north-star).

---

## Cross-cutting — Reconciler purity + wire/observation envelopes

### S-SHCP-PURITY-{01,02,03} — `@cross-cutting @reconciler-i-o @byte-equality`

- `ServiceLifecycleReconciler::reconcile` is pure sync (compile-time witness via direct sync call).
- `ServiceLifecycleView` carries inputs only — no `is_stable: bool` field.
- AllocStatusRow.terminal and LifecycleEvent.terminal carry byte-equal `TerminalCondition` values per ADR-0037 §3.

### S-SHCP-WIRE-{01,02,03} — `@wire-shape @ADR-0056`

- `ServiceSubmitEvent::Stable` serde roundtrip preserves payload.
- `ServiceSubmitEvent::Failed` serde roundtrip preserves payload for every `ServiceFailureReason` variant.
- Lockstep property: every typed `ServiceFailureReason` variant has a corresponding `ServiceFailureReasonWire` projection (proptest in DELIVER).

### S-SHCP-ENV-{01,02,03} — `@rkyv-envelope @ADR-0054-QR1`

- `ProbeResultRowEnvelope::V1` rkyv roundtrip bit-equivalent.
- V1 discriminant pinned to 0 (`const FIXTURE_V1_DISCRIMINANT: u8 = 0` in `tests/schema_evolution/probe_result_row.rs`).
- Unknown envelope variant warn-skips (observation surface gossips, not fail-fast) per ADR-0048 § "Unknown / malformed handling is asymmetric by layer".

---

## Tag glossary

| Tag | Meaning |
|---|---|
| `@walking_skeleton` | Slice 01 — the load-bearing demonstrable E2E |
| `@driving_port` | Test enters through a driving port (operator-visible surface): `overdrive job submit`, `overdrive alloc status`, the streaming wire |
| `@real-io` | Test uses real OS resources (TCP, HTTP, subprocess, cgroup); lives under `tests/integration/`, gated by `integration-tests` feature |
| `@in-memory` | Test uses Sim adapters; lives under `tests/acceptance/`, default lane |
| `@adapter-integration` | Tier 3 real-I/O test pinning the production adapter against its real backing system per `nw-tdd-methodology` Mandate 6 |
| `@kpi K<N>` | Test pins one of K1..K5 from `discuss/outcome-kpis.md` |
| `@US-<N>` | Story traceability |
| `@error` | Sad-path / parse-time-error scenario |
| `@cgroup` | Test requires real cgroup v2 (Lima sudo) |
| `@linux-only` | Test runs only under `cfg(target_os = "linux")` + `integration-tests` |

Per `.claude/rules/testing.md` § "Property-based testing": `@property`-shaped invariants land as proptests in DELIVER (e.g. S-SHCP-WIRE-03 — every typed reason has wire projection — is a proptest target). Per Mandate 11 (layer-dependent PBT mode): proptest lives at layers 1-2 only; Tier 3 sad paths stay example-based (slices 02 / 03 / 06 / 08 follow this discipline).
