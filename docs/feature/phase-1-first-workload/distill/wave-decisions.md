# DISTILL Wave Decisions — phase-1-first-workload

**Wave**: DISTILL (acceptance-designer)
**Owner**: Quinn
**Date**: 2026-04-27
**Status**: COMPLETE — handoff-ready for DELIVER (software-crafter), pending peer review by Sentinel (`nw-acceptance-designer-reviewer`).

---

## Key decisions

Seven decisions ratified during the DISTILL pass. All are derivative of project rules and prior-wave artifacts; no new architectural choice surfaces. Deviations from the generic `nw-acceptance-designer` skill methodology are owned by the project rules in `.claude/rules/testing.md` and `.claude/rules/development.md`.

| # | Decision | Rationale (one line) | Reference |
|---|---|---|---|
| DWD-1 | **Walking skeleton strategy = hybrid two-lane** (Tier 1 DST default + Tier 3 integration-tests Linux) | Project's four-tier model is load-bearing; user ratified the hybrid in orchestrator handoff. | `.claude/rules/testing.md` § Integration vs unit gating |
| DWD-2 | **No `.feature` files anywhere; Gherkin in `test-scenarios.md` is specification-only** | Project rule overrides skill default — Rust-native nextest tests through and through. | `.claude/rules/testing.md` first paragraph |
| DWD-3 | **RED scaffold style = `panic!("Not yet implemented -- RED scaffold")`** in production scaffolds; downstream non-exhaustive-match compile errors are expected and not silenced | Project rule "Downstream fallout on pre-existing tests is expected and correct" — deliberate RED state lets the DELIVER crafter drive against a known broken baseline. | `.claude/rules/testing.md` § RED scaffolds and intentionally-failing commits |
| DWD-4 | **Adapter coverage map** — every NEW driven adapter has at least one `@real-io @adapter-integration` scenario | Mandate 6 + critique Dim 9c require real-I/O coverage per adapter. | `test-scenarios.md` § Adapter Coverage Table |
| DWD-5 | **Error-path coverage = 41 %** (16 of 39 scenarios) | Mandate target ≥40 % satisfied. Errors cover scheduler boundaries, ProcessDriver failures, reconciler backoff, cgroup pre-flight failure modes. | `test-scenarios.md` § Error Path Coverage |
| DWD-6 | **Scaffold two new crates this DISTILL** — `overdrive-scheduler` (class `core`, ADR-0024) + `overdrive-worker` (class `adapter-host`, ADR-0029) — and extend three existing crates (`overdrive-core`, `overdrive-control-plane`, `overdrive-cli`) with stub variants and modules | DESIGN named the crate boundaries; DISTILL lands the empty crates so the test paths in `test-scenarios.md` have something to compile against. The scaffolds carry `panic!` bodies — DELIVER fills them. | DWD-3 + ADR-0024 + ADR-0029 + `wave-decisions.md` D4/D10 |
| DWD-7 | **Test layout = `tests/acceptance/` + `tests/integration/` per ADR-0005** | Acceptance lives in default lane (`@in-memory`); integration lives in feature-gated lane (`@real-io`). The `tests/integration.rs` entrypoint gates the whole binary on `integration-tests` per `.claude/rules/testing.md`. | `.claude/rules/testing.md` § Integration vs unit gating |

---

## Reuse Analysis (no architectural changes; confirms DESIGN's table)

DISTILL did not surface any contradiction with DESIGN's `wave-decisions.md` § Reuse Analysis table. Every dispoition there carries forward verbatim:

- `Reconciler` trait + runtime: EXTEND (new `type State` associated type; new `AnyState` enum) — mirrored in test-scenarios.md scenario 3.2 ("byte-identical (actions, next_view) for identical inputs") and the three new DST invariants 3.4 / 3.5 / 3.6.
- `Action` enum: EXTEND (three new variants) — mirrored in scenarios 3.1, 3.7, 3.9 (Start / Restart / Stop respectively are emitted by the reconciler and consumed by the action shim).
- `Driver` trait: REUSE AS-IS — mirrored in scenarios 2.1–2.10 (default-lane SimDriver + integration-tests-gated ProcessDriver, both implementing the same trait).
- `Job` / `Node` aggregates: REUSE AS-IS (no schema changes) — confirmed by the absence of any aggregate-shape scenario in this DISTILL.
- `IntentKey::for_job_stop`: NEW associated function on existing newtype — exercised by scenario 3.12.
- `AppState::driver`: EXTEND (new `Arc<dyn Driver>` field per ADR-0022) — exercised transitively by every `@real-io @adapter-integration` scenario; no direct AppState scenario because that would violate Mandate CM-A (no internal-state assertions).
- All DST invariants (`JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`, `NoDoubleScheduling`): NEW — exercised by scenarios 3.4 / 3.5 / 3.6.

No new ADR or DESIGN amendment is required.

---

## Expected RED state at DELIVER handoff

This DISTILL deliberately lands a RED state across the workspace. Per `.claude/rules/testing.md` § "Downstream fallout on pre-existing tests is expected and correct", the RED state is the correct shape — it gives the DELIVER crafter a known-failing baseline to drive against, and it prevents DISTILL from accidentally fielding fake greens.

What is RED, and why:

1. **`overdrive-scheduler` crate compiles but the `schedule(...)` function panics.** Every scheduler scenario (1.1–1.7) panics with the "Not yet implemented -- RED scaffold" message. Scenario 1.8 (dst-lint clean) does pass — the scaffold is small enough to satisfy dst-lint by construction.

2. **`overdrive-worker` crate compiles but `ProcessDriver` impls panic.** Every ProcessDriver scenario (2.2–2.8) panics. Scenarios 2.1, 2.9, 2.10 (default-lane and CgroupPath round-trip) panic in the newtype constructor.

3. **`overdrive-core::Action` gains three new variants.** Existing match sites in `overdrive-control-plane::handlers` and `overdrive-sim` may produce non-exhaustive-match compile errors — the crafter's first task is to add match arms (panic-bodied for the moment, then implementations). This is the canonical RED-fallout shape.

4. **`overdrive-core::AnyReconciler::JobLifecycle` variant added.** Match arms in `name`, `hydrate`, `reconcile` now have a `_ => unreachable!()` (or equivalent) RED scaffold body. The trait dispatch compiles; invocation panics.

5. **`overdrive-core::AnyState` enum is NEW.** The `Reconciler::reconcile` signature gains `desired: &Self::State` / `actual: &Self::State` per ADR-0021. Existing `NoopHeartbeat::reconcile` adapts to `type State = ();` and `AnyState::Unit`; the existing `reconciler_trait_signature_is_synchronous_no_async_no_clock_param` compile-fail test is updated to assert the new shape (DELIVER closes this loop).

6. **`overdrive-control-plane::reconciler_runtime::action_shim` module is NEW.** The `dispatch(...)` function panics. Every `@real-io @adapter-integration` scenario in US-03 panics until the shim is implemented.

7. **`overdrive-control-plane::cgroup_manager::control_plane` module is NEW.** Boot-path scenarios (4.1, 4.3) and pre-flight scenarios (4.4, 4.5, 4.6, 4.7) panic until implemented.

8. **`AppState::driver: Arc<dyn Driver>` field added.** Every existing `run_server_with_obs` test caller (under `crates/overdrive-control-plane/tests/integration/*.rs`) gets a compile error at the AppState construction site. The crafter migrates them mechanically (passing `Arc::new(SimDriver::new())` as the new field's value) — this is the ADR-0022 mechanical migration.

9. **`overdrive-cli` `stop` subcommand is NEW.** Scenarios 3.9, 3.10, 3.11 panic in the CLI command body until implemented.

The crafter MUST commit RED-scaffold commits with `git commit --no-verify` per `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits". The commit message MUST call out the RED state explicitly (e.g. `feat(scheduler): scaffold overdrive-scheduler crate (RED — panic bodies)`).

---

## Handoff package for DELIVER

- `docs/feature/phase-1-first-workload/distill/test-scenarios.md` — 39 scenarios across US-01..04, with `target_test:` paths.
- `docs/feature/phase-1-first-workload/distill/walking-skeleton.md` — WS extension narrative + per-step driving-port mapping.
- `docs/feature/phase-1-first-workload/distill/acceptance-review.md` — Quinn's self-review.
- `docs/feature/phase-1-first-workload/distill/wave-decisions.md` (this file) — DWD-1..DWD-7 + RED-state inventory.
- New crate `crates/overdrive-scheduler/` — `Cargo.toml` + `src/lib.rs` (panic-bodied `schedule` + typed `PlacementError`).
- New crate `crates/overdrive-worker/` — `Cargo.toml` + `src/lib.rs` (panic-bodied `ProcessDriver` impl + `CgroupPath` newtype + `cgroup_manager::workload` + `node_health` writer entrypoints).
- Existing crate extensions:
  - `crates/overdrive-core/src/reconciler.rs` — three new `Action` variants; `AnyState` enum; `Reconciler::State` associated type; `JobLifecycle` / `JobLifecycleView` / `JobLifecycleState` stub types; `AnyReconciler::JobLifecycle` and `AnyReconcilerView::JobLifecycle` variants.
  - `crates/overdrive-control-plane/src/reconciler_runtime/action_shim.rs` (NEW module) — panic-bodied `dispatch` function + `ShimError` enum.
  - `crates/overdrive-control-plane/src/cgroup_manager/mod.rs` (NEW module) + `control_plane.rs` submodule — panic-bodied pre-flight + slice-creation entrypoints.
  - `crates/overdrive-control-plane/src/lib.rs` — `AppState::driver: Arc<dyn Driver>` field; `run_server_with_obs_and_driver` rename + signature.
  - Workspace `Cargo.toml` — `overdrive-scheduler` + `overdrive-worker` added to `[workspace] members`; new workspace deps if any (none — every dep already exists).

The `cargo check --workspace` shape after DISTILL: compiles where compile-time invariants are satisfied; panics or compile-errors where the RED scaffold + match-arm-extension invites them. This is the intended state.

---

## Open questions for DELIVER

None blocking handoff. Two notes the crafter may consult:

1. **Scenario 3.3 (dst-lint of the JobLifecycle reconciler)** — the reconciler lives in `overdrive-control-plane` (class `adapter-host`, not scanned by dst-lint). Quinn's recommendation is an xtask-side structural inspector (syn-based AST walker) that asserts no banned API in the `reconcile` body, executed by the existing `cargo xtask` test fleet. The alternative — moving the reconciler into a `core` crate so dst-lint scans it — is a heavier change; the crafter chooses.

2. **`Action` exhaustiveness compile errors at existing match sites** — when the three new `Action` variants land, every `match` over `Action` in pre-existing code (handlers, sim, tests) will fail to compile. The DELIVER crafter's first task is to add the new match arms. Per project rule, those arms get `panic!("Not yet implemented -- RED scaffold")` bodies until the crafter implements each one. Do NOT silence with `_ => panic!(...)` catch-alls — the compiler's exhaustiveness check is the load-bearing safety net per the `AnyReconciler` enum-dispatch convention in `crates/overdrive-core/src/reconciler.rs`.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial DISTILL wave for `phase-1-first-workload`. 39 scenarios across US-01..04; 41 % error-path ratio; 7 walking-skeleton scenarios extending the prior feature's WS; two new crates scaffolded with panic-bodied entrypoints; three existing crates extended with new variants and modules. RED state intentional. — Quinn |
