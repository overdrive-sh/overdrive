# Acceptance Review (self-review) — phase-1-first-workload

Self-review of the test scenarios and walking-skeleton extension against the
`nw-acceptance-designer` skill checklist (items 1–15) and the four mandates from
`nw-test-design-mandates`. The reviewer-agent (Sentinel) runs against the
critique-dimensions skill separately — this file is Quinn's own pass.

Some checklist items reference Python / pytest-bdd patterns that do not apply to
this project. Per `.claude/rules/testing.md` the project uses Rust nextest tests
exclusively; those items are marked **N/A — project rule** with the rule citation.

## Self-review checklist

| # | Item | Status | Notes |
|---|---|---|---|
| 1 | All user stories from DISCUSS have at least one matching scenario (Story-to-Scenario mapping) | **PASS** | US-01 → 1.1–1.8 (8 scenarios). US-02 → 2.1–2.10 (10). US-03 → 3.1–3.14 (14). US-04 → 4.1–4.8 (8). Every story tag (`@US-01`..`@US-04`) is referenced by at least one scenario. |
| 2 | Error-path coverage at least 40% | **PASS** | 16 / 39 = 41 %. See test-scenarios.md § Error Path Coverage tally. |
| 3 | Each scenario follows GIVEN/WHEN/THEN with one behaviour | **PASS** | Every scenario has a single When (action / event) and zero or one But-style And clauses on Then for additional observable assertions. No multi-When scenarios. |
| 4 | Concrete examples, not abstractions | **PASS** | Every capacity number is concrete ("4000 mCPU / 8 GiB", "10 GiB", "4 GiB free"). Every command is concrete (`overdrive job stop payments`, `kill -9 12345`). Resource arithmetic is closed-form (`needed.cpu_milli equals 2000`, `max_free.cpu_milli equals 1000`). |
| 5 | Business language in Gherkin (no technical jargon outside user-observable contracts) | **PASS** | Operator-facing language ("Ana runs", "the workload", "the allocation transitions"). Technical terms appear ONLY when they ARE the user-observable contract — `AllocationState::Running` (visible in CLI output), `cgroup_path` (operator can `cat /sys/fs/cgroup/.../cgroup.procs`), `POST /v1/jobs/{id}:stop` (the wire shape under audit per ADR-0027). See test-scenarios.md § Mandate Compliance Summary CM-B for reasoning. |
| 6 | Walking skeletons describe user goals (not technical layer wiring) | **PASS** | Each `@walking_skeleton` title describes an operator outcome (e.g. "Submitting a 1-replica job results in a Running allocation visible via CLI"; "Stopping a Running job drives it through Draining to Terminated"). See walking-skeleton.md § Litmus test for the line-by-line check on each WS. |
| 7 | Tests invoke through driving ports only (Mandate CM-A: hexagonal boundary enforcement) | **PASS** | Every scenario's `target_test:` field names a Rust test path that enters via a driving port: CLI subprocess (`overdrive ...`), HTTP endpoint (`POST /v1/jobs/{id}:stop`), pure function (`schedule(...)`), or the DST harness (`cargo xtask dst`). No scenario imports `JobLifecycleState`, `CgroupPath` internals, or the action-shim's private types directly. |
| 8 | Pure functions extracted before fixture parametrisation (Mandate CM-D) | **PASS** | Scheduler is a pure function (US-01); JobLifecycle reconciler is pure by §18 contract (its libSQL access lives in `hydrate`; its time injection lives in `tick.now`); the action shim is the single I/O boundary; everything downstream lives behind `Driver` and `ObservationStore` traits, parametrised in tests via `SimDriver` and `SimObservationStore`. Per-test fixture parametrisation is confined to the integration-tests lane (real `ProcessDriver` against real cgroupfs). |
| 9 | Walking-skeleton boundary proof — every driven adapter has a real-I/O test (Dim 9c) | **PASS** | See test-scenarios.md § Adapter Coverage Table. Every NEW driven adapter has at least one `@real-io @adapter-integration` scenario; inherited adapters carry forward verbatim coverage from `phase-1-control-plane-core`. |
| 10 | Walking-skeleton strategy is declared in wave-decisions.md (Dim 9a) | **PASS** | DWD-1 in `distill/wave-decisions.md` declares the project's hybrid two-lane model — Tier 1 DST default (`@in-memory`) + Tier 3 integration-tests Linux (`@real-io @adapter-integration`). User confirmed in the orchestrator handoff: "User has ratified this hybrid as the WS strategy." |
| 11 | Walking-skeleton strategy-implementation match (Dim 9b) | **PASS** | The strategy declares both lanes. WS scenarios for the default lane are tagged `@in-memory` (3.14); WS scenarios for the integration-tests lane are tagged `@real-io @adapter-integration` (2.2, 3.1, 3.7, 3.9, 4.1, 4.2). No `@in-memory` tag appears on a real-I/O-required walking skeleton. |
| 12 | pytest-bdd step organisation by domain | **N/A — project rule** | `.claude/rules/testing.md`: "All acceptance and integration tests are written directly in Rust using `#[test]` / `#[tokio::test]` functions. (...) Do NOT introduce cucumber-rs, pytest-bdd, conftest.py, or any `.feature` file consumer." Rust per-crate `tests/acceptance/` and `tests/integration/` layout per ADR-0005 replaces step-organisation conventions; the test paths in `target_test:` fields enforce per-domain grouping at the file system level. |
| 13 | pytest fixture scopes (session / module / function) | **N/A — project rule** | Same rule as item 12. Rust nextest scoping is per-test (function-scope-equivalent); module-scope shared state lives in `tests/<scenario>.rs` module bodies; session-scope corresponds to the integration-test binary's `tests/integration.rs` entrypoint. The Rust idiom is sufficient — no fixture-scope abstraction is needed. |
| 14 | Production-like test environment via real services (DB, message queue) | **ADAPTED — project rule** | "Real services" in this project means: real `LocalIntentStore` (redb against `tempfile::TempDir`), real `LocalObservationStore`, real `ProcessDriver` against `/bin/sleep`, real cgroupfs writes — all gated `--features integration-tests`. The project does NOT use a separate test database server; redb is embedded. Scenarios 3.1, 3.7, 3.9, 4.1, 4.2 exercise this. |
| 15 | `capsys` / output capture for CLI subprocess scenarios | **N/A — project rule** | The Rust idiom for CLI subprocess testing is `tokio::process::Command::spawn` in the test body, capturing stdout/stderr via `Output::stdout` / `Output::stderr`. The DELIVER crafter's translation of CLI scenarios to Rust handles this naturally; no `capsys`-equivalent abstraction is needed. |

## Mandate compliance summary

| Mandate | Status | Evidence |
|---|---|---|
| **CM-A** Hexagonal boundary enforcement | PASS | Test paths enter via driving ports only. |
| **CM-B** Business language abstraction | PASS | Operator-facing Gherkin; technical terms appear only when they ARE the user-observable contract. |
| **CM-C** Walking skeleton + focused scenarios | PASS | 7 walking-skeleton (one extra: 4.2 — the burst-resilience proof closes step 6 of the journey distinctly from 4.1's enrolment proof). 32 focused. Ratio appropriate for a 4-story DELIVER feature. |
| **CM-D** Pure function extraction | PASS | Pure functions identified upfront (scheduler, reconcile bodies). Impure code lives behind `Driver` / `ObservationStore` / `Clock` / `Entropy` traits — all DST-controllable. |

## Critique-dimension self-pass

Anticipating the Sentinel reviewer's pass against `nw-ad-critique-dimensions`. For each of the 9 dimensions:

| Dim | Pattern | Self-assessment |
|---|---|---|
| 1 | Happy path bias | PASS — 41 % error-path scenarios. |
| 2 | GWT format compliance | PASS — single When per scenario; observable Then assertions. |
| 3 | Business language purity | PASS with caveat — technical terms appear only when they ARE the user-observable contract (e.g. `AllocationState::Running` is the CLI's rendered field). The reviewer should confirm this is acceptable. The alternative — replacing every type name with prose — would obscure the wire contract, which is what ADR-0027 / ADR-0011 / ADR-0021 deliberately make readable for audit. |
| 4 | Coverage completeness | PASS — every story has at least one scenario; every AC bullet on every story has a matching scenario; failure_modes from the journey YAML are mapped to error-path scenarios. |
| 5 | Walking skeleton user-centricity | PASS — see walking-skeleton.md § Litmus test. |
| 6 | Priority validation | PASS — KPIs K1..K4 each have at least one scenario tagged `@kpi:Kn`. The K4 (cluster-status responsiveness under burst) is the largest open priority — it is covered by 4.2. |
| 7 | Observable behaviour assertions | PASS — every Then asserts a return value (CLI exit code, HTTP status, function result) or an observable outcome (CLI stdout text, file existence, AllocationState transition visible in `alloc status`). No scenario asserts internal state, mock call counts, or private fields. |
| 8 | Traceability coverage | PASS — every scenario carries `@US-01..04`. Story-to-Scenario map: US-01 → 8, US-02 → 10, US-03 → 14, US-04 → 8. Environment coverage: default lane has 3.14; integration-tests lane has 2.2, 3.1, 3.7, 3.9, 4.1, 4.2. |
| 9 | Walking skeleton boundary proof | PASS — see test-scenarios.md § Adapter Coverage Table. |

## Open questions for the reviewer

None blocking. Two items the reviewer may want to weigh in on:

1. **Dim 3 caveat above.** "AllocationState::Running" appears in Gherkin Then steps. This is deliberate — it is the CLI-rendered field name operators see in `overdrive alloc status` output. Substituting prose ("the allocation is in the running state") would lose the round-trip property the test asserts. The reviewer's alternative would be welcome but I believe the current shape is correct for a contract under audit.

2. **Scenario 3.3 implementation note.** The test "JobLifecycle reconciler does not call wall-clock or RNG inside reconcile" is asserted via an xtask structural inspector, not via the standard dst-lint cargo lint, because the reconciler lives in `overdrive-control-plane` (class `adapter-host`, not scanned). The crafter implements the inspector (a syn-based AST walker for the file). The reviewer should flag if a different approach (e.g. moving the reconciler into a `core` crate so dst-lint scans it) is preferable. Note: ADR-0023 explicitly places the action shim in `overdrive-control-plane` because it performs async I/O; the reconciler itself is sync but is co-hosted by the crate. Splitting them would add a new crate just for the reconciler — defensible but heavier than the structural inspector.

## Reviewer handoff package

- `docs/feature/phase-1-first-workload/distill/test-scenarios.md` — 39 scenarios across US-01..04
- `docs/feature/phase-1-first-workload/distill/walking-skeleton.md` — WS extension narrative + per-step driving-port mapping
- `docs/feature/phase-1-first-workload/distill/wave-decisions.md` — DWD-1..DWD-7 + reuse analysis updates
- This file — self-review against checklist + critique dimensions
- RED scaffolds — see `wave-decisions.md` DWD-6 + the file list returned in the orchestrator summary
