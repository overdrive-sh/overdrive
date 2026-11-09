//! The guest-listener probe task's startup rows (D14 item 3; S-53).
//!
//! Driving port: the real `ProbeRunner::start_alloc(&AllocationSpec)` for a VM
//! allocation whose sole startup descriptor has mechanic `GuestListeners` and
//! whose `declared_listeners` is the declared set. Sim adapters at the driven
//! ports: `SimGuestListenerReports` (the reported-set port), `SimObservationStore`
//! (the row oracle, with its write-failure injection) and `SimClock`.
//!
//! Traceability: `NoFailBeforeDeadline`, `NoPassAfterEnding`, PROBE-RUN-COUNT
//! (the rows use the shared per-(allocation, probe) run state).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::panic,
    reason = "test fixtures assert their own preconditions"
)]

use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::aggregate::Listener;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::observation::{ProbeIdx, ProbeResultRow, ProbeRole, ProbeStatus};
use overdrive_core::traits::GuestListenerReports;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_listener_reports::SimGuestListenerReports;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
use proptest::prelude::*;

/// `interval_seconds` x `max_attempts` is the derived 60 s startup deadline.
const INTERVAL_SECS: u64 = 2;
const MAX_ATTEMPTS: u32 = 30;
const DEADLINE_SECS: u64 = INTERVAL_SECS * MAX_ATTEMPTS as u64;

const fn port(p: u16) -> NonZeroU16 {
    NonZeroU16::new(p).expect("non-zero port")
}

const fn listener(protocol: Proto, p: u16) -> Listener {
    Listener { port: port(p), protocol }
}

fn startup_descriptor() -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::GuestListeners,
        timeout_seconds: 5,
        interval_seconds: u32::try_from(INTERVAL_SECS).expect("small"),
        max_attempts: MAX_ATTEMPTS,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn spec(alloc: &AllocationId, declared: Vec<Listener>) -> AllocationSpec {
    AllocationSpec {
        declared_listeners: declared,
        alloc: alloc.clone(),
        identity: SpiffeId::for_allocation(&WorkloadId::new("listening").expect("workload"), alloc),
        driver: DriverPayload::Vm(VmPayload {
            command: "/bin/serve".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        probe_descriptors: vec![startup_descriptor()],
        transport: None,
        service_ports: Vec::new(),
    }
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

struct Rig {
    runner: ProbeRunner,
    obs: Arc<SimObservationStore>,
    clock: Arc<SimClock>,
    reports: Arc<SimGuestListenerReports>,
    alloc: AllocationId,
}

impl Rig {
    /// Opens the allocation's reported-set entry, then starts the allocation.
    async fn start(declared: Vec<Listener>) -> Self {
        let obs = Arc::new(SimObservationStore::single_peer(NodeId::new("n").expect("node"), 0));
        let clock = Arc::new(SimClock::new());
        let reports = Arc::new(SimGuestListenerReports::new());
        let runner = ProbeRunner::new(
            Arc::new(SimTcpProber::new()),
            Arc::new(SimHttpProber::new()),
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&obs) as Arc<dyn ObservationStore>,
            Arc::new(SimGuestStreamDial::new()),
            Arc::clone(&reports) as Arc<dyn GuestListenerReports>,
        );
        let alloc = AllocationId::new("listening-0").expect("alloc");
        reports.open(&alloc);
        let _token = runner.start_alloc(&spec(&alloc, declared));
        settle().await;
        Self { runner, obs, clock, reports, alloc }
    }

    async fn tick(&self, secs: u64) {
        self.clock.tick(Duration::from_secs(secs));
        settle().await;
    }

    async fn insert(&self, protocol: Proto, p: u16) -> bool {
        let inserted = self.reports.insert(&self.alloc, protocol, port(p));
        settle().await;
        inserted
    }

    async fn startup_row(&self) -> Option<ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(&self.alloc)
            .await
            .expect("list rows")
            .into_iter()
            .find(|row| row.role == ProbeRole::Startup && row.probe_idx == ProbeIdx::new(0))
    }
}

fn fail_reason(row: &ProbeResultRow) -> &str {
    match &row.status {
        ProbeStatus::Fail { last_fail_reason } => last_fail_reason,
        ProbeStatus::Pass => panic!("expected a Fail row, got Pass"),
    }
}

/// S-53 / D14 item 3: the insertion that completes the declared set writes
/// one `Pass` at once: the first written row, `run_seq` 1, `same_status_runs` 1,
/// with no earlier row.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task writes the startup rows"]
async fn completing_the_declared_set_writes_one_pass_row_with_no_earlier_row() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080), listener(Proto::Udp, 5353)]).await;

    assert!(rig.insert(Proto::Tcp, 8080).await);
    rig.tick(DEADLINE_SECS - 1).await;
    assert!(rig.startup_row().await.is_none(), "no row while the set is incomplete");

    assert!(rig.insert(Proto::Udp, 5353).await);
    let row = rig.startup_row().await.expect("the completing insertion writes a row at once");
    assert_eq!(row.status, ProbeStatus::Pass);
    assert_eq!(row.run_seq.get(), 1, "the Pass is the first written row");
    assert_eq!(row.same_status_runs.get(), 1);
    assert_eq!(row.role, ProbeRole::Startup);
    rig.runner.stop_alloc(&rig.alloc);
}

/// S-53 / NoFailBeforeDeadline: a set that never completes writes no row before
/// the 60 s deadline; the first `Fail` at the deadline has `same_status_runs` 1
/// and the missing listeners in declared order; one further `Fail` per interval
/// follows with the streak rising 2, 3, ...
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task deadline Fail cadence"]
async fn an_incomplete_set_writes_no_row_before_the_deadline_then_one_fail_per_interval() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080), listener(Proto::Udp, 5353)]).await;

    rig.tick(DEADLINE_SECS - 1).await;
    assert!(rig.startup_row().await.is_none(), "no Fail before the deadline");

    rig.tick(1).await;
    let first = rig.startup_row().await.expect("the first Fail lands at the deadline");
    assert_eq!(first.run_seq.get(), 1);
    assert_eq!(first.same_status_runs.get(), 1);
    assert_eq!(
        fail_reason(&first),
        format!("guest listeners not open after {DEADLINE_SECS}s: tcp:8080 udp:5353"),
    );

    for expected in 2_u32..=4 {
        rig.tick(INTERVAL_SECS).await;
        let row = rig.startup_row().await.expect("a further Fail per interval");
        assert!(matches!(row.status, ProbeStatus::Fail { .. }));
        assert_eq!(row.same_status_runs.get(), expected);
        assert_eq!(row.run_seq.get(), u64::from(expected));
    }
    rig.runner.stop_alloc(&rig.alloc);
}

/// S-53: the Fail reason lists only the listeners still missing.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task deadline Fail cadence"]
async fn the_deadline_fail_reason_names_only_the_missing_listeners() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080), listener(Proto::Udp, 5353)]).await;
    assert!(rig.insert(Proto::Tcp, 8080).await);
    rig.tick(DEADLINE_SECS).await;
    let row = rig.startup_row().await.expect("Fail at the deadline");
    assert_eq!(
        fail_reason(&row),
        format!("guest listeners not open after {DEADLINE_SECS}s: udp:5353")
    );
    rig.runner.stop_alloc(&rig.alloc);
}

/// S-53: a failing observation write on the `Pass` is retried until it lands,
/// and the failed write advanced neither `run_seq` nor `same_status_runs`.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task retries a failed Pass write"]
async fn a_failed_pass_write_is_retried_until_it_lands_without_advancing_the_counters() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080)]).await;
    rig.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
        "injected probe-row write failure",
    )));
    assert!(rig.insert(Proto::Tcp, 8080).await);

    let mut landed = rig.startup_row().await;
    for _ in 0..8 {
        if landed.is_some() {
            break;
        }
        rig.tick(INTERVAL_SECS).await;
        landed = rig.startup_row().await;
    }
    let row = landed.expect("the Pass write is retried until it lands");
    assert_eq!(row.status, ProbeStatus::Pass);
    assert_eq!(row.run_seq.get(), 1, "the failed write advanced no run_seq");
    assert_eq!(row.same_status_runs.get(), 1, "the failed write advanced no streak");
    rig.runner.stop_alloc(&rig.alloc);
}

/// S-53: a failing write on a `Fail` advances neither number: the next `Fail`
/// that lands is still the first written row.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task Fail write failure accounting"]
async fn a_failed_fail_write_advances_neither_number() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080)]).await;
    rig.tick(DEADLINE_SECS - 1).await;
    rig.obs.inject_write_failure(ObservationStoreError::Io(std::io::Error::other(
        "injected probe-row write failure",
    )));
    rig.tick(1).await;
    assert!(rig.startup_row().await.is_none(), "the failed deadline write left no row");

    rig.tick(INTERVAL_SECS).await;
    let row = rig.startup_row().await.expect("the next interval's Fail lands");
    assert!(matches!(row.status, ProbeStatus::Fail { .. }));
    assert_eq!(row.run_seq.get(), 1);
    assert_eq!(row.same_status_runs.get(), 1);
    rig.runner.stop_alloc(&rig.alloc);
}

/// S-53 / NoPassAfterEnding: `stop_alloc` mid-wait, then the set completes and
/// the deadline passes: no row write is started after the cancellation. A
/// control allocation on the same runner, not stopped, shows the task does write
/// when it is not cancelled (so the absence is not vacuous).
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task observes cancellation before a write"]
async fn no_row_is_written_after_stop_alloc() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080)]).await;
    let control = AllocationId::new("listening-control").expect("alloc");
    rig.reports.open(&control);
    let _token = rig.runner.start_alloc(&spec(&control, vec![listener(Proto::Tcp, 8080)]));
    settle().await;

    rig.tick(INTERVAL_SECS).await;
    rig.runner.stop_alloc(&rig.alloc);
    settle().await;

    assert!(rig.insert(Proto::Tcp, 8080).await);
    assert!(rig.reports.insert(&control, Proto::Tcp, port(8080)));
    settle().await;
    rig.tick(DEADLINE_SECS * 2).await;

    let control_rows = rig.obs.list_probe_results_for_alloc(&control).await.expect("list");
    assert_eq!(
        control_rows.first().map(|r| r.status.clone()),
        Some(ProbeStatus::Pass),
        "control: the uncancelled allocation writes its Pass"
    );
    assert!(rig.startup_row().await.is_none(), "a stopped allocation writes no row");
    rig.runner.stop_alloc(&control);
}

/// S-53: a report for an undeclared listener or a duplicate of an already
/// reported one never completes the declared set.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D14 guest-listener task completes only on the declared set"]
async fn an_undeclared_or_duplicate_report_never_completes_the_set() {
    let rig = Rig::start(vec![listener(Proto::Tcp, 8080), listener(Proto::Tcp, 8081)]).await;

    assert!(rig.insert(Proto::Udp, 9999).await, "an undeclared pair is recorded by the board");
    assert!(rig.insert(Proto::Tcp, 8080).await);
    assert!(!rig.insert(Proto::Tcp, 8080).await, "a duplicate is not an insertion");
    rig.tick(INTERVAL_SECS).await;
    assert!(rig.startup_row().await.is_none(), "one of two declared listeners is still missing");

    assert!(rig.insert(Proto::Tcp, 8081).await);
    let row = rig.startup_row().await.expect("both declared listeners reported");
    assert_eq!(row.status, ProbeStatus::Pass);
    rig.runner.stop_alloc(&rig.alloc);
}

/// One step of a random schedule.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// Report declared listener `i`.
    Insert(usize),
    /// Advance the clock by one (1) or two (2) seconds.
    Tick(u64),
    /// Make the next row write fail.
    FailNextWrite,
}

fn arb_step(declared: usize) -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0..declared).prop_map(Step::Insert),
        8 => (1_u64..=2).prop_map(Step::Tick),
        1 => Just(Step::FailNextWrite),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// PROBE-RUN-COUNT with the guest-listener task as a producer: over random
    /// insertion / tick / write-failure schedules the stored startup row's
    /// `run_seq` advances by 0 or 1 per step (one per written row), its
    /// `same_status_runs` is the count of consecutive written rows of its
    /// status, no `Fail` is stored before the deadline, and no row is written
    /// after the `Pass`. Failures print the proptest seed.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    #[ignore = "pending DELIVER: D14 guest-listener task is a run-numbering producer"]
    fn guest_listener_rows_keep_the_run_numbering_over_random_schedules(
        declared_count in 1_usize..=3,
        steps in proptest::collection::vec(arb_step(3), 1..=60),
    ) {
        let declared: Vec<Listener> =
            (0..declared_count).map(|i| listener(Proto::Tcp, 8000 + u16::try_from(i).expect("small"))).collect();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let rig = Rig::start(declared.clone()).await;
            let mut elapsed = 0_u64;
            let mut prev: Option<ProbeResultRow> = None;
            for step in &steps {
                match *step {
                    Step::Insert(i) => {
                        let l = &declared[i % declared.len()];
                        rig.insert(l.protocol, l.port.get()).await;
                    }
                    Step::Tick(secs) => {
                        elapsed += secs;
                        rig.tick(secs).await;
                    }
                    Step::FailNextWrite => rig.obs.inject_write_failure(ObservationStoreError::Io(
                        std::io::Error::other("injected probe-row write failure"),
                    )),
                }
                let now = rig.startup_row().await;
                match (&prev, &now) {
                    (None, None) => {}
                    (None, Some(row)) => {
                        prop_assert_eq!(row.run_seq.get(), 1, "the first written row is run 1");
                        prop_assert_eq!(row.same_status_runs.get(), 1);
                    }
                    (Some(_), None) => prop_assert!(false, "a stored row never disappears"),
                    (Some(before), Some(after)) => {
                        let delta = after.run_seq.get() - before.run_seq.get();
                        prop_assert!(delta <= 1, "at most one row per step, got {}", delta);
                        if delta == 0 {
                            prop_assert_eq!(before, after, "no new row, nothing changes");
                        } else {
                            let same = std::mem::discriminant(&before.status)
                                == std::mem::discriminant(&after.status);
                            let expected = if same { before.same_status_runs.get() + 1 } else { 1 };
                            prop_assert_eq!(after.same_status_runs.get(), expected);
                            prop_assert!(
                                !matches!(before.status, ProbeStatus::Pass),
                                "no row is written after the Pass"
                            );
                        }
                    }
                }
                if let Some(row) = &now
                    && matches!(row.status, ProbeStatus::Fail { .. })
                {
                    prop_assert!(elapsed >= DEADLINE_SECS, "no Fail before the deadline");
                }
                prev = now;
            }
            // Non-vacuity: reporting every declared listener and letting the
            // clock run must leave a stored row (the Pass, or the Fail the
            // deadline already wrote); a task that writes nothing fails here.
            for l in &declared {
                rig.insert(l.protocol, l.port.get()).await;
            }
            for _ in 0..8 {
                rig.tick(INTERVAL_SECS).await;
            }
            prop_assert!(rig.startup_row().await.is_some(), "the task wrote a row");
            rig.runner.stop_alloc(&rig.alloc);
            Ok(())
        })?;
    }
}
