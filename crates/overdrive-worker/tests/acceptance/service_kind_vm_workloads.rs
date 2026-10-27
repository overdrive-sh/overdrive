//! Acceptance coverage for ADR-0090's existing `ProbeRunner` and
//! `VmDriver` boundaries, with INB-1 (ADR-0173): a VM probe whose target is
//! inside the guest prefix — the leased `workload_addr` — runs through
//! `GuestStreamDial::dial(target, DialPurpose::Probe)`, never through the TCP
//! or HTTP prober; any other target keeps the prober path. No fallback case for `Vm + None` is present because
//! the accepted design establishes `Some(workload_addr)` as a production
//! registration precondition.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError, GuestStreamDial};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{HttpProber, ProbeFailure, ProbeOutcome};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_stream_dial::{SimDialScript, SimGuestStreamDial};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimHttpProber, SimTcpProber};
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs, SimVmm};
use overdrive_worker::VmDriver;
use overdrive_worker::probe_runner::ProbeRunner;
use overdrive_worker::probe_runner::http_prober::classify_http_status;
use overdrive_worker::vm_driver::VmHostLayout;
use proptest::prelude::*;

fn tcp_descriptor(host: &str) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Tcp { host: host.to_owned(), port: 8443 },
        timeout_seconds: 5,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

fn http_descriptor(host: Option<&str>, port: u16, path: &str) -> ProbeDescriptor {
    ProbeDescriptor {
        idx: ProbeIdx::new(0),
        role: ProbeRole::Startup,
        mechanic: ProbeMechanic::Http {
            path: path.to_owned(),
            port,
            host: host.map(str::to_owned),
        },
        timeout_seconds: 5,
        interval_seconds: 1,
        max_attempts: 30,
        failure_threshold: None,
        success_threshold: None,
        inferred: false,
    }
}

struct CapturingHttpProber {
    outcome: ProbeOutcome,
    urls: parking_lot::Mutex<Vec<String>>,
}

impl CapturingHttpProber {
    const fn new(outcome: ProbeOutcome) -> Self {
        Self { outcome, urls: parking_lot::Mutex::new(Vec::new()) }
    }

    fn urls(&self) -> Vec<String> {
        self.urls.lock().clone()
    }
}

#[async_trait]
impl HttpProber for CapturingHttpProber {
    async fn probe(&self, url: &str, _timeout: Duration) -> Result<ProbeOutcome, ProbeFailure> {
        self.urls.lock().push(url.to_owned());
        Ok(self.outcome.clone())
    }
}

fn allocation_spec(
    alloc: &str,
    driver: DriverPayload,
    workload_addr: Option<Ipv4Addr>,
    probe_descriptors: Vec<ProbeDescriptor>,
) -> AllocationSpec {
    AllocationSpec {
        alloc: AllocationId::new(alloc).expect("valid allocation ID"),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/vm-service/alloc/test")
            .expect("valid SPIFFE ID"),
        driver,
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors,
        transport: workload_addr
            .map(|address| super::support::transport(address, GUEST_NODE_GATEWAY, 0)),
        service_ports: Vec::new(),
    }
}

fn vm_payload() -> DriverPayload {
    DriverPayload::Vm(VmPayload {
        command: "/bin/true".to_owned(),
        args: Vec::new(),
        kernel: PathBuf::from("/kernel"),
        rootfs: PathBuf::from("/rootfs"),
    })
}

async fn shared_vm_driver() -> (VmDriver, Arc<ProbeRunner>) {
    let clock: Arc<dyn Clock> = Arc::new(SimClock::default());
    let observation_store: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-hook-driver").expect("valid node ID"),
        0,
    ));
    let probe_runner = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        observation_store,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    ));
    let layout = VmHostLayout {
        cgroup_root: PathBuf::from("/tmp/svm-hooks-cgroup"),
        run_dir_root: PathBuf::from("/tmp/svm-hooks-run"),
        clone_index_dir: PathBuf::from("/tmp/svm-hooks-index"),
        clone_staging_dir: PathBuf::from("/tmp/svm-hooks-staging"),
        arch: overdrive_core::vm::config::HostArch::X86_64,
        confinement: overdrive_core::vm::config::VmConfinement::confined(
            overdrive_core::vm::config::VmmIdentity {
                uid: 1000,
                gid: overdrive_core::vm::config::Gid::new(994),
                supplementary: Vec::new(),
            },
            1024,
        ),
    };
    let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(clock.clone());
    assert!(wiring.supervisor().open_after_boot());
    let sim = SimVmm::new();
    let beacon = super::support::BeaconHarness::new(&sim);
    let intake = beacon.intake().await;
    let driver = VmDriver::new(
        Arc::new(sim),
        Arc::clone(&clock),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        Arc::clone(&probe_runner),
        wiring.gate(),
        layout,
        intake,
    );
    (driver, probe_runner)
}

async fn yield_for_task_poll() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

async fn probe_result(
    obs: &SimObservationStore,
    alloc: &AllocationId,
) -> overdrive_core::observation::ProbeResultRow {
    for _ in 0..64 {
        let rows =
            obs.list_probe_results_for_alloc(alloc).await.expect("listing probe results succeeds");
        if let Some(row) = rows.into_iter().next() {
            return row;
        }
        tokio::task::yield_now().await;
    }
    panic!("probe result must be recorded after the probe interval");
}

fn dial_runner(
    http: Arc<CapturingHttpProber>,
    clock: Arc<SimClock>,
    observation_store: Arc<SimObservationStore>,
    dial: Arc<SimGuestStreamDial>,
) -> (ProbeRunner, Arc<SimTcpProber>) {
    let tcp = Arc::new(SimTcpProber::new());
    let runner = ProbeRunner::new(
        Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        http as Arc<dyn HttpProber>,
        clock as Arc<dyn Clock>,
        observation_store as Arc<dyn ObservationStore>,
        dial as Arc<dyn GuestStreamDial>,
    );
    (runner, tcp)
}

/// A workload address inside the node guest prefix (`GUEST_NODE_PREFIX`,
/// 100.95.0.0/16) — the only kind a VM allocation is leased (INB-1).
fn guest_addr(third: u8, fourth: u8) -> Ipv4Addr {
    let addr = Ipv4Addr::new(100, 95, third, fourth);
    assert!(GUEST_NODE_PREFIX.contains(&addr) && addr != GUEST_NODE_GATEWAY);
    addr
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-SVM-13 projection property (INB-1) — for every VM workload address
    /// of the guest prefix, the wildcard TCP target projects to that address
    /// and the probe runs as one `DialPurpose::Probe` dial of
    /// `workload_addr:port`; the TCP prober is never called and the
    /// declared descriptor is unchanged.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn vm_wildcard_tcp_target_projects_each_provisioned_guest_addr(
        third in 0_u8..=255,
        fourth in 2_u8..=254,
    ) {
        let guest = guest_addr(third, fourth);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime builds");
        runtime.block_on(async {
            let clock = Arc::new(SimClock::default());
            let obs = Arc::new(SimObservationStore::single_peer(
                NodeId::new("svm-13-property").expect("valid node ID"),
                0,
            ));
            let dial = Arc::new(SimGuestStreamDial::new());
            dial.script_default(SimDialScript::accept());
            let (runner, tcp) = dial_runner(
                Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass)),
                Arc::clone(&clock),
                Arc::clone(&obs),
                Arc::clone(&dial),
            );
            let descriptor = tcp_descriptor("0.0.0.0");
            let spec = allocation_spec("svm-13-property-vm", vm_payload(), Some(guest), vec![descriptor.clone()]);

            let _token = runner.start_alloc(&spec);
            yield_for_task_poll().await;
            clock.tick(Duration::from_secs(1));
            let _row = probe_result(&obs, &spec.alloc).await;

            let dials = dial.dials();
            assert_eq!(dials.len(), 1, "one dial for the one run");
            assert_eq!(dials[0].target, SocketAddrV4::new(guest, 8443));
            assert_eq!(dials[0].purpose, DialPurpose::Probe);
            assert_eq!(tcp.last_probed_host(), "", "the TCP prober is not called for a guest-prefix target");
            assert_eq!(spec.probe_descriptors, vec![descriptor]);
            runner.stop_alloc(&spec.alloc);
        });
    }
}

/// S-SVM-10 (INB-1) — for a VM allocation, omitted HTTP host, wildcard HTTP
/// host and wildcard TCP host each resolve once to the leased
/// `workload_addr` and run as `DialPurpose::Probe` dials of
/// `workload_addr:port` (the HTTP probes over their streams); no prober is
/// called; the persisted descriptors remain unchanged.
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once() {
    let http = Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass));
    let clock = Arc::new(SimClock::default());
    let observation_store = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-10").expect("valid node ID"),
        0,
    ));
    let dial = Arc::new(SimGuestStreamDial::new());
    dial.script_default(SimDialScript::Accept {
        reply: b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
    });
    let (runner, tcp) = dial_runner(
        Arc::clone(&http),
        Arc::clone(&clock),
        Arc::clone(&observation_store),
        Arc::clone(&dial),
    );
    let descriptors = vec![
        http_descriptor(None, 8080, "/healthz"),
        http_descriptor(Some("0.0.0.0"), 8081, "/ready"),
        tcp_descriptor("0.0.0.0"),
    ];
    let workload = guest_addr(0, 10);
    let spec = allocation_spec("svm-10-vm", vm_payload(), Some(workload), descriptors.clone());

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let _row = probe_result(&observation_store, &spec.alloc).await;

    let targets: std::collections::BTreeSet<SocketAddrV4> =
        dial.dials().iter().map(|d| d.target).collect();
    assert_eq!(
        targets,
        [8080, 8081, 8443].into_iter().map(|port| SocketAddrV4::new(workload, port)).collect(),
    );
    assert!(dial.dials().iter().all(|d| d.purpose == DialPurpose::Probe));
    assert!(http.urls().is_empty(), "no HTTP prober call for a guest-prefix target");
    assert_eq!(tcp.last_probed_host(), "", "no TCP prober call for a guest-prefix target");
    assert_eq!(spec.probe_descriptors, descriptors, "declared descriptors stay unchanged");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-11 (INB-1) — a non-wildcard explicit HTTP/TCP host outside the
/// guest prefix keeps the existing prober path, byte for byte, for a VM
/// allocation: no dial is made for it.
/// CONTRACT_SHAPE: unbounded-preservation.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn explicit_network_probe_hosts_are_preserved_for_vm() {
    let name = "vm";
    let driver = vm_payload();
    let workload_addr = Some(guest_addr(0, 11));
    let tcp = Arc::new(SimTcpProber::new());
    let http = Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass));
    let clock = Arc::new(SimClock::default());
    let node_id = format!("svm-11-{name}");
    let observation_store = Arc::new(SimObservationStore::single_peer(
        NodeId::new(&node_id).expect("valid node ID"),
        0,
    ));
    let dial = Arc::new(SimGuestStreamDial::new());
    let runner = ProbeRunner::new(
        Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        Arc::clone(&http) as Arc<dyn HttpProber>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&observation_store) as Arc<dyn ObservationStore>,
        Arc::clone(&dial) as Arc<dyn GuestStreamDial>,
    );
    let descriptors = vec![
        http_descriptor(Some("health.internal"), 8080, "/healthz"),
        tcp_descriptor("tcp.internal"),
    ];
    let spec =
        allocation_spec(&format!("svm-11-{name}"), driver, workload_addr, descriptors.clone());

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let _row = probe_result(&observation_store, &spec.alloc).await;

    assert_eq!(http.urls(), vec!["http://health.internal:8080/healthz"]);
    assert_eq!(tcp.last_probed_host(), "tcp.internal");
    assert_eq!(spec.probe_descriptors, descriptors, "declared descriptors stay unchanged");
    assert_eq!(dial.dial_count(), 0, "no guest dial for a target outside the guest prefix");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-13 (INB-1) — a TCP probe of a VM is healthy iff its dial of
/// `workload_addr:port` returns `Ok`: a guest application that refuses is a
/// `Fail` whose detail is the refusal's `as_str`; neither result authors an
/// allocation Running observation.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn vm_tcp_probe_records_guest_connect_outcome_without_owning_running() {
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-13").expect("valid node ID"),
        0,
    ));
    let dial = Arc::new(SimGuestStreamDial::new());
    let refusal = GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED };
    dial.script_default(SimDialScript::Refuse(refusal));
    let (runner, _tcp) = dial_runner(
        Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass)),
        Arc::clone(&clock),
        Arc::clone(&obs),
        Arc::clone(&dial),
    );
    let descriptor = tcp_descriptor("0.0.0.0");
    let spec = allocation_spec(
        "svm-13-vm",
        vm_payload(),
        Some(guest_addr(0, 42)),
        vec![descriptor.clone()],
    );

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let row = probe_result(&obs, &spec.alloc).await;

    assert_eq!(row.status, ProbeStatus::Fail { last_fail_reason: refusal.as_str().to_owned() });
    assert_eq!(
        obs.alloc_status_snapshot().len(),
        0,
        "a TCP observation must not author an allocation Running observation",
    );
    assert_eq!(spec.probe_descriptors, vec![descriptor], "declared descriptor stays unchanged");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-14 (INB-1) — an HTTP probe of a VM runs over the `GuestStream`: 2xx
/// (including 204) is `Pass`, 3xx/4xx/5xx (including 302 and 503) is `Fail`
/// with the existing status policy; the body is never consumed without
/// bound; no HTTP prober is called.
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn vm_http_probe_preserves_status_policy_and_bounded_body_handling() {
    for status in [204_u16, 302, 404, 503] {
        let clock = Arc::new(SimClock::default());
        let observation_store = Arc::new(SimObservationStore::single_peer(
            NodeId::new(&format!("svm-14-{status}")).expect("valid node ID"),
            0,
        ));
        let dial = Arc::new(SimGuestStreamDial::new());
        // An oversized body after the status line: the probe decides from
        // the status and does not read the body to its end.
        let mut reply =
            format!("HTTP/1.1 {status} Status\r\nContent-Length: 1048576\r\n\r\n").into_bytes();
        reply.extend(std::iter::repeat_n(b'x', 64 * 1024));
        dial.script_default(SimDialScript::AcceptOpen { reply });
        let http = Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass));
        let (runner, _tcp) = dial_runner(
            Arc::clone(&http),
            Arc::clone(&clock),
            Arc::clone(&observation_store),
            Arc::clone(&dial),
        );
        let descriptor = http_descriptor(None, 8080, "/healthz");
        let spec = allocation_spec(
            &format!("svm-14-{status}"),
            vm_payload(),
            Some(guest_addr(0, 14)),
            vec![descriptor.clone()],
        );

        let _token = runner.start_alloc(&spec);
        yield_for_task_poll().await;
        clock.tick(Duration::from_secs(1));
        let row = probe_result(&observation_store, &spec.alloc).await;

        let expected = match classify_http_status(status) {
            ProbeOutcome::Pass => ProbeStatus::Pass,
            ProbeOutcome::Fail { reason } => ProbeStatus::Fail { last_fail_reason: reason },
        };
        assert_eq!(row.status, expected, "HTTP {status} over the guest stream");
        assert!(http.urls().is_empty(), "no HTTP prober call for a guest-prefix target");
        assert_eq!(
            observation_store.alloc_status_snapshot().len(),
            0,
            "HTTP observations must not author allocation Running state",
        );
        assert_eq!(spec.probe_descriptors, vec![descriptor]);
        runner.stop_alloc(&spec.alloc);
    }
}

/// S-SVM-15 — `VmDriver` receives the one shared trusted runner as its exact
/// fifth constructor argument and delegates Running, Stable, and terminal to
/// the same existing hooks as `ExecDriver`; no new Driver method exists.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner() {
    let (driver, runner) = shared_vm_driver().await;
    let mut readiness = tcp_descriptor("0.0.0.0");
    readiness.role = ProbeRole::Readiness;
    let mut liveness = tcp_descriptor("0.0.0.0");
    liveness.role = ProbeRole::Liveness;
    let spec = allocation_spec(
        "svm-15-hooks",
        vm_payload(),
        Some(guest_addr(0, 15)),
        vec![tcp_descriptor("0.0.0.0"), readiness, liveness],
    );

    driver.on_alloc_running(&spec);
    assert_eq!(runner.active_alloc_count(), 1, "Running registers one allocation supervisor");
    assert!(runner.is_role_live(&spec.alloc, ProbeRole::Startup));
    assert!(runner.is_role_live(&spec.alloc, ProbeRole::Readiness));
    assert!(runner.is_role_live(&spec.alloc, ProbeRole::Liveness));

    driver.on_alloc_stable(&spec.alloc);
    assert!(!runner.is_role_live(&spec.alloc, ProbeRole::Startup));
    assert!(runner.is_role_live(&spec.alloc, ProbeRole::Readiness));
    assert!(runner.is_role_live(&spec.alloc, ProbeRole::Liveness));

    driver.on_alloc_terminal(&spec.alloc);
    assert_eq!(runner.active_alloc_count(), 0, "terminal stops the allocation supervisor");
}

/// S-SVM-16 — registering the same allocation again after restart is
/// idempotent: the effective target is re-derived from the current full
/// `AllocationSpec`, while exactly one supervisor/task set remains live.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks() {
    let (driver, runner) = shared_vm_driver().await;
    let initial = allocation_spec(
        "svm-16-restart",
        vm_payload(),
        Some(guest_addr(0, 16)),
        vec![tcp_descriptor("0.0.0.0")],
    );
    let current = allocation_spec(
        "svm-16-restart",
        vm_payload(),
        Some(guest_addr(0, 17)),
        vec![tcp_descriptor("0.0.0.0")],
    );

    driver.on_alloc_running(&initial);
    assert_eq!(runner.active_alloc_count(), 1);
    driver.on_alloc_terminal(&initial.alloc);
    assert_eq!(runner.active_alloc_count(), 0);

    driver.on_alloc_running(&current);
    driver.on_alloc_running(&current);
    assert_eq!(runner.active_alloc_count(), 1, "restart keeps exactly one live task set");
    assert!(runner.is_role_live(&current.alloc, ProbeRole::Startup));
    driver.on_alloc_terminal(&current.alloc);
}
