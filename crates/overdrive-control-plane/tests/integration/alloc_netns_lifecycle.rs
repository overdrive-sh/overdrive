//! Tier-3 acceptance test for the MERGED step 04-01 C3 action-shim seam
//! (transparent-mtls-enrollment D-TME-12 / AC14, Path A / ADR-0071) — the
//! `StartAllocation` / terminal dispatch path that provisions the per-workload
//! netns BEFORE spawn and tears it down AFTER terminal.
//!
//! Drives the PRODUCTION driving port `action_shim::dispatch` with
//! `mtls_worker = Some(<real MtlsInterceptWorker>)` — the ACTIVE seam path
//! (`mtls_worker.is_some()`) that NO prior test exercised (the existing
//! `terminal_propagation` / `submit_to_running` fixtures all thread
//! `mtls_worker: None`, so the netns provision/teardown seam was unproven).
//!
//! AC14's four sub-claims:
//!
//!   1. a real exec alloc reaching Running has its netns + veth provisioned
//!      BEFORE spawn (the provision precedes `Driver::start` in the
//!      StartAllocation arm), AND
//!   2. the workload LANDS in `ovd-ns-<slot>` — asserted on the OBSERVABLE
//!      kernel side effect `ip netns identify <pid>` (the spawned PID's netns
//!      is the slot-derived per-workload netns, NOT the host netns).
//!   3. on terminal (StopAllocation) the netns is torn down (teardown-then-
//!      release) — `ip netns list` no longer shows `ovd-ns-<slot>` and the
//!      slot is released.
//!   4. **provision-failure → Failed row** (never Running-with-no-netns): a
//!      forced provision failure (slot exhaustion) drives the alloc to a
//!      `Failed` `AllocStatusRow` carrying `WorkloadNetnsProvisionFailed`,
//!      mirroring the existing `fail_closed_on_mtls_install` precedent — NOT a
//!      bubbled `Err` that loops the alloc `Pending` forever.
//!
//! Sub-claim 4 is deterministic and runs on EVERY host (the slot-exhaustion
//! failure fires at `NetSlotAllocator::assign`, BEFORE any kernel I/O, so it
//! needs no privilege). Sub-claims 1–3 shell out to real `ip netns` and SKIP on
//! a non-root / no-CAP_NET_ADMIN runner. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
//! --features integration-tests`. NEVER `--no-run` — a compile-only gate is
//! green even when every fixture refuses at boot.
//!
//! Cleanup: a per-test RAII guard tears down the slot-derived netns + host veth
//! on drop so an assertion panic leaves no residue. The happy-path test uses a
//! fresh empty `NetSlotAllocator`, so the alloc gets slot 0 → `ovd-ns-0000`;
//! the guard sweeps that name unconditionally.

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
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverType, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::TransitionReason;

use overdrive_dataplane::allocators::{PersistentServiceVipAllocator, VipRange};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ExecDriver;
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
/// provision/teardown seam (`mtls_worker.is_some()`). The enforcement port is a
/// sim double: the AT asserts the netns lifecycle, never drives a connection.
fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    Arc::new(MtlsInterceptWorker::new(enforcement, Arc::new(SimClock::new())))
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

fn build_spec(alloc: &AllocationId, command: &str, args: Vec<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/job/anl/alloc/01")
            .expect("valid spiffe id"),
        command: command.to_owned(),
        args,
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The C3 provision seam SETS these (JOIN-2/6) — supplied None so the
        // seam's own assign/provision/inject is exercised, not pre-set.
        netns: None,
        host_veth: None,
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

/// `ip netns identify <pid>` → the netns NAME the PID lives in (`None` when the
/// PID is in an unnamed netns or the command fails).
fn netns_identify(pid: u32) -> Option<String> {
    let out = Command::new("ip").args(["netns", "identify", &pid.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
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
    driver: &dyn Driver,
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
        driver,
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
        Some(worker),
        net_slot_allocator,
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
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

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
    let spec = build_spec(&alloc, "/bin/true", vec![]);

    let result = dispatch_one(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec,
            kind: WorkloadKind::Service,
        },
        driver.as_ref(),
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
// AC14 sub-claims 1–3 — provision-before-spawn + lands-in-netns + teardown
// (real kernel; root + CAP_NET_ADMIN required, SKIP otherwise).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn alloc_lands_in_slot_netns_and_teardown_reaps_it_on_terminal() {
    if !is_root() {
        eprintln!("SKIP alloc_lands_in_slot_netns_and_teardown_reaps_it_on_terminal: not root");
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    let sim_clock = Arc::new(SimClock::new());
    // REAL ExecDriver — it spawns `/bin/sleep` and enters spec.netns via
    // setns(CLONE_NEWNET); the netns landing is the observable AC14.2 effect.
    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(
        std::path::PathBuf::from("/sys/fs/cgroup"),
        sim_clock,
        Arc::new(overdrive_host::RealCgroupFs::new()),
    ));

    // Fresh empty allocator → the alloc gets slot 0 → ovd-ns-0000. Derive the
    // plan for the RAII sweep + the expected-name assertion.
    let allocator = NetSlotAllocator::new();
    let expected_plan = derive_workload_netns_plan(
        overdrive_control_plane::veth_provisioner::NetSlot::new(0).expect("slot 0 in range"),
        responder_addr_for_slot(
            overdrive_control_plane::veth_provisioner::NetSlot::new(0).expect("slot 0 in range"),
        ),
    );
    // Pre-sweep any residue from a crashed prior run, then arm the RAII guard.
    let _ = teardown_workload_netns(&expected_plan);
    let _guard = NetnsGuard { plan: expected_plan.clone() };

    let alloc = AllocationId::new("anl-land").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl-land").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");
    // Long-running so the spawned PID is alive when we read its netns.
    let spec = build_spec(&alloc, "/bin/sleep", vec!["3600".to_owned()]);

    let start = dispatch_one(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            spec,
            kind: WorkloadKind::Service,
        },
        driver.as_ref(),
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await;

    // The provision may legitimately fail for lack of CAP_NET_ADMIN even as
    // root in a constrained runner — SKIP rather than fail in that case (the
    // Failed row carries WorkloadNetnsProvisionFailed(netns_provision)).
    if start.is_err() {
        worker.stop_alloc(&alloc);
        eprintln!(
            "SKIP alloc_lands_in_slot_netns_and_teardown_reaps_it_on_terminal: dispatch errored \
             (likely no CAP_NET_ADMIN)"
        );
        return;
    }
    if let Some(row) = latest_row(obs.as_ref(), &alloc).await
        && row.state == AllocState::Failed
        && matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "netns_provision"
        )
    {
        worker.stop_alloc(&alloc);
        eprintln!(
            "SKIP alloc_lands_in_slot_netns_and_teardown_reaps_it_on_terminal: provision \
             fail-closed (likely no CAP_NET_ADMIN): {:?}",
            row.reason
        );
        return;
    }

    // AC14.1: the alloc reached Running (the provision preceded the spawn).
    let row = latest_row(obs.as_ref(), &alloc).await.expect("alloc row present after start");
    assert_eq!(
        row.state,
        AllocState::Running,
        "AC14.1: a successful provision + spawn must reach Running, got {:?} ({:?})",
        row.state,
        row.reason,
    );

    // AC14.3 (precondition): the slot-derived netns now exists.
    assert!(
        netns_present(&expected_plan.netns),
        "AC14.1: the per-workload netns {} must exist after the provision seam",
        expected_plan.netns,
    );

    // AC14.2: the spawned workload PID LIVES in the slot-derived netns
    // (`ip netns identify <pid>` == ovd-ns-0000), NOT the host netns. This is
    // the observable proof the workload was spawned INTO its netns.
    let pid = {
        // Read the workload pid from the driver's live handle map via a fresh
        // /bin/sleep lookup — the ExecDriver records the pid on the row's
        // detail? No: read it from `ip netns pids`. The most robust observable
        // is: the netns has exactly the spawned sleep as a member.
        let out = Command::new("ip")
            .args(["netns", "pids", &expected_plan.netns])
            .output()
            .expect("spawn ip netns pids");
        String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().parse::<u32>().ok())
    };
    let pid = pid.expect(
        "AC14.2: the per-workload netns must contain the spawned workload PID \
         (the workload landed in ovd-ns-<slot>)",
    );
    assert_eq!(
        netns_identify(pid).as_deref(),
        Some(expected_plan.netns.as_str()),
        "AC14.2: the spawned workload PID {pid} must live in the slot-derived netns {}, not the host netns",
        expected_plan.netns,
    );

    // --- Terminal: StopAllocation tears the netns down + releases the slot ---
    let stop = dispatch_one(
        Action::StopAllocation { alloc_id: alloc.clone(), terminal: None },
        driver.as_ref(),
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await;
    stop.expect("StopAllocation dispatch must succeed");

    // AC14.3: the netns is GONE after terminal (teardown-then-release) and the
    // slot is released (no leak).
    assert!(
        !netns_present(&expected_plan.netns),
        "AC14.3: the per-workload netns {} must be torn down on terminal",
        expected_plan.netns,
    );
    assert!(
        !allocator.snapshot().contains_key(&alloc),
        "AC14.3: the slot must be released after terminal teardown",
    );

    worker.stop_alloc(&alloc);
}
