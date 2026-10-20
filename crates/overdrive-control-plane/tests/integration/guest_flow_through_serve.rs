//! D-17 (GH #295; feature-delta DISTILL handoff (6), § *Owner and
//! provisioner*, `ServerHandle::guest_flow_observation`): the through-`serve`
//! halves of R5-57 and R5-41, read from the running node's guest-flow owner.
//!
//! **Production composition, no test wiring.** Each cell calls
//! `run_server(config, adapters)` with `NodeAdapters::host(&config,
//! RealCgroupFs, SimKek)` — the bundle `overdrive serve` builds, the KEK the
//! only replacement (C1-AMEND: the kernel keyring refuses a cold boot) — and
//! kernel-assigned vsock listener ports, holds the returned `ServerHandle`,
//! and submits VM workloads through the server's public HTTPS API (`POST
//! /v1/workloads`, the handler `overdrive deploy` calls). Nothing here
//! installs a rule, binds a listener or supplies an address `run_server`
//! does not. The owner is read only through
//! `ServerHandle::guest_flow_observation()` (`integration-tests`); the CLI's
//! `ServeHandle` gains no pass-through.
//!
//! **Guests.** Guest-kit microVMs (T-1): the kit's kernel, the production
//! `overdrive-init` and `ovd_fwd.ko`, and the ordinary guest application
//! `overdrive-guest-peer`, whose `hold` subcommand opens one plain TCP
//! connection to a remote peer and keeps it flowing both ways.
//!
//! **Lane.** `integration-tests,kvm-tests`, native x86_64 metal: `cargo
//! xtask metal run -- cargo xtask kmod-build`, `… kmod-load`, `… cargo xtask
//! guest-kit-prepare`, then `cargo xtask metal run -- cargo nextest run -p
//! overdrive-control-plane --features integration-tests,kvm-tests
//! --run-ignored only -E 'test(guest_flow_through_serve)'`. The binary is a
//! whole member of `host-kernel-shared`.
//!
//! **Not here (routed).** The probe-runner and leg-S dial share of R5-57 and
//! the through-`serve` half of R5-63 need a guest that serves an inbound
//! TCP port the probe runner and leg S dial; the guest application has no
//! listening subcommand (DISTILL upstream issues, U-D17-DIAL).

#![allow(clippy::doc_markdown)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, AllocStatusRowBody, SubmitWorkloadRequest,
    SubmitWorkloadResponse,
};
use overdrive_control_plane::guest_network::{
    DialPhase, ForwarderInventoryRead, GuestFlowPhase, GuestOwnerObservation,
};
use overdrive_control_plane::{NodeAdapters, ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::VmRunDir;
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_testing::guest_app::{FLOWING_LINE, GUEST_PEER_GUEST_PATH, RemotePeer, subcommand};
use overdrive_testing::guest_kit::{GuestImage, GuestImageSpec, GuestKit};
use overdrive_testing::vm_fixture::default_staging_root;
use tempfile::TempDir;

/// Where the VM driver keeps each allocation's run directory.
const VM_RUN_ROOT: &str = "/run/overdrive/vm";
/// Guest RAM of every lane VM.
const LANE_VM_MEMORY_BYTES: u64 = 128 * 1024 * 1024;
/// Bound on a VM reaching Running with its flow flowing.
const BOOT_BOUND: Duration = Duration::from_secs(120);
/// Bound on the owner reflecting an end (record read, teardown).
const SETTLE_BOUND: Duration = Duration::from_secs(30);

/// One running node: the `ServerHandle`, its directories and an API client.
struct Node {
    handle: ServerHandle,
    tmp: TempDir,
    client: reqwest::Client,
    base: String,
    kit: GuestKit,
    image: GuestImage,
}

impl Node {
    /// Boots `run_server` in the composition `overdrive serve` runs and
    /// builds the workload image beside its data directory (the VM driver
    /// reflinks the rootfs master, so both live on the staging root).
    async fn boot(name: &str) -> Self {
        let kit = GuestKit::provision(&default_staging_root())
            .expect("precondition: the guest kit provisions (run `cargo xtask guest-kit-prepare`)");
        let tmp = tempfile::Builder::new()
            .prefix(&format!("d17-{name}-"))
            .tempdir_in(default_staging_root())
            .expect("server tempdir on the reflink-capable staging root");
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("operator config dir");
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse::<SocketAddr>().expect("bind addr"),
            data_dir,
            operator_config_dir: operator_config_dir.clone(),
            vsock_ports: NodeVsockPortConfig::kernel_assigned(),
            ..ServerConfig::new()
        };
        overdrive_testing::forwarder_module::expect_forwarder_module();
        let adapters = NodeAdapters::host(
            &config,
            Arc::new(RealCgroupFs::new()),
            Arc::new(SimKek::for_boot()),
        )
        .expect("the host bundle opens its stores under the node's data_dir");
        let handle = run_server(config, adapters).await.expect("serve boots on metal");
        let bound = handle.local_addr().await.expect("the API listener is bound");
        let client = client_trusting(&read_ca(&operator_config_dir));
        let image =
            kit.image(tmp.path(), name, &GuestImageSpec::workload()).expect("the workload image");
        Self {
            handle,
            tmp,
            client,
            base: format!("https://localhost:{}", bound.port()),
            kit,
            image,
        }
    }

    /// Submits a VM Job whose operator command is the guest application
    /// with `args`, through `POST /v1/workloads`.
    async fn deploy_job(&self, id: &str, args: &[String]) -> String {
        let spec = JobSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 500, memory_bytes: LANE_VM_MEMORY_BYTES },
            driver: DriverInput::Vm(VmInput {
                command: GUEST_PEER_GUEST_PATH.to_owned(),
                args: args.to_vec(),
                kernel: self.kit.kernel_path.display().to_string(),
                rootfs: self.image.rootfs.display().to_string(),
            }),
        };
        let resp = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec) })
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /v1/workloads {id}: {e}"));
        assert!(resp.status().is_success(), "deploy {id}: HTTP {}", resp.status());
        resp.json::<SubmitWorkloadResponse>().await.expect("submit response").workload_id
    }

    /// Stops a workload through `POST /v1/workloads/{id}/stop`.
    async fn stop(&self, id: &str) {
        let resp = self
            .client
            .post(format!("{}/v1/workloads/{id}/stop", self.base))
            .send()
            .await
            .unwrap_or_else(|e| panic!("stop {id}: {e}"));
        assert!(resp.status().is_success(), "stop {id}: HTTP {}", resp.status());
    }

    /// The workload's allocation rows (`GET /v1/allocs?job=<id>`).
    async fn rows(&self, id: &str) -> Vec<AllocStatusRowBody> {
        self.client
            .get(format!("{}/v1/allocs?job={id}", self.base))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET /v1/allocs {id}: {e}"))
            .json::<AllocStatusResponse>()
            .await
            .expect("alloc status response")
            .rows
    }

    /// Polls the rows until one allocation not in `except` is Running.
    async fn running_alloc_except(&self, id: &str, except: &[AllocationId]) -> AllocationId {
        let deadline = tokio::time::Instant::now() + BOOT_BOUND;
        loop {
            let rows = self.rows(id).await;
            if let Some(row) = rows.iter().find(|r| {
                r.state == AllocStateWire::Running
                    && !except.iter().any(|a| a.to_string() == r.alloc_id)
            }) {
                return AllocationId::new(&row.alloc_id).expect("a valid allocation id");
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workload {id}: no new Running allocation within {BOOT_BOUND:?}; rows {rows:#?}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// The owner's observation; `None` fails the cell (the composed path
    /// always has one).
    async fn observe(&self) -> GuestOwnerObservation {
        self.handle
            .guest_flow_observation()
            .await
            .expect("the composed boot exposes its guest-flow owner's observation")
    }

    /// Polls the observation until `done` holds, or panics with the last one.
    async fn wait_observation(
        &self,
        what: &str,
        done: impl Fn(&GuestOwnerObservation) -> bool,
    ) -> GuestOwnerObservation {
        let deadline = tokio::time::Instant::now() + SETTLE_BOUND;
        loop {
            let obs = self.observe().await;
            if done(&obs) {
                return obs;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what} not observed within {SETTLE_BOUND:?}; last observation {obs:#?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn shutdown(self) {
        self.handle.shutdown(Duration::from_secs(10)).await.expect("clean shutdown");
        drop(self.tmp);
    }
}

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("CA PEM"))
        .https_only(true)
        .use_rustls_tls()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
}

fn read_ca(operator_config_dir: &Path) -> String {
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

/// The allocation's guest console so far.
fn console_of(alloc: &AllocationId) -> String {
    let path = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).console_log();
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Waits until the allocation's guest application reports its flow moving
/// both ways.
async fn wait_flowing(alloc: &AllocationId) {
    let deadline = tokio::time::Instant::now() + BOOT_BOUND;
    while !console_of(alloc).contains(FLOWING_LINE) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "allocation {alloc}: the guest flow never reported flowing; console:\n{}",
            console_of(alloc)
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The pid of the live `cloud-hypervisor` serving `alloc` (matched by its
/// run directory in the argv).
fn hypervisor_pid(alloc: &AllocationId) -> u32 {
    let needle: PathBuf = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).path().to_path_buf();
    let needle = needle.to_string_lossy().into_owned();
    for entry in std::fs::read_dir("/proc").expect("read /proc").flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
        let argv0 = cmdline.split(|b| *b == 0).next().unwrap_or(&[]);
        if String::from_utf8_lossy(argv0).ends_with("cloud-hypervisor")
            && String::from_utf8_lossy(&cmdline).contains(&needle)
        {
            return pid;
        }
    }
    panic!("no cloud-hypervisor serves allocation {alloc}")
}

/// Pairs the owner's tables hold: Paired guest-opened flows.
fn paired_guest_flows(obs: &GuestOwnerObservation) -> u32 {
    let n = obs
        .cids
        .values()
        .flat_map(|c| c.guest_flows.values())
        .filter(|p| matches!(p, GuestFlowPhase::Paired { .. }))
        .count();
    u32::try_from(n).expect("count fits u32")
}

/// Watches the owner's tables hold: Paired host-opened dials.
fn paired_dials(obs: &GuestOwnerObservation) -> u32 {
    let n = obs
        .cids
        .values()
        .flat_map(|c| c.dials.values())
        .filter(|d| matches!(d.phase, DialPhase::Paired { .. }))
        .count();
    u32::try_from(n).expect("count fits u32")
}

/// The inventory of the owner's one serving context; any other read fails.
fn inventory(obs: &GuestOwnerObservation) -> (u32, u32) {
    match obs.forwarder_inventory {
        ForwarderInventoryRead::Read(inv) => (inv.live_pairs, inv.live_watches),
        other => panic!("the owner's context must answer inventory(), got {other:?}"),
    }
}

/// R5-57 through `serve` (D-15, D-17): with N guest-opened TCP flows held on
/// the node, the owner's own context's `inventory()` counts one pair per
/// flow and agrees with the owner's tables after every step; each
/// workload's teardown returns its pairs; at the end the inventory reads
/// zero.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 10-01"]
async fn the_forwarder_inventory_counts_every_held_guest_flow_through_serve() {
    const N: u32 = 3;
    let peer = RemotePeer::start("d17-inv", 0).expect("precondition: the remote peer starts");
    let node = Node::boot("d17-inv").await;

    let mut workloads = Vec::new();
    for i in 0..N {
        let tag = format!("d17-inv-{i}");
        let id = node
            .deploy_job(
                &tag,
                &[subcommand::HOLD.to_owned(), peer.tcp_addr().to_string(), tag.clone()],
            )
            .await;
        let alloc = node.running_alloc_except(&id, &[]).await;
        wait_flowing(&alloc).await;
        workloads.push(id);
    }

    let obs = node.wait_observation("N held flows paired", |o| paired_guest_flows(o) == N).await;
    let (pairs, watches) = inventory(&obs);
    assert_eq!(pairs, N, "one forwarder pair per held guest flow: {obs:#?}");
    assert_eq!(pairs, paired_guest_flows(&obs), "inventory agrees with the owner's flow table");
    assert_eq!(watches, paired_dials(&obs), "every watch is a Paired dial's: {obs:#?}");
    let held: u32 = obs.cids.values().map(|c| c.held_flows).sum();
    assert_eq!(held, N, "each flow counted once against its allocation's quota");

    // Each teardown returns its pairs once their end records are read.
    let mut left = N;
    for id in &workloads {
        node.stop(id).await;
        left -= 1;
        let obs = node
            .wait_observation("the stopped workload's pair returned", |o| {
                matches!(o.forwarder_inventory, ForwarderInventoryRead::Read(inv)
                    if inv.live_pairs == left)
            })
            .await;
        assert_eq!(paired_guest_flows(&obs), left, "the owner's table agrees: {obs:#?}");
    }
    let obs = node.observe().await;
    assert_eq!(inventory(&obs), (0, 0), "cleanup by teardown leaves the inventory at zero");
    assert!(obs.cids.is_empty(), "every CID was released: {obs:#?}");

    node.shutdown().await;
}

/// R5-41 through `serve` + `deploy` (the P-37 kill-and-reuse shape,
/// ATTR-TOKEN): an allocation's VMM is killed while its flow is held; the
/// workload's next allocation is placed and its VM boots. The dead launch's
/// CID entry, session and flows leave the owner; the next launch holds a
/// new launch token and its own session; its flow pairs and moves; every
/// pair the inventory counts belongs to the live launch.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "pending DELIVER step 10-01"]
async fn a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted() {
    let peer = RemotePeer::start("d17-kill", 0).expect("precondition: the remote peer starts");
    let node = Node::boot("d17-kill").await;
    let tag = "d17-kill";
    let id = node
        .deploy_job(
            tag,
            &[subcommand::HOLD.to_owned(), peer.tcp_addr().to_string(), tag.to_owned()],
        )
        .await;
    let first = node.running_alloc_except(&id, &[]).await;
    wait_flowing(&first).await;
    let before = node
        .wait_observation("the first launch's session and flow", |o| {
            o.cids.values().any(|c| c.alloc == first && c.session.is_some())
                && paired_guest_flows(o) == 1
        })
        .await;
    let (first_cid, first_entry) = before
        .cids
        .iter()
        .find(|(_, c)| c.alloc == first)
        .map(|(cid, c)| (*cid, c.clone()))
        .expect("the first launch's CID entry");

    // The VMM dies without the guest's cooperation.
    let pid = hypervisor_pid(&first);
    // SAFETY: SIGKILL to a pid read from /proc this instant; the test owns
    // the VM it deployed.
    let rc = unsafe { libc::kill(i32::try_from(pid).expect("pid fits i32"), libc::SIGKILL) };
    assert_eq!(rc, 0, "kill the first launch's VMM: {}", std::io::Error::last_os_error());

    let second = node.running_alloc_except(&id, std::slice::from_ref(&first)).await;
    wait_flowing(&second).await;
    let after = node
        .wait_observation("the dead launch gone and the next launch paired", |o| {
            o.cids.values().all(|c| c.alloc != first)
                && o.cids.values().any(|c| c.alloc == second && c.session.is_some())
                && paired_guest_flows(o) == 1
        })
        .await;
    let (second_cid, second_entry) = after
        .cids
        .iter()
        .find(|(_, c)| c.alloc == second)
        .map(|(cid, c)| (*cid, c.clone()))
        .expect("the next launch's CID entry");
    assert_ne!(second_entry.launch, first_entry.launch, "the next launch draws a new token");
    if second_cid == first_cid {
        assert_ne!(
            second_entry.session, first_entry.session,
            "the reused CID's session is the new launch's own connection"
        );
    }
    assert!(second_entry.unread.is_empty(), "no dead-launch connection is left unread: {after:#?}");
    let (pairs, _) = inventory(&after);
    assert_eq!(pairs, 1, "only the live launch's flow holds a pair: {after:#?}");
    for phase in second_entry.guest_flows.values() {
        assert!(
            matches!(phase, GuestFlowPhase::Paired { generation, .. }
                if *generation == after.forwarder_generation),
            "the next launch's flow is paired in the current context: {phase:?}"
        );
    }

    node.stop(&id).await;
    node.wait_observation("teardown", |o| o.cids.is_empty()).await;
    node.shutdown().await;
}
