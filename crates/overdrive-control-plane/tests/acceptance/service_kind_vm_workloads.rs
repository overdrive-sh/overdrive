//! Acceptance coverage for the production composition boundary.

// Contract-shape declarations intentionally use the repository-mandated token.
#![allow(clippy::doc_markdown, clippy::expect_used, clippy::missing_panics_doc)]

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use overdrive_control_plane::probe_runner_boot::compose_and_probe_runner_gate;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::guest_transport::{
    GUEST_CID_BASE, GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT, GuestCid, GuestFlowPorts,
    GuestTransportAssignment, LaunchToken,
};
use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::driver::{AllocationSpec, Driver, DriverPayload, Resources, VmPayload};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_core::traits::prober::{ProbeFailure, ProbeOutcome, TcpProber};
use overdrive_core::vm::config::{Gid, HostArch, VmConfinement, VmmIdentity};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::probers::SimHttpProber;
use overdrive_sim::{SimCgroupAccounting, SimCgroupFs};
use overdrive_worker::{VmDriver, VmHostLayout};

struct CountingTcpProber {
    calls: AtomicUsize,
}

#[async_trait]
impl TcpProber for CountingTcpProber {
    async fn probe(
        &self,
        _host: &str,
        _port: u16,
        _timeout: Duration,
    ) -> Result<ProbeOutcome, ProbeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ProbeOutcome::Pass)
    }
}

fn allocation_spec(
    alloc: &str,
    driver: DriverPayload,
    probes: Vec<ProbeDescriptor>,
) -> AllocationSpec {
    AllocationSpec {
        alloc: AllocationId::new(alloc).expect("valid allocation ID"),
        identity: SpiffeId::new("spiffe://overdrive.local/workload/vm-service/alloc/composition")
            .expect("valid SPIFFE ID"),
        driver,
        resources: Resources { cpu_milli: 100, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: probes,
        transport: Some(GuestTransportAssignment::new(
            Ipv4Addr::new(100, 95, 0, 22),
            Ipv4Addr::new(100, 95, 0, 1),
            GuestCid::new(GUEST_CID_BASE + 22).expect("an in-range guest CID"),
            LaunchToken::from_bytes([0x22; 16]).expect("a non-zero launch token"),
            GuestFlowPorts::new(GUEST_FLOW_STREAM_PORT, GUEST_FLOW_CONTROL_PORT)
                .expect("the default guest-flow listener ports"),
        )),
        service_ports: Vec::new(),
    }
}

/// S-SVM-22 — one `overdrive serve` boot performs the existing ProbeRunner
/// Earned-Trust probe exactly once, retains that returned `Arc`, gives clones
/// to the sole production VM driver, and preserves all existing VM-capability
/// success/refusal outcomes.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 2 (GuestCid / LaunchToken newtypes): the fixture's allocation spec builds a GuestCid"]
async fn one_server_boot_shares_exactly_one_trusted_probe_runner_with_vm_driver() {
    let tcp = Arc::new(CountingTcpProber { calls: AtomicUsize::new(0) });
    let clock = Arc::new(SimClock::default());
    let observation_store: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("svm-22-composition").expect("valid node ID"),
        0,
    ));
    let runner = compose_and_probe_runner_gate(
        Arc::clone(&tcp) as Arc<dyn TcpProber>,
        Arc::new(SimHttpProber::new()),
        Arc::clone(&clock) as Arc<dyn Clock>,
        observation_store,
        Arc::new(overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial::new()),
    )
    .await
    .expect("one trusted ProbeRunner is composed at boot");
    assert_eq!(tcp.calls.load(Ordering::SeqCst), 1, "the trust probe runs once per boot");

    let wiring = overdrive_core::guest_network::GuestNetworkExecWiring::new(clock.clone());
    assert!(wiring.supervisor().open_after_boot());
    let sim_vmm = overdrive_sim::SimVmm::new();
    let beacon = overdrive_worker::BeaconIntake::bind(
        Arc::new(overdrive_sim::adapters::beacon_io::SimBeaconIo::new(sim_vmm.clone())),
        overdrive_core::guest_transport::VsockListenPort::KernelAssigned,
    )
    .await
    .expect("bind the node beacon intake");
    let vm_driver = VmDriver::new(
        Arc::new(sim_vmm),
        clock.clone(),
        Arc::new(SimCgroupFs::new()),
        Arc::new(SimCgroupAccounting::new()),
        Arc::clone(&runner),
        wiring.gate(),
        VmHostLayout {
            cgroup_root: PathBuf::from("/tmp/svm-22-vm-cgroup"),
            run_dir_root: PathBuf::from("/tmp/svm-22-vm-run"),
            clone_index_dir: PathBuf::from("/tmp/svm-22-vm-index"),
            clone_staging_dir: PathBuf::from("/tmp/svm-22-vm-staging"),
            arch: HostArch::X86_64,
            confinement: VmConfinement::confined(
                VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: Vec::new() },
                1024,
            ),
        },
        beacon,
    );
    let vm = allocation_spec(
        "svm-22-vm",
        DriverPayload::Vm(VmPayload {
            command: "/sbin/init".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }),
        vec![ProbeDescriptor {
            idx: ProbeIdx::new(0),
            role: ProbeRole::Startup,
            mechanic: ProbeMechanic::Tcp { host: "0.0.0.0".to_owned(), port: 8443 },
            timeout_seconds: 5,
            interval_seconds: 1,
            max_attempts: 30,
            failure_threshold: None,
            success_threshold: None,
            inferred: false,
        }],
    );

    vm_driver.on_alloc_running(&vm);
    assert_eq!(runner.active_alloc_count(), 1, "the VM driver uses the trusted runner");
    assert_eq!(tcp.calls.load(Ordering::SeqCst), 1, "sharing does not repeat the trust probe");
    vm_driver.on_alloc_terminal(&vm.alloc);
    assert_eq!(runner.active_alloc_count(), 0);
}
