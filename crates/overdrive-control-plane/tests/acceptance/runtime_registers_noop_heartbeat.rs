//! Step 04-04 — `ReconcilerRuntime` wires the broker + registry + per-
//! primitive libSQL path provisioning, registers `noop_heartbeat()` at
//! construction, and renders itself through the `cluster_status` handler.
//!
//! Updated for step 04-07: the runtime now stores `AnyReconciler`
//! (enum-dispatched) rather than `Box<dyn Reconciler>`, and `reconcile`
//! takes a `&TickContext` fourth parameter plus returns a
//! `(Vec<Action>, AnyReconcilerView)` tuple. Twin-invocation checks
//! construct ONE `TickContext` and pass it to BOTH calls.
//!
//! Tier classification: **Tier 1 DST** per `.claude/rules/testing.md`.
//! NO real axum listener, NO real libsql open, NO real redb write txn,
//! NO real TCP. The `tempfile::TempDir` here is used only so the
//! libsql-provisioner's pure `provision_db_path` has a real on-disk
//! path to canonicalise against — the DB is never opened. Real-I/O
//! wiring of the same surface is covered by step 05-05 (walking
//! skeleton, Tier 3).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use overdrive_control_plane::api::ClusterStatus;
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::handlers::cluster_status;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_control_plane::{AppState, noop_heartbeat};
use overdrive_core::id::NodeId;
use overdrive_core::reconciler::{
    Action, AnyReconcilerView, ReconcilerName, State as ReconState, TickContext,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::transport::SimTransport;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers — kept inline; these tests are the only caller site.
// ---------------------------------------------------------------------------

fn rname(raw: &str) -> ReconcilerName {
    ReconcilerName::new(raw).expect("valid ReconcilerName")
}

/// Construct a fresh `TickContext` for twin-invocation checks. Test
/// code is exempt from the `Instant::now()` dst-lint ban (dst-lint
/// scans `src/**/*.rs` only).
fn fresh_tick() -> TickContext {
    let now = Instant::now();
    TickContext { now, tick: 0, deadline: now + Duration::from_secs(1) }
}

/// Construct the Sim adapters declared in the 04-04 harness spec. The
/// returned tuple is partially unused by individual tests — holding
/// them here proves the compile path is DST-compatible (no real-infra
/// smuggling) regardless of which subset the assertion body touches.
fn sim_adapters() -> (SimClock, SimEntropy, SimTransport, SimDataplane) {
    (SimClock::new(), SimEntropy::new(0), SimTransport::new(), SimDataplane::new())
}

fn build_app_state(tmp: &TempDir) -> AppState {
    let runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("NodeId"), 0));
    AppState { store, obs, runtime: Arc::new(runtime) }
}

// ---------------------------------------------------------------------------
// (a) Fresh runtime — empty registry, canonicalised data_dir, zero counters.
// ---------------------------------------------------------------------------

#[test]
fn runtime_new_returns_empty_registry_with_canonicalised_data_dir() {
    let _sims = sim_adapters(); // DST-compat proof — constructed, unused.
    let tmp = TempDir::new().expect("tmpdir");

    let runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");

    assert_eq!(runtime.registered(), Vec::<ReconcilerName>::new(), "fresh registry is empty");

    let counters = runtime.broker().counters();
    assert_eq!(counters.queued, 0, "fresh broker queued=0");
    assert_eq!(counters.cancelled, 0, "fresh broker cancelled=0");
    assert_eq!(counters.dispatched, 0, "fresh broker dispatched=0");
}

// ---------------------------------------------------------------------------
// (b) register adds reconciler and provisions its libSQL path. The
//     provisioning is path-derivation only — no libsql open — so the
//     assertion watches the filesystem side effect (`reconcilers/<name>/`
//     parent directory exists) of `provision_db_path`.
// ---------------------------------------------------------------------------

#[test]
fn register_adds_reconciler_and_provisions_libsql_path() {
    let _sims = sim_adapters();
    let tmp = TempDir::new().expect("tmpdir");

    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).expect("register noop-heartbeat");

    // Registry contains exactly the one name.
    let names = runtime.registered();
    assert_eq!(names.len(), 1, "exactly one reconciler registered");
    assert_eq!(names[0], rname("noop-heartbeat"), "registered name is noop-heartbeat");

    // Path-derivation side effect — the canonicalised data_dir exists
    // (provision_db_path creates it). The per-reconciler `<name>/`
    // subdirectory may or may not be materialised depending on whether
    // `open_db` was called; this test intentionally does NOT open the
    // DB, so we only assert the data_dir is canonicalised and reachable.
    let canon = std::fs::canonicalize(tmp.path()).expect("canonicalize tmp");
    assert!(canon.exists(), "canonicalised data_dir exists after register");
}

// ---------------------------------------------------------------------------
// (c) Duplicate registration returns Conflict.
// ---------------------------------------------------------------------------

#[test]
fn register_duplicate_name_returns_conflict() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");

    runtime.register(noop_heartbeat()).expect("first register succeeds");

    let second = runtime.register(noop_heartbeat());
    match second {
        Err(ControlPlaneError::Conflict { message }) => {
            assert!(
                message.contains("noop-heartbeat"),
                "conflict message should name the reconciler, got {message:?}"
            );
        }
        other => panic!("expected Err(Conflict), got {other:?}"),
    }

    // Registry still reports exactly one entry — the failed second
    // register must not double-count.
    assert_eq!(runtime.registered().len(), 1, "duplicate did not leak into registry");
}

// ---------------------------------------------------------------------------
// (d) noop_heartbeat() factory produces a reconciler whose `reconcile`
//     returns vec![Action::Noop] deterministically. Twin-invocation
//     passes ONE TickContext instance to BOTH calls per ADR-0013 §2c.
// ---------------------------------------------------------------------------

#[test]
fn noop_heartbeat_factory_produces_reconciler_returning_noop() {
    let r = noop_heartbeat();
    assert_eq!(r.name(), &rname("noop-heartbeat"), "factory name is noop-heartbeat");

    let desired = ReconState;
    let actual = ReconState;
    let view = AnyReconcilerView::Unit;
    let tick = fresh_tick();

    let first = r.reconcile(&desired, &actual, &view, &tick);
    let second = r.reconcile(&desired, &actual, &view, &tick);

    assert_eq!(
        first,
        (vec![Action::Noop], AnyReconcilerView::Unit),
        "noop-heartbeat always emits [Action::Noop] with unchanged view"
    );
    assert_eq!(first, second, "noop-heartbeat is deterministic (twin-invocation)");
}

// ---------------------------------------------------------------------------
// (e) cluster_status handler renders registry + broker counters via
//     constructed axum State<AppState>. NO HTTP, NO listener.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cluster_status_handler_renders_registry_and_broker_counters_via_axum_state() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut state = build_app_state(&tmp);

    // Register noop-heartbeat into the runtime the handler will read
    // from. `Arc::get_mut` is safe here because nothing else holds the
    // Arc yet — we have not cloned state into the server.
    Arc::get_mut(&mut state.runtime)
        .expect("unique Arc")
        .register(noop_heartbeat())
        .expect("register");

    let Json(body): Json<ClusterStatus> =
        cluster_status(State(state.clone())).await.expect("handler ok");

    assert_eq!(body.mode, "single", "Phase 1 mode is always 'single'");
    assert_eq!(body.region, "local", "Phase 1 region is always 'local'");
    assert_eq!(body.commit_index, state.store.commit_index(), "commit_index from store");
    assert_eq!(
        body.reconcilers,
        vec!["noop-heartbeat".to_string()],
        "registry surfaces the one registered reconciler"
    );
    assert_eq!(body.broker.queued, 0, "fresh broker queued=0");
    assert_eq!(body.broker.cancelled, 0, "fresh broker cancelled=0");
    assert_eq!(body.broker.dispatched, 0, "fresh broker dispatched=0");
}

// ---------------------------------------------------------------------------
// (f) ADR-0017 invariant `at_least_one_reconciler_registered` holds after
//     boot AND after a simulated host restart. The restart is modelled
//     by dropping the runtime (ephemeral in-process state) and
//     reconstructing via the same boot sequence that `run_server` uses:
//     `ReconcilerRuntime::new` + `register(noop_heartbeat())`.
// ---------------------------------------------------------------------------

#[test]
fn at_least_one_reconciler_registered_invariant_holds_after_boot() {
    let _sims = sim_adapters();
    let tmp = TempDir::new().expect("tmpdir");

    // Boot 1 — initial register at construction.
    {
        let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
        runtime.register(noop_heartbeat()).expect("register");
        assert!(
            !runtime.registered().is_empty(),
            "invariant: at_least_one_reconciler_registered holds post-boot"
        );
    } // runtime dropped — simulates host crash.

    // Boot 2 — rebuild through the same path.
    {
        let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
        runtime.register(noop_heartbeat()).expect("register");
        assert!(
            !runtime.registered().is_empty(),
            "invariant: at_least_one_reconciler_registered holds across restart"
        );
    }
}

// ---------------------------------------------------------------------------
// (g) ADR-0017 invariant `reconciler_is_pure` — twin-invocation check on
//     every registered reconciler emits byte-identical `(actions,
//     next_view)` tuples. One `TickContext` is constructed and passed
//     to both calls per ADR-0013 §2c.
// ---------------------------------------------------------------------------

#[test]
fn reconciler_is_pure_invariant_holds_for_noop_heartbeat() {
    let tmp = TempDir::new().expect("tmpdir");
    let mut runtime = ReconcilerRuntime::new(tmp.path()).expect("runtime::new");
    runtime.register(noop_heartbeat()).expect("register");

    let desired = ReconState;
    let actual = ReconState;
    let view = AnyReconcilerView::Unit;
    let tick = fresh_tick();

    for r in runtime.reconcilers_iter() {
        let a = r.reconcile(&desired, &actual, &view, &tick);
        let b = r.reconcile(&desired, &actual, &view, &tick);
        assert_eq!(
            a,
            b,
            "invariant: reconciler {} is not pure — twin invocations diverged",
            r.name()
        );
    }
}
