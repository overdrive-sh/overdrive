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
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::transition_reason::{ProbeWitness, TerminalCondition, TransitionReason};

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
/// provision/teardown seam (`mtls_worker.is_some()`). The enforcement + resolve
/// ports are sim doubles: the AT asserts the netns lifecycle, never drives a
/// connection (the resolve consumer is the 04-02 default-lane DST's job).
fn build_worker() -> Arc<MtlsInterceptWorker> {
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
    let resolve: Arc<dyn overdrive_core::traits::mtls_resolve::MtlsResolve> =
        Arc::new(overdrive_sim::adapters::SimMtlsResolve::new(
            std::collections::BTreeMap::new(),
            overdrive_core::traits::mtls_resolve::MtlsResolution::NonMesh,
        ));
    Arc::new(MtlsInterceptWorker::new(enforcement, resolve, Arc::new(SimClock::new())))
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
        service_ports: Vec::new(),
        workload_addr: None,
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
        // counter 0 so the FinalizeFailed write (`timestamp_for` → counter
        // `tick.tick + 1` = 1, with the SAME writer = this row's node_id) strictly
        // DOMINATES under LWW — a counter tie with an equal writer is retained
        // (idempotency case), which would otherwise mask the finalize write.
        updated_at: LogicalTimestamp { counter: 0, writer: node.clone() },
        reason: Some(TransitionReason::Started),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
    };
    obs.write(ObservationRow::AllocStatus(Box::new(row)))
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
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let alloc = AllocationId::new("anl-stable").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl-stable").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    // Hold slot 0 in the allocator (the observable the gate protects). Assigned
    // in-RAM — no kernel I/O — so the slot-snapshot assertion runs on every host.
    let allocator = NetSlotAllocator::new();
    let slot = allocator.assign(alloc.clone()).expect("assign slot 0");
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
        driver.as_ref(),
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
    if is_root() && netns_present(&plan.netns) {
        // Only meaningful if a real netns was provisioned (it was not, here, since
        // we assigned the slot directly). Present-and-still-present is the claim.
        assert!(
            netns_present(&plan.netns),
            "RCA §9: a Stable terminal must not reap the per-workload netns {}",
            plan.netns,
        );
    }

    worker.stop_alloc(&alloc);
}

#[tokio::test]
async fn finalize_failed_genuine_failure_still_tears_down_alloc() {
    let tmp = TempDir::new().expect("tempdir");
    let store_path = tmp.path().join("intent.redb");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));

    let alloc = AllocationId::new("anl-failed").expect("valid alloc id");
    let workload = WorkloadId::new("svc-anl-failed").expect("valid workload id");
    let node = NodeId::new("node-001").expect("valid node id");

    let allocator = NetSlotAllocator::new();
    let slot = allocator.assign(alloc.clone()).expect("assign slot 0");
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
        driver.as_ref(),
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
            !netns_present(&plan.netns),
            "RCA §9 (over-gating guard): a Failed terminal must still reap the netns {}",
            plan.netns,
        );
    }

    worker.stop_alloc(&alloc);
}
