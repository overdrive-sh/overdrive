//! Quint conformance drivers of the Service deploy stream — `progress_deadline`
//! (D10, ADR-0262, ADR-0264; instance `pd_conf`) and `stream_snapshot` (D11,
//! ADR-0263; instance `ss_conf`) of feature
//! `service-allocation-readiness-and-ending`.
//!
//! Both replay `specs/quint/service-allocation-readiness-and-ending/*.qnt` in place
//! (referenced, never copied) against the production owner, the Service submit
//! stream handler: `progress_deadline` through the production `submit_workload`
//! handler (the NDJSON lane, `Inserted` for the first stream and `Unchanged` for a
//! redeploy) and `stream_snapshot` through `build_service_stream` itself. Each
//! step's action and picks are read from the single-element path
//! `"<main>::<module>::conf_step"` ([`quint_connect::Config::nondet`]); the state
//! projection matches the trace's qualified variable names
//! (`<main>::<module>::<var>`, spike QT1).
//!
//! # Placement
//!
//! In the owner's crate `src/`, behind `#[cfg(all(test, feature =
//! "integration-tests"))]` (`.claude/rules/testing.md` § "Quint conformance",
//! "Placement and lane"): the drivers compose a crate-private `AppState`
//! (`AppState::new`, the guest-network owner, the shared intercept worker) that no
//! out-of-crate test can reach without a test seam, and they need no real
//! infrastructure beyond the in-process composition.
//!
//! # What is shared
//!
//! Only the composition ([`compose`]) and the two writes the shim performs around
//! every lifecycle terminal — the row commit and the event publish — which the
//! model has as the environment's `Commit` / `Publish` and the pinned projection
//! names as "rows written through `SimObservationStore` and `LifecycleEvent`s sent
//! on the bus at scripted points". The stream, the handler and every projected fact
//! are the production owner's.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::needless_pass_by_value
)]

mod progress_deadline;
mod stream_snapshot;

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::TransitionReason;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

use crate::AppState;
use crate::action_shim::{LifecycleEvent, build_alloc_status_row};
use crate::api::{AllocStateWire, TransitionSource};
use crate::guest_network::SharedGuestNetworkOwner as _;
use crate::identity_mgr::IdentityMgr;
use crate::reconciler_runtime::ReconcilerRuntime;

/// One node composed the way the control plane composes the pieces the stream
/// reads: the intent store, the observation store, the lifecycle bus and the
/// injected clock, with `Sim*` at the driven ports. No reconciler runs: every row
/// and event is a driver step.
struct Composition {
    state: AppState,
    clock: Arc<SimClock>,
    obs: Arc<SimObservationStore>,
    _tmp: TempDir,
}

fn node_id() -> NodeId {
    NodeId::new("local").expect("node id")
}

/// Boot the node under test.
async fn compose(seed: u64) -> Composition {
    let tmp = tempfile::tempdir().expect("tempdir");
    let node = node_id();
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(node.clone(), seed));
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("intent store"));
    let allocator = crate::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let exec_wiring =
        overdrive_core::guest_network::GuestNetworkExecWiring::new(Arc::new(SimClock::new()));
    // This crate's own test-local ports (the `overdrive-sim` doubles implement a second
    // compiled copy of the crate's port traits).
    let mtls_worker = crate::shared_network_test_ports::mtls_worker();
    mtls_worker.start_shared_owner().await.expect("the shared intercept owner starts");
    let owner = crate::shared_network_test_ports::shared_guest_network_owner();
    owner.converge_shared().await.expect("the fixture owner reports its bound ports");
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let state = AppState::new(
        store,
        store_path,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
        Arc::new(runtime),
        driver,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::new(SimDataplane::new()),
        Arc::new(SimCa::new(Arc::new(SimEntropy::new(seed)))),
        Arc::new(IdentityMgr::new(None)),
        node,
        allocator,
        crate::test_empty_listener_facts(),
        Ipv4Addr::LOCALHOST,
        mtls_worker,
        owner,
        exec_wiring.gate(),
        Arc::new(crate::guest_network::GuestAddressPool::new(
            "100.95.0.0/16".parse().expect("static guest prefix"),
            Ipv4Addr::new(100, 95, 0, 1),
            Arc::new(SimEntropy::new(0x295)),
        )),
    );
    assert!(exec_wiring.supervisor().open_after_boot(), "the EXEC gate opens after the owner");
    Composition { state, clock, obs, _tmp: tmp }
}

/// A Service allocation row of `workload`'s attempt `attempt`, stamped `counter`.
fn alloc_row(
    workload: &str,
    attempt: u32,
    counter: u64,
    state: AllocState,
    reason: TransitionReason,
    terminal: Option<TerminalCondition>,
) -> AllocStatusRow {
    let node = node_id();
    build_alloc_status_row(
        AllocationId::new(&format!("alloc-{workload}-{attempt}")).expect("alloc id"),
        WorkloadId::new(workload).expect("workload id"),
        node.clone(),
        state,
        LogicalTimestamp::dominating(counter - 1, node, None),
        Some(reason),
        None,
        terminal,
        None,
        WorkloadKind::Service,
        Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        None,
        None,
    )
}

/// The shim's commit step: the row is written through the observation store.
async fn commit(composition: &Composition, row: &AllocStatusRow) {
    composition
        .obs
        .write_alloc_lifecycle(
            row.clone(),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("the row commits");
}

/// The shim's publish step: the event for the committed write goes on the bus.
fn publish(composition: &Composition, row: &AllocStatusRow) {
    let _ = composition.state.lifecycle_events.send(LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        workload_id: row.workload_id.clone(),
        from: AllocStateWire::Running,
        to: AllocStateWire::from(row.state),
        reason: row.reason.clone().expect("a reason"),
        detail: row.detail.clone(),
        source: TransitionSource::Reconciler,
        at: format!("{}@local", row.updated_at.counter),
        terminal: row.terminal.clone(),
    });
}
