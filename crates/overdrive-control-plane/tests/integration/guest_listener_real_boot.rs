//! service-allocation-readiness-and-ending, Gate G-10 items (9) and (10) and
//! the guest half of the `guest_report` protocol (D13 items 1, 8 and 10, D14,
//! D15), as Tier-3 real-boot evidence on native metal (DG-8, ruled
//! 2026-11-05; user ruling 2026-11-04: no writer seam in `overdrive-init`).
//!
//! **Production composition, no test wiring.** Each test calls
//! `run_server(config, adapters)` with `NodeAdapters::host(..)` (only the KEK
//! replaced, as `guest_flow_through_serve.rs` does), boots a real guest-kit
//! microVM whose PID 1 is the production `overdrive-init`, and submits a
//! Service through `POST /v1/workloads`, the handler `overdrive deploy`
//! calls. The Service carries the single `GuestListeners` startup descriptor
//! the D15 default rule produces (interval 2 s, 30 attempts, so a 60 s startup
//! deadline). The workload is the guest application's `bind` subcommand: it
//! binds exactly the sockets a scenario names and does nothing else. The only
//! state observed is production-written: the allocation snapshot (`GET
//! /v1/allocs`) with its startup `ProbeResultRow`s and allocation rows. No
//! board is read and nothing in `overdrive-init` is observed.
//!
//! Lane: `integration-tests,kvm-tests`, native x86_64 metal, after
//! `cargo xtask kmod-build`, `kmod-load` and `guest-kit-prepare` (see
//! `guest_flow_through_serve.rs`): `cargo xtask metal run -- cargo nextest run
//! -p overdrive-control-plane --features integration-tests,kvm-tests
//! --run-ignored only -E 'test(guest_listener_real_boot)'`.
//!
//! **Not authored: G-10 item (9) first clause (init's own binds).** The
//! ruled fixture declares a port init itself binds (TCP intake 61000, a UDP
//! slot 61001..=62024). The Service validator rejects every port in that range
//! (`ListenerPortReserved`, `aggregate/mod.rs:500-511`), so such a Service can
//! never be deployed and the structural claim "init's binds produce no `LISTEN`
//! line" has no host-visible effect on any deployable Service: an unreported
//! undeclared pair is never stored (D13 item 3). It is routed to DESIGN as a
//! gap (see `distill/upstream-issues.md`, DG-13). The second clause of item
//! (9), the workload's own listener produces a report, is the first test here.
//!
//! **Not authored: `::ffff:<non-loopback>` Pass (item (10) last clause).** The
//! workload would need the leased `workload_addr`, which no pinned seam gives
//! the guest application before it binds.
//!
//! **Observable limit of the drain rule.** `ListenBeforeExit` and
//! `DeliveredBeforeExit` are guest-writer orderings; the host consequence is
//! only that every declared report is on the board before the exit report.
//! The probe task cancels at the exit (D1), so a workload that binds and
//! exits at once races the `Pass` write; the tight race is V36's (1,000
//! boots) evidence. The test here holds the listeners long enough for the
//! `Pass` to land and asserts the report precedes the exit in the rows.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::api::{
    AllocStateWire, AllocStatusResponse, ProbeStatusJson, SubmitWorkloadRequest,
    SubmitWorkloadResponse,
};
use overdrive_control_plane::{NodeAdapters, ServerConfig, ServerHandle, run_server};
use overdrive_core::aggregate::probe_descriptor::{ProbeDescriptor, ProbeMechanic};
use overdrive_core::aggregate::{DriverInput, ResourcesInput, VmInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::observation::{ProbeIdx, ProbeRole};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_testing::guest_app::{BOUND_PREFIX, GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::{GuestImage, GuestImageSpec, GuestKit};
use overdrive_testing::vm_fixture::default_staging_root;
use tempfile::TempDir;

/// Guest RAM of every lane VM.
const LANE_VM_MEMORY_BYTES: u64 = 128 * 1024 * 1024;
/// Bound on a VM reaching Running.
const BOOT_BOUND: Duration = Duration::from_secs(40);
/// The startup deadline of the default guest-listener descriptor
/// (`interval_seconds` 2 times `max_attempts` 30).
const STARTUP_DEADLINE: Duration = Duration::from_secs(60);
/// Bound on the deadline `Fail` row after `Running` (deadline plus slack),
/// inside the lane's 120 s per-test budget with the boot bound before it.
const FAIL_BOUND: Duration = Duration::from_secs(75);
/// Bound on the `Pass` row after the workload bound its listeners.
const PASS_BOUND: Duration = Duration::from_secs(20);

/// A declared listener the workload binds on the wildcard address (control).
const REPORTED_PORT: u16 = 18_084;
/// Declared listeners the workload binds on loopback addresses only.
const LOOPBACK_V4_PORT: u16 = 18_081;
const LOOPBACK_V6_PORT: u16 = 18_082;
const LOOPBACK_MAPPED_PORT: u16 = 18_083;
/// Declared listeners of the held-then-exit scenario.
const TCP_PORT: u16 = 18_091;
const UDP_PORT: u16 = 18_092;

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

/// One running node and the guest image its workloads boot.
struct Node {
    handle: ServerHandle,
    tmp: TempDir,
    client: reqwest::Client,
    base: String,
    kit: GuestKit,
    image: GuestImage,
}

impl Node {
    async fn boot(name: &str) -> Self {
        let kit = GuestKit::provision(&default_staging_root())
            .expect("precondition: the guest kit provisions (run `cargo xtask guest-kit-prepare`)");
        let tmp = tempfile::Builder::new()
            .prefix(&format!("gl-{name}-"))
            .tempdir_in(default_staging_root())
            .expect("server tempdir on the reflink-capable staging root");
        // The confined VMM identity traverses every ancestor of its clone by
        // name, so each directory the node creates under the tempdir must be
        // 0755. The metal runner's umask is 027 and `tempfile` creates 0700;
        // `overdrive serve` runs under umask 022. nextest runs one test per
        // process, so setting the process umask is local to this test.
        {
            use std::os::unix::fs::PermissionsExt as _;
            // SAFETY: `umask` has no preconditions and cannot fail.
            unsafe { libc::umask(0o022) };
            std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755))
                .expect("make the server tempdir traversable by the confined VMM identity");
        }
        let data_dir = tmp.path().join("data");
        let operator_config_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("operator config dir");
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("bind addr"),
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
        let image =
            kit.image(tmp.path(), name, &GuestImageSpec::workload()).expect("the workload image");
        Self {
            handle,
            tmp,
            client: client_trusting(&read_ca(&operator_config_dir)),
            base: format!("https://localhost:{}", bound.port()),
            kit,
            image,
        }
    }

    /// Submits a Service VM that declares `declared` (`(port, protocol)`),
    /// carries the default guest-listener startup gate, and runs the guest
    /// application `bind <hold> <exit> <specs>...`.
    async fn deploy(
        &self,
        id: &str,
        declared: &[(u16, &str)],
        hold_seconds: u64,
        exit_code: i32,
        specs: &[String],
    ) -> String {
        let mut args =
            vec![subcommand::BIND.to_owned(), hold_seconds.to_string(), exit_code.to_string()];
        args.extend(specs.iter().cloned());
        let spec = ServiceSpecInput {
            id: id.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 500, memory_bytes: LANE_VM_MEMORY_BYTES },
            driver: DriverInput::Vm(VmInput {
                command: GUEST_PEER_GUEST_PATH.to_owned(),
                args,
                kernel: self.kit.kernel_path.display().to_string(),
                rootfs: self.image.rootfs.display().to_string(),
            }),
            listeners: declared
                .iter()
                .map(|(port, protocol)| ListenerInput {
                    port: *port,
                    protocol: (*protocol).to_owned(),
                })
                .collect(),
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

    /// Polls until an allocation of `id` is Running; its id.
    async fn running_alloc(&self, id: &str) -> String {
        let deadline = tokio::time::Instant::now() + BOOT_BOUND;
        loop {
            let snapshot = self.snapshot(id).await;
            if let Some(row) = snapshot.rows.iter().find(|r| r.state == AllocStateWire::Running) {
                return row.alloc_id.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "workload {id}: no Running allocation within {BOOT_BOUND:?}; rows {:#?}",
                snapshot.rows
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn shutdown(self) {
        self.handle.shutdown(Duration::from_secs(10)).await.expect("clean shutdown");
        drop(self.tmp);
    }
}

/// Boots a node, runs `body`, and shuts the node down even when `body`
/// fails, so a failed assertion ends the test instead of leaving the guest
/// and the server alive until the runner's timeout; the failure is then
/// re-raised.
async fn with_node<F>(name: &str, body: F)
where
    F: for<'a> FnOnce(&'a Node) -> futures::future::BoxFuture<'a, ()>,
{
    use futures::FutureExt as _;
    let node = Node::boot(name).await;
    let outcome = std::panic::AssertUnwindSafe(body(&node)).catch_unwind().await;
    node.shutdown().await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// The startup row (role `startup`, index 0) of `alloc`, if any.
fn startup_status(snapshot: &AllocStatusResponse, alloc: &str) -> Option<ProbeStatusJson> {
    snapshot
        .probe_results
        .iter()
        .find(|r| r.alloc_id == alloc && r.role == ProbeRole::Startup.as_str() && r.probe_idx == 0)
        .map(|r| r.status.clone())
}

/// The Service's startup descriptor is the guest-listener mechanic (D15).
fn assert_guest_listener_descriptor(snapshot: &AllocStatusResponse) {
    assert!(
        snapshot.probes.iter().any(|p| p.role == ProbeRole::Startup
            && p.idx == ProbeIdx::new(0)
            && matches!(p.mechanic, ProbeMechanic::GuestListeners)),
        "the Service's sole startup descriptor is the guest-listener mechanic: {:#?}",
        snapshot.probes
    );
}

/// The guest console so far, for failure messages.
fn console_of(alloc: &str) -> String {
    let Ok(id) = overdrive_core::id::AllocationId::new(alloc) else { return String::new() };
    let path = overdrive_core::vm::config::VmRunDir::for_alloc(
        std::path::Path::new("/run/overdrive/vm"),
        &id,
    )
    .console_log();
    std::fs::read_to_string(path).unwrap_or_default()
}

/// G-10 (9) second clause, and the guest half of `guest_report` (D13 items 1
/// and 10): a real guest workload binds every declared listener (TCP and UDP),
/// holds them, and exits. The host sees the startup `Pass` row (every declared
/// listener reported) while the allocation is `Running`, and only afterwards
/// the exit report's terminal row. A report missing for either declared
/// listener would hold the `Pass` back (the set is not complete) and the
/// scenario would fail.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest side, in-guest listen hooks and LISTEN lines before EXIT (guest_report)"]
async fn every_declared_listener_a_real_workload_binds_is_reported_before_it_exits() {
    with_node("report-before-exit", |node| Box::pin(report_before_exit(node))).await;
}

async fn report_before_exit(node: &Node) {
    let id = "report-before-exit";
    let declared = [(TCP_PORT, "tcp"), (UDP_PORT, "udp")];
    let specs = vec![format!("tcp:0.0.0.0:{TCP_PORT}"), format!("udp:0.0.0.0:{UDP_PORT}")];
    node.deploy(id, &declared, 8, 0, &specs).await;

    let alloc = node.running_alloc(id).await;
    let mut pass_seen_while_running = false;
    let deadline = tokio::time::Instant::now() + PASS_BOUND + Duration::from_secs(20);
    loop {
        let snapshot = node.snapshot(id).await;
        assert_guest_listener_descriptor(&snapshot);
        let row = snapshot.rows.iter().find(|r| r.alloc_id == alloc);
        let state = row.map(|r| r.state);
        let status = startup_status(&snapshot, &alloc);
        assert!(
            !matches!(status, Some(ProbeStatusJson::Fail { .. })),
            "no Fail row is written for a workload that opened every declared listener: {status:?}; \
             console:\n{}",
            console_of(&alloc)
        );
        if matches!(status, Some(ProbeStatusJson::Pass)) && state == Some(AllocStateWire::Running) {
            pass_seen_while_running = true;
        }
        if matches!(state, Some(AllocStateWire::Terminated | AllocStateWire::Failed)) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the workload exits and its terminal row lands within the bound; state {state:?}, \
             startup row {status:?}; console:\n{}",
            console_of(&alloc)
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(
        pass_seen_while_running,
        "the startup Pass row (every declared listener reported) was observed while the \
         allocation was Running, before the exit report's terminal row; console:\n{}",
        console_of(&alloc)
    );
    assert!(console_of(&alloc).contains(BOUND_PREFIX), "the workload bound its sockets");
}

/// G-10 (10), Tier 3: binds to `127.0.0.1`, `::1` and `::ffff:127.0.0.1`
/// produce no `LISTEN` line, so the declared listeners they serve are never
/// reported. The startup row is `Fail` at the deadline and its reason names
/// each loopback-bound port as unreported; the declared listener the workload
/// bound on the wildcard address is reported and is not named. No row exists
/// before the deadline, and `Running` is unaffected (D14 item 3, G-10
/// "Explicitly unaffected").
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// CONTRACT_SHAPE: bounded-change.
#[ignore = "pending DELIVER: D13 guest side, loopback and v4-mapped binds are not reports (item 8)"]
async fn a_loopback_only_bind_never_satisfies_a_declared_listener() {
    with_node("loopback-binds", |node| Box::pin(loopback_binds(node))).await;
}

async fn loopback_binds(node: &Node) {
    let id = "loopback-binds";
    let declared = [
        (LOOPBACK_V4_PORT, "tcp"),
        (LOOPBACK_V6_PORT, "tcp"),
        (LOOPBACK_MAPPED_PORT, "tcp"),
        (REPORTED_PORT, "tcp"),
    ];
    let specs = vec![
        format!("tcp:127.0.0.1:{LOOPBACK_V4_PORT}"),
        format!("tcp:[::1]:{LOOPBACK_V6_PORT}"),
        format!("tcp:[::ffff:127.0.0.1]:{LOOPBACK_MAPPED_PORT}"),
        format!("tcp:0.0.0.0:{REPORTED_PORT}"),
    ];
    node.deploy(id, &declared, 0, 0, &specs).await;

    let alloc = node.running_alloc(id).await;
    let running_since = tokio::time::Instant::now();
    let deadline = running_since + FAIL_BOUND;
    let reason = loop {
        let snapshot = node.snapshot(id).await;
        assert_guest_listener_descriptor(&snapshot);
        let status = startup_status(&snapshot, &alloc);
        match status {
            Some(ProbeStatusJson::Pass) => panic!(
                "a loopback-only bind satisfied a declared listener: the set reached Pass; \
                 console:\n{}",
                console_of(&alloc)
            ),
            Some(ProbeStatusJson::Fail { last_fail_reason }) => {
                assert!(
                    running_since.elapsed() + Duration::from_secs(5) >= STARTUP_DEADLINE,
                    "no Fail row is written before the startup deadline; first row after {:?}",
                    running_since.elapsed()
                );
                break last_fail_reason;
            }
            None => {}
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the deadline Fail row lands within {FAIL_BOUND:?}; console:\n{}",
            console_of(&alloc)
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert!(
        reason.contains("guest listeners not open after"),
        "the Fail reason is the guest-listener reason: {reason}"
    );
    for port in [LOOPBACK_V4_PORT, LOOPBACK_V6_PORT, LOOPBACK_MAPPED_PORT] {
        assert!(
            reason.contains(&format!("tcp:{port}")),
            "the reason names the loopback-bound port tcp:{port} as unreported: {reason}"
        );
    }
    assert!(
        !reason.contains(&format!("tcp:{REPORTED_PORT}")),
        "the wildcard-bound declared listener is reported and is not named: {reason}"
    );
    let rows = node.snapshot(id).await.rows;
    assert!(
        rows.iter().any(|r| r.alloc_id == alloc),
        "the allocation row exists: the gate never took Running away: {rows:#?}"
    );
}
