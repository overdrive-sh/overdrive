//! D9 (ADR-0261), gate G-6 — liveness supervision begins when the
//! allocation becomes `Stable`; the probe runner owns the gate.
//!
//! Traceability: obligation S-31 (runner contract: `LivenessOnlyAfterStable`,
//! `NoLivenessForEndedId`), through the production `VmDriver` hooks
//! (`on_alloc_running` / `on_alloc_stable` / `on_alloc_terminal`) over a real
//! `ProbeRunner`, `SimClock` and `SimTcpProber`. No test seam: the observables
//! are the runner's own inspection surface (`is_role_live`,
//! `active_alloc_count`), the stored probe rows, and the prober's call count.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::SpiffeId;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::vm::config::{Gid, HostArch, VmConfinement, VmmIdentity};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::{BeaconIntake, VmDriver, VmHostLayout};

const INTERVAL_SECS: u32 = 2;

fn descriptor(role: ProbeRole) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role,
        mechanic: ProbeMechanic::Tcp { host: "127.0.0.1".to_owned(), port: 8080 },
        timeout_seconds: 1,
        interval_seconds: INTERVAL_SECS,
        max_attempts: 30,
        failure_threshold: (role == ProbeRole::Liveness).then_some(3),
        success_threshold: (role == ProbeRole::Readiness).then_some(1),
        inferred: false,
    }
}

fn spec(alloc: &AllocationId, roles: &[ProbeRole]) -> AllocationSpec {
    AllocationSpec {
        declared_listeners: Vec::new(),
        alloc: alloc.clone(),
        identity: SpiffeId::from_str("spiffe://overdrive.local/test/svc").expect("spiffe id"),
        driver: DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: "/kernel".into(),
            rootfs: "/rootfs".into(),
        }),
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: roles.iter().map(|r| descriptor(*r)).collect(),
        transport: None,
        service_ports: Vec::new(),
    }
}

/// One control-plane process: a production `VmDriver` over a real runner.
struct Process {
    driver: VmDriver,
    runner: Arc<ProbeRunner>,
    clock: Arc<SimClock>,
    obs: Arc<SimObservationStore>,
    tcp: Arc<SimTcpProber>,
}

async fn process() -> Process {
    let clock = Arc::new(SimClock::new());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("liveness-gate").expect("node id"),
        0,
    ));
    let tcp = Arc::new(SimTcpProber::new());
    let runner = Arc::new(ProbeRunner::new(
        Arc::clone(&tcp) as _,
        Arc::new(SimHttpProber::new()),
        Arc::clone(&clock) as _,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
        Arc::new(SimGuestStreamDial::new()),
        Arc::new(overdrive_worker::GuestListenerBoard::new()),
    ));
    let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(clock.clone());
    assert!(wiring.supervisor().open_after_boot());
    let vmm = overdrive_sim::SimVmm::new();
    let beacon = BeaconIntake::bind(
        Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(vmm.clone())),
        overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
    )
    .await
    .expect("bind the node beacon intake");
    let driver = VmDriver::new(
        Arc::new(vmm),
        clock.clone(),
        Arc::new(overdrive_sim::SimCgroupFs::new()),
        Arc::new(overdrive_sim::SimCgroupAccounting::new()),
        Arc::clone(&runner),
        wiring.gate(),
        VmHostLayout {
            cgroup_root: PathBuf::from("/tmp/liveness-gate-cgroup"),
            run_dir_root: PathBuf::from("/tmp/liveness-gate-run"),
            clone_index_dir: PathBuf::from("/tmp/liveness-gate-index"),
            clone_staging_dir: PathBuf::from("/tmp/liveness-gate-staging"),
            arch: HostArch::X86_64,
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: Vec::new() },
                1024,
            ),
        },
        beacon,
        Arc::new(overdrive_worker::GuestListenerBoard::new()),
    );
    Process { driver, runner, clock, obs, tcp }
}

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

impl Process {
    /// One probe period, then let the probe tasks run.
    async fn period(&self) {
        self.clock.tick(Duration::from_secs(u64::from(INTERVAL_SECS)));
        settle().await;
    }

    async fn liveness_rows(
        &self,
        alloc: &AllocationId,
    ) -> Vec<overdrive_core::observation::ProbeResultRow> {
        self.obs
            .list_probe_results_for_alloc(alloc)
            .await
            .expect("list probe rows")
            .into_iter()
            .filter(|r| r.role == ProbeRole::Liveness)
            .collect()
    }
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("alloc id")
}

const ALL: [ProbeRole; 3] = [ProbeRole::Startup, ProbeRole::Readiness, ProbeRole::Liveness];

/// S-31 — `LivenessOnlyAfterStable`: `on_alloc_running` supervises startup and
/// readiness but not liveness; `on_alloc_stable` retires startup and starts
/// liveness; the terminal hook removes the supervisor.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn liveness_supervision_begins_when_the_allocation_becomes_stable() {
    let p = process().await;
    let alloc = aid("gate-alloc-a");

    p.driver.on_alloc_running(&spec(&alloc, &ALL));
    assert_eq!(p.runner.active_alloc_count(), 1);
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Startup));
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Readiness));
    assert!(
        !p.runner.is_role_live(&alloc, ProbeRole::Liveness),
        "a Running allocation that is not yet Stable has no liveness supervision"
    );

    p.driver.on_alloc_stable(&alloc);
    assert!(!p.runner.is_role_live(&alloc, ProbeRole::Startup), "Stable retires startup");
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Readiness), "readiness is unchanged");
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Liveness), "Stable starts liveness");
    assert_eq!(p.runner.active_alloc_count(), 1);

    p.driver.on_alloc_terminal(&alloc);
    assert_eq!(p.runner.active_alloc_count(), 0);
    for role in ALL {
        assert!(!p.runner.is_role_live(&alloc, role), "{role:?} ends with the allocation");
    }
}

/// S-31 — `LivenessStreakStartsAtStable`: no liveness row exists before the
/// `Stable` hook; the first lands one interval after it with its streak at 1.
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn first_liveness_row_lands_one_interval_after_stable_with_streak_one() {
    let p = process().await;
    let alloc = aid("gate-alloc-b");
    p.driver.on_alloc_running(&spec(&alloc, &ALL));
    settle().await;

    p.period().await;
    p.period().await;
    assert!(p.liveness_rows(&alloc).await.is_empty(), "no liveness row before Stable");

    p.driver.on_alloc_stable(&alloc);
    settle().await;
    assert!(p.liveness_rows(&alloc).await.is_empty(), "tick-then-sleep: none at Stable itself");

    p.period().await;
    let rows = p.liveness_rows(&alloc).await;
    assert_eq!(rows.len(), 1, "exactly one liveness row one interval after Stable");
    assert_eq!(rows[0].same_status_runs.get(), 1, "the streak starts at 1");
}

/// S-31 — a second `on_alloc_stable` spawns nothing (idempotent).
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn a_second_stable_hook_spawns_no_second_liveness_task() {
    let p = process().await;
    let alloc = aid("gate-alloc-c");
    p.driver.on_alloc_running(&spec(&alloc, &[ProbeRole::Liveness]));
    settle().await;
    p.period().await;
    assert_eq!(p.tcp.probe_call_count(), 0, "no liveness probe runs before Stable");

    p.driver.on_alloc_stable(&alloc);
    p.driver.on_alloc_stable(&alloc);
    settle().await;
    p.period().await;
    assert_eq!(p.tcp.probe_call_count(), 1, "one liveness task, one probe per interval");
}

/// S-31 — `NoLivenessForEndedId`: the terminal hook ran first; a late
/// `on_alloc_stable` leaves the supervisor count unchanged and spawns nothing.
/// Passes on HEAD and must keep passing once D9 lands.
#[tokio::test]
async fn stable_hook_after_the_end_starts_no_liveness() {
    let p = process().await;
    let alloc = aid("gate-alloc-d");
    p.driver.on_alloc_running(&spec(&alloc, &ALL));
    p.driver.on_alloc_terminal(&alloc);
    assert_eq!(p.runner.active_alloc_count(), 0);

    p.driver.on_alloc_stable(&alloc);
    settle().await;
    p.period().await;
    assert_eq!(p.runner.active_alloc_count(), 0, "an ended allocation is not re-registered");
    assert!(!p.runner.is_role_live(&alloc, ProbeRole::Liveness));
    assert_eq!(p.tcp.probe_call_count(), 0, "nothing probes an ended allocation");
}

/// Unaffected-state control: startup and readiness supervision follow
/// `Running` and `Stable` as before D9. Passes on HEAD.
#[tokio::test]
async fn startup_and_readiness_supervision_are_unchanged_by_the_liveness_gate() {
    let p = process().await;
    let alloc = aid("gate-alloc-e");
    p.driver.on_alloc_running(&spec(&alloc, &[ProbeRole::Startup, ProbeRole::Readiness]));
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Startup));
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Readiness));
    p.driver.on_alloc_stable(&alloc);
    assert!(!p.runner.is_role_live(&alloc, ProbeRole::Startup));
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Readiness));
    assert_eq!(p.runner.active_alloc_count(), 1);
}

/// S-31 — a Job-shaped allocation (no liveness descriptor) is unchanged: the
/// Stable hook neither errors nor registers a liveness role. Passes on HEAD.
#[tokio::test]
async fn an_allocation_without_a_liveness_descriptor_never_has_a_liveness_role() {
    let p = process().await;
    let alloc = aid("gate-alloc-f");
    p.driver.on_alloc_running(&spec(&alloc, &[ProbeRole::Readiness]));
    p.driver.on_alloc_stable(&alloc);
    assert!(!p.runner.is_role_live(&alloc, ProbeRole::Liveness));
    assert!(p.runner.is_role_live(&alloc, ProbeRole::Readiness));
}

/// S-31 — `start_liveness` is atomic with `stop_alloc` (LG-2): over seeds,
/// whichever wins, the end state is either "liveness live and still
/// cancellable" or "allocation gone and nothing live"; never a live liveness
/// role for a removed supervisor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// CONTRACT_SHAPE: bounded-change.
async fn concurrent_stop_and_start_liveness_never_leave_liveness_for_an_ended_id() {
    let p = process().await;
    let handle = tokio::runtime::Handle::current();
    for seed in 0_u64..64 {
        let alloc = aid(&format!("gate-race-{seed}"));
        p.driver.on_alloc_running(&spec(&alloc, &ALL));
        let stop_first = seed % 2 == 0;
        let spin = usize::try_from(seed % 7).expect("small");
        std::thread::scope(|s| {
            let stop = s.spawn(|| {
                let _g = handle.enter();
                if !stop_first {
                    for _ in 0..spin * 1_000 {
                        std::hint::spin_loop();
                    }
                }
                p.runner.stop_alloc(&alloc);
            });
            let start = s.spawn(|| {
                let _g = handle.enter();
                if stop_first {
                    for _ in 0..spin * 1_000 {
                        std::hint::spin_loop();
                    }
                }
                p.runner.start_liveness(&alloc);
            });
            stop.join().expect("stop thread");
            start.join().expect("start thread");
        });
        // seed printed on failure through the message.
        assert_eq!(
            p.runner.active_alloc_count(),
            0,
            "seed {seed}: the stop always wins in the end"
        );
        assert!(
            !p.runner.is_role_live(&alloc, ProbeRole::Liveness),
            "seed {seed}: liveness must not outlive a removed supervisor"
        );
    }
}

/// S-31 — control-plane restart between `Stable` and the first liveness row:
/// the new process has no supervisor for the old id, so its `on_alloc_stable`
/// is a no-op; the successor's supervisor registers fresh and its own `Stable`
/// starts its own liveness. (Boot reclamation making the old VM terminal is
/// owned and covered by `vm_reclamation_boot`.)
#[tokio::test]
/// CONTRACT_SHAPE: bounded-change.
async fn a_restarted_process_starts_liveness_only_for_the_successor_at_its_own_stable() {
    let old = aid("gate-old");
    let first = process().await;
    first.driver.on_alloc_running(&spec(&old, &ALL));
    first.driver.on_alloc_stable(&old);
    drop(first);

    let p = process().await;
    p.driver.on_alloc_stable(&old);
    assert_eq!(p.runner.active_alloc_count(), 0, "no supervisor for the old id");
    assert!(!p.runner.is_role_live(&old, ProbeRole::Liveness));

    let successor = aid("gate-successor");
    p.driver.on_alloc_running(&spec(&successor, &ALL));
    assert!(!p.runner.is_role_live(&successor, ProbeRole::Liveness));
    p.driver.on_alloc_stable(&successor);
    assert!(p.runner.is_role_live(&successor, ProbeRole::Liveness));
    assert!(!p.runner.is_role_live(&old, ProbeRole::Liveness));
}
