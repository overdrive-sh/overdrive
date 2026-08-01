//! Step 02-03 / Slice 3A.3 scenario 3.7 — walking-skeleton:
//! `killed_workload_is_restarted_with_fresh_alloc_id`.
//!
//! Submits a 1-replica job; waits until the alloc is Running; SIGKILLs
//! the workload externally; drives the convergence loop forward; and
//! asserts the alloc recovers under the (deterministic, same) `alloc_id`
//! (Phase 1 reuses `mint_alloc_id(workload_id)` per ADR-0023).
//!
//! # The contract (ADR-0078 § D6)
//!
//! Phase 3 asserts on the DURABLE crash facts the recovered `Running`
//! row carries — `restart_count` and `last_terminated` — never on a
//! transient `Failed` row. This is strictly stronger than the assertion
//! it replaced: the old shape proved only "a `Failed` row existed at some
//! instant"; this proves the SIGKILL was classified as a *crash* (not an
//! intentional stop), that the workload recovered, that exactly one
//! restart was counted, and that the recovered row strictly dominates the
//! terminal it describes — all from state no LWW merge can discard.
//!
//! The "fresh `alloc_id`" framing in the scenario name reflects the
//! Phase-2+ direction; in Phase 1 single-mode the alloc id is a pure
//! function of the job id (`alloc-{workload_id}-0`), so observable rebirth
//! is the state transition Terminated → Running with a distinct PID
//! at the driver layer.
//!
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::reconciler_runtime::{ReconcilerRuntime, run_convergence_tick};
use overdrive_control_plane::worker::exit_observer;
use overdrive_control_plane::{AppState, noop_heartbeat, workload_lifecycle};
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Job, JobSpecInput, ResourcesInput,
};
use overdrive_core::id::NodeId;
use overdrive_core::reconcilers::TargetResource;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, ObservationStore};
use overdrive_core::transition_reason::TransitionReason;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use overdrive_worker::ExecDriver;
use tempfile::TempDir;

use super::cleanup::AllocCleanup;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn killed_workload_is_restarted_with_fresh_alloc_id() {
    let tmp = TempDir::new().expect("tempdir");
    let mut runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime");
    runtime.register(noop_heartbeat()).await.expect("register noop");
    runtime.register(workload_lifecycle()).await.expect("register workload-lifecycle");

    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("open store"));
    let obs: Arc<dyn ObservationStore> =
        Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("node id"), 0));
    // Driver-internal clock (SIGTERM grace timing) is SimClock; the
    // convergence-tick's `tick.now_unix` snapshot needs real wall-clock
    // advancement so the WorkloadLifecycle backoff predicate
    // `tick.now_unix < last_failure_seen_at + backoff(attempts)`
    // crosses against the test's real-wall-clock pacing
    // (`tokio::time::sleep(20ms)` between manual ticks). Explicit
    // `SystemClock` at the call site per `.claude/rules/development.md`
    // § "Port-trait dependencies" — the choice is visible, not
    // silently inherited.
    let driver: Arc<dyn Driver> = Arc::new(ExecDriver::new(
        std::path::PathBuf::from("/sys/fs/cgroup"),
        Arc::new(overdrive_sim::adapters::clock::SimClock::new()),
        Arc::new(overdrive_host::RealCgroupFs::new()),
    ));

    let allocator = overdrive_control_plane::test_default_allocator(
        Arc::clone(&store) as Arc<dyn overdrive_core::traits::intent_store::IntentStore>
    );
    let state = AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(overdrive_host::SystemClock),
        Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        overdrive_core::id::NodeId::new("writer-1").unwrap(),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    );

    // Spawn the exit-observer subsystem. In production this is wired
    // by `run_server_with_obs_and_driver`; tests construct it directly
    // so the watcher's `ExitEvent`s are consumed and classified into
    // `AllocStatusRow`s on the obs store. Without this spawn, the
    // production `ExecDriver`'s per-alloc watcher task fires (sending
    // events on its mpsc channel) but nothing reads the receiver, so
    // no `Failed`/`Terminated` row ever appears in obs.
    let _exit_observer = exit_observer::spawn(
        state.obs.clone(),
        state.driver.clone(),
        state.lifecycle_events.clone(),
        state.clock.clone(),
    );

    // Cleanup guard — fires when the test exits (panic or success) and
    // mass-kills every workload cgroup the test created via
    // `cgroup.kill`. Prevents the `LEAK` flag from nextest without
    // depending on the `tokio::test` runtime that owns the `Child`
    // handles being alive at drop time.
    let _cleanup = AllocCleanup {
        obs: state.obs.clone(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // Use a distinct workload_id so the derived cgroup scope
    // (`alloc-recovery-0.scope`) does not collide with the scope used by
    // submit_to_running (`alloc-payments-0.scope`) when both tests run in
    // parallel under nextest.
    let job = Job::from_submit(JobSpecInput {
        id: "recovery".to_string(),
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

    let target = TargetResource::new("workload/recovery").expect("valid target");
    let workload_lifecycle_name =
        overdrive_core::reconcilers::ReconcilerName::new("workload-lifecycle")
            .expect("workload-lifecycle reconciler name");
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);

    // Phase 1: drive to first Running.
    let mut tick_n = 0_u64;
    let mut first_running = false;
    while tick_n < 30 && !first_running {
        run_convergence_tick(
            &state,
            &workload_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        first_running = rows.iter().any(|r| r.state == AllocState::Running);
        tick_n += 1;
    }
    assert!(first_running, "alloc must reach Running before crash");

    // Phase 2: simulate crash by SIGKILLing the workload PID
    // externally. Step 01-02's worker exit-observer subsystem reads
    // the natural `child.wait()` resolution and writes
    // `AllocState::Failed` to obs — the synthetic-write workaround
    // that previously stood in here is gone.
    //
    // Read the workload PID from `cgroup.procs`. The action shim has
    // already written a `Running` row at this point, and `ExecDriver`
    // has placed the spawned `/bin/sleep` PID into the workload scope
    // (same pattern `AllocCleanup` uses for cleanup). The PID is the
    // SIGKILL target.
    let rows = state.obs.alloc_status_rows().await.expect("read rows");
    let prior = rows.into_iter().find(|r| r.state == AllocState::Running).expect("running row");
    let scope = std::path::PathBuf::from("/sys/fs/cgroup")
        .join("overdrive.slice/workloads.slice")
        .join(format!("{}.scope", prior.alloc_id));
    let procs_text =
        std::fs::read_to_string(scope.join("cgroup.procs")).expect("read cgroup.procs");
    let pid: libc::pid_t = procs_text
        .lines()
        .find_map(|line| line.trim().parse::<i32>().ok())
        .expect("workload PID present in cgroup.procs");
    // SAFETY: SIGKILL on a child PID owned by this test. The PID was
    // minted by `ExecDriver::start` in this same test and resides in
    // the workload's cgroup scope; it is alive at the moment we read
    // it from `cgroup.procs`. `libc::kill` returns 0 on success and
    // -1 on error (with `errno` set); we ignore the return because
    // the assertion downstream is on the obs row the watcher writes,
    // not on the syscall result.
    let _ = unsafe { libc::kill(pid, libc::SIGKILL) };

    // Phase 3: drive convergence until the DURABLE crash facts appear on
    // the recovered Running row (ADR-0078 § D6).
    //
    // `last_terminated` and `restart_count` survive the LWW merge by
    // construction, so there is no transient window to catch and no race
    // to lose. The previous shape polled for a row in `AllocState::Failed`
    // and captured its counter — but that `Failed` row is transient BY
    // DESIGN (the reconciler's whole job is to replace it), and the test
    // only won the race at HEAD because of the pre-ADR-0077 defect: the
    // exit observer stamped `prior.counter + 1`, the restart write stamped
    // `tick.tick + 1`, the two tied on the very next tick, and `dominates`
    // returns `false` on `Equal` — so the restart write was silently
    // DROPPED and `Failed` lingered long enough to be seen. Under ADR-0077
    // the restart write dominates immediately and whether a 20 ms poll
    // lands inside the exit-observer-write → shim-write window is a
    // genuine race the test has no way to win reliably.
    //
    // Real wall-clock sleep (not just yield) between ticks so the OS
    // can deliver SIGCHLD, ExecDriver's per-alloc watcher (`child.wait()`)
    // can resolve, the watcher can write to its mpsc, and the
    // exit_observer can drain it and write the Failed row to obs.
    // Under heavy parallel test load `yield_now` alone has been
    // observed to complete the tick budget before the OS reaper has
    // had a chance to deliver the signal — `tokio::time::sleep`
    // releases the current task to the runtime AND advances real
    // wall-clock during which the kernel can do its work.
    let mut recovered: Option<AllocStatusRow> = None;
    while tick_n < 150 && recovered.is_none() {
        run_convergence_tick(
            &state,
            &workload_lifecycle_name,
            &target,
            start + Duration::from_millis(tick_n.saturating_mul(100)),
            tick_n,
            deadline,
        )
        .await
        .expect("tick");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let rows = state.obs.alloc_status_rows().await.expect("read rows");
        recovered =
            rows.into_iter().find(|r| r.state == AllocState::Running && r.restart_count >= 1);
        tick_n += 1;
    }
    let row = recovered.expect(
        "alloc must converge to a Running row carrying restart_count >= 1 after SIGKILL \
         within the Phase-3 tick budget",
    );

    // The crash happened, exactly once.
    assert_eq!(row.restart_count, 1, "exactly one observed restart");

    // The crash is durably described on the converged row.
    let lt = row.last_terminated.as_ref().expect("recovered row must carry last_terminated");
    assert_eq!(lt.state, AllocState::Failed, "the SIGKILL was classified Failed");
    assert!(
        matches!(lt.reason, Some(TransitionReason::WorkloadCrashedImmediately { .. })),
        "the SIGKILL must be classified as a crash, not an intentional stop: {:?}",
        lt.reason,
    );

    // The recovered row strictly dominates the terminal it summarises.
    assert!(
        row.updated_at.dominates(&lt.terminated_at),
        "the recovered Running row must dominate the Failed row it snapshots",
    );
}
