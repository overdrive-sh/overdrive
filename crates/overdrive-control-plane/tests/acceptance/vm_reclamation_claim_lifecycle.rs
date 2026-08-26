//! S-VM-77 / S-VM-78 (ADR-0083 §D7, `brief.md` §105a.2 / §105a.3, GH #42).
//!
//! **Path note (a step 02-02 correction, mirroring the `exit_observer.rs`
//! roadmap-path correction the dispatch already made):** this feature's
//! own `distill/test-scenarios.md` (DWD-16) routes BOTH scenarios'
//! driving ports to `overdrive-control-plane`-resident production code —
//! the exit observer's loop body (`worker/exit_observer.rs`) and
//! `hydrate_actual` (`reconciler_runtime.rs`), neither of which lives in
//! `overdrive-core`. `overdrive-control-plane` depends on
//! `overdrive-core`, never the reverse, so a real component-scope test
//! for either scenario is structurally unreachable from an
//! `overdrive-core` test crate (the exact reasoning DWD-16 already
//! applies to S-VM-77/79). This file therefore lives in
//! `overdrive-control-plane`'s own `tests/acceptance/` tree rather than
//! at the `crates/overdrive-core/tests/acceptance/` path the roadmap
//! note suggested.
//!
//! Tier classification: **Tier 1 DST** — `SimObservationStore` +
//! `SimDriver` + a thin `Driver` port-boundary test double
//! (`ClaimTrackingDriver`), no real infra. Gated on `integration-tests`
//! only because it lives beside this crate's other Sim-backed
//! acceptance suites, not because it touches real I/O.

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, run_convergence_tick,
};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverType, ExitEvent,
    ExitKind, Resources,
};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_reconcilers::{AnyReconciler, AnyState, VmReclamation};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use parking_lot::Mutex;
use proptest::prelude::*;
use tempfile::TempDir;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// `ClaimTrackingDriver` — a `Driver` port-boundary test double wrapping a
// real `SimDriver` (reused for real exit-event injection via
// `inject_exit_after`), with an INDEPENDENTLY controlled claim set.
//
// `SimDriver` itself correctly keeps the `live_allocations` /
// `release_supervision` trait DEFAULTS regardless of its `DriverType`
// (brief.md §105a.3: "ExecDriver keeps the default... reclamation only
// ever acts on VM allocations") — it is not VM-shaped and does not model
// the authorship claim. This wrapper is the legitimate Mockist-TDD
// port-boundary double standing in for `VmDriver`'s real claim tracking,
// without needing the full `VmDriver` + `SimVmm` + cgroup assembly.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ClaimTrackingDriver {
    inner: Arc<SimDriver>,
    held: Arc<Mutex<BTreeSet<AllocationId>>>,
    released: Arc<Mutex<Vec<AllocationId>>>,
}

impl ClaimTrackingDriver {
    fn new(inner: Arc<SimDriver>) -> Self {
        Self {
            inner,
            held: Arc::new(Mutex::new(BTreeSet::new())),
            released: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn claim(&self, alloc: &AllocationId) {
        self.held.lock().insert(alloc.clone());
    }

    /// Models transition 4 (brief.md §105a.3 row 4) — the exit watcher's
    /// OWN drop-guard abandoning a claim it never handed off, valid
    /// ONLY while the claim is still `Held`. Deliberately a DIFFERENT
    /// code path from `release_supervision` (transitions 5/6's
    /// mechanism, the observer's/a shim arm's release): production
    /// models transition 4 as the watcher's own map mutation, never a
    /// `release_supervision` call. "Held → ∅, and only from Held" is
    /// exactly `BTreeSet::remove`'s own no-op-if-absent semantics — a
    /// claim already handed off (or already released by any other path)
    /// is untouched.
    fn abandon_if_held(&self, alloc: &AllocationId) {
        self.held.lock().remove(alloc);
    }
}

#[async_trait]
impl Driver for ClaimTrackingDriver {
    fn r#type(&self) -> DriverType {
        self.inner.r#type()
    }

    async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
        let handle = self.inner.start(spec).await?;
        self.claim(&spec.alloc);
        Ok(handle)
    }

    fn release_for_exit_emission(&self, handle: &AllocationHandle) {
        self.inner.release_for_exit_emission(handle);
    }

    async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
        self.inner.stop(handle).await
    }

    async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
        self.inner.status(handle).await
    }

    async fn resize(
        &self,
        handle: &AllocationHandle,
        resources: Resources,
    ) -> Result<(), DriverError> {
        self.inner.resize(handle, resources).await
    }

    fn take_exit_receiver(&self) -> Option<mpsc::Receiver<ExitEvent>> {
        self.inner.take_exit_receiver()
    }

    /// Every variant is supervised (brief.md §105a.3 DD-1(b.i)) — this
    /// double models `Held` membership only (this file's scenarios never
    /// need to distinguish `Starting`/`Live`/`EndingInFlight`).
    fn live_allocations(&self) -> Option<Vec<AllocationId>> {
        Some(self.held.lock().iter().cloned().collect())
    }

    /// The abandonment boundary under test (S-VM-77): removes the claim
    /// AND records the call so the test can assert "exactly once, on
    /// every `RetryOutcome` arm."
    fn release_supervision(&self, alloc: &AllocationId) {
        self.held.lock().remove(alloc);
        self.released.lock().push(alloc.clone());
    }
}

// ---------------------------------------------------------------------------
// Harness — mirrors `crash_recovery_obs_write_rejected.rs`'s shape.
// ---------------------------------------------------------------------------

struct Harness {
    state: AppState,
    sim_obs: Arc<SimObservationStore>,
    driver: ClaimTrackingDriver,
    target: TargetResource,
    #[allow(dead_code)]
    sim_clock: Arc<SimClock>,
    #[allow(dead_code)]
    ticker_handle: tokio::task::JoinHandle<()>,
}

async fn build_harness(tmp: &TempDir, workload_id: &str, driver_type: DriverType) -> Harness {
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let node_id = NodeId::new("local").expect("node id");
    let sim_obs = Arc::new(SimObservationStore::single_peer(node_id.clone(), 0));
    let obs: Arc<dyn ObservationStore> = sim_obs.clone();

    let sim_clock = Arc::new(SimClock::new());
    // `driver_type` MUST match the submitted job's `DriverInput` kind for
    // `action_shim::dispatch`'s START routing to find this entry — the
    // S-VM-77 tests (below) drive a real `DriverInput::Exec` job to
    // Running, so they pass `DriverType::Exec`. The S-VM-78 test never
    // drives a tick (it calls `hydrate_actual_for_test` directly) and
    // needs the registry keyed `DriverType::Vm` instead — see that
    // test's own call site.
    let sim_driver = Arc::new(SimDriver::with_clock(driver_type, sim_clock.clone()));
    let driver = ClaimTrackingDriver::new(sim_driver);
    let driver_dyn: Arc<dyn Driver> = Arc::new(driver.clone());

    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver_dyn,
        sim_clock.clone(),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        NodeId::new("writer-1").expect("valid NodeId"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    exit_observer::spawn(
        state.obs.clone(),
        state.drivers.get(driver_type).cloned().expect("registry has the requested entry"),
        state.lifecycle_events.clone(),
        sim_clock.clone(),
    );

    let job = Job::from_submit(JobSpecInput {
        id: workload_id.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    })
    .expect("valid job spec");
    let archived = overdrive_core::aggregate::WorkloadIntent::Job(job.clone())
        .archive_for_store()
        .expect("rkyv archive");
    let key = IntentKey::for_workload(&job.id);
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put job");

    let target = TargetResource::new(&format!("workload/{workload_id}")).expect("valid target");

    let ticker_clock = sim_clock.clone();
    let ticker_handle = tokio::spawn(async move {
        loop {
            ticker_clock.tick(Duration::from_millis(50));
            tokio::task::yield_now().await;
        }
    });

    Harness { state, sim_obs, driver, target, sim_clock, ticker_handle }
}

async fn drive_to_first_running(h: &Harness, start: Instant) -> AllocationId {
    let workload_lifecycle_name =
        overdrive_core::reconcilers::ReconcilerName::new("workload-lifecycle")
            .expect("workload-lifecycle reconciler name");
    let deadline = start + Duration::from_secs(120);
    let mut tick_n = 0_u64;
    loop {
        run_convergence_tick(
            &h.state,
            &workload_lifecycle_name,
            &h.target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        let rows = h.state.obs.alloc_status_rows().await.expect("read rows");
        if let Some(row) = rows
            .iter()
            .find(|r| r.state == overdrive_core::traits::observation_store::AllocState::Running)
        {
            return row.alloc_id.clone();
        }
        tick_n += 1;
        assert!(tick_n < 60, "alloc must reach Running before the test injects an exit");
    }
}

/// D3 complement-equality (@contract-shape:bounded-change, 02-02
/// review): seeds an unrelated, ALREADY-held "noise" claim so the
/// caller can assert it survives the alloc-under-test's release
/// untouched. Shared by all four S-VM-77 example tests below.
fn seed_noise_claim(h: &Harness, label: &str) -> AllocationId {
    let noise = AllocationId::new(&format!("alloc-s-vm-77-{label}-noise-0")).expect("valid id");
    h.driver.claim(&noise);
    noise
}

/// Pairs with [`seed_noise_claim`] — asserts the noise claim was never
/// touched by another allocation's release.
fn assert_noise_claim_untouched(h: &Harness, noise: &AllocationId) {
    assert!(
        h.driver.held.lock().contains(noise),
        "an unrelated allocation's claim must never be touched by another's release \
         (complement-equality, D3)"
    );
}

// ---------------------------------------------------------------------------
// S-VM-77 — the claim releases on every `RetryOutcome` arm.
// ---------------------------------------------------------------------------

/// `Wrote` arm: the obs write succeeds. The claim must be released.
#[tokio::test]
async fn release_supervision_fires_on_wrote_arm() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "s-vm-77-wrote", DriverType::Exec).await;
    let noise = seed_noise_claim(&h, "wrote");
    let start = Instant::now();
    let alloc = drive_to_first_running(&h, start).await;
    assert!(h.driver.held.lock().contains(&alloc), "the driver must hold the claim before exit");

    h.driver.inner.inject_exit_after(
        &alloc,
        Duration::from_millis(200),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && h.driver.released.lock().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        h.driver.released.lock().as_slice(),
        std::slice::from_ref(&alloc),
        "release_supervision must fire exactly once on the Wrote arm"
    );
    assert!(!h.driver.held.lock().contains(&alloc), "the claim must converge to absent");
    assert_noise_claim_untouched(&h, &noise);
}

/// `Failed` arm: the obs write is rejected on every attempt (a retryable
/// `Io(PermissionDenied)`, re-injected enough times to exhaust the
/// bounded retry budget — mirrors `terminal_obs_write_escalates_via_
/// lifecycle_event`'s precedent shape). The claim must STILL be released
/// — release-only-on-`Wrote` is the NEW-1 failure this test exists to
/// catch.
#[tokio::test]
async fn release_supervision_fires_on_failed_arm() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "s-vm-77-failed", DriverType::Exec).await;
    let noise = seed_noise_claim(&h, "failed");
    let start = Instant::now();
    let alloc = drive_to_first_running(&h, start).await;

    for _ in 0..16 {
        h.sim_obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )));
    }

    h.driver.inner.inject_exit_after(
        &alloc,
        Duration::from_millis(200),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && h.driver.released.lock().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        h.driver.released.lock().as_slice(),
        std::slice::from_ref(&alloc),
        "release_supervision must fire exactly once on the Failed arm — release-only-on-Wrote \
         leaves a Failed allocation claimed forever (SD-1's unstoppable-orphan failure)"
    );
    assert!(!h.driver.held.lock().contains(&alloc), "the claim must converge to absent");
    assert_noise_claim_untouched(&h, &noise);
}

/// `NoPriorRow` arm: an `ExitEvent` arrives for an allocation the
/// observer has never seen a prior row for. The claim must STILL be
/// released.
#[tokio::test]
async fn release_supervision_fires_on_no_prior_row_arm() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "s-vm-77-noprior", DriverType::Vm).await;
    let noise = seed_noise_claim(&h, "noprior");

    // `driver.start()` directly (mirrors VmDriver::start's step-0 claim,
    // brief.md §105a.3 transition 1) WITHOUT ever routing through
    // `action_shim`/a convergence tick — no `AllocStatusRow` is ever
    // written for this alloc, so the observer's `find_prior_row`
    // resolves `None`. `release_for_exit_emission` is fired manually
    // right after `start` to open the driver's UNRELATED
    // Running-confirmed liveness gate (`driver.rs`'s own contract: the
    // exit watcher parks on it until the action shim confirms `Running`
    // committed) — production normally fires it from the action shim
    // after `obs.write(Running)` resolves `Ok`; this test never writes
    // that row at all, which is the whole point of `NoPriorRow`.
    let alloc = AllocationId::new("alloc-s-vm-77-noprior-0").expect("valid id");
    let spec = overdrive_core::traits::driver::AllocationSpec {
        alloc: alloc.clone(),
        identity: overdrive_core::id::SpiffeId::from_str("spiffe://overdrive.local/test/wl")
            .expect("valid SpiffeId"),
        driver: overdrive_core::traits::driver::DriverPayload::Exec(
            overdrive_core::traits::driver::ExecPayload {
                command: "/bin/true".to_owned(),
                args: vec![],
            },
        ),
        resources: overdrive_core::traits::driver::Resources {
            cpu_milli: 100,
            memory_bytes: 32 * 1024 * 1024,
        },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    };
    let handle = h.driver.start(&spec).await.expect("SimDriver::start succeeds");
    h.driver.release_for_exit_emission(&handle);

    h.driver.inner.inject_exit_after(
        &alloc,
        Duration::from_millis(200),
        ExitKind::Crashed { exit_code: Some(1), signal: None },
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && h.driver.released.lock().is_empty() {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        h.driver.released.lock().as_slice(),
        std::slice::from_ref(&alloc),
        "release_supervision must fire exactly once on the NoPriorRow arm — an attempt that can \
         never author an ending must still be concluded"
    );
    assert!(!h.driver.held.lock().contains(&alloc), "the claim must converge to absent");
    assert_noise_claim_untouched(&h, &noise);
}

/// A claim never taken (start never reached step 0) is unaffected by a
/// release call — idempotent, an unknown id is a no-op.
#[tokio::test]
async fn release_supervision_on_unclaimed_alloc_is_idempotent_noop() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "s-vm-77-unclaimed", DriverType::Vm).await;
    // D3 complement-equality: unlike the other three sites, THIS noise
    // claim proves "release is idempotent" is distinct from "release
    // corrupts unrelated state" — an ACTUALLY-held claim must survive
    // releasing a never-claimed id untouched.
    let noise = seed_noise_claim(&h, "unclaimed");
    let unclaimed = AllocationId::new("alloc-never-claimed-0").expect("valid id");
    let driver_dyn: &dyn Driver = &h.driver;
    driver_dyn.release_supervision(&unclaimed);
    driver_dyn.release_supervision(&unclaimed);
    assert!(
        !h.driver.held.lock().contains(&unclaimed),
        "an unclaimed alloc must never appear in the held set"
    );
    assert_noise_claim_untouched(&h, &noise);
    let _ = h.state.obs.alloc_status_rows().await; // keep `state` alive/used
}

/// One releasing "arm" in the transition table (brief.md §105a.3). Three
/// variants model transition 5 — the exit observer's release, fired
/// identically on EVERY `RetryOutcome` (the abandonment-boundary rule:
/// "one release call at the bottom of the observer's loop body, outside
/// `match outcome`, covering all three arms" — the arm never changes
/// WHICH call fires, only WHY it fires); `ShimArm` models transition 6
/// (a shim arm's terminal-row release); `WatcherAbandoned` models
/// transition 4 (the watcher's own drop-guard abandonment, "Held → ∅,
/// and only from Held").
#[derive(Debug, Clone, Copy)]
enum ReleaseOrigin {
    ObserverWrote,
    ObserverFailed,
    ObserverNoPriorRow,
    ShimArm,
    WatcherAbandoned,
}

fn release_origin_strategy() -> impl Strategy<Value = ReleaseOrigin> {
    prop_oneof![
        Just(ReleaseOrigin::ObserverWrote),
        Just(ReleaseOrigin::ObserverFailed),
        Just(ReleaseOrigin::ObserverNoPriorRow),
        Just(ReleaseOrigin::ShimArm),
        Just(ReleaseOrigin::WatcherAbandoned),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// S-VM-77 / brief.md §113 "P5" — the MANDATED proptest over the
    /// transition table (§105a.3): "from any interleaving of transitions
    /// 3–6 for one allocation the map converges to absent, and no
    /// interleaving leaves an entry when both the watcher has finished
    /// and an authorship attempt has concluded." Generates an arbitrary
    /// sequence of 1..=6 releases drawn from BOTH actors this scenario's
    /// own Given/When/Then names — "ANY interleaving of the observer's
    /// release and a shim terminal-row arm's release for the same
    /// allocation" — plus the watcher's own transition-4 abandonment,
    /// and asserts convergence to absent under every generated ordering
    /// and multiplicity, with no panic. This is the coverage the four
    /// hand-picked examples above cannot provide: each of them releases
    /// its claim EXACTLY ONCE, so a release that is only safe the FIRST
    /// time it is called (broken idempotency) would slip past all four
    /// while still violating the map's convergence property — exactly
    /// the "two callers may both fire for one allocation" contract
    /// `Driver::release_supervision`'s docstring (`driver.rs`) commits
    /// to. D3 complement-equality: a second, unrelated "noise" claim
    /// must survive every generated sequence untouched
    /// (@contract-shape:bounded-change).
    #[test]
    fn release_supervision_converges_to_absent_under_any_interleaving(
        origins in prop::collection::vec(release_origin_strategy(), 1..=6),
    ) {
        let driver = ClaimTrackingDriver::new(Arc::new(SimDriver::new(DriverType::Vm)));
        let driver_dyn: &dyn Driver = &driver;

        let target = AllocationId::new("alloc-svm77-pbt-target").expect("valid id");
        let noise = AllocationId::new("alloc-svm77-pbt-noise").expect("valid id");

        // Transition 1: both allocations start Held.
        driver.claim(&target);
        driver.claim(&noise);

        for origin in &origins {
            match origin {
                ReleaseOrigin::ObserverWrote
                | ReleaseOrigin::ObserverFailed
                | ReleaseOrigin::ObserverNoPriorRow
                | ReleaseOrigin::ShimArm => driver_dyn.release_supervision(&target),
                ReleaseOrigin::WatcherAbandoned => driver.abandon_if_held(&target),
            }
        }

        prop_assert!(
            !driver.held.lock().contains(&target),
            "the claim must converge to absent under ANY interleaving/count of releases: {:?}",
            origins,
        );
        prop_assert!(
            driver.held.lock().contains(&noise),
            "an unrelated allocation's claim must never be touched by another's release \
             (complement-equality, D3): {:?}",
            origins,
        );
    }
}

// ---------------------------------------------------------------------------
// S-VM-78 — the hydration read order is `observe()` first, supervision
// LAST: a claim taken before `hydrate_actual`'s supervision read is
// observed as held, so the arriving allocation is never reclaimed.
// ---------------------------------------------------------------------------

/// Restructured per 02-02 review D2: the ORIGINAL shape took the claim
/// before `hydrate_actual` was even called, so it could not distinguish
/// `observe()`-first from supervision-first — both orders would observe
/// the SAME pre-existing claim identically. This version schedules the
/// two reads as genuinely separate steps with the claim interleaved
/// BETWEEN them (t1 < t2 < t3, matching S-VM-78's own Given/When/Then):
/// `hydrate_actual` runs as a spawned task and pauses INSIDE
/// `observe()` (t1) via [`SimVmHostState::arm_observe_barrier`]; the
/// test waits for that pause to be genuinely entered, THEN takes the
/// claim (t2), THEN releases the barrier so `observe()` returns and the
/// LAST step — the supervision read — runs afterward (t3 > t2). Only
/// this shape can distinguish the two orderings (see the deletion-test
/// proof recorded in this step's commit).
#[tokio::test]
async fn hydrate_actual_observes_a_freshly_taken_claim_and_never_authorises_it() {
    let tmp = TempDir::new().expect("tempdir");
    let h = build_harness(&tmp, "s-vm-78", DriverType::Vm).await;

    // The host observation surface (run dir) exists — this is what makes
    // the allocation appear on `actual.host`'s VM-exclusive surface at
    // all, mirroring a VM mid-boot-race (§103 step 1: the run directory
    // exists from step 1 onward, before the claim's own Held phase ends).
    let arriving = AllocationId::new("alloc-s-vm-78-arriving-0").expect("valid id");
    let sim_host = SimVmHostState::new();
    sim_host.set_run_dir(arriving.clone());
    // Arm the interleaving seam BEFORE the sim is handed to the runtime
    // — the NEXT `observe()` call (the one `hydrate_actual`'s spawned
    // task is about to make) will pause here.
    let barrier = sim_host.arm_observe_barrier();
    let vm_host_state: Arc<dyn VmHostState> = Arc::new(sim_host);

    let mut state_for_task = h.state.clone();
    state_for_task.vm_host_state = vm_host_state;

    // t1 (scheduled): `hydrate_actual` runs concurrently and will block
    // inside `observe()` the instant it starts.
    let hydrate_handle = tokio::spawn(async move {
        let reconciler = AnyReconciler::VmReclamation(VmReclamation::new());
        let target = TargetResource::new("node/local").expect("valid target");
        hydrate_actual_for_test(&reconciler, &target, &state_for_task)
            .await
            .expect("hydrate_actual")
    });

    // t1 (observed): block until `observe()` has GENUINELY started —
    // not merely until the task was scheduled.
    barrier.wait_for_observe_started().await;

    // t2: the claim is taken strictly BETWEEN `observe()`'s start and
    // its return — the only shape that can distinguish
    // `observe()`-first from supervision-first hydration ordering
    // (§103 step 0 / §105a.2's asymmetry argument).
    h.driver.claim(&arriving);

    // t3: release `observe()` so it returns and the supervision read —
    // the LAST step of `hydrate_vm_reclamation_actual` — runs after the
    // claim is visible.
    barrier.release_observe();

    let result = hydrate_handle.await.expect("hydrate_actual task must not panic");
    let AnyState::VmReclamation(vm_state) = result else {
        panic!("expected AnyState::VmReclamation");
    };

    assert!(
        vm_state.host.run_dirs.contains(&arriving),
        "observe() must report the arriving allocation's run directory"
    );
    assert!(
        !vm_state.supervision.reclamation_authorised(&arriving),
        "the supervision read (LAST) must see the claim taken strictly between observe()'s \
         start and return — a booting VM must never be authorised for reclamation (brief.md \
         §105a.2's asymmetry argument)"
    );
}
