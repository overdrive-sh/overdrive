//! S-ROH-B-01 / B-03 — hydrated `AnyState` characterization golden.
//!
//! **Post-02-04 (ADR-0086 S3) role.** `hydrate_desired_for_test` /
//! `hydrate_actual_for_test` were rewired to build a `HydrationContext` from
//! `AppState` and drive the port-backed `AnyReconciler::hydrate_*` forwarding
//! (the moved per-reconciler `hydrate_*` trait methods), so this test now
//! asserts the PORT-DRIVEN hydrated `AnyState` reproduces the committed pre-move
//! golden fixtures byte-for-byte, per variant (S-ROH-B-01) and into the matching
//! `AnyState` variant on both sides (S-ROH-B-03). The golden fixtures under
//! `tests/acceptance/fixtures/hydration_golden/` are the FIXED pre-move baseline
//! and are NEVER regenerated — a drift now means the move was not
//! behaviour-preserving.
//!
//! **CONTRACT_SHAPE: unbounded-preservation (characterization baseline).** This
//! test is the S2-gate artifact of the ADR-0086 hydration move: it snapshots the
//! `AnyState` produced by the **still-present central `hydrate_desired` /
//! `hydrate_actual` free functions** in
//! `overdrive-control-plane/src/reconciler_runtime.rs`, per reconciler, for a
//! FIXED representative row set, and pins each snapshot as a committed golden
//! fixture.
//!
//! **Why it exists / why it is captured NOW (do not reorder — hard gate).** The
//! migration is single-cut: step 02-04 (ADR-0086 S3) DELETES the central
//! `hydrate_*` free fns and moves each reconciler's hydration onto the
//! `Reconciler` trait impls. After that cut there is **no live old-vs-new A/B
//! diff** to prove equivalence against. So while the central free fns STILL
//! EXIST (post-02-02, this step), we capture the pre-move hydrated `AnyState`
//! as a characterization golden. 02-04's S-ROH-B-01 / S-ROH-B-03 equivalence
//! bars assert the port-driven `AnyReconciler::hydrate_*` reproduces THIS
//! golden — per variant. Without it those bars have no expected baseline.
//! (ADR-0086 D8/D9; feature-delta § "HARD DELIVER GATE"; DISTILL Open Question
//! 2.)
//!
//! **What it covers.** All 8 `AnyReconciler` variants — `NoopHeartbeat`,
//! `WorkloadLifecycle`, `WorkflowLifecycle`, `ServiceMapHydrator`,
//! `BackendDiscoveryBridge`, `ServiceLifecycle`, `SvidLifecycle`,
//! `VmReclamation` — so 02-04 can assert per-variant equivalence (B-03).
//! Each variant's `hydrate_desired` AND `hydrate_actual` output is pinned.
//!
//! **The representative row set.** ONE fixed `AppState`, seeded with:
//!   * a `Job` intent (`job-app`, Exec) + a Running `AllocStatusRow`
//!     (`job-app-0`),
//!   * a `Service` intent (`svc-app`, Exec, one `tcp:8080` listener, one
//!     declared startup probe) + a Running `AllocStatusRow` (`svc-app-0`) + a
//!     `Pass` startup `ProbeResultRow`.
//! This makes `WorkloadLifecycle`, `ServiceLifecycle`, `SvidLifecycle`, and the
//! `BackendDiscoveryBridge` *actual* side richly non-trivial. `NoopHeartbeat`
//! is `Unit` by construction. `WorkflowLifecycle`, `ServiceMapHydrator`, and
//! `VmReclamation` capture at their HONEST baseline for this fixture — their
//! read surfaces (workflow-instance intents, listener facts / hydration-result
//! rows, a `Vm`-driver + `VmHostState` observation) are infrastructure this
//! broad Exec-only fixture does not populate, so they hydrate to empty/default.
//! Empty/default is a valid pinned characterization: 02-04 must reproduce it
//! EXACTLY through the ports, byte for byte. The golden is the SNAPSHOT, not a
//! property.
//!
//! **Determinism.** Hydration is a pure read over the fixed seeded stores
//! (`LocalIntentStore` + `SimObservationStore`, `SimClock`, `SimEntropy(0)`),
//! and every state slot is a `BTreeMap` / `UnixInstant` / `Duration` /
//! `SocketAddr` / `SpiffeId` — no `Instant`, no `HashMap`, no wall-clock read.
//! The pretty-`Debug` snapshot is therefore byte-stable across runs.
//!
//! **Capture protocol.** `assert_or_capture_golden` writes the fixture on first
//! run (when absent) and asserts equality thereafter. The committed fixtures
//! under `tests/acceptance/fixtures/hydration_golden/` ARE the baseline; a
//! drift after they are committed fails the test loudly.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::print_stderr,
    clippy::missing_const_for_fn,
    clippy::unused_async
)]

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, ServiceV2,
    WorkloadIntent, WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{AnyReconciler, AnyState};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixed fixture inputs
// ---------------------------------------------------------------------------

/// A fixed wall-clock instant for every `started_at` / probe timestamp so the
/// hydrated snapshot is byte-stable (no `Instant`, no `now()`).
const FIXED_STARTED_AT_SECS: u64 = 1_700_000_000;
const FIXED_PROBE_AT_MS: u64 = 1_700_000_000_000;

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn workload_id(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn target(raw: &str) -> TargetResource {
    TargetResource::new(raw).expect("valid target")
}

/// Build the representative `AppState` (same adapter wiring as the sibling
/// `service_lifecycle_hydrate` / `service_backends_hydrate_desired` fixtures:
/// `LocalIntentStore` + `SimObservationStore` + `SimClock` + Exec `SimDriver`
/// + default allocator + `IdentityMgr::new(None)` + empty listener facts).
async fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

fn exec_job(id: &str) -> Job {
    Job::from_submit(JobSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/serve".to_owned(),
            args: vec!["--job".to_owned()],
        }),
    })
    .expect("valid job spec")
}

fn service_with_startup_probe(id: &str) -> ServiceV2 {
    ServiceV2::from_submit(ServiceSpecInput {
        id: id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_owned(), args: vec![] }),
        listeners: vec![ListenerInput { port: 8080, protocol: "tcp".to_owned() }],
        startup_probes: vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8080 },
            timeout_seconds: 5,
            interval_seconds: 2,
            max_attempts: 30,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("valid service spec")
}

async fn persist_intent(state: &AppState, intent: &WorkloadIntent, kind: WorkloadKind) {
    let wid = match intent {
        WorkloadIntent::Job(j) => j.id.clone(),
        WorkloadIntent::Service(s) => s.id.clone(),
        WorkloadIntent::Schedule(_) => panic!("fixture uses no Schedule"),
    };
    let archived = intent.archive_for_store().expect("rkyv archive");
    state
        .store
        .put(IntentKey::for_workload(&wid).as_bytes(), archived.as_ref())
        .await
        .expect("put intent");
    state
        .store
        .put(IntentKey::for_workload_kind(&wid).as_bytes(), &[kind.discriminator_byte()])
        .await
        .expect("put kind");
}

async fn write_running_alloc(state: &AppState, wid: &WorkloadId, aid: &str, kind: WorkloadKind) {
    let row = AllocStatusRow {
        alloc_id: alloc_id(aid),
        workload_id: wid.clone(),
        node_id: node_id("local"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(
            FIXED_STARTED_AT_SECS,
        ))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
}

// ---------------------------------------------------------------------------
// Golden capture / assertion
// ---------------------------------------------------------------------------

fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/acceptance/fixtures/hydration_golden")
}

/// Golden handling mode. The default assertion run (`GoldenMode::Assert`)
/// FAILS when a committed fixture is absent — an absent fixture is never a
/// silent re-capture (review D4: deleting a fixture must not silently
/// re-bless). Regeneration is an explicit, `#[ignore]`-gated opt-in
/// (`GoldenMode::Regenerate`).
#[derive(Clone, Copy)]
enum GoldenMode {
    /// Committed fixture MUST exist and MUST match. Absent ⇒ failure.
    Assert,
    /// Deliberate (re)capture — writes the fixture. Ignored by default.
    Regenerate,
}

/// Assert against the committed baseline (fail if absent) or, under
/// `GoldenMode::Regenerate`, (re)write the fixture. The committed fixtures ARE
/// the S2-gate baseline; they are the load-bearing artifact.
fn handle_golden(mode: GoldenMode, file: &str, actual: &str) {
    let dir = golden_dir();
    let path = dir.join(file);
    match mode {
        GoldenMode::Assert => {
            let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "\n\nMISSING committed hydration golden `{file}` ({e}).\n\
                     An ABSENT S2-gate fixture is a FAILURE, not a silent re-capture — a deleted\n\
                     fixture must never silently re-bless the current hydration output (review D4).\n\
                     To DELIBERATELY (re)generate the fixtures, run the ignored regeneration test:\n\
                     \n\
                     cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \\\n\
                       --test acceptance --features integration-tests --run-ignored all \\\n\
                       -E 'test(regenerate_hydration_characterization_goldens)'\n\
                     \n\
                     then COMMIT the regenerated fixture.\n",
                )
            });
            assert_eq!(
                actual, expected,
                "\n\nHYDRATION CHARACTERIZATION GOLDEN DRIFT for `{file}`.\n\
                 The pre-move hydrated `AnyState` snapshot changed. This golden is the\n\
                 ADR-0086 S2-gate baseline for the B-01/B-03 equivalence bars (step 02-04).\n\
                 If this is an INTENDED pre-move change, run the ignored regeneration test\n\
                 (see the MISSING-fixture message above) and COMMIT; otherwise the central\n\
                 hydration behaviour regressed and 02-04's baseline would be wrong.\n"
            );
        }
        GoldenMode::Regenerate => {
            std::fs::create_dir_all(&dir).expect("create fixtures dir");
            std::fs::write(&path, actual).expect("write golden");
            eprintln!(
                "REGENERATED hydration golden `{file}` ({} bytes) — COMMIT the fixture (S2-gate).",
                actual.len()
            );
        }
    }
}

/// Exhaustive `AnyState` → variant-name projection. The exhaustive match is
/// compile-forcing: a new `AnyState` variant breaks this until the golden is
/// extended to cover it (the "all 8 variants covered" gate, made structural).
fn variant_name(s: &AnyState) -> &'static str {
    match s {
        AnyState::Unit => "Unit",
        AnyState::WorkloadLifecycle(_) => "WorkloadLifecycle",
        AnyState::WorkflowLifecycle(_) => "WorkflowLifecycle",
        AnyState::ServiceMapHydrator(_) => "ServiceMapHydrator",
        AnyState::BackendDiscoveryBridge(_) => "BackendDiscoveryBridge",
        AnyState::ServiceLifecycle(_) => "ServiceLifecycle",
        AnyState::SvidLifecycle(_) => "SvidLifecycle",
        AnyState::VmReclamation(_) => "VmReclamation",
    }
}

fn render(
    name: &str,
    expected_variant: &str,
    tgt: &TargetResource,
    desired: &AnyState,
    actual: &AnyState,
) -> String {
    format!(
        "# ADR-0086 S2-gate characterization golden — reconciler `{name}`\n\
         # target: {target}\n\
         # captured from the PRE-MOVE central hydrate_desired/hydrate_actual free\n\
         # fns (reconciler_runtime.rs), step 02-03, before the S3 (02-04) cut.\n\
         # expected AnyState variant: {expected_variant}\n\
         \n\
         == hydrate_desired -> AnyState::{desired_variant} ==\n\
         {desired:#?}\n\
         \n\
         == hydrate_actual -> AnyState::{actual_variant} ==\n\
         {actual:#?}\n",
        name = name,
        target = tgt.as_str(),
        expected_variant = expected_variant,
        desired_variant = variant_name(desired),
        actual_variant = variant_name(actual),
        desired = desired,
        actual = actual,
    )
}

// ---------------------------------------------------------------------------
// S-ROH-B-01 — the characterization golden
// ---------------------------------------------------------------------------

/// Assert the pre-move hydrated `AnyState` for ALL 8 reconcilers, both
/// hydration sides, against the committed representative fixtures. An absent
/// fixture is a FAILURE (review D4) — see [`handle_golden`].
#[tokio::test]
async fn pre_move_hydrated_anystate_golden_covers_all_eight_reconcilers() {
    drive_characterization(GoldenMode::Assert).await;
}

/// Deliberate fixture (re)generation — run on demand when the pre-move
/// hydration output legitimately changes. `#[ignore]` so it never runs in
/// normal execution; the committed fixtures are the load-bearing artifact.
#[tokio::test]
#[ignore = "fixture regeneration tool — run on demand to (re)capture the S2-gate goldens, then COMMIT; the committed fixtures are the load-bearing artifact"]
async fn regenerate_hydration_characterization_goldens() {
    drive_characterization(GoldenMode::Regenerate).await;
}

/// Drive all 8 reconcilers through both hydration sides against the fixed
/// representative fixture and either assert against, or regenerate, each
/// committed golden per `mode`.
async fn drive_characterization(mode: GoldenMode) {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 0))
        as Arc<dyn ObservationStore>;
    let state = build_app_state(&tmp, Arc::clone(&obs)).await;

    // --- seed the fixed representative row set ---
    let job = workload_id("job-app");
    let svc = workload_id("svc-app");
    persist_intent(&state, &WorkloadIntent::Job(exec_job("job-app")), WorkloadKind::Job).await;
    persist_intent(
        &state,
        &WorkloadIntent::Service(service_with_startup_probe("svc-app")),
        WorkloadKind::Service,
    )
    .await;
    write_running_alloc(&state, &job, "job-app-0", WorkloadKind::Job).await;
    write_running_alloc(&state, &svc, "svc-app-0", WorkloadKind::Service).await;
    // Pass startup probe for the service alloc (LWW winner).
    state
        .obs
        .write_probe_result(ProbeResultRow {
            alloc_id: alloc_id("svc-app-0"),
            probe_idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            status: ProbeStatus::Pass,
            last_observed_at_unix_ms: FIXED_PROBE_AT_MS,
            inferred: false,
        })
        .await
        .expect("write probe result");

    // --- (name, expected variant, reconciler, target) — all 8 ---
    // Targets follow each reconciler's hydrate dispatch: workload/<id> for the
    // workload-keyed set, service/<id> for the hydrator, node/<id> for
    // vm-reclamation, and any valid target for the target-agnostic arms
    // (NoopHeartbeat / WorkflowLifecycle scan or ignore the target).
    let cases: Vec<(&str, &str, AnyReconciler, TargetResource)> = vec![
        (
            "noop_heartbeat",
            "Unit",
            overdrive_control_plane::noop_heartbeat(),
            target("workload/job-app"),
        ),
        (
            "workload_lifecycle",
            "WorkloadLifecycle",
            overdrive_control_plane::workload_lifecycle(),
            target("workload/job-app"),
        ),
        (
            "workflow_lifecycle",
            "WorkflowLifecycle",
            overdrive_control_plane::workflow_lifecycle(),
            target("workflow/wf-app"),
        ),
        (
            "service_map_hydrator",
            "ServiceMapHydrator",
            overdrive_control_plane::service_map_hydrator(std::net::Ipv4Addr::LOCALHOST),
            target("service/1"),
        ),
        (
            "backend_discovery_bridge",
            "BackendDiscoveryBridge",
            overdrive_control_plane::backend_discovery_bridge(
                std::net::Ipv4Addr::LOCALHOST,
                node_id("writer-1"),
            ),
            target("workload/svc-app"),
        ),
        (
            "service_lifecycle",
            "ServiceLifecycle",
            overdrive_control_plane::service_lifecycle(),
            target("workload/svc-app"),
        ),
        (
            "svid_lifecycle",
            "SvidLifecycle",
            overdrive_control_plane::svid_lifecycle(),
            target("workload/svc-app"),
        ),
        (
            "vm_reclamation",
            "VmReclamation",
            overdrive_control_plane::vm_reclamation(),
            target("node/local"),
        ),
    ];

    assert_eq!(cases.len(), 8, "the golden MUST cover all 8 AnyReconciler variants");

    for (name, expected_variant, reconciler, tgt) in &cases {
        let desired = hydrate_desired_for_test(reconciler, tgt, &state)
            .await
            .unwrap_or_else(|e| panic!("hydrate_desired for `{name}` must succeed pre-move: {e}"));
        let actual = hydrate_actual_for_test(reconciler, tgt, &state)
            .await
            .unwrap_or_else(|e| panic!("hydrate_actual for `{name}` must succeed pre-move: {e}"));

        // B-03 anchor — each reconciler hydrates to its OWN AnyState variant on
        // both sides (NoopHeartbeat is `Unit` both sides).
        assert_eq!(
            variant_name(&desired),
            *expected_variant,
            "hydrate_desired for `{name}` must produce AnyState::{expected_variant}",
        );
        assert_eq!(
            variant_name(&actual),
            *expected_variant,
            "hydrate_actual for `{name}` must produce AnyState::{expected_variant}",
        );

        handle_golden(
            mode,
            &format!("{name}.txt"),
            &render(name, expected_variant, tgt, &desired, &actual),
        );
    }
}
