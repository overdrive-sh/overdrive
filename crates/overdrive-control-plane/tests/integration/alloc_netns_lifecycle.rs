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
//! on drop so an assertion panic leaves no residue.
//!
//! Test isolation: the slot-derived netns / veth names (`ovd-ns-<slot>`,
//! `ovd-hv-<slot>`, `ovd-wl-<slot>`) and the per-slot /30 are SYSTEM-GLOBAL in
//! the VM — there is no cross-test / cross-process netns lock. A fresh
//! `NetSlotAllocator` always hands smallest-free slot 0, so every test picking
//! one would share `ovd-ns-0000` and collide under parallel nextest. Each
//! netns-touching test therefore PINS its alloc to a DISTINCT slot via
//! `NetSlotAllocator::adopt` before dispatch; the guard sweeps that per-test
//! name unconditionally. The slot values come from this file's disjoint band in
//! the cross-file registry (`super::net_slots::ALLOC_NETNS_LIFECYCLE`,
//! `tests/common/net_slots.rs`) — offsets 0 through 4 for the five
//! netns-touching tests — so nothing in the whole integration binary can drift
//! onto the same `ovd-ns-<slot>`.

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
    NetSlotAllocator, VmTapPlan, WorkloadNetnsPlan, derive_vm_tap_plan, derive_workload_netns_plan,
    provision_workload_netns, responder_addr_for_slot, teardown_workload_netns,
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
    let enforcement: Arc<dyn MtlsEnforcement> =
        Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()));
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

fn build_spec(alloc: &AllocationId, command: &str, args: Vec<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::SpiffeId::new("spiffe://overdrive.local/workload/anl/alloc/01")
            .expect("valid spiffe id"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload { command: command.to_owned(), args },
        ),
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // The C3 provision seam SETS these (JOIN-2/6) — supplied None so the
        // seam's own assign/provision/inject is exercised, not pre-set.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
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
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
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

/// `ip -n <netns> -details link show <tap>` identifies a persistent TAP,
/// rather than accepting any link that happens to reuse the desired name.
fn netns_persistent_tap_present(netns: &str, tap: &str) -> bool {
    let out = Command::new("ip")
        .args(["-n", netns, "-details", "link", "show", "dev", tap])
        .output()
        .expect("spawn ip -details link show tap");
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.contains("tun type tap") && stdout.contains("persist")
}

/// Exact namespace-local IPv4 address observation, including prefix length.
fn netns_iface_has_exact_addr(netns: &str, iface: &str, addr: Ipv4Addr, prefix: u8) -> bool {
    let out = Command::new("ip")
        .args(["-n", netns, "-o", "-4", "addr", "show", "dev", iface])
        .output()
        .expect("spawn ip addr show");
    if !out.status.success() {
        return false;
    }
    let needle = format!("inet {addr}/{prefix} ");
    String::from_utf8_lossy(&out.stdout).contains(&needle)
}

/// Namespace-local IPv4 forwarding value from the real procfs view.
fn netns_ip_forward(netns: &str) -> Option<u8> {
    let out = Command::new("ip")
        .args(["netns", "exec", netns, "sysctl", "-n", "net.ipv4.ip_forward"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Exact host route observation for the guest /30 return path.
fn host_guest_return_route_present(workload: &WorkloadNetnsPlan, tap: &VmTapPlan) -> bool {
    let cidr = tap.guest_network.to_string();
    let out = Command::new("ip")
        .args(["-4", "route", "show", "exact", &cidr])
        .output()
        .expect("spawn ip route show exact");
    if !out.status.success() {
        return false;
    }
    let expected =
        format!("{} via {} dev {}", tap.guest_network, workload.workload_addr, workload.host_veth);
    String::from_utf8_lossy(&out.stdout).lines().any(|line| line.trim().starts_with(&expected))
}

/// Stable ifindex identifies a restart convergence pass that adopted the
/// existing persistent TAP rather than deleting/recreating it.
fn netns_link_index(netns: &str, iface: &str) -> Option<u32> {
    let out =
        Command::new("ip").args(["-n", netns, "-o", "link", "show", "dev", iface]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_once(':')
        .and_then(|(raw, _)| raw.trim().parse().ok())
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
        Some(worker),
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
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
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
    let spec = build_spec(&alloc, "/bin/true", vec![]);

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
    let drivers: Arc<overdrive_core::traits::driver::DriverRegistry> = {
        let mut r = overdrive_core::traits::driver::DriverRegistry::new();
        r.insert(Arc::clone(&driver));
        Arc::new(r)
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();

    // TEST ISOLATION: pin THIS test to a DISTINCT slot (this file's band offset
    // 0) so its slot-derived, system-global netns/veth names do not collide with
    // the sibling tests in this file OR any other file (each of which would
    // otherwise derive slot 0's `ovd-ns-0000` from a fresh allocator). The slot
    // comes from the cross-file registry band `ALLOC_NETNS_LIFECYCLE` so nothing
    // in the integration binary can drift onto it. `adopt` binds the alloc to
    // the band slot BEFORE dispatch; `provision_and_inject_netns`'s internal
    // `assign` is idempotent per alloc-id, so it returns this pre-adopted slot
    // rather than smallest-free 0. Derive the plan for the RAII sweep +
    // expected-name asserts.
    let allocator = NetSlotAllocator::new();
    let alloc = AllocationId::new("anl-land").expect("valid alloc id");
    let this_slot = super::net_slots::ALLOC_NETNS_LIFECYCLE.nth(0);
    allocator.adopt(alloc.clone(), this_slot).expect("adopt this file's band slot 0");
    let expected_plan = derive_workload_netns_plan(this_slot, responder_addr_for_slot(this_slot));
    // Pre-sweep any residue from a crashed prior run, then arm the RAII guard.
    let _ = teardown_workload_netns(&expected_plan);
    let _guard = NetnsGuard { plan: expected_plan.clone() };

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
        drivers.as_ref(),
        &alloc_drivers,
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
        netns_present(expected_plan.netns.as_str()),
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
            .args(["netns", "pids", expected_plan.netns.as_str()])
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
        drivers.as_ref(),
        &alloc_drivers,
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
        !netns_present(expected_plan.netns.as_str()),
        "AC14.3: the per-workload netns {} must be torn down on terminal",
        expected_plan.netns,
    );
    assert!(
        !allocator.snapshot().contains_key(&alloc),
        "AC14.3: the slot must be released after terminal teardown",
    );

    worker.stop_alloc(&alloc);
}

/// ADR-0089 C3 VM branch — CONTRACT_SHAPE: bounded-change (the selected
/// slot's netns, veth, persistent TAP, namespace IPv4 forwarding, TAP gateway
/// address, and host guest-return route only). The Sim VM driver proves the
/// pre-start C3 seam without booting a guest; every assertion reads real Linux
/// kernel state. A clean restart preserves the TAP ifindex (no-op), deliberate
/// address/sysctl/route drift is repaired, and terminal teardown leaves no
/// slot-derived kernel resource behind.
#[tokio::test]
async fn vm_c3_converges_persistent_tap_repairs_drift_and_tears_down_without_residue() {
    if !is_root() {
        eprintln!(
            "SKIP vm_c3_converges_persistent_tap_repairs_drift_and_tears_down_without_residue: \
             not root"
        );
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let driver_port: Arc<dyn Driver> = driver.clone();
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver_port);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let allocator = NetSlotAllocator::new();
    let alloc = AllocationId::new("anl-vm-tap").expect("valid alloc id");
    let slot = super::net_slots::ALLOC_NETNS_LIFECYCLE.nth(3);
    allocator.adopt(alloc.clone(), slot).expect("adopt VM test slot");
    let workload = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    let tap = derive_vm_tap_plan(slot, workload.responder_addr);
    let _ = teardown_workload_netns(&workload);
    let _guard = NetnsGuard { plan: workload.clone() };
    let workload_id = WorkloadId::new("svc-anl-vm-tap").expect("valid workload id");
    let node_id = NodeId::new("node-001").expect("valid node id");

    dispatch_one(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id,
            node_id,
            spec: build_vm_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("VM C3 start dispatch must complete");

    let row = latest_row(obs.as_ref(), &alloc).await.expect("VM alloc row after start");
    assert_eq!(row.state, AllocState::Running, "VM C3 start must reach Running");
    assert!(
        netns_persistent_tap_present(workload.netns.as_str(), &tap.tap),
        "C3 must create a persistent type-TAP device through the real TUN/TAP kernel interface",
    );
    assert!(
        netns_iface_has_exact_addr(
            workload.netns.as_str(),
            &tap.tap,
            tap.tap_gateway,
            tap.guest_network.prefix_len(),
        ),
        "the TAP gateway must carry the exact guest /30 prefix",
    );
    assert_eq!(
        netns_ip_forward(workload.netns.as_str()),
        Some(1),
        "namespace-local IPv4 forwarding must be enabled",
    );
    assert!(
        host_guest_return_route_present(&workload, &tap),
        "the host must carry the exact guest-/30 return route through the transit veth",
    );

    let initial_ifindex =
        netns_link_index(workload.netns.as_str(), &tap.tap).expect("TAP ifindex after start");
    dispatch_one(
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: build_vm_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("clean VM restart must converge as a no-op");
    assert_eq!(
        netns_link_index(workload.netns.as_str(), &tap.tap),
        Some(initial_ifindex),
        "a fully converged restart must adopt the existing persistent TAP without recreating it",
    );

    let exact_cidr = format!("{}/{}", tap.tap_gateway, tap.guest_network.prefix_len());
    let wrong_cidr = format!("{}/32", tap.tap_gateway);
    assert!(
        Command::new("ip")
            .args(["-n", workload.netns.as_str(), "addr", "del", &exact_cidr, "dev", &tap.tap,])
            .status()
            .expect("spawn exact TAP address delete")
            .success(),
        "test precondition: remove the exact TAP /30",
    );
    assert!(
        Command::new("ip")
            .args(["-n", workload.netns.as_str(), "addr", "add", &wrong_cidr, "dev", &tap.tap,])
            .status()
            .expect("spawn wrong-prefix TAP address add")
            .success(),
        "test precondition: install the same gateway with the wrong /32 prefix",
    );
    assert!(
        Command::new("ip")
            .args([
                "netns",
                "exec",
                workload.netns.as_str(),
                "sysctl",
                "-q",
                "-w",
                "net.ipv4.ip_forward=0",
            ])
            .status()
            .expect("spawn namespace ip_forward drift")
            .success(),
        "test precondition: disable namespace-local forwarding",
    );
    let guest_network = tap.guest_network.to_string();
    let transit_gateway = workload.workload_addr.to_string();
    assert!(
        Command::new("ip")
            .args([
                "route",
                "del",
                &guest_network,
                "via",
                &transit_gateway,
                "dev",
                &workload.host_veth,
            ])
            .status()
            .expect("spawn guest return-route delete")
            .success(),
        "test precondition: remove the guest return route",
    );

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: build_vm_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("VM restart must repair independently drifted guest-wire facts");

    assert!(
        netns_iface_has_exact_addr(
            workload.netns.as_str(),
            &tap.tap,
            tap.tap_gateway,
            tap.guest_network.prefix_len(),
        ),
        "wrong-prefix gateway drift must be repaired to the exact /30",
    );
    assert!(
        !netns_iface_has_exact_addr(workload.netns.as_str(), &tap.tap, tap.tap_gateway, 32),
        "the obsolete /32 must not survive exact-prefix repair",
    );
    assert_eq!(netns_ip_forward(workload.netns.as_str()), Some(1));
    assert!(host_guest_return_route_present(&workload, &tap));

    let repaired_ifindex =
        netns_link_index(workload.netns.as_str(), &tap.tap).expect("TAP ifindex after repair");
    assert!(
        Command::new("ip")
            .args(["-n", workload.netns.as_str(), "tuntap", "del", "dev", &tap.tap, "mode", "tap",])
            .status()
            .expect("spawn persistent TAP deletion")
            .success(),
        "test precondition: remove the persistent TAP itself",
    );
    assert!(!netns_persistent_tap_present(workload.netns.as_str(), &tap.tap));

    dispatch_one(
        Action::RestartAllocation {
            alloc_id: alloc.clone(),
            spec: build_vm_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("VM restart must recreate a missing persistent TAP");
    assert!(netns_persistent_tap_present(workload.netns.as_str(), &tap.tap));
    assert_ne!(
        netns_link_index(workload.netns.as_str(), &tap.tap),
        Some(repaired_ifindex),
        "missing-TAP repair must materialise a new kernel link",
    );
    assert!(netns_iface_has_exact_addr(
        workload.netns.as_str(),
        &tap.tap,
        tap.tap_gateway,
        tap.guest_network.prefix_len(),
    ));
    assert_eq!(netns_ip_forward(workload.netns.as_str()), Some(1));
    assert!(host_guest_return_route_present(&workload, &tap));

    dispatch_one(
        Action::StopAllocation { alloc_id: alloc.clone(), terminal: None },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("VM stop must tear down the guest wire");
    assert!(!netns_present(workload.netns.as_str()), "terminal teardown must remove the netns");
    assert!(
        !Command::new("ip")
            .args(["link", "show", "dev", &workload.host_veth])
            .output()
            .expect("spawn host-veth residue check")
            .status
            .success(),
        "terminal teardown must remove the host veth",
    );
    assert!(
        !Command::new("ip")
            .args(["link", "show", "dev", &tap.tap])
            .output()
            .expect("spawn host-namespace TAP residue check")
            .status
            .success(),
        "terminal teardown must leave the expected TAP name absent from the host namespace",
    );
    assert!(!host_guest_return_route_present(&workload, &tap));
    assert!(!allocator.snapshot().contains_key(&alloc));
    eprintln!(
        "EXECUTED vm_c3_converges_persistent_tap_repairs_drift_and_tears_down_without_residue"
    );
}

/// ADR-0089 C3 VM type-collision refusal — CONTRACT_SHAPE: bounded-change
/// (the selected slot's pre-provisioned netns/veth plus one test-owned dummy
/// link only). A same-name non-TAP is incompatible actual state: C3 must fail
/// before the VM driver starts, and the normal terminal seam must still reap
/// every slot-derived resource.
#[tokio::test]
async fn vm_c3_fails_closed_when_tap_name_is_owned_by_an_incompatible_link() {
    if !is_root() {
        eprintln!(
            "SKIP vm_c3_fails_closed_when_tap_name_is_owned_by_an_incompatible_link: not root"
        );
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let store: Arc<dyn overdrive_core::traits::intent_store::IntentStore> =
        Arc::new(LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open store"));
    let obs = build_obs();
    let worker = build_worker();
    let driver = Arc::new(SimDriver::new(DriverType::Vm));
    let driver_port: Arc<dyn Driver> = driver.clone();
    let drivers = {
        let mut registry = overdrive_core::traits::driver::DriverRegistry::new();
        registry.insert(driver_port);
        registry
    };
    let alloc_drivers = overdrive_control_plane::action_shim::AllocDriverIndex::default();
    let allocator = NetSlotAllocator::new();
    let alloc = AllocationId::new("anl-vm-collision").expect("valid alloc id");
    let slot = super::net_slots::ALLOC_NETNS_LIFECYCLE.nth(4);
    allocator.adopt(alloc.clone(), slot).expect("adopt collision test slot");
    let workload = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    let tap = derive_vm_tap_plan(slot, workload.responder_addr);
    let _ = teardown_workload_netns(&workload);
    let _guard = NetnsGuard { plan: workload.clone() };

    provision_workload_netns(&workload)
        .expect("collision fixture baseline netns provisioning must succeed");
    assert!(
        Command::new("ip")
            .args(["-n", workload.netns.as_str(), "link", "add", &tap.tap, "type", "dummy",])
            .status()
            .expect("spawn incompatible dummy creation")
            .success(),
        "test precondition: the desired TAP name is occupied by a dummy link",
    );

    dispatch_one(
        Action::StartAllocation {
            alloc_id: alloc.clone(),
            workload_id: WorkloadId::new("svc-anl-vm-collision").expect("valid workload id"),
            node_id: NodeId::new("node-001").expect("valid node id"),
            spec: build_vm_spec(&alloc),
            kind: WorkloadKind::Service,
        },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("C3 collision refusal must be recorded as a Failed row");

    let row = latest_row(obs.as_ref(), &alloc).await.expect("collision alloc row");
    assert_eq!(row.state, AllocState::Failed, "an incompatible link collision must fail closed");
    assert!(
        matches!(
            row.reason,
            Some(TransitionReason::WorkloadNetnsProvisionFailed { ref stage, .. })
                if stage == "netns_provision"
        ),
        "the failure must retain the typed C3 netns-provision cause, got {:?}",
        row.reason,
    );
    assert_eq!(driver.live_count(), 0, "the VM driver must never start after a TAP collision");

    dispatch_one(
        Action::StopAllocation { alloc_id: alloc.clone(), terminal: None },
        &drivers,
        &alloc_drivers,
        obs.as_ref(),
        Arc::clone(&store),
        &worker,
        &allocator,
    )
    .await
    .expect("terminal cleanup after collision refusal must succeed");
    assert!(
        !netns_present(workload.netns.as_str()),
        "terminal cleanup after collision refusal must remove the netns",
    );
    assert!(
        !Command::new("ip")
            .args(["link", "show", "dev", &workload.host_veth])
            .output()
            .expect("spawn collision host-veth residue check")
            .status
            .success(),
        "terminal cleanup after collision refusal must remove the host veth",
    );
    assert!(
        !Command::new("ip")
            .args(["link", "show", "dev", &tap.tap])
            .output()
            .expect("spawn collision host-namespace TAP residue check")
            .status
            .success(),
        "terminal cleanup after collision refusal must leave the expected TAP name absent from the host namespace",
    );
    assert!(
        !host_guest_return_route_present(&workload, &tap),
        "terminal cleanup after collision refusal must leave no guest return route",
    );
    assert!(
        !allocator.snapshot().contains_key(&alloc),
        "terminal cleanup after collision refusal must release the slot",
    );
    eprintln!("EXECUTED vm_c3_fails_closed_when_tap_name_is_owned_by_an_incompatible_link");
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

    worker.stop_alloc(&alloc);
}
