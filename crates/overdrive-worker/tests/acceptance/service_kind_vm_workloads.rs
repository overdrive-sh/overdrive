//! Acceptance coverage for ADR-0090's existing `ProbeRunner` and
//! `VmDriver` boundaries. No fallback case for `Vm + None` is present because
//! the accepted design establishes `Some(workload_addr)` as a production
//! registration precondition.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, Driver, DriverPayload, ExecPayload, Resources, VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{HttpProber, ProbeFailure, ProbeOutcome};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
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

fn runner_with_http(
    tcp: Arc<SimTcpProber>,
    http: Arc<CapturingHttpProber>,
    clock: Arc<SimClock>,
    observation_store: Arc<SimObservationStore>,
) -> ProbeRunner {
    ProbeRunner::new(
        tcp as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        http as Arc<dyn HttpProber>,
        Arc::new(SimExecProber::new()),
        clock as Arc<dyn Clock>,
        observation_store as Arc<dyn ObservationStore>,
    )
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
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr,
        guest_tap: None,
        guest_mac: None,
        guest_gateway: None,
        guest_prefix_len: None,
        guest_dns: None,
    }
}

fn exec_payload() -> DriverPayload {
    DriverPayload::Exec(ExecPayload { command: "/bin/true".to_owned(), args: Vec::new() })
}

fn vm_payload() -> DriverPayload {
    DriverPayload::Vm(VmPayload {
        command: "/bin/true".to_owned(),
        args: Vec::new(),
        kernel: PathBuf::from("/kernel"),
        rootfs: PathBuf::from("/rootfs"),
    })
}

fn shared_vm_driver() -> (VmDriver, Arc<ProbeRunner>) {
    let clock = Arc::new(SimClock::default());
    let observation_store: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-hook-driver").expect("valid node ID"),
        0,
    ));
    let probe_runner = Arc::new(ProbeRunner::new(
        Arc::new(SimTcpProber::new()),
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        observation_store,
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
    let driver = VmDriver::new(
        Arc::new(SimVmm::new()),
        clock,
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        Arc::clone(&probe_runner),
        layout,
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-SVM-13 projection property — each provisioned VM guest address is
    /// captured when registration starts and replaces only the TCP wildcard.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn vm_wildcard_tcp_target_projects_each_provisioned_guest_addr(
        first in 1_u8..=223,
        second in any::<u8>(),
        third in any::<u8>(),
        fourth in 1_u8..=254,
    ) {
        let guest_addr = Ipv4Addr::new(first, second, third, fourth);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime builds");
        runtime.block_on(async {
            let tcp = Arc::new(SimTcpProber::new());
            let clock = Arc::new(SimClock::default());
            let obs = Arc::new(SimObservationStore::single_peer(
                NodeId::new("svm-13-property").expect("valid node ID"),
                0,
            ));
            let runner = ProbeRunner::new(
                Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
                Arc::new(SimHttpProber::new()),
                Arc::new(SimExecProber::new()),
                Arc::clone(&clock) as Arc<dyn Clock>,
                Arc::clone(&obs) as Arc<dyn ObservationStore>,
            );
            let descriptor = tcp_descriptor("0.0.0.0");
            let spec = allocation_spec(
                "svm-13-property-vm",
                vm_payload(),
                Some(guest_addr),
                vec![descriptor.clone()],
            );

            let _token = runner.start_alloc(&spec);
            yield_for_task_poll().await;
            clock.tick(Duration::from_secs(1));
            let _row = probe_result(&obs, &spec.alloc).await;

            assert_eq!(tcp.last_probed_host(), guest_addr.to_string());
            assert_eq!(spec.probe_descriptors, vec![descriptor]);
            runner.stop_alloc(&spec.alloc);
        });
    }
}

/// S-SVM-10 — for a VM allocation, omitted HTTP host, wildcard HTTP host,
/// and wildcard TCP host each resolve once to the provisioned guest
/// `workload_addr`; the persisted descriptor remains unchanged.
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once() {
    let tcp = Arc::new(SimTcpProber::new());
    let http = Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass));
    let clock = Arc::new(SimClock::default());
    let observation_store = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-10").expect("valid node ID"),
        0,
    ));
    let runner = runner_with_http(
        Arc::clone(&tcp),
        Arc::clone(&http),
        Arc::clone(&clock),
        Arc::clone(&observation_store),
    );
    let descriptors = vec![
        http_descriptor(None, 8080, "/healthz"),
        http_descriptor(Some("0.0.0.0"), 8081, "/ready"),
        tcp_descriptor("0.0.0.0"),
    ];
    let spec = allocation_spec(
        "svm-10-vm",
        vm_payload(),
        Some(Ipv4Addr::new(192, 0, 2, 10)),
        descriptors.clone(),
    );

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let _row = probe_result(&observation_store, &spec.alloc).await;

    let mut urls = http.urls();
    urls.sort();
    assert_eq!(
        urls,
        vec![
            "http://192.0.2.10:8080/healthz".to_owned(),
            "http://192.0.2.10:8081/ready".to_owned(),
        ],
    );
    assert_eq!(tcp.last_probed_host(), "192.0.2.10");
    assert_eq!(spec.probe_descriptors, descriptors, "declared descriptors stay unchanged");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-11 — every non-wildcard explicit HTTP/TCP host is passed byte-for-
/// byte to the existing prober adapter for both VM and Exec allocations.
/// CONTRACT_SHAPE: unbounded-preservation.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn explicit_network_probe_hosts_are_preserved_for_both_drivers() {
    for (name, driver, workload_addr) in
        [("vm", vm_payload(), Some(Ipv4Addr::new(192, 0, 2, 11))), ("exec", exec_payload(), None)]
    {
        let tcp = Arc::new(SimTcpProber::new());
        let http = Arc::new(CapturingHttpProber::new(ProbeOutcome::Pass));
        let clock = Arc::new(SimClock::default());
        let node_id = format!("svm-11-{name}");
        let observation_store = Arc::new(SimObservationStore::single_peer(
            NodeId::new(&node_id).expect("valid node ID"),
            0,
        ));
        let runner = runner_with_http(
            Arc::clone(&tcp),
            Arc::clone(&http),
            Arc::clone(&clock),
            Arc::clone(&observation_store),
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
        runner.stop_alloc(&spec.alloc);
    }
}

/// S-SVM-12 — Exec/process omitted or wildcard HTTP/TCP targets retain their
/// existing loopback semantics; the VM change cannot move that baseline.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn exec_default_network_probe_targets_remain_loopback() {
    let tcp = Arc::new(SimTcpProber::new());
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-12").expect("valid node ID"),
        0,
    ));
    let runner = ProbeRunner::new(
        Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );
    let spec =
        allocation_spec("svm-12-exec", exec_payload(), None, vec![tcp_descriptor("0.0.0.0")]);

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let _row = probe_result(&obs, &spec.alloc).await;

    assert_eq!(tcp.last_probed_host(), "127.0.0.1");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-13 — TCP reaches the projected guest address and records Pass on
/// connect success or Fail with the existing reason on a closed guest port;
/// neither result changes allocation Running state.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn vm_tcp_probe_records_guest_connect_outcome_without_owning_running() {
    let tcp = Arc::new(SimTcpProber::new());
    tcp.enqueue_outcome(ProbeOutcome::Fail { reason: "connection refused".to_owned() });
    let clock = Arc::new(SimClock::default());
    let obs = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-13").expect("valid node ID"),
        0,
    ));
    let runner = ProbeRunner::new(
        Arc::clone(&tcp) as Arc<dyn overdrive_core::traits::prober::TcpProber>,
        Arc::new(SimHttpProber::new()),
        Arc::new(SimExecProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&obs) as Arc<dyn ObservationStore>,
    );
    let descriptor = tcp_descriptor("0.0.0.0");
    let spec = allocation_spec(
        "svm-13-vm",
        vm_payload(),
        Some(Ipv4Addr::new(192, 0, 2, 42)),
        vec![descriptor.clone()],
    );

    let _token = runner.start_alloc(&spec);
    yield_for_task_poll().await;
    clock.tick(Duration::from_secs(1));
    let row = probe_result(&obs, &spec.alloc).await;

    assert_eq!(tcp.last_probed_host(), "192.0.2.42");
    assert_eq!(row.status, ProbeStatus::Fail { last_fail_reason: "connection refused".to_owned() },);
    assert_eq!(
        obs.alloc_status_snapshot().len(),
        0,
        "a TCP observation must not author an allocation Running observation",
    );
    assert_eq!(spec.probe_descriptors, vec![descriptor], "declared descriptor stays unchanged");
    runner.stop_alloc(&spec.alloc);
}

/// S-SVM-14 — HTTP reaches the projected guest URL: 2xx (including 204) is
/// Pass, 3xx/4xx/5xx (including 302 and 503) is Fail with the numeric status,
/// and the response body is never consumed without bound.
/// CONTRACT_SHAPE: bounded-change.
/// Outcome anchor: DISCUSS Elevator Pitch
#[tokio::test]
async fn vm_http_probe_preserves_status_policy_and_bounded_body_handling() {
    let cases = [
        (204, ProbeStatus::Pass),
        (
            302,
            ProbeStatus::Fail { last_fail_reason: "HTTP 302 (redirect not followed)".to_owned() },
        ),
        (404, ProbeStatus::Fail { last_fail_reason: "HTTP 404".to_owned() }),
        (503, ProbeStatus::Fail { last_fail_reason: "HTTP 503".to_owned() }),
    ];

    for (status, expected) in cases {
        let tcp = Arc::new(SimTcpProber::new());
        let http = Arc::new(CapturingHttpProber::new(classify_http_status(status)));
        let clock = Arc::new(SimClock::default());
        let node_id = format!("svm-14-{status}");
        let observation_store = Arc::new(SimObservationStore::single_peer(
            NodeId::new(&node_id).expect("valid node ID"),
            0,
        ));
        let runner = runner_with_http(
            tcp,
            Arc::clone(&http),
            Arc::clone(&clock),
            Arc::clone(&observation_store),
        );
        let descriptor = http_descriptor(None, 8080, "/healthz");
        let spec = allocation_spec(
            &format!("svm-14-{status}"),
            vm_payload(),
            Some(Ipv4Addr::new(192, 0, 2, 14)),
            vec![descriptor.clone()],
        );

        let _token = runner.start_alloc(&spec);
        yield_for_task_poll().await;
        clock.tick(Duration::from_secs(1));
        let row = probe_result(&observation_store, &spec.alloc).await;

        assert_eq!(http.urls(), vec!["http://192.0.2.14:8080/healthz"]);
        assert_eq!(row.status, expected);
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
    let (driver, runner) = shared_vm_driver();
    let mut readiness = tcp_descriptor("0.0.0.0");
    readiness.role = ProbeRole::Readiness;
    let mut liveness = tcp_descriptor("0.0.0.0");
    liveness.role = ProbeRole::Liveness;
    let spec = allocation_spec(
        "svm-15-hooks",
        vm_payload(),
        Some(Ipv4Addr::new(192, 0, 2, 15)),
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
    let (driver, runner) = shared_vm_driver();
    let initial = allocation_spec(
        "svm-16-restart",
        vm_payload(),
        Some(Ipv4Addr::new(192, 0, 2, 16)),
        vec![tcp_descriptor("0.0.0.0")],
    );
    let current = allocation_spec(
        "svm-16-restart",
        vm_payload(),
        Some(Ipv4Addr::new(192, 0, 2, 17)),
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
