//! GH #295 P4-S13 — the `run_server` boot order and its refusals (operator
//! projection of P4-S14), through the ordinary production composition.
//!
//! Every body enters through `run_server(config, adapters)` (Lima root, real
//! cgroupfs) over the host bundle (`NodeAdapters::host`, NO-OVERRIDE-SEAMS)
//! with `vm: { SimVmm, SimBeaconIo, Required }` (DG-4), the node's
//! `SimMtlsIntercept`, kernel-assigned vsock ports, and the production
//! guest-flow owner the composition builds itself over
//! `GuestFlowAdapter::host()` (real listeners, real forwarder probe; the lane
//! loads `ovd_fwd`, FWD-PREREQ). Each refusal is one data row whose fault enters at a driven
//! port or through the node's real configuration:
//!
//! | Row | Fault | Typed refusal | `health.startup.refused` reason |
//! |---|---|---|---|
//! | VIP overlap (D8-PREFIX-VIP) | `ServerConfig.vip_range` meets the guest prefix | `GuestNetworkBoot(GuestPrefixOverlapsVipRange)` | `guest_network.probe` |
//! | node address (D8-PREFIX-CFG) | a real dummy interface carries an address inside the prefix | `GuestNetworkBoot(GuestPrefixOverlapsNodeAddress)` | `guest_network.probe` |
//! | fd budget | the process's soft `RLIMIT_NOFILE` is lowered | `GuestNetworkBoot(FdBudget)` | `guest_network.probe` |
//! | step 8 (ADR-0184) | `SimMtlsIntercept`'s next table transaction fails | `MtlsBoot(SharedOwner)` | `mtls.shared_owner` |
//! | beacon bind (DG-4) | `SimBeaconIo`'s next bind fails | `VmmBoot` / beacon bind refusal | `vm_driver.beacon_bind` |
//!
//! `OwnerCgroupPlacement` (D24) has no row here: `run_server` enrols the
//! process into `overdrive.slice/control-plane.slice` before
//! `probe_startup` runs (`create_and_enrol_control_plane_slice`), so no
//! production `serve` boot reaches that refusal; its owner-level body is
//! `guest_network::prefix_boot_acceptance` (source-local).
//!
//! The step-8 row's table read-back is the QPROJ-7 arbiter
//! (`SimMtlsIntercept::shared_table_state`), Tier-3 pair R5-30.

#![allow(clippy::doc_markdown, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::error::{ControlPlaneError, MtlsBootError, VmmBootError};
use overdrive_control_plane::guest_network::GuestNetworkError;
use overdrive_control_plane::{NodeAdapters, ServerConfig, VmCapability, run_server};
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_dataplane::allocators::VipRange;
use overdrive_sim::adapters::{
    SimBeaconIo, SimInterceptFault, SimMtlsIntercept, SimSharedTable, SimVmm,
};
use overdrive_worker::BeaconIntakeError;
use serial_test::serial;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

// ---------------------------------------------------------------------------
// Event capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct EventRow {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct EventCollector(Arc<Mutex<Vec<EventRow>>>);

impl<S: Subscriber> Layer<S> for EventCollector {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name =
            visitor.0.get("name").cloned().unwrap_or_else(|| event.metadata().name().to_owned());
        self.0.lock().expect("event collector lock").push(EventRow { name, fields: visitor.0 });
    }
}

impl EventCollector {
    fn refusal_reasons(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("event collector lock")
            .iter()
            .filter(|row| row.name == "health.startup.refused")
            .filter_map(|row| row.fields.get("reason").cloned())
            .collect()
    }

    fn boot_phases(&self) -> Vec<(String, String)> {
        self.0
            .lock()
            .expect("event collector lock")
            .iter()
            .filter(|row| row.name == "guest_network.shared_owner_boot_phase")
            .map(|row| {
                (
                    row.fields.get("phase").cloned().unwrap_or_default(),
                    row.fields.get("transition").cloned().unwrap_or_default(),
                )
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// The node under test
// ---------------------------------------------------------------------------

/// One node's state across boots: the shared table, the VMM and beacon sims,
/// the data directory.
struct Node {
    tmp: TempDir,
    intercept: Arc<SimMtlsIntercept>,
    vmm: SimVmm,
    beacon: Arc<SimBeaconIo>,
}

impl Node {
    fn new() -> Self {
        let vmm = SimVmm::new();
        let beacon = Arc::new(SimBeaconIo::new(vmm.clone()));
        Self {
            tmp: TempDir::new().expect("tempdir"),
            intercept: Arc::new(SimMtlsIntercept::new()),
            vmm,
            beacon,
        }
    }

    fn config(&self) -> ServerConfig {
        let data_dir = self.tmp.path().join("data");
        let operator_config_dir = self.tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
        super::node_adapters::test_config(data_dir, operator_config_dir)
    }

    /// The host bundle over `config` with the node's simulated intercept and
    /// its `SimVmm` + `SimBeaconIo` pair (DG-4) at `Required` capability.
    fn adapters(&self, config: &ServerConfig, dataplane: Dataplane) -> NodeAdapters {
        let mut adapters = match dataplane {
            Dataplane::Sim => super::node_adapters::node_adapters(config),
            Dataplane::Host => super::node_adapters::node_adapters_with_host_dataplane(config),
        };
        adapters.boot.mtls.intercept = Arc::clone(&self.intercept) as _;
        adapters.vm.vmm = Arc::new(self.vmm.clone());
        adapters.vm.beacon = Arc::clone(&self.beacon) as _;
        adapters.vm.capability = VmCapability::Required;
        adapters
    }

    async fn boot(
        &self,
        config: ServerConfig,
    ) -> Result<overdrive_control_plane::ServerHandle, ControlPlaneError> {
        self.boot_over(config, Dataplane::Sim).await
    }

    async fn boot_over(
        &self,
        config: ServerConfig,
        dataplane: Dataplane,
    ) -> Result<overdrive_control_plane::ServerHandle, ControlPlaneError> {
        let adapters = self.adapters(&config, dataplane);
        run_server(config, adapters).await
    }
}

/// Which dataplane composer a boot uses.
#[derive(Debug, Clone, Copy)]
enum Dataplane {
    /// `SimDataplaneComposer` (no veth pair, no XDP).
    Sim,
    /// The production `HostDataplaneComposer` (provisions the configured
    /// interfaces).
    Host,
}

/// Boot `config` under an event collector; the refusal and the collected
/// events.
async fn refused_boot(node: &Node, config: ServerConfig) -> (ControlPlaneError, EventCollector) {
    refused_boot_over(node, config, Dataplane::Sim).await
}

/// [`refused_boot`] over the chosen dataplane composer.
async fn refused_boot_over(
    node: &Node,
    config: ServerConfig,
    dataplane: Dataplane,
) -> (ControlPlaneError, EventCollector) {
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));
    match node.boot_over(config, dataplane).await {
        Err(refusal) => (refusal, collector),
        Ok(handle) => {
            handle
                .shutdown(Duration::from_secs(1))
                .await
                .expect("the unexpected server shuts down");
            panic!("the boot must refuse before publishing the server");
        }
    }
}

/// A root-namespace dummy interface carrying one address, removed on drop.
struct DummyInterface {
    name: &'static str,
}

impl DummyInterface {
    fn with_address(name: &'static str, cidr: &str) -> Self {
        for args in
            [vec!["link", "add", name, "type", "dummy"], vec!["addr", "add", cidr, "dev", name]]
        {
            let status = std::process::Command::new("ip").args(&args).status().expect("spawn ip");
            assert!(status.success(), "`ip {args:?}` succeeds");
        }
        Self { name }
    }

    fn index(&self) -> u32 {
        let raw = std::fs::read_to_string(format!("/sys/class/net/{}/ifindex", self.name))
            .expect("read ifindex");
        raw.trim().parse().expect("ifindex is a number")
    }
}

impl Drop for DummyInterface {
    fn drop(&mut self) {
        let output = std::process::Command::new("ip").args(["link", "del", self.name]).output();
        eprintln!("[P4-S13] cleanup `ip link del {}`: {output:?}", self.name);
    }
}

/// Restores the soft `RLIMIT_NOFILE` on drop.
struct NofileGuard {
    previous: libc::rlimit,
}

impl NofileGuard {
    fn lower_soft_to(soft: u64) -> Self {
        let mut previous = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: `getrlimit` writes into the provided, properly aligned struct.
        let read = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut previous) };
        assert_eq!(read, 0, "getrlimit(RLIMIT_NOFILE)");
        let lowered = libc::rlimit { rlim_cur: soft, rlim_max: previous.rlim_max };
        // SAFETY: `setrlimit` reads the provided struct; lowering the soft limit
        // below the hard limit needs no privilege. `#[serial(rlimit)]` keeps
        // every body that touches the process's limit exclusive.
        let set = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lowered) };
        assert_eq!(set, 0, "setrlimit(RLIMIT_NOFILE)");
        Self { previous }
    }
}

impl Drop for NofileGuard {
    fn drop(&mut self) {
        // SAFETY: restores the struct `getrlimit` filled in.
        let restored = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const self.previous) };
        eprintln!("[P4-S13] restore RLIMIT_NOFILE -> {restored}");
    }
}

// ---------------------------------------------------------------------------
// P4-S13 — the boot order
// ---------------------------------------------------------------------------

/// P4-S13 — A `serve` boot with the guest network composed runs its steps in
/// the pinned order and opens the EXEC gate only after the supervisor.
/// CONTRACT_SHAPE: bounded-change.
///
/// Given a fresh node. When `run_server` boots with the VM override, then the
/// boot completes, the shared table is complete (step 8), the stale sweep
/// ran after boot reclamation, and no refusal was emitted.
#[tokio::test]
#[serial(cgroup)]
async fn serve_boots_through_the_pinned_order_with_the_guest_network_composed() {
    let node = Node::new();
    let collector = EventCollector::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(collector.clone()));

    let handle = node.boot(node.config()).await.expect("the node boots");

    assert!(collector.refusal_reasons().is_empty(), "no refusal");
    assert_eq!(
        node.intercept.shared_table_state(),
        SimSharedTable::Complete,
        "step 8 wrote the table"
    );
    assert_eq!(
        collector.boot_phases(),
        vec![
            ("vm_reclamation".to_owned(), "started".to_owned()),
            ("vm_reclamation".to_owned(), "completed".to_owned()),
            ("stale_sweep".to_owned(), "started".to_owned()),
            ("stale_sweep".to_owned(), "completed".to_owned()),
        ],
        "boot reclamation precedes the stale sweep"
    );
    assert!(node.beacon.is_bound(), "the node beacon intake is bound at composition");
    handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
}

// ---------------------------------------------------------------------------
// P4-S13 — boot refusals (one data row each)
// ---------------------------------------------------------------------------

/// P4-S13 / P4-S14 (D8-PREFIX-VIP) — A configured VIP range overlapping the
/// guest prefix refuses boot at `probe_startup`, before the dataplane's veth
/// step and before anything is written.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 13"]
async fn a_vip_range_overlapping_the_guest_prefix_refuses_boot_before_any_effect() {
    let node = Node::new();
    let overlapping: ipnet::Ipv4Net = "100.95.128.0/17".parse().expect("CIDR");
    let config = ServerConfig {
        vip_range: VipRange::new(vec![overlapping], std::collections::BTreeSet::new())
            .expect("VIP range"),
        dataplane: Some(overdrive_control_plane::dataplane_config::DataplaneConfig {
            client_iface: "nd295-s13-cli".to_owned(),
            backend_iface: "nd295-s13-bk".to_owned(),
        }),
        ..node.config()
    };

    // The production dataplane composer, so the probe is shown to run before
    // its veth step.
    let (refusal, events) = refused_boot_over(&node, config, Dataplane::Host).await;

    assert!(
        matches!(
            refusal,
            ControlPlaneError::GuestNetworkBoot(GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range })
                if prefix == GUEST_NODE_PREFIX && vip_range == overlapping
        ),
        "the VIP range is named: {refusal:?}"
    );
    assert_eq!(events.refusal_reasons(), vec!["guest_network.probe".to_owned()]);
    for iface in ["nd295-s13-cli", "nd295-s13-bk"] {
        assert!(
            !std::path::Path::new(&format!("/sys/class/net/{iface}")).exists(),
            "probe_startup runs before the veth step: {iface} was never created"
        );
    }
    assert_eq!(node.intercept.shared_table_state(), SimSharedTable::Absent, "nothing written");
}

/// P4-S13 / P4-S14 (D8-PREFIX-CFG) — A node interface address inside the
/// guest prefix refuses boot at `probe_startup`, naming the interface and the
/// address.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 13"]
async fn a_node_address_inside_the_guest_prefix_refuses_boot_naming_the_interface() {
    let node = Node::new();
    let dummy = DummyInterface::with_address("nd295-pfx0", "100.95.200.1/24");
    let index = dummy.index();

    let (refusal, events) = refused_boot(&node, node.config()).await;

    assert!(
        matches!(
            &refusal,
            ControlPlaneError::GuestNetworkBoot(GuestNetworkError::GuestPrefixOverlapsNodeAddress { prefix, interface, address })
                if *prefix == GUEST_NODE_PREFIX
                    && interface.get() == index
                    && *address == "100.95.200.1/24".parse::<ipnet::Ipv4Net>().expect("CIDR")
        ),
        "the interface and its address are named: {refusal:?}"
    );
    assert_eq!(events.refusal_reasons(), vec!["guest_network.probe".to_owned()]);
    assert_eq!(node.intercept.shared_table_state(), SimSharedTable::Absent, "nothing written");
}

/// P4-S13 — A soft `RLIMIT_NOFILE` below the owner's fd budget refuses boot
/// at `probe_startup`.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup, rlimit)]
async fn an_rlimit_nofile_below_the_owner_budget_refuses_boot() {
    let node = Node::new();
    let _limit = NofileGuard::lower_soft_to(1_024);

    let (refusal, events) = refused_boot(&node, node.config()).await;

    assert!(
        matches!(
            refusal,
            ControlPlaneError::GuestNetworkBoot(GuestNetworkError::FdBudget { required, available: 1_024 })
                if required > 1_024
        ),
        "FdBudget names the available limit: {refusal:?}"
    );
    assert_eq!(events.refusal_reasons(), vec!["guest_network.probe".to_owned()]);
}

/// P4-S13 / ADR-0184 — A failed boot write of the shared mTLS table (step 8)
/// refuses boot before forwarding opens and leaves the table as it was; the
/// supervised restart's boot converges.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[serial(cgroup)]
async fn a_failed_step8_table_write_refuses_boot_and_the_restart_converges() {
    let node = Node::new();
    node.intercept.fail_next_table_transaction(SimInterceptFault::NftRuleInstall {
        op: "shared-table-transaction",
        errno: libc::EBUSY,
    });

    let (refusal, events) = refused_boot(&node, node.config()).await;

    assert!(
        matches!(refusal, ControlPlaneError::MtlsBoot(MtlsBootError::SharedOwner { .. })),
        "step 8 refuses typed: {refusal:?}"
    );
    assert_eq!(events.refusal_reasons(), vec!["mtls.shared_owner".to_owned()]);
    assert_eq!(
        node.intercept.shared_table_state(),
        SimSharedTable::Absent,
        "the table is as it was"
    );

    let handle = node.boot(node.config()).await.expect("the restart's boot converges");
    assert_eq!(node.intercept.shared_table_state(), SimSharedTable::Complete);
    handle.shutdown(Duration::from_secs(1)).await.expect("graceful shutdown");
}

/// P4-S13 / DG-4 — A failed bind of the node beacon intake refuses boot at
/// `compose_vm_driver` with reason `vm_driver.beacon_bind`.
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[serial(cgroup)]
async fn a_failed_beacon_bind_refuses_boot() {
    let node = Node::new();
    node.beacon.fail_next_bind(std::io::ErrorKind::AddrInUse);
    let config = node.config();
    let expected_port = config.vsock_ports.beacon();

    let (refusal, events) = refused_boot(&node, config).await;

    assert!(
        matches!(
            &refusal,
            ControlPlaneError::VmmBoot(VmmBootError::BeaconBind {
                source: BeaconIntakeError::Bind { port, source },
            }) if *port == expected_port && source.kind() == std::io::ErrorKind::AddrInUse
        ),
        "the typed refusal preserves the configured port and bind error: {refusal:?}"
    );
    assert!(!node.beacon.is_bound(), "nothing is bound: {refusal:?}");
    assert_eq!(events.refusal_reasons(), vec!["vm_driver.beacon_bind".to_owned()]);
    assert_eq!(node.intercept.shared_table_state(), SimSharedTable::Absent, "nothing written");
}
