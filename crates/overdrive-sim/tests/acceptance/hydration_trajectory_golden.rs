//! S-ROH-B-02 — reconcile trajectory replay-equivalence golden.
//!
//! **Post-02-04 (ADR-0086 S3) role.** `hydrate_desired_for_test` /
//! `hydrate_actual_for_test` now drive the port-backed `AnyReconciler::hydrate_*`
//! forwarding (the moved per-reconciler hydrate bodies through a
//! `HydrationContext`), so this test now asserts the PORT-DRIVEN seeded reconcile
//! trajectory (emitted `Action`s + `View` evolution + observation rows)
//! reproduces the committed pre-move golden bit-for-bit under seed
//! `TRAJECTORY_SEED`, AND is bit-reproducible across two same-seed runs. The
//! golden fixture is the FIXED pre-move baseline and is NEVER regenerated.
//!
//! **CONTRACT_SHAPE: unbounded-preservation (replay-equivalence baseline).**
//! This is the second S2-gate artifact of the ADR-0086 hydration move. It
//! captures the PRE-MOVE reconcile trajectory — the emitted `Action`s, the
//! `View` evolution, and the observation-row progression — under a FIXED seed,
//! and pins it as a committed golden.
//!
//! **Why now (hard gate, do not reorder).** The move is single-cut: step 02-04
//! (ADR-0086 S3) deletes the central `hydrate_*` free fns and drives hydration
//! through the new `AnyReconciler::hydrate_*` ports. After that cut there is no
//! live old-vs-new trajectory to diff. So while the pre-move hydration path
//! still exists (post-02-02, this step), we capture the seeded trajectory as
//! the baseline. 02-04's S-ROH-B-02 assertion — "same seed → bit-identical
//! trajectory, matching the pre-move path" — asserts the port-driven path
//! reproduces THIS golden. Without it, B-02 has no expected baseline
//! (feature-delta § "HARD DELIVER GATE"; DISTILL Open Question 2; ADR-0086 D8).
//!
//! **Bit-reproducibility (the task's non-blocker requirement).** The trajectory
//! is captured by driving `AnyReconciler::reconcile` — a PURE synchronous
//! function (ADR-0035; `reconcile` reads no clock, no entropy, no store) —
//! over a deterministic seeded fixture, threading the `View` from tick to tick
//! and scripting a fixed observation-row progression. Because `reconcile` is
//! pure, its `(Vec<Action>, next View)` output is a deterministic function of
//! `(desired, actual, view, tick)`; every one of those is fixed here
//! (`LocalIntentStore` + seeded `SimObservationStore`, `SimClock`,
//! `SimEntropy(SEED)`, fixed `now_unix`, threaded view). The single-loop /
//! single-clock model (ADR-0086 D8) is therefore bit-reproducible **by the
//! purity contract**, not by luck — and the test proves it by running the whole
//! trajectory TWICE under the same seed and asserting the two are byte-identical
//! BEFORE comparing against the committed golden. `tick.now` (`Instant`) is used
//! only for the per-tick deadline budget and never leaks into an `Action` /
//! `View`; the twice-identical assertion is the structural guard that it does
//! not.
//!
//! **Fixed seed.** `TRAJECTORY_SEED` below. Recorded in the golden header.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::unused_async
)]

use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::{
    ReconcilerRuntime, hydrate_actual_for_test, hydrate_desired_for_test,
};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::reconcilers::{TargetResource, TickContext};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{AnyReconciler, AnyReconcilerView, AnyState, WorkloadLifecycleView};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

/// The fixed DST seed the trajectory is pinned to (recorded in the golden).
const TRAJECTORY_SEED: u64 = 0x00B0_2AA5;
/// Fixed logical `now_unix` base so the reconcile trajectory carries no
/// wall-clock nondeterminism.
const NOW_UNIX_BASE_SECS: u64 = 1_700_000_000;

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn workload_id(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}

fn alloc_id(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

/// Build the seeded `AppState`. Both nondeterminism seeds — the
/// `SimObservationStore` gossip seed and the `SimEntropy` serial seed — are
/// threaded from `TRAJECTORY_SEED`.
async fn build_seeded_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
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
            overdrive_sim::adapters::entropy::SimEntropy::new(TRAJECTORY_SEED),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

async fn seed_job_intent(state: &AppState, wid: &WorkloadId) {
    let job = Job::from_submit(JobSpecInput {
        id: wid.as_str().to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/serve".to_owned(),
            args: vec![],
        }),
    })
    .expect("valid job spec");
    let archived = WorkloadIntent::Job(job).archive_for_store().expect("rkyv archive");
    state
        .store
        .put(IntentKey::for_workload(wid).as_bytes(), archived.as_ref())
        .await
        .expect("put job intent");
    state
        .store
        .put(
            IntentKey::for_workload_kind(wid).as_bytes(),
            &[WorkloadKind::Job.discriminator_byte()],
        )
        .await
        .expect("put kind");
}

/// One scripted observation mutation the trajectory applies before a tick.
enum Step {
    /// No observation change this tick (re-tick against the current rows).
    NoChange,
    /// Write/overwrite the alloc row with the given state + optional crash
    /// reason at LWW counter `counter`.
    Alloc { aid: &'static str, state: AllocState, reason: Option<TransitionReason>, counter: u64 },
}

async fn apply_step(state: &AppState, wid: &WorkloadId, step: &Step) {
    let Step::Alloc { aid, state: alloc_state, reason, counter } = step else { return };
    let started_at = matches!(alloc_state, AllocState::Running)
        .then(|| UnixInstant::from_unix_duration(Duration::from_secs(NOW_UNIX_BASE_SECS)));
    let row = AllocStatusRow {
        alloc_id: alloc_id(aid),
        workload_id: wid.clone(),
        node_id: node_id("local"),
        state: *alloc_state,
        updated_at: LogicalTimestamp { counter: *counter, writer: node_id("writer-1") },
        reason: reason.clone(),
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Job,
        listeners: Vec::new(),
        started_at,
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    };
    state.obs.write(ObservationRow::AllocStatus(Box::new(row))).await.expect("write alloc row");
}

/// Stable projection of the observation rows for `wid` — avoids incidental
/// noise (writer node id, LWW counter) in the golden while keeping the
/// lifecycle-observable fields.
async fn project_rows(state: &AppState, wid: &WorkloadId) -> String {
    let mut rows = state.obs.alloc_status_rows().await.expect("read rows");
    rows.retain(|r| &r.workload_id == wid);
    rows.sort_by(|a, b| a.alloc_id.as_str().cmp(b.alloc_id.as_str()));
    let projected: Vec<String> = rows
        .iter()
        .map(|r| {
            format!(
                "{{ alloc={alloc}, state={state:?}, reason={reason:?}, restart_count={rc} }}",
                alloc = r.alloc_id.as_str(),
                state = r.state,
                reason = r.reason,
                rc = r.restart_count,
            )
        })
        .collect();
    format!("[{}]", projected.join(", "))
}

/// Drive the full deterministic trajectory once and return its rendered form.
/// Threads the `WorkloadLifecycleView` tick-to-tick; captures per step the
/// observation-row projection, the emitted `Action`s, and the next `View`.
async fn run_trajectory() -> String {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), TRAJECTORY_SEED))
        as Arc<dyn ObservationStore>;
    let state = build_seeded_app_state(&tmp, Arc::clone(&obs)).await;

    let wid = workload_id("traj-job");
    seed_job_intent(&state, &wid).await;

    let reconciler: AnyReconciler = overdrive_control_plane::workload_lifecycle();
    let tgt = TargetResource::new("workload/traj-job").expect("valid target");

    // A scripted, deterministic lifecycle: empty -> Running -> crash ->
    // re-tick-on-crash -> recovered Running. The reconciler's decisions at
    // each step are pinned by the golden — we do not predict them.
    let script: [Step; 5] = [
        Step::NoChange,
        Step::Alloc { aid: "traj-job-0", state: AllocState::Running, reason: None, counter: 1 },
        Step::Alloc {
            aid: "traj-job-0",
            state: AllocState::Terminated,
            reason: Some(TransitionReason::WorkloadCrashedImmediately {
                exit_code: Some(1),
                signal: None,
                stderr_tail: None,
            }),
            counter: 2,
        },
        Step::NoChange,
        Step::Alloc { aid: "traj-job-1", state: AllocState::Running, reason: None, counter: 1 },
    ];

    // The pinned per-run base `Instant` for the tick budget; its VALUE never
    // enters the trajectory (see module docs — proven by the twice-identical
    // assertion in the test below).
    let base = Instant::now();

    // View threaded across ticks — this IS the "View evolution" universe slot.
    let mut view = AnyReconcilerView::WorkloadLifecycle(WorkloadLifecycleView::default());

    let mut out = String::new();
    write!(
        out,
        "# ADR-0086 S2-gate reconcile-trajectory golden — reconciler `workload-lifecycle`\n\
         # seed: {seed:#010x}\n\
         # target: {target}\n\
         # captured from the PRE-MOVE hydrate path (hydrate_*_for_test +\n\
         # AnyReconciler::reconcile), step 02-03, before the S3 (02-04) cut.\n\
         # universe: observation rows (projected) + emitted Actions + View evolution.\n\n",
        seed = TRAJECTORY_SEED,
        target = tgt.as_str(),
    )
    .expect("write to String");

    for (i, step) in script.iter().enumerate() {
        apply_step(&state, &wid, step).await;

        let desired: AnyState =
            hydrate_desired_for_test(&reconciler, &tgt, &state).await.expect("hydrate_desired");
        let actual: AnyState =
            hydrate_actual_for_test(&reconciler, &tgt, &state).await.expect("hydrate_actual");

        let now = base + Duration::from_millis((i as u64) * 100);
        let tick = TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(
                NOW_UNIX_BASE_SECS + (i as u64) * 30,
            )),
            tick: i as u64,
            deadline: now + Duration::from_secs(3600),
        };

        let (actions, next_view) = reconciler.reconcile(&desired, &actual, &view, &tick);
        let rows = project_rows(&state, &wid).await;

        write!(
            out,
            "== tick {i} ==\n\
             observation_rows: {rows}\n\
             emitted_actions:\n{actions:#?}\n\
             next_view:\n{next_view:#?}\n\n",
            i = i,
            rows = rows,
            actions = actions,
            next_view = next_view,
        )
        .expect("write to String");

        view = next_view;
    }

    out
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/acceptance/fixtures/hydration_trajectory/workload_lifecycle_trajectory.txt")
}

// ---------------------------------------------------------------------------
// S-ROH-B-02 — the seeded reconcile-trajectory golden
// ---------------------------------------------------------------------------

/// Capture the pre-move seeded reconcile trajectory and pin it as the committed
/// replay-equivalence golden. Also proves the trajectory is bit-reproducible
/// under the fixed seed (the task's non-blocker requirement) by running it
/// twice and asserting byte-identity before comparing against the golden.
#[tokio::test]
async fn pre_move_reconcile_trajectory_is_reproducible_and_pinned() {
    // (1) Bit-reproducibility under the fixed seed — the B-02 property, proven
    // structurally (pure `reconcile` over a deterministic fixture). If this
    // fails, the trajectory is NOT bit-reproducible and B-02 has no stable
    // baseline — surface it rather than pinning a flapping golden.
    let run_a = run_trajectory().await;
    let run_b = run_trajectory().await;
    assert_eq!(
        run_a, run_b,
        "\n\nPRE-MOVE RECONCILE TRAJECTORY IS NOT BIT-REPRODUCIBLE under seed {:#010x}.\n\
         The single-loop/single-clock model (ADR-0086 D8) requires bit-identical\n\
         trajectories for the same seed; a diff here means a nondeterminism source\n\
         leaked into the reconcile output (BLOCKS B-02).\n",
        TRAJECTORY_SEED,
    );

    // (2) Assert the reproducible trajectory against the committed golden (the
    // 02-04 B-02 baseline). An ABSENT fixture is a FAILURE, not a silent
    // re-capture (review D4) — regeneration is the explicit, `#[ignore]`-gated
    // `regenerate_reconcile_trajectory_golden` opt-in below.
    let path = golden_path();
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "\n\nMISSING committed reconcile-trajectory golden ({e}).\n\
             An ABSENT S2-gate fixture is a FAILURE, not a silent re-capture — a deleted\n\
             fixture must never silently re-bless the current trajectory (review D4).\n\
             To DELIBERATELY (re)generate it, run the ignored regeneration test:\n\
             \n\
             cargo xtask lima run -- cargo nextest run -p overdrive-sim \\\n\
               --test acceptance --features integration-tests --run-ignored all \\\n\
               -E 'test(regenerate_reconcile_trajectory_golden)'\n\
             \n\
             then COMMIT the regenerated fixture.\n",
        )
    });
    assert_eq!(
        run_a, expected,
        "\n\nRECONCILE TRAJECTORY GOLDEN DRIFT.\n\
         The pre-move seeded trajectory changed. This golden is the ADR-0086\n\
         S2-gate baseline for the B-02 replay-equivalence bar (step 02-04).\n\
         If intended, run the ignored regeneration test (see the MISSING-fixture\n\
         message above) and COMMIT; otherwise reconcile behaviour regressed and\n\
         02-04's baseline would be wrong.\n"
    );
}

/// Deliberate trajectory-golden (re)generation — run on demand when the
/// pre-move seeded trajectory legitimately changes. `#[ignore]` so it never
/// runs in normal execution; the committed fixture is the load-bearing
/// artifact.
#[tokio::test]
#[ignore = "fixture regeneration tool — run on demand to (re)capture the S2-gate trajectory golden, then COMMIT; the committed fixture is the load-bearing artifact"]
async fn regenerate_reconcile_trajectory_golden() {
    let run = run_trajectory().await;
    let path = golden_path();
    std::fs::create_dir_all(path.parent().expect("fixtures parent")).expect("create fixtures dir");
    std::fs::write(&path, &run).expect("write trajectory golden");
    eprintln!(
        "REGENERATED reconcile-trajectory golden ({} bytes) — COMMIT the fixture (S2-gate).",
        run.len()
    );
}
