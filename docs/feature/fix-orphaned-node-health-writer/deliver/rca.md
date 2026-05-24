# RCA — Orphaned `write_node_health_row` RED scaffold

**Feature ID**: `fix-orphaned-node-health-writer`
**Wave**: DELIVER (bug-fix scope; RCA pre-approved by user 2026-05-24)
**Branch**: `marcus-sa/node-health-writer`
**Paradigm**: object-oriented (per project `CLAUDE.md`)

## Defect

`crates/overdrive-worker/src/node_health.rs::write_node_health_row` is
`panic!("Not yet implemented -- RED scaffold")`. No GitHub issue
tracks it; no slice spec covers it. The function is referenced from
nowhere — Phase 1 boot path silently skips ADR-0025 step 5.

## Root cause chain

1. **Scaffold origin**: Created in commit `ed5975d8` ("persist
   backoff inputs, recompute deadline each tick") as a 119-line
   drop-in. Author intended it to GREEN as part of slice 4 (docstring
   says *"Phase: phase-1-first-workload, slice 4 (US-04 cgroup
   isolation shares the boot path with the `node_health` writer)"*).
2. **Slice 4 spec did not cover it**: `slices/slice-4-control-plane-isolation.md`
   Scope (in) = cgroup pre-flight + CgroupManager + burst integration
   test. No mention of `NodeHealthRow` in scope, KPIs, or failure
   modes. The docstring's "shares the boot path" claim was
   aspirational, never reflected in the slice contract.
3. **Roadmap gate missed it**: `grep node_health|NodeHealthRow` in
   `docs/feature/phase-1-first-workload/deliver/roadmap.json` returns
   zero matches. 7/7 steps completed; `all_steps_done: true`. Orphaned
   scaffold invisible to DELIVER's completion criteria.
4. **ADR contract mandates the write**: ADR-0025 step 5 ("Write one
   `node_health` row to `ObservationStore`") + ADR-0029 amendment
   (writer relocated to worker-subsystem startup). Alternative D
   ("no `node_health` row in Phase 1") was explicitly REJECTED:
   *"breaks the §18 architectural invariant: every node writes its
   own rows."*
5. **Latent failure mode**: `run_server_with_obs_and_driver`
   (`crates/overdrive-control-plane/src/lib.rs:771`) wires
   `ObservationStore`, calls `ExecDriver::new` and
   `create_workloads_slice_with_controllers`, but never invokes
   `write_node_health_row`. `GET /v1/nodes`
   (`handlers.rs:935`) returns `[]` on a healthy single-node
   deployment.

## Spec sources (authoritative)

- `docs/product/architecture/adr-0025-single-node-startup-wiring.md`
  § 3 — boot sequence with step 5 mandatory; § Amendment 2026-04-27
  for relocation to worker.
- `docs/product/architecture/adr-0029-overdrive-worker-crate-extraction.md`
  — worker subsystem hosts ProcessDriver + cgroup mgmt + node_health
  writer.
- `crates/overdrive-core/src/traits/observation_store.rs:342-362`
  — `NodeHealthRow = NodeHealthRowV1` shape (`node_id`, `region`,
  `last_heartbeat: LogicalTimestamp`).
- `crates/overdrive-core/src/traits/observation_store.rs:489-502`
  — `ObservationRow::NodeHealth(NodeHealthRow)` variant; the only
  write path.

## Approved fix (user 2026-05-24)

### Part A — implement writer body

`crates/overdrive-worker/src/node_health.rs::write_node_health_row`:

- Resolve `NodeId`: `config.id_override` first (parse via `NodeId`
  constructor returning `Result`); fallback to `NodeId::from_hostname()`
  (or equivalent — verify exact API during RED→GREEN). On both failure
  paths, return `NodeHealthWriteError::IdResolve(...)`.
- Build `NodeHealthRowV1 { node_id, region: config.region.clone(),
  last_heartbeat: <clock.logical_now()> }`.
- Write via `obs.write(ObservationRow::NodeHealth(row)).await
  .map_err(|e| NodeHealthWriteError::Write(e.to_string()))?`.

Note: the function signature today does NOT take a `Clock` — the
existing scaffold takes only `obs` + `config`. Adding a `clock`
parameter is part of this fix (the existing test pin will be
removed, so the signature break is tolerable; the production call
site doesn't exist yet).

### Part B — wire into boot path

- New helper `overdrive_worker::start_local_node(obs, node_config,
  clock) -> Result<(), NodeHealthWriteError>` that wraps the
  `write_node_health_row` call. Single-purpose now (the ADR-0029
  contract boundary); can absorb future worker-startup concerns
  without re-touching the control-plane call site.
- Call site in
  `crates/overdrive-control-plane/src/lib.rs::run_server_with_obs_and_driver`,
  inserted between the cgroup-workloads-slice creation
  (`overdrive_worker::cgroup_manager::create_workloads_slice_with_controllers`,
  ~line 794) and the rustls install (~line 805). This places the
  write AFTER `ObservationStore` is wired and BEFORE the listener
  binds, per ADR-0025 step 5 ordering. Failure shape:
  `error::ControlPlaneError` typed `#[from]` variant on
  `NodeHealthWriteError` (per `.claude/rules/development.md`
  § "Errors → pass-through embedding") — NOT
  `ControlPlaneError::internal(...)` (per
  `feedback_no_expect_in_production` adjacent rule about
  preserving typed-error structure across composition boundaries).
- `ServerConfig` may need an additive `[node]` block carrying
  `id_override: Option<String>`, `region: String` (default
  `"local"`), and `capacity: Resources`. Investigate during step
  PREPARE; if absent, add it (additive — no migration needed since
  Phase 1 single-cut greenfield per
  `feedback_single_cut_greenfield_migrations`).

### Test rewrite

1. **DELETE** the existing
   `write_node_health_row_is_red_scaffold_until_slice_4_green` test
   (per `.claude/rules/testing.md` § RED scaffolds: "When slice 4
   GREEN lands, this test is REMOVED").
2. **ADD** unit test in `crates/overdrive-worker/src/node_health.rs`
   `#[cfg(test)] mod tests`: against `SimObservationStore`, call
   `write_node_health_row(&obs, &config, &clock)`, then
   `obs.node_health_rows()` returns exactly one row with the
   expected `node_id` (from override), `region`, and a
   `LogicalTimestamp` derived from the injected `SimClock`.
3. **ADD** regression test (the bug-fix-shaped one) under
   `crates/overdrive-control-plane/tests/integration/` (gated
   `integration-tests`): boot `run_server_with_obs_and_driver`
   with a `SimObservationStore`, then assert
   `obs.node_health_rows()` returns exactly one row. **This test
   fails today** (no boot-time write) and passes after Part B
   wiring. This is the regression contract.
4. **ADD** Lima integration test (gated `integration-tests`,
   Linux-only) hitting `GET /v1/nodes` post-boot — the
   operator-visible KPI. Use the existing test-server harness from
   `tests/integration/backend_discovery_bridge/test_server.rs` as a
   template.

## Files affected

| File | Change |
|---|---|
| `crates/overdrive-worker/src/node_health.rs` | Body implementation + signature gains `clock: &Arc<dyn Clock>` + test rewrite |
| `crates/overdrive-worker/src/lib.rs` | Export new `start_local_node` helper |
| `crates/overdrive-control-plane/src/lib.rs` | Call `overdrive_worker::start_local_node` in `run_server_with_obs_and_driver`; thread `NodeConfig` through `ServerConfig` if needed |
| `crates/overdrive-control-plane/src/error.rs` | `#[from] NodeHealthWriteError` variant on `ControlPlaneError` |
| `crates/overdrive-control-plane/tests/integration/...` | New regression test (in-process) + Lima `GET /v1/nodes` test |
| Existing tests asserting `GET /v1/nodes` returns `[]` on fresh boot | Update to expect the boot-time row |

## Risks

- **High likelihood, low severity**: existing tests that assert
  `GET /v1/nodes` returns `[]` on fresh boot will break. Trivial
  updates; the new expected state is more honest.
- **Medium severity**: boot will refuse to start if the
  `node_health` write errors. Per ADR-0025 this is INTENDED — mirrors
  `LocalIntentStore::open` failure shape. Document the new failure
  mode in `error.rs`'s typed variant docstring.
- **Low**: hostname resolution non-determinism in tests. Mitigated by
  `NodeConfig.id_override`.
- **Low**: `NodeHealthRowEnvelope` rkyv golden-bytes fixture may be
  missing. The envelope at `traits/observation_store.rs:623` already
  has `discriminant_offset_from_end` pinned, suggesting a fixture
  exists. The crafter must verify per `.claude/rules/testing.md`
  § "Archive schema-evolution roundtrip"; if missing, add it
  in-scope (not a deferral — schema-evolution fixtures are
  blocking per project rules).

## What this fix is NOT

- NOT a periodic heartbeat reconciler. ADR-0025 frames the writer as
  one-shot at boot. Future heartbeats land in a separate slice
  (ADR-0011 will own the periodic-write reconciler when it materialises;
  do not anticipate that here per the project's "no aspirational code"
  discipline).
- NOT a refactor of `run_server_with_obs_and_driver`. Single call
  site added; existing boot order preserved.
- NOT a worker-startup framework. The `start_local_node` helper is a
  single-purpose function today; it grows when ADR-0029 introduces
  more worker-startup concerns. Per `feedback_single_cut_greenfield_migrations`
  and `feedback_dont_over_confirm_directive_commands`, no speculative
  scaffolding.
