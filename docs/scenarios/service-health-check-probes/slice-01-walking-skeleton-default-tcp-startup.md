# Slice 01 — Walking Skeleton: Default TCP-connect startup probe end-to-end

**Walking Skeleton.** This slice establishes the structural foundation that every subsequent slice composes onto.

**Stories:** US-01
**Priority:** P0
**KPI:** K1 (Service-submit honesty rate ≥99%)
**Dependencies:** None (ADR-0047 / ADR-0037 / ADR-0032 / ADR-0033 already landed)

## Outcome the operator can verify

Ana writes `payments-minimal.toml` with a `[service]` block, one `[[listener]]`, no `[[health_check.*]]` sections. She runs `overdrive job submit payments-minimal.toml`. The CLI prints:

- (Happy) `Service 'payments-minimal' is stable\n  settled_in: <real Duration>\n  witness: startup probe #0 (tcp 0.0.0.0:<port>)` — exit code 0
- (Sad — RCA-A coinflip) `Service 'payments-minimal' failed: workload exited within startup deadline\n  exit_code: 1\n  elapsed: 0.05s\n  stderr_tail: "..."` — exit code 1
- (Sad — never binds) `Service 'payments-minimal' failed: startup probe timed out\n  probe: startup #0 (tcp 0.0.0.0:<port>)\n  attempts: 30/30\n  last_fail: connection refused\n  elapsed: 60.0s` — exit code 1

## Foundation this slice establishes

| Component | Location (proposed) | Notes |
|---|---|---|
| `ProbeRunner` trait | `crates/overdrive-worker/src/probe_runner.rs` (new) | TCP-only mechanic for this slice |
| `ProbeResultRow` | `crates/overdrive-core/src/observation/probe_result.rs` (new) | LWW per `(alloc_id, probe_idx)`; rkyv-archived |
| `TerminalCondition::Stable { settled_in, witness }` | `crates/overdrive-core/src/terminal_condition.rs` (extend) | Additive variant per ADR-0037 §5 |
| `TerminalCondition::Failed { reason: ServiceFailureReason }` | same | Additive variant |
| `enum ServiceFailureReason { StartupProbeFailed { ... }, EarlyExit { exit_code } }` | same module | New |
| `ServiceLifecycleReconciler` | `crates/overdrive-control-plane/src/reconcilers/service_lifecycle.rs` (new) | Split from JobLifecycle per ADR-0047 |
| `ServiceSubmitEvent::Stable` / `ServiceSubmitEvent::Failed` wire variants | `crates/overdrive-control-plane/src/api.rs` (extend) | Per ADR-0032 Amendment 2026-05-10 |
| Default TCP probe inference | ServiceSpec validator in `overdrive-core` | When no probes declared AND ≥1 listener |
| CLI handler match arms | `crates/overdrive-cli/src/commands/job.rs` | Replace any `"live"` literal path for Service kind |

## Acceptance test (regression guard for K1)

`crates/overdrive-cli/tests/integration/service_honest_stable.rs` (NEW; gated behind `integration-tests` feature):

- Fixture A: `coinflip-as-service.toml` (Service block, exec exits 1 at T0+30ms). Assert 99/100 deterministic seeds emit `Failed { EarlyExit { exit_code: 1 } }` — closes K1 baseline.
- Fixture B: `quick-bind.toml` (Service binds 8080 within 600ms). Assert Stable emitted with `settled_in` parseable as a Duration in [500ms, 2000ms].
- Fixture C: `never-binds.toml` (Service never binds 8080). Assert Failed `StartupProbeFailed` with `last_fail: "connection refused"` after `startup_deadline` elapses.
- Fixture D (byte-equality): assert AllocStatusRow.terminal and the captured LifecycleEvent.terminal bytes are equal under rkyv archive serialisation.

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-cli --features integration-tests -E 'test(service_honest_stable)'` passes green.

Manual demo: stand up `cargo overdrive serve`, submit `coinflip-as-service.toml`, observe CLI prints `Failed` (NOT `is running (took live)`).

## Out of scope for this slice

- HTTP probes (US-02 / Slice 02)
- Exec probes (US-03 / Slice 03)
- Readiness probes (US-04 / Slice 04)
- Liveness probes (US-05 / Slice 05)
- Probes section in `alloc status` render (US-06 / Slice 06)
- Kind rejection for Job/Schedule (US-07 / Slice 07; can land in parallel)
- (Note: US-08's `EarlyExit` reason variant IS in scope here because Slice 01 must establish the Failed reason enum; US-08 / Slice 08 hardens the coinflip-regression test and renders the full multi-line failure surface.)
