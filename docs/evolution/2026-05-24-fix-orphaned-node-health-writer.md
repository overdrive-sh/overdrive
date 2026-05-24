# fix-orphaned-node-health-writer — Feature Evolution

**Feature ID**: `fix-orphaned-node-health-writer`
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/node-health-writer`
**Date**: 2026-05-24
**Commits** (7 total, in landing order):
- `0538084d` — `test(node-health): add boot-time writer regression test, remove orphan scaffold pin`
- `cfdfcb4c` — `chore(des): record execution-log for step 01-01 phases`
- `6da2d033` — `fix(node-health): wire boot-time NodeHealthRow writer into control-plane boot path`
- `424ff462` — `chore(des): record execution-log for step 01-02 phases`
- `831cfa56` — `refactor(node-health): drop stale SCAFFOLD const from node_health module`
- `f86a9eca` — `chore(worker): drop crate-level SCAFFOLD marker now that all modules are GREEN`
- `9750161d` — `test(node-health): add start_local_node wrapper passthrough test`

**Status**: Delivered. Ready for PR against `origin/main`.

---

## Symptom

`overdrive_worker::node_health::write_node_health_row` was a `panic!("Not yet implemented -- RED scaffold")` from Phase 1 with no GitHub issue tracking it. The `phase-1-first-workload` feature was marked complete (`all_steps_done: true`, 7/7 roadmap steps) with this writer still unimplemented. The `run_server_with_obs_and_driver` boot path (`crates/overdrive-control-plane/src/lib.rs:771`) wired the `ObservationStore`, constructed `ExecDriver`, created cgroup slices, minted TLS material, opened the IntentStore, built the reconciler runtime, and bound the listener — but **never invoked the writer**. The Phase 1 boot path silently skipped ADR-0025 step 5; `GET /v1/nodes` returned `[]` on a healthy single-node deployment.

## Root cause

DISTILL-wave scaffold tucked into `crates/overdrive-worker/src/node_health.rs` (the file's docstring claimed *"Phase: phase-1-first-workload, slice 4 (US-04 cgroup isolation shares the boot path with the `node_health` writer)"*), but the slice 4 spec at `docs/feature/phase-1-first-workload/slices/slice-4-control-plane-isolation.md` did NOT enumerate the writer in Scope (in), KPIs, or failure modes. The roadmap at `docs/feature/phase-1-first-workload/deliver/roadmap.json` had zero references to `node_health` / `NodeHealthRow` across its 7 steps. The aspirational docstring was the only thing tying the scaffold to a slice; the gate that should have caught the gap — DELIVER's all-steps-complete check — was satisfied because the work was never scheduled.

ADR-0025 step 5 ("Write one `node_health` row to ObservationStore") and ADR-0029 (writer relocated to worker-subsystem startup) jointly mandate the writer; Alternative D ("no `node_health` row in Phase 1") was explicitly REJECTED in ADR-0025 as *"breaks the §18 architectural invariant: every node writes its own rows."*

## Fix

Two-step `/nw-bugfix` → `/nw-deliver` roadmap. RCA pre-approved by user 2026-05-24 in transcript; full RCA at `docs/feature/fix-orphaned-node-health-writer/deliver/rca.md`.

### Step 01-01 — Regression test (RED)

Added `crates/overdrive-control-plane/tests/integration/node_health_writer_runs_at_boot.rs::boot_writes_exactly_one_node_health_row_to_observation_store` — boots `run_server_with_obs_and_driver` against a `SimObservationStore`, asserts `obs.node_health_rows().len() == 1` with override-derived `node_id`, configured `region`, and non-default `last_heartbeat.counter`. Port-to-port: deletes the future `start_local_node` call and the test flips RED. Deliberate RED state — fails with `assertion left: 0, right: 1` against current `origin/main`.

Removed the orphan `#[should_panic(expected = "Not yet implemented")]` pin from `crates/overdrive-worker/src/node_health.rs::tests` per `.claude/rules/testing.md` § *RED scaffolds*: *"When slice 4 GREEN lands, this test is REMOVED."* The new boot-time regression IS the structural contract.

### Step 01-02 — Implementation (GREEN)

Five production edits + extended test surface:

1. **Writer body** (`crates/overdrive-worker/src/node_health.rs`) — implemented `write_node_health_row` per ADR-0025 step 5. Resolves `NodeId` via `config.id_override` → `hostname::get()` fallback (three distinguishable failure paths surfaced as discrete `NodeHealthWriteError::IdResolve` messages per `.claude/rules/development.md` § *Distinct failure modes get distinct error variants*). Builds `NodeHealthRowV1` with `last_heartbeat: LogicalTimestamp { counter: clock.unix_now().as_secs(), writer: node_id.clone() }` per § *Persist inputs, not derived state*. Writes via `obs.write(ObservationRow::NodeHealth(row))`. Signature gained mandatory `clock: &Arc<dyn Clock>` parameter per § *Port-trait dependencies* (never builder-defaulted; tests cannot silently inherit wall-clock behaviour).
2. **Worker-startup helper** (`crates/overdrive-worker/src/lib.rs`) — new `pub async fn start_local_node(obs, config, clock)` exported from `lib.rs`. Single-purpose passthrough today; the ADR-0029 contract-boundary entry point so the control-plane composition root only knows the worker subsystem by its driving port. Phase 2+ additions (heartbeat reconciler scheduling, capacity probe, driver-readiness handshake) extend this function without changing the boundary. `NodeConfig` re-exported alongside.
3. **Boot-path wiring** (`crates/overdrive-control-plane/src/lib.rs`) — inserted `overdrive_worker::start_local_node(&obs, &server_config.node, &clock).await.map_err(error::ControlPlaneError::from)?` between `create_workloads_slice_with_controllers` (~L810) and `rustls::crypto::ring::default_provider().install_default()` (~L835) per ADR-0025 step 5 ordering: ObservationStore wired, before TLS material, before listener bind. `ServerConfig` gained an additive `node: NodeConfig` field; `Clock` threaded through.
4. **Typed error variant** (`crates/overdrive-control-plane/src/error.rs`) — `ControlPlaneError::NodeHealthWrite(#[from] overdrive_worker::node_health::NodeHealthWriteError)`. NOT `ControlPlaneError::internal("...", e)` per `.claude/rules/development.md` § *"Never flatten a typed error to `Internal(String)` at a composition boundary"*. Slots into the existing `Cgroup` / `Tls` / `ViewStoreBoot` sibling-variant pattern; `to_response` arm preserves the established INTERNAL_SERVER_ERROR shape.
5. **Hostname dep** — added `hostname.workspace = true` to `crates/overdrive-worker/Cargo.toml`.

Test surface:
- 01-01 regression test now GREEN.
- Two unit tests on the writer (`write_with_id_override_resolves_to_override_and_uses_clock`, `write_without_id_override_falls_back_to_hostname`) covering both `resolve_node_id` branches as DISTINCT tests.
- Lima integration test: `boot_writes_node_health_row_visible_via_get_v1_nodes` exercises the full HTTPS chain through to the operator-visible KPI.
- Updated 6 existing tests that asserted `GET /v1/nodes` returns `[]` on fresh boot to expect the new "one row at boot" reality. None loosened to `!rows.is_empty()`; honest count assertions throughout.

### Adversarial review iteration

Reviewer flagged the crate-level `pub const SCAFFOLD: bool = true;` in `crates/overdrive-worker/src/lib.rs:27` and Cargo.toml comment as stale — grep confirmed `cgroup_manager.rs` and `driver.rs` had no SCAFFOLD markers (GREENed in Phase 1 slices 2 + 4); with `node_health` now GREEN the crate is fully implemented, so the crate-level markers were lies. Dropped in commit `f86a9eca` per `.claude/rules/development.md` § *Deletion discipline* ("removed is removed").

### Mutation iteration

`cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-worker` surfaced one MISSED mutant: `start_local_node` body → `Ok(())`. Killer test lived in the control-plane integration suite, but `--package overdrive-worker --test-workspace=false` scoped the build's feature space such that the control-plane `integration-tests`-gated regression test wasn't compiled. Added a co-located unit test `start_local_node_wrapper_observably_writes_row` in `node_health.rs::tests` (commit `9750161d`) asserting the wrapper observably writes a row. Mutation re-run: 2/2 caught, 100.0% kill rate, PASS.

## Files touched

| File | Net change | Reason |
|---|---|---|
| `crates/overdrive-worker/src/node_health.rs` | +194 / −9 | Writer body + 3 unit tests |
| `crates/overdrive-worker/src/lib.rs` | +51 / −5 | `start_local_node` helper + `NodeConfig` re-export; SCAFFOLD const dropped |
| `crates/overdrive-worker/Cargo.toml` | +1 / −5 | hostname dep added; SCAFFOLD comment dropped |
| `crates/overdrive-control-plane/src/lib.rs` | +~40 / −~5 | `start_local_node` call site; `ServerConfig.node: NodeConfig` field; Clock thread-through |
| `crates/overdrive-control-plane/src/error.rs` | +5 | `NodeHealthWrite` variant |
| `crates/overdrive-control-plane/tests/integration.rs` | +1 | Mod declaration |
| `crates/overdrive-control-plane/tests/integration/node_health_writer_runs_at_boot.rs` | +~160 | New file: 2 integration tests |
| 6 existing test files | +/− small | Updated `[]` → `[{...}]` expectations |

## Verification

| Gate | Outcome |
|---|---|
| Step 01-01 regression test (boot) | GREEN: `1, 1` (boot writes the row) |
| Step 01-02 unit tests (override + hostname + wrapper) | 3 / 3 PASS |
| Lima integration: `GET /v1/nodes` | PASS (single-element array) |
| `cargo nextest run --workspace --features integration-tests` (Lima) | 1034 / 1034 + 10 skipped |
| `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` (Lima) | clean |
| `cargo check --workspace --features integration-tests` (Lima) | clean |
| dst-lint (Lima) | clean |
| `cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-worker` | 2 / 2 caught, kill rate 100.0% — PASS |
| `cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane` | 0 mutants (additive-only diff) — vacuous PASS |
| DES integrity (`verify_deliver_integrity`) | All 2 steps have complete DES traces |
| Pre-existing flake `s_cp_04_broadcast_emits_exactly_n_events_in_order` | NOT a regression — slow-proptest contention pre-dates this feature |

## What this fix is NOT

- **Not a periodic heartbeat reconciler.** ADR-0025 frames the writer as one-shot at boot. Future periodic-write reconciler lands in its own slice when ADR-0011's periodic-write surface materialises.
- **Not a worker-startup framework.** `start_local_node` is single-purpose today. It grows when ADR-0029 introduces more worker-startup concerns. No speculative scaffolding per `feedback_single_cut_greenfield_migrations`.
- **Not a refactor of `run_server_with_obs_and_driver`.** Single call site added; existing boot order preserved.

## Lessons

1. **DISTILL scaffolds need roadmap enumeration.** A `panic!("Not yet implemented -- RED scaffold")` body co-located with future-feature DISTILL wave is invisible to the DELIVER all-steps-complete gate unless a roadmap step explicitly schedules its GREEN. The next prevention candidate: an xtask CI gate enumerating every `RED scaffold` panic in `crates/*/src/` and asserting each maps to either (a) an open roadmap step or (b) a tracked GitHub issue. Out of scope for this fix; surface candidate for a future tooling slice.
2. **Per-package mutation testing's blind spot.** `cargo xtask mutants --package X` enables `--features integration-tests` only on package X's feature space. Tests in another package gated by THAT package's `integration-tests` feature don't compile and silently can't kill. The reliable fix is co-located coverage in the mutated package — even for thin wrappers that look "too trivial to test." A single observable-write assertion in the mutated crate's tests closes the gap.
3. **Reviewer "BLOCKER" with wrong stated reasoning can still be correct.** The adversarial reviewer cited the RCA as mandating the SCAFFOLD removal — the RCA didn't. But grep confirmed the const WAS stale (sibling modules had been GREENed in Phase 1 and the marker was the last lie standing). Verifying the underlying claim before either applying or pushing back is cheap and resolves correctness from a wrong-citation finding.

## SSOT references

- ADR-0025 — Single-node startup wiring; § 3 step 5 (write the row) and § Amendment 2026-04-27 (relocate to worker startup).
- ADR-0029 — `overdrive-worker` crate extraction; worker subsystem owns the writer.
- `.claude/rules/development.md` — Errors (typed `#[from]`, no `Internal(String)` flatten); Port-trait dependencies (mandatory `Clock` injection); Persist inputs, not derived state; Deletion discipline; Single-cut greenfield migrations.
- `.claude/rules/testing.md` — Integration vs unit gating (workspace `integration-tests` convention); RED scaffolds (`#[should_panic(expected = "RED scaffold")]` removal on GREEN); Mutation testing (per-feature kill rate ≥ 80%, vacuous-pass shape, per-package scoping caveat).
