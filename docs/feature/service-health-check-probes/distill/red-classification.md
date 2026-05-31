# Pre-DELIVER Fail-for-the-Right-Reason Classification

**Feature:** `service-health-check-probes`
**Wave:** DISTILL (Phase 4 — pre-handoff gate)
**Author:** Quinn (nw-acceptance-designer)
**Date:** 2026-05-24

Per `nw-distill` § "Pre-DELIVER fail-for-the-right-reason gate" / `nw-tdd-methodology` § "RED scaffolds — `#[should_panic(expected = "RED scaffold")]`" — every acceptance scaffold MUST fail on first run with a `MISSING_FUNCTIONALITY` signal (the production body's `todo!()` panic, OR the test body's intentional `panic!("Not yet implemented -- RED scaffold ...")`), NEVER with `IMPORT_ERROR` / `FIXTURE_BROKEN` / `SETUP_FAILURE`.

This file is the mental-walkthrough classification per scaffold. The Rust convention here uses `#[should_panic(expected = "RED scaffold")]` on the test body, which means each scaffold should report **PASS** (test panicked as expected) — the scaffold IS the assertion that "this functionality is not yet implemented." On any GREEN transition the scaffold-author drops the attribute and writes the real assertion.

DELIVER reads this file at the START of each slice to confirm:
1. Every scaffold's failure mode is structurally `MISSING_FUNCTIONALITY` (not a test bug).
2. The corresponding production scaffold has a `todo!("RED scaffold: ...")` body that names the same slice ID.
3. Removing the `#[should_panic]` attribute and replacing the body with real assertions IS the GREEN transition.

---

## Classification matrix

Format: `S-ID` → `category` (file + scenario name).

### Slice 01 — Walking skeleton (default TCP startup probe)

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-01-01 | `MISSING_FUNCTIONALITY` (correct RED) | `overdrive-worker/tests/acceptance/probe_runner_tcp_outcome.rs` | `SimTcpProber::probe` (`overdrive-sim/src/adapters/probers.rs`) |
| S-SHCP-01-02 | `MISSING_FUNCTIONALITY` | same file | same scaffold |
| S-SHCP-01-03 | `MISSING_FUNCTIONALITY` | same file | trait wiring witness |
| S-SHCP-RECON-01 | `MISSING_FUNCTIONALITY` | `overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs` | `ServiceLifecycleReconciler` body (`overdrive-control-plane/src/reconcilers/service_lifecycle/mod.rs`) — lands in slice 01 DELIVER |
| S-SHCP-RECON-02 | `MISSING_FUNCTIONALITY` | same file | same scaffold |
| S-SHCP-RECON-03 | `MISSING_FUNCTIONALITY` | same file | same scaffold |
| S-SHCP-RECON-04 | `MISSING_FUNCTIONALITY` | same file | same scaffold |
| S-SHCP-INFER-01 | `MISSING_FUNCTIONALITY` | `overdrive-core/tests/acceptance/health_check_toml_parse.rs` | TOML parser extension (`overdrive-core/src/aggregate/workload_spec.rs`) — lands in slice 01 DELIVER |
| S-SHCP-INFER-02 | `MISSING_FUNCTIONALITY` | same file | same scaffold |
| S-SHCP-ENV-01..03 | `MISSING_FUNCTIONALITY` | `overdrive-core/tests/acceptance/probe_result_row_envelope.rs` | `ProbeResultRowEnvelope::latest` / `into_latest` (`overdrive-core/src/observation/probe_result_row.rs`) |
| S-SHCP-PURITY-01..03 | `MISSING_FUNCTIONALITY` | `overdrive-control-plane/tests/acceptance/service_lifecycle_purity.rs` | reconciler scaffold |
| S-SHCP-WIRE-01..03 | `MISSING_FUNCTIONALITY` | `overdrive-control-plane/tests/acceptance/service_submit_event_v2.rs` | `ServiceSubmitEvent` variant additions (`overdrive-control-plane/src/api.rs` — extension) |
| S-SHCP-INT-01-01..03 | `MISSING_FUNCTIONALITY` (Tier 3 — Lima) | `overdrive-worker/tests/integration/probe_runner/real_tcp_probe.rs` | `TokioTcpProber::probe` (`overdrive-worker/src/probe_runner/tcp_prober.rs`) |
| S-SHCP-INT-CLI-01..05 | `MISSING_FUNCTIONALITY` (Tier 3 — Lima, in-process server) | `overdrive-cli/tests/integration/service_honest_stable.rs` | end-to-end composition root (multi-crate wiring lands in slice 01 DELIVER) |

### Slice 02 — HTTP startup probe

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-02-01..04 | `MISSING_FUNCTIONALITY` | `overdrive-worker/tests/acceptance/probe_runner_http_outcome.rs` | `SimHttpProber::probe` |
| S-SHCP-PARSE-01 | `MISSING_FUNCTIONALITY` | `overdrive-core/tests/acceptance/health_check_toml_parse.rs` | parser extension |
| S-SHCP-PARSE-02 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-PARSE-08 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-INT-02-01..03 | `MISSING_FUNCTIONALITY` (Tier 3) | `overdrive-worker/tests/integration/probe_runner/real_http_probe.rs` | `HyperHttpProber::probe` (`overdrive-worker/src/probe_runner/http_prober.rs`) |

### Slice 03 — Exec startup probe (in-cgroup)

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-03-01..04 | `MISSING_FUNCTIONALITY` | `overdrive-worker/tests/acceptance/probe_runner_exec_outcome.rs` | `SimExecProber::probe` |
| S-SHCP-PARSE-03 | `MISSING_FUNCTIONALITY` | `overdrive-core/tests/acceptance/health_check_toml_parse.rs` | parser extension |
| S-SHCP-PARSE-04 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-INT-03-01..03 | `MISSING_FUNCTIONALITY` (Tier 3 — Lima sudo + cgroup) | `overdrive-worker/tests/integration/probe_runner/real_exec_probe_cgroup.rs` | `CgroupExecProber::probe` (`overdrive-worker/src/probe_runner/exec_prober.rs`) |

### Slice 04 — Readiness → Backend.healthy

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-RECON-07 | `MISSING_FUNCTIONALITY` | `overdrive-control-plane/tests/acceptance/service_lifecycle_readiness.rs` | reconcile body extension (slice 04 DELIVER) |
| S-SHCP-RECON-08 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-RECON-08b | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-RECON-08c | `MISSING_FUNCTIONALITY` | same | same |

### Slice 05 — Liveness → RestartAllocation

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-RECON-09 | `MISSING_FUNCTIONALITY` | `overdrive-control-plane/tests/acceptance/service_lifecycle_liveness.rs` | reconcile body extension (slice 05 DELIVER) |
| S-SHCP-RECON-10 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-RECON-11 | `MISSING_FUNCTIONALITY` | same | same |

### Slice 06 — CLI Probes section

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-CLI-01..06 | `MISSING_FUNCTIONALITY` | `overdrive-cli/tests/acceptance/probes_section_render.rs` | `render.rs` Service-kind handler extension (slice 06 DELIVER) |

### Slice 07 — Kind rejection (Job/Schedule)

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-PARSE-05 | `MISSING_FUNCTIONALITY` | `overdrive-core/tests/acceptance/health_check_toml_parse.rs` | parser extension |
| S-SHCP-PARSE-06 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-PARSE-07 | `MISSING_FUNCTIONALITY` | same | regression guard |
| S-SHCP-CLI-12..14 | `MISSING_FUNCTIONALITY` | `overdrive-cli/tests/acceptance/probes_kind_rejection_cli.rs` | CLI error rendering surface |

### Slice 08 — EarlyExit hardening + multi-line render

| ID | Category | Location | Production scaffold |
|---|---|---|---|
| S-SHCP-RECON-05 | `MISSING_FUNCTIONALITY` | `service_lifecycle_stable.rs` | reconcile body hardening |
| S-SHCP-RECON-06 | `MISSING_FUNCTIONALITY` | same | same |
| S-SHCP-CLI-07..11 | `MISSING_FUNCTIONALITY` | `overdrive-cli/tests/acceptance/service_early_exit_render.rs` | CLI render extension |
| S-SHCP-INT-CLI-01 | covered in slice 01 | (K1 north-star) | end-to-end wiring |

---

## Categories NOT present (verify before DELIVER PREPARE)

This pre-flight check confirms ZERO scaffolds are in the wrong category:

- ZERO `IMPORT_ERROR` scaffolds. Every production module imported by a test has a scaffold file with `// SCAFFOLD: true` and the correct fn / struct / enum signatures so the test compiles.
- ZERO `FIXTURE_BROKEN` scaffolds. No test-side fixture (`#[tokio::test]` runtime setup, TempDir construction) panics before the assertion fires; the `panic!("...")` body IS the assertion site.
- ZERO `SETUP_FAILURE` scaffolds. Every `#[should_panic(expected = "RED scaffold")]` attribute pairs with a `panic!("Not yet implemented -- RED scaffold ...")` body that names the scenario ID.
- ZERO `WRONG_ASSERTION` scaffolds. No scaffold asserts on internal struct fields or private mutation details. Assertion targets (when the scaffold goes GREEN in DELIVER) are observable-port-exposed names: emitted Action variants, View counter fields publicly readable, wire-event serde roundtrip equality, CLI output strings, exit codes, `/proc/<pid>/cgroup` membership.

---

## How DELIVER reads this file

1. At slice PREPARE phase: read this file's per-slice table to confirm the scaffold count + ID range.
2. At slice RED phase: run `cargo xtask lima run -- cargo nextest run -p {crate} -E 'test(S-SHCP-{slice})'`. Every test should report PASS (the `#[should_panic]` attribute fires on the `todo!()` or intentional `panic!()`).
3. At slice GREEN phase (per ADR-025 3-phase canon): for each scenario, remove the `#[should_panic(expected = "RED scaffold")]` attribute, replace the `panic!("...")` body with the real assertions named in the scenario, implement the production body (replace `todo!()` with real code), confirm the test goes green via the same nextest invocation.
4. At slice COMMIT phase: confirm `grep -r "todo!.*RED scaffold" crates/{owned-by-slice}` returns zero matches in the slice's owned files.

---

## Cross-reference

- `docs/feature/service-health-check-probes/distill/test-scenarios.md` — full Gherkin specification per scenario.
- `docs/feature/service-health-check-probes/feature-delta.md` § "Wave: DISTILL / [REF] Scaffolds" — file inventory.
- `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits" — the convention.
- `nw-distill` § "Pre-DELIVER fail-for-the-right-reason gate" — the gate procedure.
