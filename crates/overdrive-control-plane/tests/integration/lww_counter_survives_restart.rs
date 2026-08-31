//! T2 (ADR-0077 § D7 Layer 3) — the cross-restart LWW regression test.
//!
//! The RCA's Probe A promoted to a permanent test
//! (`docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`).
//!
//! # The defect
//!
//! Observation rows are fsync-durable across restarts; the convergence
//! tick is not — `spawn_convergence_loop` declares `tick_n = 0` on every
//! process start. The deleted `timestamp_for` derived a row's LWW counter
//! from the tick alone, so after a restart every write for a SURVIVING
//! allocation stamped a counter BELOW the durable row's, `dominates`
//! returned `false`, and the write was silently discarded — for a window
//! equal to the previous process's uptime (measured: prior counter 4 →
//! 0.5 s; 269 → 29 s; 522 → 52 s). The operator-visible symptom was worse
//! than a delay: `overdrive job stop` printed `Stopped workload ...` and
//! exited **0** while the store still read `Running`, because
//! `ObservationStore::write` returns `Ok(())` on a dropped write.
//!
//! # Why this test must reopen a REAL store
//!
//! The substrate behaviour under test is "the tick counter resets while
//! the rows do not." An in-memory fixture cannot express it — the rows
//! would vanish with the process. So this drops and reopens a real
//! `LocalObservationStore` on the same redb path, exactly as
//! `overdrive-store-local`'s own `restart_round_trip_alloc_status`
//! (`tests/acceptance/local_observation_store.rs`) does, and then drives
//! the PRODUCTION driving port `action_shim::dispatch` at `tick = 0`.
//!
//! Run via `cargo xtask lima run -- cargo nextest run -p
//! overdrive-control-plane --features integration-tests`. Never
//! `--no-run` — a compile-only gate proves nothing about a substrate.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;

/// The pre-restart counter. Deliberately the RCA's measured 29 s-outage
/// value: a process that ran ~27 s before restarting. Any value above the
/// post-restart tick reproduces the defect; this one keeps the test tied
/// to a measurement rather than an invented number.
const PRIOR_COUNTER: u64 = 269;

/// The tick the post-restart dispatch runs at. `0` is not a contrivance —
/// it is the literal initialiser in `spawn_convergence_loop`, so this is
/// the FIRST convergence tick of the new process.
const POST_RESTART_TICK: u64 = 0;

fn ids() -> (AllocationId, WorkloadId, NodeId) {
    (
        AllocationId::new("lww-restart-alloc").expect("valid alloc id"),
        WorkloadId::new("lww-restart-workload").expect("valid workload id"),
        // The compile-time literal `run_server` uses (`lib.rs`), which is
        // what makes `dominates`' tiebreak arm evaluate `"local" > "local"`
        // → deterministic `false`. Using any other writer here would let
        // the tiebreak rescue a tied counter and mask the defect.
        NodeId::new("local").expect("valid node id"),
    )
}

/// The `Running` row that survives the restart.
fn surviving_running_row(counter: u64) -> AllocStatusRow {
    let (alloc_id, workload_id, node_id) = ids();
    AllocStatusRow {
        alloc_id,
        workload_id,
        node_id,
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: ids().2 },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn tick_at(tick: u64) -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick,
        deadline: now + Duration::from_secs(120),
    }
}

/// The VIP allocator the dispatch signature requires; the `StopAllocation`
/// arm never touches it, so a one-address pool suffices.
fn build_vip_allocator(
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> {
    let cidr = ipnet::Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 prefix");
    let range = VipRange::new(vec![cidr], std::collections::BTreeSet::new()).expect("vip range");
    Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(range, store)))
}

/// Drive the PRODUCTION driving port for one action at a given tick.
async fn dispatch_one_at_tick(
    action: Action,
    obs: &dyn ObservationStore,
    intent: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
    tick: u64,
) -> Result<(), overdrive_control_plane::action_shim::ShimError> {
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    let net_slot_allocator = NetSlotAllocator::new();

    dispatch(
        vec![action],
        drivers.as_ref(),
        &alloc_drivers,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &overdrive_sim::adapters::clock::SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick_at(tick),
        &ids().2,
        build_vip_allocator(intent),
        &broker,
        None,
        None,
        &net_slot_allocator,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
}

// ---------------------------------------------------------------------------
// THE regression test.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_after_restart_wins_the_lww_merge_at_tick_zero() {
    let tmp = TempDir::new().expect("tempdir");
    let obs_path = tmp.path().join("observation.redb");
    let intent: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent"));
    let (alloc_id, _, _) = ids();

    // --- Process lifetime 1: the alloc reaches Running at a high counter. ---
    {
        let store = LocalObservationStore::open(&obs_path).expect("open (lifetime 1)");
        store
            .write_alloc_lifecycle(
                surviving_running_row(PRIOR_COUNTER),
                overdrive_core::traits::observation_store::TransitionSource::Reconciler,
            )
            .await
            .expect("seed the surviving Running row");
        drop(store);
    }

    // --- Process lifetime 2: a NEW process, so the tick is back at 0. ---
    let store = LocalObservationStore::open(&obs_path).expect("open (lifetime 2)");

    // The substrate fact this whole ADR is about: the ROW survived the
    // restart at its full counter, while the tick did not.
    let survived = store.alloc_status_row(&alloc_id).await.expect("read after reopen");
    let survived = survived.expect("the Running row must survive the reopen");
    assert_eq!(
        survived.updated_at.counter, PRIOR_COUNTER,
        "precondition: the durable counter survives the restart unchanged"
    );
    assert_eq!(survived.state, AllocState::Running, "precondition: the alloc is still Running");

    // The operator stops the workload on the FIRST tick of the new process.
    dispatch_one_at_tick(
        Action::StopAllocation { alloc_id: alloc_id.clone(), terminal: None },
        &store,
        Arc::clone(&intent),
        POST_RESTART_TICK,
    )
    .await
    .expect("StopAllocation dispatch");

    let after = store.alloc_status_row(&alloc_id).await.expect("read after stop");
    let after = after.expect("a row must still exist");

    // The claim the whole ADR exists to make true. Pre-fix this read
    // `Running` — `dispatch` returned `Ok(())`, the CLI printed
    // "Stopped workload ..." and exited 0, and the store had discarded the
    // write.
    assert_eq!(
        after.state,
        AllocState::Terminated,
        "the post-restart Stop MUST win the LWW merge — a `Running` here is the \
         reproduced defect (dispatch returns Ok(()) on a dropped write, so the \
         operator sees success while the store disagrees)"
    );
    assert_eq!(
        after.updated_at.counter,
        PRIOR_COUNTER + 1,
        "the counter derives from the PRIOR row (269 + 1), not from the tick \
         (which would have stamped {})",
        POST_RESTART_TICK + 1
    );
}

// ---------------------------------------------------------------------------
// Falsifiability witness — proves the assertion above is not vacuous.
//
// If the store accepted ANY write at this key regardless of stamp, the test
// above would pass no matter what `dominating` computed. This pins the
// substrate's actual rejection behaviour against the EXACT stamp the deleted
// `timestamp_for` produced at tick 0, so the regression test's green is
// load-bearing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tick_derived_stamp_at_tick_zero_is_still_silently_dropped() {
    let tmp = TempDir::new().expect("tempdir");
    let obs_path = tmp.path().join("observation.redb");
    let (alloc_id, workload_id, node_id) = ids();

    {
        let store = LocalObservationStore::open(&obs_path).expect("open (lifetime 1)");
        store
            .write_alloc_lifecycle(
                surviving_running_row(PRIOR_COUNTER),
                overdrive_core::traits::observation_store::TransitionSource::Reconciler,
            )
            .await
            .expect("seed the surviving Running row");
        drop(store);
    }

    let store = LocalObservationStore::open(&obs_path).expect("open (lifetime 2)");

    // Exactly what `timestamp_for(tick, node_id)` minted at `tick = 0`:
    // `(counter = 1, writer = "local")`. Same writer as the prior, so
    // `dominates`' tiebreak arm evaluates `"local" > "local"` → false.
    let tick_derived = AllocStatusRow {
        alloc_id: alloc_id.clone(),
        workload_id,
        node_id: node_id.clone(),
        state: AllocState::Terminated,
        updated_at: LogicalTimestamp { counter: POST_RESTART_TICK + 1, writer: node_id },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: None,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };

    let outcome = store
        .write_alloc_lifecycle(
            tick_derived,
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await;
    assert!(
        outcome.is_ok(),
        "the drop is SILENT: `write` returns Ok(()) even though the row was discarded — \
         this is why the CLI printed success while the store disagreed"
    );

    let after = store.alloc_status_row(&alloc_id).await.expect("read after the dropped write");
    let after = after.expect("a row must still exist");
    assert_eq!(
        after.state,
        AllocState::Running,
        "a tick-derived stamp at tick 0 loses to the surviving counter — the store still \
         reads Running. THIS is the shape the regression test above proves is gone."
    );
    assert_eq!(after.updated_at.counter, PRIOR_COUNTER, "and the surviving row is untouched");
}
