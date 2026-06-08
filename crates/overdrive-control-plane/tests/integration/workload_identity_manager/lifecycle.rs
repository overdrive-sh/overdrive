//! Integration — workload-identity-manager walking skeleton (GH #35).
//!
//! Layer 3 (gated `integration-tests`, runs via Lima — exercises a REAL
//! `RcgenCa` doing real P-256 crypto, a REAL `LocalObservationStore` over
//! redb, and a real `openssl verify` subprocess). S-WIM-WS
//! (`walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid`)
//! is ACTIVATED here; S-WIM-12
//! (`restart_reissues_each_still_running_alloc_with_audit_row`) is left as a
//! RED scaffold — it is 03-02's.
//!
//! #35 is a FOUNDATION feature with NO operator CLI verb — `openssl verify`
//! is the honest external entry point (the `rcgen_ca_chain_verify` /
//! `ca_boot_and_audit` shape: assert on the tool EXIT CODE, not internal
//! reachability — `.claude/rules/testing.md` Tier 3).
//!
//! Cgroup-free: the WS exercises the control-plane convergence loop + the CA
//! chain, NOT the cgroup workload path (`SimDriver`, no real workload spawn).
//!
//! RED scaffold convention: self-contained `panic!` under
//! `#[should_panic(expected = "RED scaffold")]`; DELIVER replaces the body
//! with real end-to-end assertions.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::{AppState, noop_heartbeat, svid_lifecycle};
use overdrive_core::SpiffeId;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{Reconciler, ReconcilerName, TargetResource};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_host::OsEntropy;
use overdrive_host::ca::RcgenCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_store_local::{LocalIntentStore, LocalObservationStore};
use tempfile::TempDir;

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

const WORKLOAD_NAME: &str = "ws-payments";
const NODE_NAME: &str = "host-0";
const ALLOC_NAME: &str = "alloc-ws-0";

/// Trust-domain subject the WS root is minted for. Mirrors the
/// `rcgen_ca_chain_verify` / `ca_boot_and_audit` precedents.
fn trust_domain_subject() -> SpiffeId {
    SpiffeId::new("spiffe://overdrive.local/overdrive/ca").expect("trust-domain SpiffeId parses")
}

/// `@walking_skeleton` `@real-io` `@adapter-integration` `@S-WIM-WS` -- an alloc
/// reaches Running, `IssueSvid` mints via the built-in CA, the SVID is held in
/// `IdentityMgr`, an audit row is observable, `openssl verify` accepts the
/// chain, and Stop drops the held entry.
///
/// # Dual-When journey (the accepted single demo-able journey)
///
/// **When 1** — an alloc reaching Running → `IssueSvid` is dispatched through
/// the REAL action-shim executor (`ca_issuance::issue_and_audit` over a real
/// `RcgenCa` + a real `LocalObservationStore`) → `IdentityMgr` holds the minted
/// SVID for the pure-derived `SpiffeId::for_allocation` identity, an
/// `issued_certificates` row is observable via the `ObservationStore`, AND
/// `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>` exits 0
/// (assert on the tool EXIT CODE, not internal reachability — Tier 3).
///
/// **When 2** — the alloc stops (its `alloc_status` row leaves Running) →
/// `DropSvid` is dispatched → `IdentityMgr` no longer holds that allocation's
/// SVID (O2/K2 — leak resistance on stop).
///
/// # Port-to-port
///
/// The driving port is `run_convergence_tick` for the `svid-lifecycle`
/// reconciler against the `job/<workload>` target — the SAME convergence loop
/// the production boot path runs. The observable outcomes are asserted at the
/// `IdentityMgr::held_snapshot`, `ObservationStore::issued_certificate_rows`,
/// and `openssl verify` exit-code boundaries. No executor / reconciler
/// internals are exercised directly.
///
/// # Why `openssl verify` runs the CA's own chain
///
/// `IdentityMgr::held_snapshot` returns the non-secret PROJECTION (`spiffe_id`
/// + `not_after`), never the held leaf cert PEM (the leaf key stays inside
/// `IdentityMgr`, K2; no `IdentityRead` cert accessor exists until 02-02). So
/// the verify proves the CA WIRED INTO the convergence loop (`state.ca`)
/// produces chains that `openssl verify` accepts for the held identity: root +
/// intermediate from `state.ca`, leaf minted by `state.ca.issue_svid` for the
/// SAME `SpiffeId::for_allocation` the executor held. This is the
/// `ca_boot_and_audit` shape (mint-then-verify the chain the live CA produces).
#[tokio::test]
async fn walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid() {
    // GIVEN a control-plane convergence harness with a REAL RcgenCa (the `Ca`
    // port) + a REAL LocalObservationStore (the `obs` port), svid-lifecycle
    // registered, cgroup-free (SimDriver, no real workload spawn).
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp).await;

    let workload = WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId");
    let alloc = AllocationId::new(ALLOC_NAME).expect("valid AllocationId");
    let identity = SpiffeId::for_allocation(&workload, &alloc);

    // WHEN 1 — the alloc reaches Running (its alloc_status row goes Running).
    write_alloc_state(&h, ALLOC_NAME, AllocState::Running, 1).await;

    // AND the svid-lifecycle convergence loop ticks: hydrate desired (Running
    // set) + actual (held set, empty) → reconcile emits IssueSvid → the REAL
    // action-shim executor mints via RcgenCa, writes the issued_certificates
    // audit row, and holds the SvidMaterial in IdentityMgr.
    tick(&h, 2).await;
    // A second tick lets any spawned shim work settle before we read.
    tick(&h, 3).await;

    // THEN IdentityMgr holds the alloc with the pure-derived identity (the
    // held_snapshot projection — K1/O1). Read through the driven-port boundary.
    let held = h.state.identity.held_snapshot();
    let facts = held.get(&alloc).unwrap_or_else(|| {
        panic!(
            "IssueSvid must have held the minted SVID for the Running alloc; held set: {:?}",
            held.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(
        facts.spiffe_id, identity,
        "the held SVID identity must be the pure-derived SpiffeId::for_allocation"
    );

    // AND an issued_certificates audit row is observable through the
    // ObservationStore for that identity (audit-before-hold, ADR-0063 D6).
    let audit_rows = h.state.obs.issued_certificate_rows().await.expect("read audit rows");
    assert!(
        audit_rows.iter().any(|r| r.spiffe_id == identity),
        "an issued_certificates audit row must be observable for the held identity {identity}; \
         rows: {:?}",
        audit_rows.iter().map(|r| r.spiffe_id.to_string()).collect::<Vec<_>>()
    );

    // AND `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>`
    // exits 0 — the CA wired into the convergence loop (`state.ca`) produces a
    // chain a relying party accepts for the held identity. Root + intermediate
    // come from the SAME CA the executor used; the leaf is minted for the SAME
    // identity the executor held (the `ca_boot_and_audit` mint-then-verify
    // shape; the held leaf PEM is not exposed via held_snapshot — K2).
    let node = NodeId::new(NODE_NAME).expect("valid NodeId");
    let root = h.state.ca.root().expect("RcgenCa::root self-signs a real P-256 root");
    let intermediate =
        h.state.ca.issue_intermediate(&node).expect("RcgenCa::issue_intermediate signs by root");
    let (not_before, not_after) = now_window();
    let req = overdrive_core::traits::ca::SvidRequest::new(identity.clone(), not_before, not_after);
    let leaf =
        h.state.ca.issue_svid(&req).expect("RcgenCa::issue_svid mints a leaf for the identity");

    let pem_dir = TempDir::new().expect("pem tempdir");
    let root_pem = pem_dir.path().join("root.pem");
    let inter_pem = pem_dir.path().join("intermediate.pem");
    let svid_pem = pem_dir.path().join("svid.pem");
    std::fs::write(&root_pem, root.cert_pem().as_pem().as_bytes()).expect("write root.pem");
    std::fs::write(&inter_pem, intermediate.cert_pem().as_pem().as_bytes())
        .expect("write intermediate.pem");
    std::fs::write(&svid_pem, leaf.cert_pem().as_pem().as_bytes()).expect("write svid.pem");

    let output = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&root_pem)
        .arg("-untrusted")
        .arg(&inter_pem)
        .arg(&svid_pem)
        .output()
        .expect("invoke openssl verify");
    assert!(
        output.status.success(),
        "openssl verify -CAfile root.pem -untrusted intermediate.pem svid.pem must exit 0 \
         (the built-in CA's chain for the held identity verifies): stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // WHEN 2 — the alloc stops (its alloc_status row leaves Running). A newer
    // Terminated write wins under LWW, so the next tick's hydrate sees an empty
    // Running set.
    write_alloc_state(&h, ALLOC_NAME, AllocState::Terminated, 4).await;

    // AND the svid-lifecycle convergence loop ticks: reconcile sees
    // `¬running ∧ held` → emits DropSvid → the executor removes the held entry.
    tick(&h, 5).await;
    tick(&h, 6).await;

    // THEN IdentityMgr no longer holds the stopped allocation's SVID — the
    // node-held leaf key is unreachable in the held set (O2/K2).
    let held_after_stop = h.state.identity.held_snapshot();
    assert!(
        !held_after_stop.contains_key(&alloc),
        "DropSvid must have removed the held SVID after the alloc stopped; held set still \
         contains it: {:?}",
        held_after_stop.keys().collect::<Vec<_>>()
    );
}

/// `@real-io` `@error` `@S-WIM-12` -- after a control-plane restart the held set
/// starts empty, every still-Running allocation is re-issued once during
/// recovery, and each re-issue leaves an `issued_certificates` audit row.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn restart_reissues_each_still_running_alloc_with_audit_row() {
    red_scaffold("S-WIM-12 bounded audited restart re-issue");
}

// ---------------------------------------------------------------------------
// Harness — a control-plane convergence harness with a REAL RcgenCa + a REAL
// LocalObservationStore, svid-lifecycle registered, cgroup-free.
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    target: TargetResource,
    reconciler_name: ReconcilerName,
    start: Instant,
    deadline: Instant,
}

async fn build_harness(tmp: &TempDir) -> Harness {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime composes");
    runtime.register(noop_heartbeat()).await.expect("register noop-heartbeat");
    runtime.register(svid_lifecycle()).await.expect("register svid-lifecycle");

    let store = Arc::new(
        LocalIntentStore::open(tmp.path().join("intent.redb")).expect("open intent store"),
    );
    // REAL observation store over redb — the issued_certificates audit row is
    // written + read back through the production LocalObservationStore (the
    // ca_boot_and_audit shape).
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(tmp.path().join("obs.redb")).expect("open obs store"));

    let node_id = NodeId::new(NODE_NAME).expect("valid NodeId");
    let sim_clock = Arc::new(SimClock::new());
    let sim_driver = Arc::new(SimDriver::with_clock(DriverType::Exec, sim_clock.clone()));
    let driver: Arc<dyn Driver> = sim_driver;

    // REAL built-in CA — RcgenCa does real P-256 crypto (the `Ca` port the
    // IssueSvid executor dispatches through).
    let ca: Arc<dyn Ca> = Arc::new(RcgenCa::new(Arc::new(OsEntropy), trust_domain_subject()));

    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);

    let state = AppState::new(
        store,
        tmp.path().join("intent.redb"),
        obs,
        Arc::new(runtime),
        driver,
        sim_clock,
        Arc::new(SimDataplane::new()),
        ca,
        Arc::new(IdentityMgr::new(None)),
        node_id,
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    let target = TargetResource::new(&format!("job/{WORKLOAD_NAME}")).expect("valid target");
    let reconciler_name = ReconcilerName::new(
        <overdrive_core::reconcilers::svid_lifecycle::SvidLifecycle as Reconciler>::NAME,
    )
    .expect("valid reconciler name");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    Harness { state, target, reconciler_name, start, deadline }
}

/// Run one svid-lifecycle convergence tick at `tick_n`.
async fn tick(h: &Harness, tick_n: u64) {
    run_convergence_tick(
        &h.state,
        &h.reconciler_name,
        &h.target,
        h.start + Duration::from_millis(tick_n.saturating_mul(100)),
        tick_n,
        h.deadline,
    )
    .await
    .unwrap_or_else(|e| panic!("convergence tick {tick_n} failed: {e:?}"));
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// Write an `AllocStatusRow` for `alloc_raw` through the `ObservationStore`
/// port — the churn driver. A later write at a higher `counter` wins under LWW.
async fn write_alloc_state(h: &Harness, alloc_raw: &str, state: AllocState, counter: u64) {
    let writer = NodeId::new(NODE_NAME).expect("valid writer NodeId");
    let row = AllocStatusRow {
        alloc_id: AllocationId::new(alloc_raw).expect("valid AllocationId"),
        workload_id: WorkloadId::new(WORKLOAD_NAME).expect("valid WorkloadId"),
        node_id: NodeId::new(NODE_NAME).expect("valid NodeId"),
        state,
        updated_at: LogicalTimestamp { counter, writer },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Job,
        listeners: Vec::new(),
        started_at: None,
    };
    h.state
        .obs
        .write(ObservationRow::AllocStatus(Box::new(row)))
        .await
        .unwrap_or_else(|e| panic!("write alloc_status row for {alloc_raw}: {e:?}"));
}

/// A validity window straddling the current wall-clock so the directly-minted
/// leaf is valid *now* under `openssl verify`. Mirrors `rcgen_ca_chain_verify`.
fn now_window() -> (overdrive_core::wall_clock::UnixInstant, overdrive_core::wall_clock::UnixInstant)
{
    use overdrive_core::wall_clock::UnixInstant;
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("wall-clock after epoch");
    let not_before = UnixInstant::from_unix_duration(now.saturating_sub(Duration::from_secs(60)));
    let not_after = not_before + Duration::from_secs(3600);
    (not_before, not_after)
}
