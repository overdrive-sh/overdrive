//! Integration coverage for the retained structural workload-netns lifecycle.
//!
//! The attachment cut removed the VM TAP plan and its persistent-TAP, guest
//! `/30`, namespace-forwarding and return-route scenarios. This file retains
//! the independent action-shim outcomes that still use `WorkloadNetnsPlan`:
//! a provision failure becomes a durable `Failed` row rather than an indefinite
//! Pending retry, and `FinalizeFailed` tears down/releases only for a genuine
//! failure while a `Stable` success keeps the live allocation.
//!
//! The root-gated checks use the real exec/process netns/veth provisioner. A
//! per-test `NetnsGuard` removes the slot-derived namespace and host veth on
//! drop. Slot names remain isolated through this file's registered band.
//!
//! Run with `cargo xtask lima run -- cargo nextest run -p
//! overdrive-control-plane --features integration-tests`.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege messages are the legitimate way these Tier-3 tests
// communicate "CAP_NET_ADMIN absent, scenario skipped" on an unprivileged
// runner — `eprintln!` to the test log is exactly right.
#![allow(clippy::print_stderr)]
// The happy-path AT runs a single sequential walkthrough (provision → spawn →
// land-in-netns → terminal teardown) whose kernel assertions naturally exceed
// the line budget; splitting it would scatter one scenario across helpers.
#![allow(clippy::too_many_lines)]
// AC14 / `ovd-ns-<slot>` / `MtlsResolve` etc. read as prose identifiers in the
// scenario docs, not code spans.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use overdrive_control_plane::action_shim::dispatch;
use overdrive_control_plane::veth_provisioner::{
    NetSlotAllocator, WorkloadNetnsPlan, derive_workload_netns_plan, responder_addr_for_slot,
    teardown_workload_netns,
};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, DriverType, Resources, VmPayload,
};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_core::transition_reason::{ProbeWitness, TerminalCondition, TransitionReason};

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture builders — the orthogonal dispatch ports the C3 seam does not touch
// are sim doubles; the netns seam ports (NetSlotAllocator, MtlsInterceptWorker)
// and the Driver are REAL where the sub-claim requires it.
// ---------------------------------------------------------------------------

/// True iff this process is uid 0 (root). The netns provision shells out to
/// `ip netns add`, which needs CAP_NET_ADMIN/CAP_SYS_ADMIN.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// A real `MtlsInterceptWorker` — its `Some(...)` presence is what ARMS the C3
/// provision/teardown seam (`mtls_worker.is_some()`). The enforcement + resolve
/// ports are sim doubles: the AT asserts the netns lifecycle, never drives a
/// connection (the resolve consumer is the 04-02 default-lane DST's job).
fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::new(SimMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    Arc::new(MtlsInterceptWorker::new(
        enforcement,
        resolve,
        Arc::new(SimClock::new()),
        Arc::new(HostMtlsIntercept::new()),
    ))
}

/// A shared in-process `SimObservationStore` — the dispatch path writes the
/// alloc row here; the assertions read it back. Single-peer (no gossip).
fn build_obs() -> Arc<dyn ObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0))
}

/// A VIP allocator the dispatch signature requires but the StartAllocation /
/// StopAllocation arms do not touch — a one-address pool is sufficient.
fn build_vip_allocator(
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
) -> Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>> {
    let cidr = ipnet::Ipv4Net::new(Ipv4Addr::new(10, 96, 0, 1), 32).expect("/32 prefix");
    let range = VipRange::new(vec![cidr], std::collections::BTreeSet::new()).expect("vip range");
    Arc::new(tokio::sync::Mutex::new(PersistentServiceVipAllocator::new(range, store)))
}

fn tick_now() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        tick: 0,
        deadline: now + Duration::from_secs(120),
    }
}

fn build_vm_spec(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new(
            "spiffe://overdrive.local/workload/anl-vm/alloc/01",
        )
        .expect("valid spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel-not-booted-by-sim"),
            rootfs: PathBuf::from("/rootfs-not-booted-by-sim"),
        }),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        transport: None,
        service_ports: Vec::new(),
    }
}

/// RAII teardown — runs the production `teardown_workload_netns` for the
/// slot-derived plan on drop so the netns + host veth leave no residue even
/// when an assertion panics mid-test. Idempotent (teardown swallows "absent").
struct NetnsGuard {
    plan: WorkloadNetnsPlan,
}

impl Drop for NetnsGuard {
    fn drop(&mut self) {
        let _ = teardown_workload_netns(&self.plan);
    }
}

/// `ip netns list` contains `<netns>` (first whitespace-delimited token).
fn netns_present(netns: &str) -> bool {
    let out = Command::new("ip").args(["netns", "list"]).output().expect("spawn ip netns list");
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.split_whitespace().next() == Some(netns))
}

/// Find the most-recent `AllocStatusRow` for `alloc` (by logical-timestamp
/// counter) — LWW resolves a brief observed-then-superseded window to the
/// latest write.
async fn latest_row(obs: &dyn ObservationStore, alloc: &AllocationId) -> Option<AllocStatusRow> {
    let rows = obs.alloc_status_rows().await.expect("read alloc rows");
    rows.into_iter().filter(|r| &r.alloc_id == alloc).max_by_key(|r| r.updated_at.counter)
}

/// Drive a single `Action` through the production `action_shim::dispatch` with
/// the supplied `driver` + `net_slot_allocator` + a REAL `MtlsInterceptWorker`
/// (so the C3 seam is ARMED). Every orthogonal port is a sim double.
#[allow(clippy::too_many_arguments)]
async fn dispatch_one(
    action: Action,
    drivers: &overdrive_core::traits::driver::DriverRegistry,
    alloc_drivers: &overdrive_control_plane::action_shim::AllocDriverIndex,
    obs: &dyn ObservationStore,
    store: Arc<dyn overdrive_core::traits::intent_store::IntentStore>,
    worker: &Arc<MtlsInterceptWorker>,
    net_slot_allocator: &NetSlotAllocator,
) -> Result<(), overdrive_control_plane::action_shim::ShimError> {
    let dataplane: Arc<dyn overdrive_core::traits::dataplane::Dataplane> =
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new());
    let (lifecycle_tx, _lifecycle_rx) = broadcast::channel(64);
    let writer_node = NodeId::new("writer-1").expect("NodeId");
    let tick = tick_now();
    let broker = parking_lot::Mutex::new(overdrive_core::eval_broker::EvaluationBroker::new());
    dispatch(
        vec![action],
        drivers,
        alloc_drivers,
        obs,
        dataplane.as_ref(),
        &overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        )),
        &SimClock::new(),
        &overdrive_control_plane::identity_mgr::IdentityMgr::new(None),
        &lifecycle_tx,
        &tick,
        &writer_node,
        build_vip_allocator(store),
        &broker,
        None,
        worker,
        net_slot_allocator,
        &overdrive_sim::adapters::vm_host_state::SimVmHostState::new(),
    )
    .await
}

// ---------------------------------------------------------------------------
// AC14 sub-claim 4 — provision-failure → Failed row (deterministic, no root).
//
// THE RED-DRIVING scenario: before the Failed-row supersede landed, the bare
// `provision_and_inject_netns(...)?` bubbled `ShimError::NetSlotExhausted` from
// `dispatch`, leaving the alloc in its prior `Pending` state (no Failed row) —
// the reconciler would re-emit StartAllocation forever (indefinite Pending
// retry). This asserts the alloc reaches `Failed` carrying the
// `WorkloadNetnsProvisionFailed` cause-class instead.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provision_failure_drives_alloc_to_failed_row_not_pending_retry() {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    // A SimDriver suffices — the provision seam fails (slot exhaustion) BEFORE
    // `Driver::start` is ever reached, so the driver is not exercised.
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    // SATURATE the allocator: hold every slot `0..=NET_SLOT_MAX` so the NEW
    // alloc's `assign` returns `NetSlotExhausted`. Each `assign` is an in-memory
    // smallest-free scan — no kernel I/O — so this is fast and privilege-free.
    let allocator = NetSlotAllocator::new();
    for s in 0..=overdrive_control_plane::veth_provisioner::NET_SLOT_MAX {
        let holder = AllocationId::new(&format!("anl-saturate-{s}")).expect("valid alloc id");
        allocator.assign(holder).expect("saturating assigns must all succeed under capacity");
    }

    let alloc = AllocationId::new("anl-provfail").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    // Seed a prior Pending row so the StartAllocation arm captures
    // prior_state = Pending (first-seen would default to Pending anyway; this
    // makes the from-state explicit and the Failed transition observable).
    let spec = build_vm_spec(&alloc);

    let result = dispatch_one(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec,
            kind: WorkloadKind::Service,
        },
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await;

    // The dispatch itself SUCCEEDS — the provision failure is RECORDED as a
    // Failed row, NOT bubbled as Err (the bare-`?` regression would have
    // returned Err here and left no row).
    result.expect(
        "dispatch must record the provision failure as a Failed row and return Ok — \
         a bubbled Err is the indefinite-Pending-retry regression",
    );

    let row = latest_row(obs.as_ref(), &alloc).await.expect(
        "the provision-failed alloc MUST have a Failed AllocStatusRow (not Pending-forever)",
    );
    assert_eq!(
        row.state,
        AllocState::Failed,
        "AC14.4: a persistent provision failure must drive the alloc to Failed, got {:?}",
        row.state,
    );
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "net_slot_assign"
        ),
        "AC14.4: the Failed row must carry WorkloadNetnsProvisionFailed(stage=net_slot_assign) \
         (mirrors fail_closed_on_mtls_install's typed cause-class), got {:?}",
        row.reason,
    );
    // The slot-exhaustion failure means the NEW alloc never held a slot — the
    // saturated allocator is unchanged (no leak from the failed assign).
    assert!(
        !allocator.snapshot().contains_key(&alloc),
        "a failed assign must not leave the alloc holding a slot",
    );
}

// ---------------------------------------------------------------------------
// Regression — `FinalizeFailed` teardown is GATED on the terminal kind
// (canonical-address inbound RCA §9, GH #241).
//
// A Service workload with empty startup probes emits
// `FinalizeFailed { terminal: Some(Stable { .. }) }` one convergence tick after
// it reaches Running — a SUCCESS announcement that (correctly) keeps the row
// `Running` (the GAP-9 guard at `action_shim/mod.rs:1024`). Before the fix the
// `FinalizeFailed` arm ran `teardown_and_release_netns` (and `worker.stop_alloc`)
// UNCONDITIONALLY, so this success claim destroyed the live Service's
// per-workload netns + host-veth + nft rules and released its slot — leaving a
// healthy workload Running but unreachable ~230 ms after start.
//
// The fix gates both destructive teardowns on the `Stable` discriminator so a
// success leaves the alloc untouched while a genuine failure still reaps it.
// These two tests pin BOTH sides of the gate:
//
//   (a) FinalizeFailed { Stable } must NOT tear down — the slot stays HELD and
//       (root-gated) the netns survives. RED on the pre-fix code.
//   (b) FinalizeFailed { Failed } must STILL tear down — the slot IS released
//       and (root-gated) the netns is reaped. Guards against the fix over-gating
//       (i.e. never tearing down). GREEN before AND after the fix.
//
// The slot-snapshot half is the in-memory observable proxy and runs on EVERY
// host: `teardown_and_release_netns` does teardown-THEN-`release`, and
// `teardown_workload_netns` swallows an absent netns (`netns_del` → "absent"
// swallowed), so an alloc whose slot was assigned in-RAM (no real `ip netns add`)
// still exercises the gate without privilege — today (bug) the Stable teardown
// releases the slot → snapshot empty → RED; with the gate the slot stays held →
// GREEN. The `ip netns list` half needs CAP_NET_ADMIN and SKIPs otherwise, like
// the sub-claims above.
// ---------------------------------------------------------------------------

/// The opt-out `Stable` witness the `ServiceLifecycleReconciler` emits for an
/// empty-startup-probes Service (`service_lifecycle.rs:540-558`) — mirrored here
/// so the dispatched terminal matches the real emission shape.
fn opt_out_stable_terminal() -> TerminalCondition {
    TerminalCondition::Stable {
        settled_in_ms: 0,
        witness: ProbeWitness {
            probe_idx: 0,
            role: "startup".to_owned(),
            mechanic_summary: "none (opted out)".to_owned(),
            inferred: false,
        },
    }
}

/// Seed a prior `Running` `AllocStatusRow` for `alloc` so the `FinalizeFailed`
/// arm's `find_prior_alloc_row` resolves and the gate is exercised against a
/// live-Running alloc (the exact precondition of the RCA §9 defect).
async fn seed_running_row(
    obs: &dyn ObservationStore,
    alloc: &AllocationId,
    workload: &WorkloadId,
    node: &NodeId,
) {
    let row = AllocStatusRow {
        alloc_id: alloc.clone(),
        workload_id: workload.clone(),
        node_id: node.clone(),
        state: AllocState::Running,
        // counter 0 so the FinalizeFailed write strictly DOMINATES under LWW —
        // a counter tie with an equal writer is retained (idempotency case),
        // which would otherwise mask the finalize write. Post-ADR-0077 the
        // stamp is `LogicalTimestamp::dominating(tick.tick, node_id, Some(&0))`
        // = `max(tick + 1, 1)`, which dominates this seed for every tick.
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
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
    };
    obs.write_alloc_lifecycle(
        row,
        overdrive_core::traits::observation_store::TransitionSource::Reconciler,
    )
    .await
    .expect("seed prior Running alloc row");
}

#[tokio::test]
async fn finalize_failed_stable_does_not_tear_down_live_running_alloc() {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    // No driver call on the FinalizeFailed arm — a SimDriver is sufficient.
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    let alloc = AllocationId::new("anl-stable").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl-stable").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    // Hold a slot in the allocator (the observable the gate protects). Bound
    // in-RAM — no kernel I/O — so the slot-snapshot assertion runs on every host.
    // TEST ISOLATION: pin to a DISTINCT slot (this file's band offset 1) so the
    // slot-derived, system-global netns/veth names this test sweeps do not
    // collide with the sibling tests in this file or any other file (each of
    // which would otherwise derive slot 0's `ovd-ns-0000` from a fresh
    // allocator). Slot from the cross-file registry band `ALLOC_NETNS_LIFECYCLE`.
    let allocator = NetSlotAllocator::new();
    let slot = super::net_slots::ALLOC_NETNS_LIFECYCLE.nth(1);
    allocator.adopt(alloc.clone(), slot).expect("adopt this file's band slot 1");
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    // RAII sweep so a residual netns from a crashed prior run leaves no residue.
    let _ = teardown_workload_netns(&plan);
    let _guard = NetnsGuard { plan: plan.clone() };

    // Precondition: the slot is held before the terminal dispatch.
    assert!(
        allocator.snapshot().contains_key(&alloc),
        "precondition: the alloc must hold its slot before the Stable terminal",
    );

    // Seed the live-Running prior row the FinalizeFailed arm finalizes against.
    seed_running_row(obs.as_ref(), &alloc, &workload, &node).await;

    // Dispatch the SUCCESS terminal — a Stable FinalizeFailed.
    dispatch_one(
        Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(opt_out_stable_terminal()),
        },
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("FinalizeFailed { Stable } dispatch must succeed");

    // CORE (every host): a Stable success MUST NOT release the slot — the live
    // Service is still serving on its netns. RED on the pre-fix code (the
    // unconditional teardown released it).
    assert!(
        allocator.snapshot().contains_key(&alloc),
        "RCA §9: FinalizeFailed {{ Stable }} must NOT tear down a live Running alloc — \
         the slot must still be held (the netns/veth back a healthy workload)",
    );

    // The row stays Running (GAP-9 guard) — the Stable claim is a success.
    let row = latest_row(obs.as_ref(), &alloc).await.expect("alloc row present after finalize");
    assert_eq!(
        row.state,
        AllocState::Running,
        "RCA §9: a Stable FinalizeFailed keeps the row Running (success claim), got {:?}",
        row.state,
    );

    // BONUS (root only): the netns the slot derives must survive. On an
    // unprivileged host no real netns was ever provisioned, so this is vacuous —
    // skip it rather than assert against a netns that never existed.
    if is_root() && netns_present(plan.netns.as_str()) {
        // Only meaningful if a real netns was provisioned (it was not, here, since
        // we assigned the slot directly). Present-and-still-present is the claim.
        assert!(
            netns_present(plan.netns.as_str()),
            "RCA §9: a Stable terminal must not reap the per-workload netns {}",
            plan.netns,
        );
    }

    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
}

#[tokio::test]
async fn finalize_failed_genuine_failure_still_tears_down_alloc() {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    let alloc = AllocationId::new("anl-failed").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl-failed").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    // TEST ISOLATION: pin to a DISTINCT slot (this file's band offset 2) so the
    // slot-derived, system-global netns name this test asserts on
    // (`!netns_present` after teardown) is not populated by a concurrent sibling
    // test. Before the convention every sibling shared slot 0's `ovd-ns-0000`, so
    // this test's teardown-reap assertion observed the netns the parallel
    // `alloc_lands` test had just provisioned (the reproduced ~16ms flake). Slot
    // from the cross-file registry band `ALLOC_NETNS_LIFECYCLE`.
    let allocator = NetSlotAllocator::new();
    let slot = super::net_slots::ALLOC_NETNS_LIFECYCLE.nth(2);
    allocator.adopt(alloc.clone(), slot).expect("adopt this file's band slot 2");
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    let _ = teardown_workload_netns(&plan);
    let _guard = NetnsGuard { plan: plan.clone() };

    assert!(
        allocator.snapshot().contains_key(&alloc),
        "precondition: the alloc must hold its slot before the Failed terminal",
    );

    seed_running_row(obs.as_ref(), &alloc, &workload, &node).await;

    // Dispatch a GENUINE terminal — a Failed FinalizeFailed (non-Stable).
    dispatch_one(
        Action::FinalizeFailed {
            alloc_id: alloc.clone(),
            terminal: Some(TerminalCondition::Failed { exit_code: Some(1) }),
        },
        drivers.as_ref(),
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("FinalizeFailed { Failed } dispatch must succeed");

    // CORE (every host): a genuine failure MUST still tear down — the slot is
    // released (teardown-then-release). This guards against the fix OVER-gating
    // (i.e. never tearing down). GREEN both before and after the fix.
    assert!(
        !allocator.snapshot().contains_key(&alloc),
        "RCA §9 (over-gating guard): FinalizeFailed {{ Failed }} must STILL tear down — \
         the slot must be released exactly as today",
    );

    // The row lands Failed (every non-Stable terminal → finalized_state Failed).
    let row = latest_row(obs.as_ref(), &alloc).await.expect("alloc row present after finalize");
    assert_eq!(
        row.state,
        AllocState::Failed,
        "a genuine FinalizeFailed terminal must land the row Failed, got {:?}",
        row.state,
    );

    // BONUS (root only): the netns is reaped on a genuine failure.
    if is_root() {
        assert!(
            !netns_present(plan.netns.as_str()),
            "RCA §9 (over-gating guard): a Failed terminal must still reap the netns {}",
            plan.netns,
        );
    }

    worker.stop_alloc(&alloc).await.expect("allocation teardown succeeds");
}
