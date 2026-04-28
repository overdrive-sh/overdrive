# Bugfix RCA: Convergence tick loop never spawned in production server

**Status**: User-approved 2026-04-28. Approved fix shape is **Option B2** (broker-driven §18 wiring), not B1 (IntentStore-scan polling).

---

## Bug summary

`run_convergence_tick` (`crates/overdrive-control-plane/src/reconciler_runtime.rs:164`) is exercised by every integration test that asserts on convergence, but is **never** spawned as a background task by the production server boot path `run_server_with_obs_and_driver` (`crates/overdrive-control-plane/src/lib.rs:284-403`). In production, `submit_job` and `stop_job` only write to the `IntentStore`; nothing drains the broker; no allocations are ever scheduled, started, stopped, or restarted by drivers; `cluster_status.broker.dispatched` permanently reads `0`.

The function's docstring at `reconciler_runtime.rs:155-158` claims *"Production wiring spawns a tokio task that calls this in a loop with `clock.sleep(tick_cadence)` between invocations"* — describing behavior that does not exist. This is an aspirational-doc violation that contributed to the bug surviving review.

## Root cause chain (3 compounding causes)

### A. Production omission

- `run_server_with_obs_and_driver` (`lib.rs:284-403`) constructs `AppState` (line 354), spawns the axum HTTP task (line 400), and returns. There is no `tokio::spawn` of a tick loop between them.
- `Grep run_convergence_tick` returns 14 hits, all in `tests/...` or its own definition + docstring. Zero in `src/`.
- `run_convergence_tick` is `pub async fn`, so the compiler does not warn about the missing call site (it is reachable from the test crate).

### B. `submit_job`/`stop_job` do not enqueue evaluations; no target-enumeration path

- `submit_job` (`handlers.rs:99-150`) writes via `state.store.put_if_absent` and returns. No `state.runtime.broker().enqueue(...)` call. `Grep broker.*submit|enqueue|Evaluation` in `handlers.rs` → zero hits.
- Tests pass `TargetResource::new("job/payments")` hardcoded; production has no equivalent enumeration path.
- This violates whitepaper §18 *Triggering Model — Hybrid by Design*: "External state changes (job submission, ...) produce a typed `Evaluation` enqueued through Raft."

### C. No automated gate on `broker.dispatched`

- `cluster_status` (`handlers.rs:299-340`) exposes `broker.dispatched` (line 340). No test asserts it ever advances under traffic. `dispatched=0` is structurally indistinguishable from "no submissions yet" vs "convergence loop dead."
- The 5 tests that boot the real server (`submit_round_trip`, `describe_round_trip`, `idempotent_resubmit`, `concurrent_submit_toctou`, `server_lifecycle`) submit jobs but do not assert on convergence outcomes.
- The 3 tests that assert on convergence (`submit_to_running.rs`, `crash_recovery.rs`, `stop_to_terminated.rs`) call `run_convergence_tick` directly per-tick, bypassing `run_server_with_obs_and_driver` entirely (Fixture-Theater shape).

## Approved fix: Option B2 (broker-driven §18 wiring)

Rejected: Option B1 (IntentStore-scan per tick) because it is throwaway intermediate code that contradicts §18 and would be deleted when the broker wiring lands properly.

### Code changes

1. **`crates/overdrive-control-plane/src/lib.rs` — `ServerConfig`:**
   - Add `tick_cadence: Duration` (default `DEFAULT_TICK_CADENCE` = 100ms from `reconciler_runtime.rs:144`).
   - Add `clock: Arc<dyn overdrive_core::traits::clock::Clock>` (default `Arc::new(SystemClock)` from `overdrive-host`).
   - Update `Default` impl so existing test fixtures using `..Default::default()` continue to compile.

2. **`crates/overdrive-control-plane/src/lib.rs` — `ServerHandle`:**
   - Add `convergence_task: tokio::task::JoinHandle<()>` and `convergence_shutdown: tokio_util::sync::CancellationToken`.
   - Update `shutdown(...)` ordering: cancel `convergence_shutdown` → await `convergence_task` → axum graceful → await `server_task`. Reversing this is a medium-risk failure shape (reconciler tasks holding `Arc<dyn Driver>` refs while axum tries to shut down).

3. **`crates/overdrive-control-plane/src/lib.rs` — `run_server_with_obs_and_driver`:**
   - Between `AppState::new` (line 354) and listener bind (line 371), spawn the convergence loop:
     ```rust
     let convergence_shutdown = tokio_util::sync::CancellationToken::new();
     let convergence_task = {
         let state = state.clone();
         let clock = config.clock.clone();
         let cadence = config.tick_cadence;
         let token = convergence_shutdown.clone();
         tokio::spawn(async move {
             let mut tick_n: u64 = 0;
             loop {
                 let now = clock.now();
                 let deadline = now + cadence;
                 // §18 broker-driven: drain pending evaluations
                 let pending = state.runtime.broker().drain_pending();
                 for eval in pending {
                     if let Err(e) = run_convergence_tick(
                         &state, eval.target(), now, tick_n, deadline,
                     ).await {
                         tracing::warn!(target: "overdrive::reconciler", ?e, "tick error");
                     }
                 }
                 tick_n = tick_n.saturating_add(1);
                 tokio::select! {
                     _ = clock.sleep(cadence) => {},
                     _ = token.cancelled()    => break,
                 }
             }
         })
     };
     ```

4. **`crates/overdrive-control-plane/src/handlers.rs` — `submit_job` and `stop_job`:**
   - After the `IntentStore` write succeeds, call `state.runtime.broker().enqueue(Evaluation::new(<reconciler_name>, TargetResource::new(format!("job/{job_id}"))))`. The broker keys by `(reconciler, target_resource)` and collapses duplicates per §18 evaluation-broker semantics.

5. **`crates/overdrive-control-plane/src/reconciler_runtime.rs` — `run_convergence_tick`:**
   - When `reconcile` returns `actions.len() > 0` (i.e., desired ≠ actual), re-enqueue `(reconciler_name, target)` so the next tick re-evaluates. This is the "level-triggered inside the reconciler" half of the §18 hybrid model and is mandatory: without it, the reconciler runs once after submit, the broker drains empty, and convergence stalls.
   - Replace the aspirational docstring at lines 155-158 with one that names the actual call site (`run_server_with_obs_and_driver` in `lib.rs`) and references the `ServerHandle::convergence_task` shutdown ordering.

6. **`crates/overdrive-control-plane/Cargo.toml`:**
   - Add `tokio-util = { workspace = true, features = ["rt"] }` if not already present.

7. **`crates/overdrive-cli/...` (the `overdrive serve` call site):**
   - Construct `Arc::new(overdrive_host::SystemClock)` and pass it through `ServerConfig.clock`. Per CLAUDE.md, `overdrive-host` is the only crate permitted to instantiate `SystemClock`.

## Files affected

- `crates/overdrive-control-plane/src/lib.rs`
- `crates/overdrive-control-plane/src/handlers.rs`
- `crates/overdrive-control-plane/src/reconciler_runtime.rs`
- `crates/overdrive-control-plane/Cargo.toml`
- `crates/overdrive-cli/src/...` (whichever file calls `run_server_with_obs_and_driver`)
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/convergence_loop_spawned_in_production_boot.rs` (new — regression test)

Existing test fixtures using `..Default::default()` for `ServerConfig` should continue to compile if the `Default` impl is updated correctly.

## Risk: Medium

- **Existing real-server tests** (`submit_round_trip`, `describe_round_trip`, `idempotent_resubmit`, `concurrent_submit_toctou`, `server_lifecycle`, `observation_empty_rows`): after the fix, these tests will see the convergence loop spawn under their `Default::default()`-constructed `ServerConfig`. With `Arc::new(SystemClock)` (real time) they may race; on Linux they could leak `ProcessDriver`-spawned processes. **Required follow-up**: audit each test for orphaned-process risk under the new spawn behavior. Mitigation in this PR: ensure `ServerHandle::shutdown` cancels the convergence loop and joins the task before letting `Arc<dyn Driver>` drop.
- **Startup-order race**: `LocalObservationStore` is initialized synchronously in `wire_single_node_observation` (`lib.rs:251`) before `AppState::new`. No race.
- **DST invariants and existing acceptance tests**: unaffected — they don't go through `run_server_with_obs_and_driver`.
- **Phase 1 single-node scope**: per CLAUDE.md, Phase 1 has no node registration / multi-region. The broker-driven shape is correct for single-node and extends naturally to multi-node when §18's full Raft-enqueued evaluation path lands.

## Regression test design

**Path**: `crates/overdrive-control-plane/tests/integration/job_lifecycle/convergence_loop_spawned_in_production_boot.rs`

**Gate**: `#![cfg(feature = "integration-tests")]` at the top, registered in `tests/integration.rs` per `.claude/rules/testing.md` § "Layout — integration tests live under `tests/integration/`".

**Shape** (SimClock + SimDriver, runs in default lane on macOS):

```rust
#![cfg(feature = "integration-tests")]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_sim::adapters::{SimClock, SimDriver, SimObservationStore};
// ... AllocState, JobSpecInput, etc.

#[tokio::test]
async fn submitted_job_reaches_running_via_real_server_boot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let clock = Arc::new(SimClock::new(/* seed */ 42));
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: temp.path().join("data"),
        operator_config_dir: temp.path().join("config"),
        allow_no_cgroups: true,
        tick_cadence: Duration::from_millis(100),
        clock: clock.clone(),
        ..Default::default()
    };
    let obs = Arc::new(SimObservationStore::new());
    let driver = Arc::new(SimDriver::new(/* DriverType::Process */));

    let handle = run_server_with_obs_and_driver(config, obs.clone(), driver.clone())
        .await
        .expect("server boot");
    let endpoint = format!("https://{}", handle.local_addr().await.expect("bound"));

    submit_job_http(&endpoint, "payments", /* replicas */ 1).await;

    for _ in 0..30 {
        clock.advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
    }

    // Assertion 1: broker.dispatched advanced (catches Root Cause C — activity gate).
    let info = get_cluster_info(&endpoint).await;
    assert!(
        info.broker.dispatched >= 1,
        "broker.dispatched must advance under steady-state traffic; got {}",
        info.broker.dispatched
    );

    // Assertion 2: workload reached Running (catches A + B together).
    let allocs = get_alloc_status(&endpoint).await;
    assert!(
        allocs.iter().any(|a| a.job_id == "payments" && a.state == "Running"),
        "submitted job must reach Running via the production convergence loop; got {:?}",
        allocs
    );

    handle.shutdown(Duration::from_secs(1)).await;
}
```

**Why this is the right test**:
- Boots `run_server_with_obs_and_driver` end-to-end (closes the seam all 5 existing real-server tests leave open).
- SimClock + SimDriver → runs on macOS in the default `--features integration-tests` lane (no Linux kernel, no `ProcessDriver` cleanup).
- Asserts on **both** `broker.dispatched ≥ 1` (Root Cause C) AND alloc reaches `Running` (Roots A + B).
- Today: `dispatched=0`, no `Running`, fails. After the fix: passes. Would have caught the bug at PR time.

## Test-coverage gaps to flag (NOT in scope for this PR)

- DST harness instantiates `run_convergence_tick` directly; no DST property targets `run_server_with_obs_and_driver` end-to-end. File a follow-up to extend the DST harness to drive the production server in-process.
- 5 existing real-server tests (`submit_round_trip` etc.) audit for orphaned-process risk under new spawn behavior — should land in the same PR as a precaution, depending on what the audit shows.

## Project-rule cross-references

- Whitepaper §18 *Reconciler and Workflow Primitives* / *Triggering Model — Hybrid by Design* — defines the broker-driven evaluation shape this fix conforms to.
- Whitepaper §21 *Deterministic Simulation Testing* — `SimClock` / `SimDriver` injection used by the regression test.
- `.claude/rules/testing.md` § "Integration vs unit gating" — placement of the regression test under `tests/integration/` behind `#![cfg(feature = "integration-tests")]`.
- `.claude/rules/development.md` § "Reconciler I/O" — `tick.now` from `TickContext`, no `Instant::now()` inside `reconcile`. (The spawn loop itself is in a wiring crate, where direct `Clock::now`/`Clock::sleep` via the injected trait is allowed.)
- CLAUDE.md "Repository structure" — `overdrive-host` owns `SystemClock`; `overdrive-sim` owns `SimClock`; reconcilers and the wiring crate consume the trait.
- Memory `feedback_single_cut_greenfield_migrations.md` — landing B2 directly (not B1 then B2) honors the single-cut rule.
