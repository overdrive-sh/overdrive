//! service-allocation-readiness-and-ending, D13 item 10 / DG-6 / F-5: the
//! production composition (`run_server`) shares ONE `GuestListenerBoard`
//! between the VM driver (the writer: its beacon reader records the guest's
//! `LISTEN` lines) and the probe runner (the reader: its guest-listener
//! startup task waits for the declared set).
//!
//! The DISTILL scaffolds create one board per consumer, which compiles and is
//! inert, so the type system cannot catch a missing share. This test observes
//! the share behaviourally, through the production composition and nothing
//! else: a `LISTEN` line arriving on the node's beacon intake (the driver's
//! driven port, `SimBeaconIo`) must complete the probe task's set and produce
//! the Service's startup `Pass` row, read through the public allocation
//! snapshot (`GET /v1/allocs`). With two boards the line lands on the
//! driver's board, the task waits on its own empty one, and no `Pass` row ever
//! appears.
//!
//! **Production composition, Sim adapters at driven ports only.** `run_server`
//! over the host bundle with the VMM and the beacon replaced by `SimVmm` and
//! `SimBeaconIo` (the `VmAdapters` seam of ADR-0219) and the mTLS intercept by
//! `SimMtlsIntercept`, exactly as `shared_guest_network_startup.rs` boots. The
//! guest's `READY` carries the launch token the production guest-network
//! owner leased, read through `ServerHandle::guest_flow_observation()`. The
//! workload is submitted through `POST /v1/workloads`, the handler `overdrive
//! deploy` calls. Nothing is installed that `run_server` does not install; the
//! board is never touched by the test.
//!
//! Lane: `integration-tests`, native metal (`cargo xtask metal run --`), the
//! `host-kernel-shared` group like every binary-wide `run_server` boot.
//!
//! Traceability: DG-6 (ruled 2026-11-05), D13 item 10 composition paragraph,
//! Gate G-10; the design pins the shared board as a DELIVER obligation checked
//! at the composition root.

#![allow(clippy::doc_markdown, reason = "per-test CONTRACT_SHAPE is an exact protocol marker")]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, ProbeStatusJson, SubmitWorkloadRequest,
    SubmitWorkloadResponse,
};
use overdrive_control_plane::node_adapters::VmCapability;
use overdrive_control_plane::{ServerHandle, run_server};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_transport::GuestCid;
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_core::vm::beacon::BeaconConnKey;
use overdrive_core::vm::config::VsockPort;
use overdrive_sim::adapters::{SimBeaconIo, SimMtlsIntercept, SimVmm};
use overdrive_testing::vm_fixture::default_staging_root;
use tempfile::TempDir;

/// Bound on the Service allocation reaching `Running`.
const RUNNING_BOUND: Duration = Duration::from_secs(60);
/// Bound on a startup row appearing after the completing `LISTEN` line.
const ROW_BOUND: Duration = Duration::from_secs(20);
/// How long "no row yet" is observed before a partial or irrelevant report is
/// judged to have changed nothing (the startup deadline is 60 s, so a `Fail`
/// cannot land inside it).
const QUIET_WINDOW: Duration = Duration::from_secs(4);

/// The declared TCP listener of the Service.
const TCP_PORT: u16 = 8080;
/// The declared UDP listener of the Service.
const UDP_PORT: u16 = 5353;
/// A port the Service does not declare.
const UNDECLARED_PORT: u16 = 9999;

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("CA PEM"))
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

fn read_ca(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    let text = std::fs::read_to_string(operator_config_dir.join(".overdrive/config"))
        .expect("the trust triple run_server wrote");
    let doc: toml::Value = toml::from_str(&text).expect("trust triple TOML");
    let encoded = doc["contexts"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["name"].as_str() == Some("local")))
        .and_then(|c| c["ca"].as_str())
        .expect("a local context with a CA");
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(encoded).expect("base64"))
        .expect("PEM is UTF-8")
}

/// One production node over `SimVmm` and `SimBeaconIo`.
struct Node {
    handle: ServerHandle,
    tmp: TempDir,
    client: reqwest::Client,
    base: String,
    vmm: SimVmm,
    beacon: Arc<SimBeaconIo>,
}

impl Node {
    async fn boot() -> Self {
        // The node's data directory sits on the staging root: the VM driver
        // FICLONEs each rootfs into it, and a clone is intra-filesystem.
        let tmp = tempfile::Builder::new()
            .prefix("guest-listener-board-")
            .tempdir_in(default_staging_root())
            .expect("server tempdir on the reflink-capable staging root");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("operator config dir");
        let config = super::node_adapters::test_config(data_dir, operator_config_dir.clone());

        let vmm = SimVmm::new();
        let beacon = Arc::new(SimBeaconIo::new(vmm.clone()));
        let mut adapters = super::node_adapters::node_adapters(&config);
        adapters.boot.mtls.intercept = Arc::new(SimMtlsIntercept::new());
        // A SimVmm process is not a host pid: the cgroup ports are simulated too.
        adapters.cgroup_fs = Arc::new(overdrive_sim::SimCgroupFs::new());
        adapters.vm.cgroup_accounting = Arc::new(overdrive_sim::SimCgroupAccounting::new());
        adapters.vm.vmm = Arc::new(vmm.clone());
        adapters.vm.beacon = Arc::clone(&beacon) as _;
        adapters.vm.capability = VmCapability::Required;

        let handle = run_server(config, adapters).await.expect("serve boots on metal");
        let bound = handle.local_addr().await.expect("the API listener is bound");
        Self {
            handle,
            tmp,
            client: client_trusting(&read_ca(&operator_config_dir)),
            base: format!("https://localhost:{}", bound.port()),
            vmm,
            beacon,
        }
    }

    /// Submits a Service VM declaring a TCP and a UDP listener with the
    /// guest-listener startup gate, through `POST /v1/workloads`.
    async fn deploy_guest_listener_service(&self, id: &str) -> String {
        let kernel = self.tmp.path().join("kernel.img");
        let rootfs = self.tmp.path().join("rootfs.ext4");
        // SimVmm boots nothing, but the driver validates the image format first:
        // the x86 boot-protocol magic (0xAA55 at 510, "HdrS" at 514) is enough.
        let mut image = vec![0_u8; 4096];
        image[510..512].copy_from_slice(&[0x55, 0xAA]);
        image[514..518].copy_from_slice(b"HdrS");
        std::fs::write(&kernel, image).expect("kernel stand-in");
        std::fs::write(&rootfs, vec![0_u8; 1 << 20]).expect("rootfs stand-in");
        let spec = ServiceSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/sbin/workload".to_owned(),
                args: Vec::new(),
                kernel: kernel.display().to_string(),
                rootfs: rootfs.display().to_string(),
            }),
            listeners: vec![
                ListenerInput { port: TCP_PORT, protocol: "tcp".to_owned() },
                ListenerInput { port: UDP_PORT, protocol: "udp".to_owned() },
            ],
            startup_probes: vec![ProbeDescriptor {
                idx: ProbeIdx::new(0),
                role: ProbeRole::Startup,
                mechanic: ProbeMechanic::GuestListeners,
                timeout_seconds: 5,
                interval_seconds: 2,
                max_attempts: 30,
                failure_threshold: None,
                success_threshold: None,
                inferred: false,
            }],
            readiness_probes: Vec::new(),
            liveness_probes: Vec::new(),
        };
        let resp = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /v1/workloads {id}: {e}"));
        assert!(resp.status().is_success(), "deploy service {id}: HTTP {}", resp.status());
        resp.json::<SubmitWorkloadResponse>().await.expect("submit response").workload_id
    }

    async fn snapshot(&self, id: &str) -> AllocStatusResponse {
        self.client
            .get(format!("{}/v1/allocs?job={id}", self.base))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET /v1/allocs {id}: {e}"))
            .json::<AllocStatusResponse>()
            .await
            .expect("alloc status response")
    }

    /// The leased CID and launch token of the workload's allocation, once
    /// the VMM holds its device: what the guest read from its command line.
    async fn guest_identity(&self, id: &str) -> (GuestCid, String, u32) {
        let deadline = tokio::time::Instant::now() + RUNNING_BOUND;
        loop {
            if let Some(observation) = self.handle.guest_flow_observation().await {
                let holders = self.vmm.vsock_device_holders();
                if let Some((cid, entry)) = observation.cids.iter().next()
                    && let Some(pid) = holders.get(cid)
                {
                    return (*cid, entry.launch.to_string(), *pid);
                }
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no launch took a CID lease and a VMM device within {RUNNING_BOUND:?}; \
                 observation: {:#?}; vsock holders: {:?}; rows: {:?}",
                self.handle.guest_flow_observation().await.map(|o| o.cids),
                self.vmm.vsock_device_holders(),
                self.snapshot(id)
                    .await
                    .rows
                    .iter()
                    .map(|r| (r.state, r.reason.clone()))
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The startup rows of the workload's allocation as `(alloc, status)`.
    async fn startup_rows(&self, id: &str) -> Vec<ProbeStatusJson> {
        self.snapshot(id)
            .await
            .probe_results
            .into_iter()
            .filter(|row| row.role == ProbeRole::Startup.as_str() && row.probe_idx == 0)
            .map(|row| row.status)
            .collect()
    }

    async fn running(&self, id: &str) -> bool {
        self.snapshot(id).await.rows.iter().any(|row| row.state == AllocStateWire::Running)
    }
}

/// A `LISTEN` line arriving on the driver's beacon intake reaches the probe
/// runner's guest-listener task: the two share one board.
///
/// Boundary scenario G-10 (3) and the DG-6 composition obligation. The
/// stimulus is the guest's own channel (the beacon line); the oracle is the
/// production-written startup row. A report for an undeclared port and a
/// partial set change nothing; the report that completes the declared set
/// writes the one `Pass` row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 host shared board, one GuestListenerBoard shared by VmDriver and ProbeRunner"]
async fn a_listen_line_on_the_drivers_beacon_completes_the_probe_tasks_declared_set() {
    let node = Node::boot().await;
    let id = node.deploy_guest_listener_service("shared-board").await;

    // The guest boots: READY with the leased launch token, on the node's beacon.
    let (cid, launch, pid) = node.guest_identity(&id).await;
    let beacon_port = node.handle.vsock_ports().beacon.expect("a Vm driver composed a beacon port");
    let key = BeaconConnKey { peer_cid: cid, peer_port: VsockPort::new(50_000) };
    node.beacon
        .queue_connection(
            key,
            pid,
            &[format!("READY pid=1 port={beacon_port} launch={launch}").as_str()],
        )
        .expect("the guest's beacon connection queues");

    let deadline = tokio::time::Instant::now() + RUNNING_BOUND;
    while !node.running(&id).await {
        assert!(tokio::time::Instant::now() < deadline, "the allocation reaches Running");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(node.startup_rows(&id).await.is_empty(), "no startup row exists before any report");

    // An undeclared port, then a partial declared set: no row.
    for line in [format!("LISTEN tcp {UNDECLARED_PORT}"), format!("LISTEN tcp {TCP_PORT}")] {
        assert!(node.beacon.guest_write(key, &line), "the guest writes {line}");
        tokio::time::sleep(QUIET_WINDOW).await;
        assert!(
            node.startup_rows(&id).await.is_empty(),
            "after `{line}` the declared set is not complete, so no startup row is written"
        );
    }

    // The report that completes the declared set: one Pass row.
    assert!(node.beacon.guest_write(key, &format!("LISTEN udp {UDP_PORT}")));
    let deadline = tokio::time::Instant::now() + ROW_BOUND;
    loop {
        let rows = node.startup_rows(&id).await;
        if rows == vec![ProbeStatusJson::Pass] {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the completing report writes the startup Pass row within {ROW_BOUND:?}; rows: {rows:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    node.handle.shutdown(Duration::from_secs(10)).await.expect("clean shutdown");
}
