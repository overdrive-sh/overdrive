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

use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::id::{AllocationId, NodeId, SpiffeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole, ProbeStatus};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{
    AllocationSpec, DriverPayload, ExecPayload, Resources, VmPayload,
};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::ProbeOutcome;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::{SimExecProber, SimHttpProber, SimTcpProber};
use overdrive_worker::probe_runner::ProbeRunner;
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
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_default_and_wildcard_network_probe_targets_resolve_to_workload_addr_once() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-10 / VM default target projection)");
}

/// S-SVM-11 — every non-wildcard explicit HTTP/TCP host is passed byte-for-
/// byte to the existing prober adapter for both VM and Exec allocations.
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[should_panic(expected = "RED scaffold")]
fn explicit_network_probe_hosts_are_preserved_for_both_drivers() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-11 / explicit host preservation)");
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
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_http_probe_preserves_status_policy_and_bounded_body_handling() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-14 / VM HTTP status contract)");
}

/// S-SVM-15 — `VmDriver` receives the one shared trusted runner as its exact
/// fifth constructor argument and delegates Running, Stable, and terminal to
/// the same existing hooks as `ExecDriver`; no new Driver method exists.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-15 / VmDriver hook delegation)");
}

/// S-SVM-16 — registering the same allocation again after restart is
/// idempotent: the effective target is re-derived from the current full
/// `AllocationSpec`, while exactly one supervisor/task set remains live.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks() {
    panic!("Not yet implemented -- RED scaffold (S-SVM-16 / restart re-registration)");
}
