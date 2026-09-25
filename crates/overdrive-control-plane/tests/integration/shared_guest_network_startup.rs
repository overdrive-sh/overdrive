//! GH #295 — the shared guest-network owner through the ordinary production
//! composition.
//!
//! - S-ND295-00: the shared-network startup proof is a hard gate. These bodies
//!   enter through `run_server_with_obs_and_driver`; the simulation adapter
//!   scripts only the accepted owner-port result and does not install a
//!   consequence or mutate the EXEC state.
//! - S-ND295-13B: boot-phase telemetry across ordinary `run_server`.
//! - S-ND295-10, 11, 12 (Lima root, real kernel): ordinary `run_server` over
//!   the injected recording VMM of the accepted `ServerConfig.vmm_override`
//!   port. The real owner provisions, activates, audits, and tears down; the
//!   tests observe the kernel and mutate it only from outside.

#![allow(clippy::doc_markdown)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::guest_network::{
    GuestNetworkError, GuestNetworkFact, GuestNetworkOperation, GuestNetworkProbeStage,
    GuestNetworkScratchComplement, GuestNetworkScratchCount,
};
use overdrive_control_plane::{ServerConfig, run_server, run_server_with_obs_and_driver};
use overdrive_core::guest_network::{GuestNetworkExecSupervisor, GuestNetworkExecWiring};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Clone, Default)]
struct EventRow {
    name: String,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: std::collections::BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), value.to_string());
    }
}

#[derive(Clone, Default)]
struct EventCollector {
    inner: Arc<Mutex<Vec<EventRow>>>,
}

struct StaleAttachmentCleanup {
    tap: String,
    guard: overdrive_netlink::nft::bridge::BridgeGuardSpec,
}

impl Drop for StaleAttachmentCleanup {
    fn drop(&mut self) {
        let mut cleanup_guard = || {
            let _ = overdrive_netlink::nft::bridge::delete_owned_guard(
                &self.guard,
                &std::collections::BTreeSet::new(),
            );
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut cleanup_guard));
        let tap = self.tap.clone();
        let _ = overdrive_netlink::block_on_host_netlink(move || async move {
            let client = overdrive_netlink::Client::new()?;
            client.del_link(&tap).await
        });
    }
}

impl EventCollector {
    fn snapshot(&self) -> Vec<EventRow> {
        self.inner.lock().expect("event collector lock").clone()
    }
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event")
            .or_else(|| visitor.fields.get("name"))
            .cloned()
            .unwrap_or_else(|| event.metadata().name().to_owned());
        self.inner
            .lock()
            .expect("event collector lock")
            .push(EventRow { name, fields: visitor.fields });
    }
}

const fn observed(value: u32) -> GuestNetworkScratchCount {
    GuestNetworkScratchCount::Observed(value)
}

const fn residue_complement() -> GuestNetworkScratchComplement {
    GuestNetworkScratchComplement {
        bridges: observed(0),
        taps: observed(0),
        endpoint_maps: observed(0),
        counter_maps: observed(0),
        endpoint_entries: observed(0),
        tcx_programs: observed(0),
        tcx_links: observed(1),
        endpoint_map_pins: observed(0),
        counter_map_pins: observed(0),
        tcx_link_pins: GuestNetworkScratchCount::Unavailable,
        bridge_guard_tables: observed(0),
        bridge_guard_chains: observed(0),
        bridge_guard_sets: observed(0),
        bridge_guard_rules: observed(0),
        bridge_guard_members: observed(0),
    }
}

fn config(tmp: &TempDir) -> ServerConfig {
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create config dir");
    ServerConfig {
        bind: "127.0.0.1:0".parse().expect("loopback bind"),
        data_dir,
        operator_config_dir,
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        dataplane_override: Some(Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new())),
        ..ServerConfig::new(
            Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
            std::sync::Arc::new(overdrive_sim::adapters::SimMtlsIntercept::new()),
            std::sync::Arc::new(overdrive_sim::adapters::SimGuestDnsFactory::default()),
        )
    }
}

const fn postcondition_failure(stage: GuestNetworkProbeStage) -> GuestNetworkError {
    GuestNetworkError::PostconditionMismatch {
        operation: GuestNetworkOperation::StartupProbe,
        expected: GuestNetworkFact::StartupProbe { stage, passed: true },
        observed: Some(GuestNetworkFact::StartupProbe { stage, passed: false }),
    }
}

async fn run_refusing_boot(
    fault: GuestNetworkError,
) -> (
    ControlPlaneError,
    Arc<SimSharedGuestNetworkOwner>,
    Arc<GuestNetworkExecSupervisor>,
    Vec<EventRow>,
) {
    let tmp = TempDir::new().expect("tempdir");
    let config = config(&tmp);
    let owner = Arc::new(SimSharedGuestNetworkOwner::default());
    owner.script_next_probe_error(fault);
    let wiring = GuestNetworkExecWiring::new(Arc::clone(&config.clock));
    let retained_supervisor = wiring.supervisor();
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("netns-density-startup").expect("node id"),
        0,
    ));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Vm));
    let owner_port: Arc<dyn overdrive_control_plane::guest_network::SharedGuestNetworkOwner> =
        owner.clone();
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let vm_host_state = Arc::new(overdrive_sim::adapters::vm_host_state::SimVmHostState::new());
    let result = run_server_with_obs_and_driver(
        config,
        obs,
        driver,
        vm_host_state,
        owner_port,
        wiring,
        overdrive_worker::cgroup_manager::CgroupManager::new(
            std::path::PathBuf::from("/sys/fs/cgroup"),
            std::sync::Arc::new(overdrive_sim::adapters::SimCgroupFs::new()),
        ),
    )
    .await;
    let error = match result {
        Err(error) => error,
        Ok(handle) => {
            handle
                .shutdown(Duration::from_secs(1))
                .await
                .expect("unexpectedly published server still shuts down");
            panic!("scratch-probe refusal must prevent production admission publication");
        }
    };
    (error, owner, retained_supervisor, collector.snapshot())
}

fn assert_startup_refusal_event(events: &[EventRow]) {
    assert!(events.iter().any(|event| {
        event.name == "health.startup.refused"
            || event.fields.get("name").map(String::as_str) == Some("health.startup.refused")
    }));
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn ordinary_probe_faults_refuse_before_convergence_or_publication() {
    let faults = [
        GuestNetworkError::Io {
            operation: GuestNetworkOperation::TcxAttach,
            source: std::io::Error::other("scripted TCX load/verifier refusal"),
        },
        postcondition_failure(GuestNetworkProbeStage::Classifier),
        postcondition_failure(GuestNetworkProbeStage::OriginalDestination),
        postcondition_failure(GuestNetworkProbeStage::DetachedLinkGuard),
    ];

    for fault in faults {
        let (error, owner, supervisor, events) = run_refusing_boot(fault).await;
        assert!(matches!(error, ControlPlaneError::GuestNetworkBoot(_)));
        assert_eq!(owner.calls(), [GuestNetworkOperation::StartupProbe]);
        assert!(supervisor.is_boot_closed());
        assert_startup_refusal_event(&events);
    }
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn cleanup_failure_refuses_and_preserves_primary_cleanup_and_observed_residue() {
    let observed = residue_complement();
    let fault = GuestNetworkError::StartupProbeCleanup {
        primary: Some(Box::new(postcondition_failure(GuestNetworkProbeStage::DetachedLinkGuard))),
        cleanup: Box::new(GuestNetworkError::Io {
            operation: GuestNetworkOperation::TcxLinkUnpin,
            source: std::io::Error::other("scripted scratch link-unpin refusal"),
        }),
        observed,
    };

    let (error, owner, supervisor, events) = run_refusing_boot(fault).await;
    let ControlPlaneError::GuestNetworkBoot(GuestNetworkError::StartupProbeCleanup {
        primary,
        cleanup,
        observed,
    }) = error
    else {
        panic!("expected source-honest startup cleanup aggregate");
    };

    assert!(matches!(
        primary.as_deref(),
        Some(GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::StartupProbe,
            ..
        })
    ));
    assert!(matches!(
        cleanup.as_ref(),
        GuestNetworkError::Io { operation: GuestNetworkOperation::TcxLinkUnpin, .. }
    ));
    assert_eq!(observed.tcx_links, GuestNetworkScratchCount::Observed(1));
    assert_eq!(observed.tcx_link_pins, GuestNetworkScratchCount::Unavailable);
    assert!(!observed.is_fully_observed());
    assert!(!observed.is_empty());
    assert_eq!(owner.calls(), [GuestNetworkOperation::StartupProbe]);
    assert!(supervisor.is_boot_closed());
    assert_startup_refusal_event(&events);
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn production_host_owner_boots_only_after_real_shared_identity_is_exact() {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "SKIP production_host_owner_boots_only_after_real_shared_identity_is_exact: root required"
        );
        return;
    }
    let tmp = TempDir::new().expect("tempdir");
    let handle = run_server(config(&tmp), Arc::new(overdrive_host::RealCgroupFs::new()))
        .await
        .expect("production host owner passes isolated probe, sweep, converge and audit");

    let netlink = overdrive_netlink::Client::new().expect("typed host netlink client");
    assert_eq!(
        netlink.observe_link("ovd-gbr0").await.expect("observe shared bridge"),
        Some(true),
        "the exact production bridge is administratively up before admission"
    );
    let guard = overdrive_netlink::nft::bridge::BridgeGuardSpec::new(
        "overdrive-mtls".to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical guard spec");
    assert!(matches!(
        overdrive_netlink::nft::bridge::observe(&guard, &std::collections::BTreeSet::new())
            .expect("non-repairing production guard observation"),
        overdrive_netlink::nft::bridge::BridgeGuardObservation::Exact { .. }
    ));
    assert!(Path::new("/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints").exists());
    assert!(Path::new("/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters").exists());

    let (reply, source) = tokio::task::spawn_blocking(|| {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").expect("bind DNS client");
        socket.set_read_timeout(Some(Duration::from_secs(2))).expect("bound DNS read timeout");
        let query = [
            0x29, 0x5a, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 7, b'm', b'i',
            b's', b's', b'i', b'n', b'g', 0, 0, 1, 0, 1,
        ];
        socket
            .send_to(&query, (std::net::Ipv4Addr::new(100, 95, 0, 1), 53))
            .expect("query the exact production shared gateway");
        let mut reply = [0_u8; 512];
        let (length, source) = socket.recv_from(&mut reply).expect("shared DNS reply");
        (reply[..length].to_vec(), source)
    })
    .await
    .expect("DNS client task joins");
    assert_eq!(&reply[..2], &[0x29, 0x5a], "transaction ID is preserved");
    assert_eq!(
        source.ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(100, 95, 0, 1)),
        "the wildcard-owned responder source-pins the reply to the exact shared gateway"
    );
    handle.shutdown(Duration::from_secs(10)).await.expect("production owner drains cleanly");
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn boot_reclamation_removes_a_prior_epoch_tap_before_shared_convergence_and_admission() {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("SKIP boot_reclamation_removes_a_prior_epoch_tap: root required");
        return;
    }
    let stale_tap = format!("ovd-tp-{:04x}", 0xfffd_u16);
    overdrive_netlink::create_persistent_tap(&stale_tap, 4_200)
        .expect("create one typed prior-epoch persistent TAP");
    let guard = overdrive_netlink::nft::bridge::BridgeGuardSpec::new(
        "overdrive-mtls".to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical guard spec");
    let _cleanup = StaleAttachmentCleanup { tap: stale_tap.clone(), guard: guard.clone() };
    for result in [
        overdrive_netlink::nft::bridge::converge_table(&guard),
        overdrive_netlink::nft::bridge::converge_chain(&guard),
        overdrive_netlink::nft::bridge::converge_set(&guard),
        overdrive_netlink::nft::bridge::converge_rules(&guard),
        overdrive_netlink::nft::bridge::insert_member(&guard, &stale_tap),
    ] {
        assert!(matches!(
            result.expect("converge prior-epoch guard fact"),
            overdrive_netlink::nft::bridge::BridgeGuardMutationOutcome::Converged { .. }
        ));
    }

    let tmp = TempDir::new().expect("tempdir");
    let handle = run_server(config(&tmp), Arc::new(overdrive_host::RealCgroupFs::new()))
        .await
        .expect("boot sweep precedes production shared convergence");
    let client = overdrive_netlink::Client::new().expect("typed host netlink client");
    assert_eq!(client.observe_link(&stale_tap).await.expect("observe stale TAP"), None);
    assert!(matches!(
        overdrive_netlink::nft::bridge::observe(&guard, &std::collections::BTreeSet::new())
            .expect("post-sweep guard observation"),
        overdrive_netlink::nft::bridge::BridgeGuardObservation::Exact { .. }
    ));
    handle.shutdown(Duration::from_secs(10)).await.expect("production owner drains cleanly");
}

fn required_field<'a>(event: &'a EventRow, field: &str) -> &'a str {
    event.fields.get(field).unwrap_or_else(|| panic!("{} event is missing {field}", event.name))
}

fn u64_field(event: &EventRow, field: &str) -> u64 {
    required_field(event, field)
        .parse()
        .unwrap_or_else(|error| panic!("{} {field} must be u64: {error}", event.name))
}

fn bool_field(event: &EventRow, field: &str) -> bool {
    required_field(event, field)
        .parse()
        .unwrap_or_else(|error| panic!("{} {field} must be bool: {error}", event.name))
}

const D14A_TCP_EVENT: &str = "guest_network.shared_owner_startup_probe_tcp_stage_completed";
const D14A_ATTACHMENT_EVENT: &str = "guest_network.shared_owner_startup_probe_attachment_reopened";
const D14A_DETACHED_EVENT: &str =
    "guest_network.shared_owner_startup_probe_detached_guard_completed";
const D14A_CLEANUP_EVENT: &str = "guest_network.shared_owner_startup_probe_cleanup_observed";

fn assert_tcp_stage_event(
    event: &EventRow,
    stage: &str,
    program_id: u64,
    expected_before: &mut u64,
) {
    assert_eq!(required_field(event, "stage"), stage);
    assert_eq!(u64_field(event, "program_id"), program_id);
    let peer_before = u64_field(event, "peer_intercept_before");
    let peer_after = u64_field(event, "peer_intercept_after");
    let gateway_before = u64_field(event, "gateway_intercept_before");
    let gateway_after = u64_field(event, "gateway_intercept_after");
    assert_eq!(peer_before, *expected_before);
    assert_eq!(peer_after.checked_sub(peer_before), Some(1));
    assert_eq!(gateway_before, peer_after);
    assert_eq!(gateway_after.checked_sub(gateway_before), Some(1));
    *expected_before = gateway_after;
}

/// S-ND295-00 — ordinary boot admits only after D14A's five deterministic
/// completion events prove the real classifier, attachment, guard and cleanup.
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::too_many_lines,
    reason = "one real-boot D14A trace keeps every completion field auditable"
)]
#[tokio::test]
async fn production_startup_exercises_classifier_and_detached_guard_before_admission() {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "SKIP production_startup_exercises_classifier_and_detached_guard_before_admission: root required"
        );
        return;
    }

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let tmp = TempDir::new().expect("tempdir");
    let handle = run_server(config(&tmp), Arc::new(overdrive_host::RealCgroupFs::new()))
        .await
        .expect("D14A real classifier and detached-guard probe permit admission");
    handle.shutdown(Duration::from_secs(10)).await.expect("production owner drains cleanly");

    let events: Vec<_> = collector
        .snapshot()
        .into_iter()
        .filter(|event| {
            [D14A_TCP_EVENT, D14A_ATTACHMENT_EVENT, D14A_DETACHED_EVENT, D14A_CLEANUP_EVENT]
                .contains(&event.name.as_str())
        })
        .collect();
    assert_eq!(
        events.iter().map(|event| event.name.as_str()).collect::<Vec<_>>(),
        [
            D14A_TCP_EVENT,
            D14A_TCP_EVENT,
            D14A_ATTACHMENT_EVENT,
            D14A_DETACHED_EVENT,
            D14A_CLEANUP_EVENT,
        ]
    );

    let program_id = u64_field(&events[0], "program_id");
    let mut next_intercept = u64_field(&events[0], "peer_intercept_before");
    assert_tcp_stage_event(&events[0], "classifier", program_id, &mut next_intercept);
    assert_tcp_stage_event(&events[1], "original_destination", program_id, &mut next_intercept);

    assert_eq!(u64_field(&events[2], "program_id"), program_id);
    assert_eq!(u64_field(&events[2], "program_count"), 1);
    let _revision = u64_field(&events[2], "revision");
    assert!(u64_field(&events[2], "ifindex") > 0);

    assert_eq!(u64_field(&events[3], "program_id"), program_id);
    assert_eq!(u64_field(&events[3], "classifier_intercept_before"), next_intercept);
    assert_eq!(u64_field(&events[3], "classifier_intercept_after"), next_intercept);
    let guard_packets_before = u64_field(&events[3], "guard_packets_before");
    let guard_packets_after = u64_field(&events[3], "guard_packets_after");
    let guard_bytes_before = u64_field(&events[3], "guard_bytes_before");
    let guard_bytes_after = u64_field(&events[3], "guard_bytes_after");
    assert_eq!(guard_packets_after.checked_sub(guard_packets_before), Some(1));
    assert!(guard_bytes_after.checked_sub(guard_bytes_before).is_some_and(|delta| delta > 0));
    assert_eq!(u64_field(&events[3], "host_datagrams"), 0);

    assert!(!bool_field(&events[4], "primary_failed"));
    assert!(!bool_field(&events[4], "cleanup_failed"));
    assert!(bool_field(&events[4], "fully_observed"));
    assert!(bool_field(&events[4], "empty"));
    for field in [
        "bridges",
        "taps",
        "endpoint_maps",
        "counter_maps",
        "endpoint_entries",
        "tcx_programs",
        "tcx_links",
        "endpoint_map_pins",
        "counter_map_pins",
        "tcx_link_pins",
        "bridge_guard_tables",
        "bridge_guard_chains",
        "bridge_guard_sets",
        "bridge_guard_rules",
        "bridge_guard_members",
    ] {
        assert_eq!(required_field(&events[4], field), "Observed(0)", "cleanup field {field}");
    }
}

// ---------------------------------------------------------------------------
// S-ND295-13B — boot-phase telemetry across ordinary `run_server`.
// ---------------------------------------------------------------------------

const BOOT_PHASE_EVENT: &str = "guest_network.shared_owner_boot_phase";

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-13B — Boot phases are visible to the operator in order
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts() {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "SKIP production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts: root required"
        );
        return;
    }

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let tmp = TempDir::new().expect("tempdir");
    let handle = run_server(config(&tmp), Arc::new(overdrive_host::RealCgroupFs::new()))
        .await
        .expect("ordinary production boot admits after reclamation and sweep");
    handle.shutdown(Duration::from_secs(10)).await.expect("production owner drains cleanly");

    let phases: Vec<EventRow> =
        collector.snapshot().into_iter().filter(|event| event.name == BOOT_PHASE_EVENT).collect();
    for phase in &phases {
        eprintln!("[boot-phase] {:?}", phase.fields);
    }
    assert_eq!(
        phases
            .iter()
            .map(|event| (required_field(event, "phase"), required_field(event, "transition")))
            .collect::<Vec<_>>(),
        [
            ("vm_reclamation", "started"),
            ("vm_reclamation", "completed"),
            ("stale_sweep", "started"),
            ("stale_sweep", "completed"),
        ],
        "exactly the four boot-phase boundaries, reclamation completed before the sweep starts"
    );
    let node_ids: std::collections::BTreeSet<&str> =
        phases.iter().map(|event| required_field(event, "node_id")).collect();
    assert_eq!(node_ids.len(), 1, "every boot-phase event names the one booting node");
}

// ---------------------------------------------------------------------------
// Ordinary production composition with an injected recording VMM
// (S-ND295-10, S-ND295-11, S-ND295-12).
//
// `run_server` composes the real shared-switch owner, the real VM driver and
// the real cgroup tree; the only substitution is the accepted
// `ServerConfig.vmm_override` port (ADR-0083 §D8). The recording VMM wraps the
// deterministic `SimVmm` process double, reads the real kernel attachment at
// `Vmm::create`, and plays the guest side of the beacon protocol (READY, then
// wait for EXEC / SHUTDOWN). It installs no TAP, link, pin, endpoint entry, or
// guard member: every attachment effect it observes is the production owner's.
// Workloads are deployed and stopped through the server's HTTPS API, never by
// spawning the `overdrive` binary. The whole control-plane integration binary
// is `host-kernel-shared`, and each boot's production stale sweep removes any
// attachment a panicking predecessor left behind.
// ---------------------------------------------------------------------------

const SHARED_BRIDGE: &str = "ovd-gbr0";
const ENDPOINT_MAP_PIN: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints";
const LINK_PIN_DIR: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/links";
const VM_WAIT: Duration = Duration::from_secs(30);

fn canonical_guard() -> overdrive_netlink::nft::bridge::BridgeGuardSpec {
    overdrive_netlink::nft::bridge::BridgeGuardSpec::new(
        "overdrive-mtls".to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical guard spec")
}

fn link_pin(tap: &str, attach: &str) -> PathBuf {
    PathBuf::from(format!("{LINK_PIN_DIR}/{tap}-{attach}"))
}

fn guard_inventory(
    observation: overdrive_netlink::nft::bridge::BridgeGuardObservation,
) -> overdrive_netlink::nft::bridge::BridgeGuardInventory {
    use overdrive_netlink::nft::bridge::BridgeGuardObservation;
    match observation {
        BridgeGuardObservation::Absent { inventory }
        | BridgeGuardObservation::Exact { inventory }
        | BridgeGuardObservation::Conflict { inventory } => inventory,
    }
}

fn guard_has_member(
    inventory: &overdrive_netlink::nft::bridge::BridgeGuardInventory,
    tap: &str,
) -> bool {
    inventory.members.iter().any(|member| {
        member.identity
            == overdrive_netlink::nft::bridge::BridgeGuardMemberIdentity::Ifname(tap.to_owned())
    })
}

/// Everything the real kernel reports about one allocation's attachment. Read
/// failures are retained as their debug rendering so a snapshot stays
/// comparable byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachmentReadBack {
    tap: String,
    identity: Result<overdrive_netlink::PersistentTapIdentity, String>,
    bridge: Result<Option<overdrive_netlink::ObservedLinkIdentity>, String>,
    debug_msg_mask: Result<u32, String>,
    guard_member: Result<bool, String>,
    endpoint_entry: Option<Result<bool, String>>,
    ingress: Result<overdrive_dataplane::guest_tcx::GuestTcxAttachment, String>,
    ingress_pin: bool,
    egress: Result<overdrive_dataplane::guest_tcx::GuestTcxAttachment, String>,
    egress_pin: bool,
}

impl AttachmentReadBack {
    const fn link(&self) -> Option<&overdrive_netlink::ObservedLinkIdentity> {
        match &self.identity {
            Ok(
                overdrive_netlink::PersistentTapIdentity::Persistent { link, .. }
                | overdrive_netlink::PersistentTapIdentity::Incompatible { link, .. },
            ) => Some(link),
            _ => None,
        }
    }

    fn ifindex(&self) -> Option<u32> {
        self.link().map(|link| link.ifindex)
    }
}

async fn read_attachment(tap: &str) -> AttachmentReadBack {
    use futures::FutureExt as _;
    use overdrive_dataplane::guest_tcx::{TcxAttachPoint, endpoint_present, query_attachment};

    let client = overdrive_netlink::Client::new();
    let (identity, bridge) = match &client {
        Ok(client) => (
            client.observe_persistent_tap_identity(tap).await.map_err(|error| format!("{error:?}")),
            client.observe_link_identity(SHARED_BRIDGE).await.map_err(|error| format!("{error:?}")),
        ),
        Err(error) => (Err(format!("{error:?}")), Err(format!("{error:?}"))),
    };
    let debug_msg_mask =
        std::panic::AssertUnwindSafe(overdrive_netlink::ethtool::debug_msg_mask(tap))
            .catch_unwind()
            .await
            .map_or_else(
                |_| Err("debug_msg_mask panicked".to_owned()),
                |result| result.map_err(|error| format!("{error:?}")),
            );
    let guard_member =
        overdrive_netlink::nft::bridge::observe(&canonical_guard(), &BTreeSet::new())
            .map(|observation| guard_has_member(&guard_inventory(observation), tap))
            .map_err(|error| format!("{error:?}"));
    let mut readback = AttachmentReadBack {
        tap: tap.to_owned(),
        identity,
        bridge,
        debug_msg_mask,
        guard_member,
        endpoint_entry: None,
        ingress: query_attachment(tap, TcxAttachPoint::Ingress)
            .map_err(|error| format!("{error:?}")),
        ingress_pin: link_pin(tap, "ingress").exists(),
        egress: query_attachment(tap, TcxAttachPoint::Egress).map_err(|error| format!("{error:?}")),
        egress_pin: link_pin(tap, "egress").exists(),
    };
    readback.endpoint_entry = readback.ifindex().map(|ifindex| {
        endpoint_present(ENDPOINT_MAP_PIN, ifindex).map_err(|error| format!("{error:?}"))
    });
    readback
}

/// One VM launch as the injected VMM observed it.
#[derive(Debug, Clone, Default)]
struct LaunchRecord {
    alloc: String,
    tap: Option<String>,
    guest_mac: Option<[u8; 6]>,
    /// The real kernel attachment when the VMM was asked to start.
    at_create: Option<AttachmentReadBack>,
    /// The real kernel attachment immediately before the guest's READY.
    before_ready: Option<AttachmentReadBack>,
    /// The guest received the host's EXEC acknowledgement.
    exec_received: bool,
}

/// The injected VMM. Per launch it reads the real kernel attachment, stages
/// the rootfs clone the port contract requires, starts one real stand-in
/// process (`sleep`, never the `overdrive` binary) whose pid the VM driver
/// places in the allocation's cgroup scope, and plays the guest side of the
/// beacon. `terminate` kills the stand-in; its exit watch reports the real
/// ending.
#[derive(Clone)]
struct RecordingVmm {
    launches: Arc<Mutex<Vec<LaunchRecord>>>,
    /// Kill requests for live stand-ins, keyed by pid. The reaper task owns
    /// each child, so a kill can never reach a reaped and reused pid.
    kills: Arc<Mutex<std::collections::BTreeMap<u32, tokio::sync::oneshot::Sender<()>>>>,
}

impl RecordingVmm {
    fn new() -> Self {
        Self {
            launches: Arc::new(Mutex::new(Vec::new())),
            kills: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
        }
    }

    fn launch_of(&self, alloc: &str) -> Option<LaunchRecord> {
        self.launches.lock().expect("launch log lock").iter().find(|l| l.alloc == alloc).cloned()
    }
}

#[async_trait::async_trait]
impl overdrive_core::traits::vmm::Vmm for RecordingVmm {
    fn kind(&self) -> &'static str {
        "recording"
    }

    async fn probe(&self) -> Result<(), overdrive_core::traits::vmm::VmmProbeError> {
        Ok(())
    }

    async fn create(
        &self,
        config: &overdrive_core::vm::config::VmConfig,
    ) -> overdrive_core::traits::vmm::Result<overdrive_core::traits::vmm::VmProcess> {
        use std::os::unix::process::ExitStatusExt as _;

        let tap = config.network.as_ref().map(|network| network.tap.clone());
        let at_create = match &tap {
            Some(tap) => Some(read_attachment(tap).await),
            None => None,
        };
        let index = {
            let mut launches = self.launches.lock().expect("launch log lock");
            launches.push(LaunchRecord {
                alloc: config.alloc.to_string(),
                tap: tap.clone(),
                guest_mac: config.network.as_ref().map(|network| network.mac),
                at_create,
                before_ready: None,
                exec_received: false,
            });
            launches.len() - 1
        };

        // The per-launch rootfs clone: replaced, never adopted.
        let clone_dest = config.rootfs.clone_dest().to_path_buf();
        if tokio::fs::try_exists(&clone_dest).await? {
            tokio::fs::remove_file(&clone_dest).await?;
        }
        tokio::fs::copy(config.rootfs.master(), &clone_dest).await?;

        let mut child = tokio::process::Command::new("sleep")
            .arg("infinity")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let pid = child.id().ok_or_else(|| std::io::Error::other("stand-in exited at spawn"))?;
        let (diagnostics, _writer) = overdrive_core::traits::vmm::VmmDiagnostics::new();
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
        self.kills.lock().expect("kill table lock").insert(pid, kill_tx);
        let kills = Arc::clone(&self.kills);
        let tail = diagnostics.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                Ok(()) = &mut kill_rx => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            kills.lock().expect("kill table lock").remove(&pid);
            let (exit_code, signal) = status.map_or((None, None), |status| {
                (status.code(), status.signal().and_then(|signal| u8::try_from(signal).ok()))
            });
            let _ = exit_tx.send(overdrive_core::traits::vmm::VmmExit {
                exit_code,
                signal,
                stderr_tail: tail.console_tail(),
            });
        });

        let socket = config.run_dir.beacon_socket(overdrive_core::vm::beacon::BEACON_VSOCK_PORT);
        tokio::spawn(play_guest(socket, tap, Arc::clone(&self.launches), index));
        Ok(overdrive_core::traits::vmm::VmProcess {
            control: overdrive_core::traits::vmm::VmControl {
                pid,
                api_socket: config.run_dir.api_socket(),
            },
            exit: overdrive_core::traits::vmm::VmExitWatch::new(exit_rx),
            diagnostics,
        })
    }

    async fn terminate(
        &self,
        control: &overdrive_core::traits::vmm::VmControl,
        _grace: Duration,
    ) -> overdrive_core::traits::vmm::Result<overdrive_core::traits::vmm::VmTermination> {
        // Already gone (reaped, or never ours) is `Killed`, idempotently.
        let kill = self.kills.lock().expect("kill table lock").remove(&control.pid);
        if let Some(kill) = kill {
            let _ = kill.send(());
        }
        Ok(overdrive_core::traits::vmm::VmTermination::Killed)
    }
}

/// The guest side of the beacon protocol: read the attachment back, send
/// READY, record the EXEC acknowledgement, and hold the session until the
/// host asks for SHUTDOWN or closes it.
async fn play_guest(
    socket: PathBuf,
    tap: Option<String>,
    launches: Arc<Mutex<Vec<LaunchRecord>>>,
    index: usize,
) {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let deadline = tokio::time::Instant::now() + VM_WAIT;
    let mut stream = loop {
        match tokio::net::UnixStream::connect(&socket).await {
            Ok(stream) => break stream,
            Err(error) if tokio::time::Instant::now() < deadline => {
                eprintln!("[guest] beacon {} not ready yet: {error}", socket.display());
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => {
                eprintln!("[guest] beacon {} never accepted: {error}", socket.display());
                return;
            }
        }
    };
    if let Some(tap) = &tap {
        let before_ready = read_attachment(tap).await;
        launches.lock().expect("launch log lock")[index].before_ready = Some(before_ready);
    }
    let ready = format!(
        "{}\n",
        overdrive_core::vm::beacon::BeaconMessage::Ready {
            pid: 1,
            port: overdrive_core::vm::beacon::BEACON_VSOCK_PORT,
        }
    );
    if let Err(error) = stream.write_all(ready.as_bytes()).await {
        eprintln!("[guest] READY write failed: {error}");
        return;
    }
    let (read, _write) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        eprintln!("[guest] host -> guest: {line}");
        if line.starts_with("EXEC ") {
            launches.lock().expect("launch log lock")[index].exec_received = true;
        }
        if line == "SHUTDOWN" {
            break;
        }
    }
}

fn read_ca_from_trust_triple(operator_config_dir: &Path) -> String {
    use base64::Engine as _;

    let text = std::fs::read_to_string(operator_config_dir.join(".overdrive").join("config"))
        .expect("read the trust triple the server wrote");
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|contexts| {
            contexts
                .iter()
                .find(|context| context.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|context| context.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("local context carries the CA");
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(ca).expect("base64 CA"))
        .expect("CA PEM is UTF-8")
}

/// One deployed, activated VM allocation.
#[derive(Debug, Clone)]
struct DeployedVm {
    workload: String,
    alloc: String,
    address: std::net::Ipv4Addr,
    launch: LaunchRecord,
}

impl DeployedVm {
    fn tap(&self) -> &str {
        self.launch.tap.as_deref().expect("a networked VM launch names its TAP")
    }

    const fn guest_mac(&self) -> [u8; 6] {
        self.launch.guest_mac.expect("a networked VM launch names its guest MAC")
    }
}

/// A production node booted through `run_server` over the recording VMM.
struct VmNode {
    tmp: TempDir,
    handle: Option<overdrive_control_plane::ServerHandle>,
    client: reqwest::Client,
    base: String,
    vmm: RecordingVmm,
    kernel: PathBuf,
    rootfs: PathBuf,
}

impl VmNode {
    async fn boot() -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let artifacts = tmp.path().join("vm-artifacts");
        std::fs::create_dir_all(&artifacts).expect("create VM artifact dir");
        // A header the per-allocation kernel preflight accepts on either
        // supported architecture: the ELF magic at offset 0 (x86_64) and the
        // arm64 Image magic `ARM\x64` at offset 0x38 (aarch64).
        let kernel = artifacts.join("vmlinuz");
        let mut header = vec![0_u8; overdrive_core::vm::config::KERNEL_MAGIC_WINDOW];
        header[..4].copy_from_slice(b"\x7fELF");
        header[0x38..0x3c].copy_from_slice(b"ARM\x64");
        std::fs::write(&kernel, header).expect("stage kernel image");
        let rootfs = artifacts.join("rootfs.img");
        std::fs::write(&rootfs, b"nd295-recording-vmm-rootfs").expect("stage rootfs image");

        let vmm = RecordingVmm::new();
        let config = ServerConfig { vmm_override: Some(Arc::new(vmm.clone())), ..config(&tmp) };
        let handle = run_server(config, Arc::new(overdrive_host::RealCgroupFs::new()))
            .await
            .expect("ordinary production composition boots over the injected VMM");
        let bound = handle.local_addr().await.expect("bound HTTPS address");
        let ca_pem = read_ca_from_trust_triple(&tmp.path().join("conf"));
        let client = reqwest::Client::builder()
            .add_root_certificate(
                reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM"),
            )
            .https_only(true)
            .use_rustls_tls()
            .build()
            .expect("build HTTPS client");
        Self {
            tmp,
            handle: Some(handle),
            client,
            base: format!("https://localhost:{}", bound.port()),
            vmm,
            kernel,
            rootfs,
        }
    }

    async fn rows(&self, workload: &str) -> Vec<overdrive_control_plane::api::AllocStatusRowBody> {
        let response = self
            .client
            .get(format!("{}/v1/allocs?job={workload}", self.base))
            .send()
            .await
            .expect("GET /v1/allocs");
        assert!(response.status().is_success(), "alloc status for {workload}: {response:?}");
        response
            .json::<overdrive_control_plane::api::AllocStatusResponse>()
            .await
            .expect("decode alloc status")
            .rows
    }

    /// Poll the operator surface until a row of `workload` satisfies `done`.
    /// Every observed change is appended to the test log.
    async fn wait_for_row(
        &self,
        workload: &str,
        what: &str,
        done: impl Fn(&overdrive_control_plane::api::AllocStatusRowBody) -> bool,
    ) -> overdrive_control_plane::api::AllocStatusRowBody {
        let deadline = tokio::time::Instant::now() + VM_WAIT;
        let mut last = String::new();
        loop {
            let rows = self.rows(workload).await;
            let rendered = format!("{rows:?}");
            if rendered != last {
                eprintln!("[alloc-status {workload}] {rendered}");
                last = rendered;
            }
            if let Some(row) = rows.into_iter().find(|row| done(row)) {
                return row;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{workload} never reached {what}; last observed rows {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_for_launch(&self, alloc: &str) -> LaunchRecord {
        let deadline = tokio::time::Instant::now() + VM_WAIT;
        loop {
            if let Some(launch) = self.vmm.launch_of(alloc)
                && launch.before_ready.is_some()
                && launch.exec_received
            {
                return launch;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the VMM launch of {alloc} never reached READY and EXEC: {:?}",
                self.vmm.launch_of(alloc)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Deploy one VM Job through the HTTPS API and wait until the production
    /// action owner has started it and released its command.
    async fn deploy(&self, workload: &str) -> DeployedVm {
        use overdrive_core::aggregate::{DriverInput, JobSpecInput, ResourcesInput, VmInput};

        let spec = JobSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            driver: DriverInput::Vm(VmInput {
                command: "/bin/true".to_owned(),
                args: vec![],
                kernel: self.kernel.display().to_string(),
                rootfs: self.rootfs.display().to_string(),
            }),
        };
        let response = self
            .client
            .post(format!("{}/v1/workloads", self.base))
            .json(&overdrive_control_plane::api::SubmitWorkloadRequest {
                spec: overdrive_core::api::submit::SubmitSpecInput::Job(spec),
            })
            .send()
            .await
            .expect("POST /v1/workloads");
        assert!(response.status().is_success(), "deploy {workload}: {response:?}");
        let row = self
            .wait_for_row(workload, "Running", |row| {
                row.state == overdrive_control_plane::api::AllocStateWire::Running
            })
            .await;
        let launch = self.wait_for_launch(&row.alloc_id).await;
        DeployedVm {
            workload: workload.to_owned(),
            address: row.workload_addr.expect("a Running VM row carries its guest address"),
            alloc: row.alloc_id,
            launch,
        }
    }

    /// Stop one workload through the HTTPS API and wait until none of its
    /// allocations is live. An allocation the supervisor killed ends Failed
    /// through the crash path rather than Terminated, so both are terminal.
    async fn stop(&self, vm: &DeployedVm) {
        use overdrive_control_plane::api::AllocStateWire;

        let response = self
            .client
            .post(format!("{}/v1/workloads/{}/stop", self.base, vm.workload))
            .send()
            .await
            .expect("POST /v1/workloads/{id}/stop");
        assert!(response.status().is_success(), "stop {}: {response:?}", vm.workload);
        let deadline = tokio::time::Instant::now() + VM_WAIT;
        let mut last = String::new();
        loop {
            let rows = self.rows(&vm.workload).await;
            let rendered = format!("{rows:?}");
            if rendered != last {
                eprintln!("[alloc-status {}] {rendered}", vm.workload);
                last = rendered;
            }
            if rows.iter().any(|row| row.alloc_id == vm.alloc)
                && rows.iter().all(|row| {
                    matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
                })
            {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{} never stopped; last observed rows {last}",
                vm.workload
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown(Duration::from_secs(10)).await.expect("production owner drains");
        }
        drop(self.tmp);
    }
}

/// Read the attachment until its complement is empty (TAP, endpoint entry at
/// `ifindex`, both link pins, and guard member all absent) or the wait
/// expires; return the last read-back either way, for the caller's oracle.
async fn wait_for_absent_attachment(tap: &str, ifindex: u32) -> AttachmentReadBack {
    let deadline = tokio::time::Instant::now() + VM_WAIT;
    loop {
        let readback = read_attachment(tap).await;
        let endpoint =
            overdrive_dataplane::guest_tcx::endpoint_present(ENDPOINT_MAP_PIN, ifindex).ok();
        let absent = matches!(
            readback.identity,
            Ok(overdrive_netlink::PersistentTapIdentity::Absent { .. })
        ) && endpoint == Some(false)
            && !readback.ingress_pin
            && !readback.egress_pin
            && readback.guard_member == Ok(false);
        if absent || tokio::time::Instant::now() >= deadline {
            return readback;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn is_root() -> bool {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-11 — A workload is admitted only after its complete attachment is read back down
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 06-02 (S-ND295-11)"]
async fn ordinary_provision_reads_back_the_complete_attachment_down_before_injected_vmm_start() {
    if !is_root() {
        eprintln!("SKIP ordinary_provision_reads_back_the_complete_attachment_down: root required");
        return;
    }
    let node = VmNode::boot().await;
    let vm = node.deploy("nd295-provision-down").await;
    let at_create =
        vm.launch.at_create.clone().expect("the injected VMM read the attachment at create");
    let before_ready =
        vm.launch.before_ready.clone().expect("the guest read the attachment before READY");
    eprintln!("[S-ND295-11] at create: {at_create:#?}");
    eprintln!("[S-ND295-11] before READY: {before_ready:#?}");

    // The VMM saw no start until the real kernel held the complete attachment
    // with the TAP administratively down.
    let Ok(overdrive_netlink::PersistentTapIdentity::Persistent { link, owner_uid }) =
        &at_create.identity
    else {
        panic!("the TAP is an exact persistent TAP at VMM start: {:?}", at_create.identity);
    };
    assert_eq!(link.kind, overdrive_netlink::ObservedLinkKind::Tap);
    assert!(!link.up, "the TAP is administratively down when the VMM is started");
    assert_eq!(*owner_uid, Some(0), "the TAP is owned by uid 0, never by the VMM identity");
    let bridge = at_create
        .bridge
        .clone()
        .expect("bridge observation succeeds")
        .expect("the shared bridge exists");
    assert_eq!(bridge.kind, overdrive_netlink::ObservedLinkKind::Bridge);
    assert_eq!(link.master_ifindex, Some(bridge.ifindex), "the TAP's master is the shared bridge");
    assert_eq!(at_create.debug_msg_mask, Ok(0), "the TAP's debug message level is zero");
    assert_eq!(at_create.guard_member, Ok(true), "the TAP is a bridge-guard member");
    assert_eq!(
        at_create.endpoint_entry,
        Some(Ok(true)),
        "the endpoint entry exists for its ifindex"
    );
    let ingress = at_create.ingress.clone().expect("ingress TCX query succeeds");
    assert_eq!(ingress.program_ids.len(), 1, "exactly the ingress classifier is attached");
    assert!(at_create.ingress_pin, "the ingress link is pinned");
    let egress = at_create.egress.clone().expect("egress TCX query succeeds");
    assert_eq!(egress.program_ids.len(), 1, "exactly the egress classifier is attached");
    assert!(at_create.egress_pin, "the egress link is pinned");
    assert_ne!(
        ingress.program_ids, egress.program_ids,
        "the ingress and egress attach points hold their two distinct programs"
    );

    // The same TAP is still down, at the same ifindex, when the guest reports
    // READY.
    let before_ready_link = before_ready.link().expect("the TAP still exists before READY");
    assert_eq!(before_ready_link.ifindex, link.ifindex, "same TAP ifindex through READY");
    assert!(!before_ready_link.up, "the TAP stays down through the VMM's READY");

    node.stop(&vm).await;
    node.shutdown().await;
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-12 — Teardown leaves nothing behind and converges on parts already gone
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 06-02 (S-ND295-12)"]
async fn two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal() {
    if !is_root() {
        eprintln!("SKIP two_attachment_teardown_releases_last: root required");
        return;
    }
    let node = VmNode::boot().await;
    let unrelated = node.deploy("nd295-teardown-keep").await;
    let named = node.deploy("nd295-teardown-named").await;
    let unrelated_before = read_attachment(unrelated.tap()).await;
    let named_before = read_attachment(named.tap()).await;
    eprintln!("[S-ND295-12] unrelated before: {unrelated_before:#?}");
    eprintln!("[S-ND295-12] named before: {named_before:#?}");
    let named_ifindex = named_before.ifindex().expect("the named TAP exists before teardown");

    // Out of band: the named allocation's TAP is deleted before the stop. The
    // kernel removes its bridge port with it; its endpoint entry, link pins,
    // and guard member remain for the production teardown.
    overdrive_netlink::Client::new()
        .expect("host netlink client")
        .del_link(named.tap())
        .await
        .expect("delete the named TAP out of band");
    let named_tap_gone = read_attachment(named.tap()).await;
    assert!(
        matches!(
            named_tap_gone.identity,
            Ok(overdrive_netlink::PersistentTapIdentity::Absent { .. })
        ),
        "the out-of-band deletion removed the TAP: {:?}",
        named_tap_gone.identity
    );

    // The production stop converges on the part already gone.
    node.stop(&named).await;
    let named_after = wait_for_absent_attachment(named.tap(), named_ifindex).await;
    eprintln!("[S-ND295-12] named after: {named_after:#?}");
    assert!(
        matches!(named_after.identity, Ok(overdrive_netlink::PersistentTapIdentity::Absent { .. })),
        "the named TAP is absent: {:?}",
        named_after.identity
    );
    assert_eq!(
        overdrive_dataplane::guest_tcx::endpoint_present(ENDPOINT_MAP_PIN, named_ifindex)
            .map_err(|error| format!("{error:?}")),
        Ok(false),
        "no endpoint entry remains for the named TAP's ifindex"
    );
    assert!(!named_after.ingress_pin, "the named ingress link pin is gone");
    assert!(!named_after.egress_pin, "the named egress link pin is gone");
    assert_eq!(named_after.guard_member, Ok(false), "the named guard member is gone");

    // The unrelated attachment is unchanged in every observed fact.
    let unrelated_after = read_attachment(unrelated.tap()).await;
    assert_eq!(
        unrelated_after, unrelated_before,
        "the unrelated attachment is byte-equal across the named teardown"
    );

    // The named lease was released after that teardown: the next admission
    // receives its address, on a fresh TAP.
    let successor = node.deploy("nd295-teardown-successor").await;
    assert_eq!(successor.address, named.address, "the released address is the next assigned");
    assert_eq!(successor.tap(), named.tap(), "the successor's TAP carries the released name");
    let successor_at_create =
        successor.launch.at_create.clone().expect("the successor's attachment was read");
    assert_ne!(
        successor_at_create.ifindex(),
        Some(named_ifindex),
        "the successor owns a new TAP, not a survivor of the named teardown"
    );

    node.stop(&successor).await;
    node.stop(&unrelated).await;
    node.shutdown().await;
}

// ---------------------------------------------------------------------------
// S-ND295-10 test support: a test-held TAP queue, a packet socket, and one
// hand-built guest frame. Test-only raw `TUNSETIFF` / `AF_PACKET`: no
// production API attaches a queue to a raised TAP or captures on the bridge.
// ---------------------------------------------------------------------------

/// Attach one test-held queue to an existing TAP, without a packet-info
/// header, in non-blocking mode.
fn attach_test_queue(tap: &str) -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open("/dev/net/tun")?;
    // SAFETY: `ifreq` is a plain C struct for which all-zero bytes are valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(tap.bytes()) {
        // `c_char` is signed on x86_64 and unsigned on aarch64; copy the byte.
        *slot = libc::c_char::from_ne_bytes([byte]);
    }
    request.ifr_ifru.ifru_flags = libc::c_short::try_from(libc::IFF_TAP | libc::IFF_NO_PI)
        .expect("TAP flags fit the ifreq flag field");
    // SAFETY: the descriptor is an open `/dev/net/tun` file owned by `file`,
    // and `request` is an initialised `ifreq` that outlives the call.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &raw mut request) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

/// A non-blocking `AF_PACKET` socket receiving every frame on `ifindex`.
fn packet_socket(ifindex: u32) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    let protocol = u16::try_from(libc::ETH_P_ALL).expect("ETH_P_ALL fits u16").to_be();
    // SAFETY: plain socket creation with constant arguments.
    let raw = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            i32::from(protocol),
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a freshly created descriptor this function owns.
    let socket = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
    // SAFETY: `sockaddr_ll` is a plain C struct for which all-zero bytes are valid.
    let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
    address.sll_protocol = protocol;
    address.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits i32");
    // SAFETY: `address` is an initialised `sockaddr_ll` whose exact size is
    // passed, and `socket` is an open descriptor.
    let rc = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            u32::try_from(std::mem::size_of::<libc::sockaddr_ll>()).expect("sockaddr_ll size"),
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(socket)
}

/// Read every frame available on `fd` for `window`, counting those that carry
/// `marker`.
async fn frames_with_marker(fd: std::os::fd::RawFd, marker: &[u8], window: Duration) -> usize {
    let deadline = tokio::time::Instant::now() + window;
    let mut seen = 0;
    let mut buffer = [0_u8; 2048];
    while tokio::time::Instant::now() < deadline {
        // SAFETY: `buffer` is valid for `buffer.len()` writable bytes and `fd`
        // is an open descriptor owned by the caller for this call's duration.
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        match usize::try_from(read) {
            Ok(length) if length > 0 => {
                if buffer[..length].windows(marker.len()).any(|window| window == marker) {
                    seen += 1;
                }
            }
            _ => tokio::time::sleep(Duration::from_millis(10)).await,
        }
    }
    seen
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = header
        .chunks(2)
        .map(|word| u32::from(u16::from_be_bytes([word[0], word.get(1).copied().unwrap_or(0)])))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !u16::try_from(sum).expect("folded checksum fits u16")
}

/// One valid guest frame: the guest's own MAC and address, UDP to a peer
/// guest, carrying `marker`.
fn guest_frame(
    source_mac: [u8; 6],
    source: std::net::Ipv4Addr,
    destination_mac: [u8; 6],
    destination: std::net::Ipv4Addr,
    marker: &[u8],
) -> Vec<u8> {
    let udp_length = u16::try_from(8 + marker.len()).expect("UDP length");
    let ip_length = 20 + udp_length;
    let mut frame = Vec::with_capacity(14 + usize::from(ip_length));
    frame.extend_from_slice(&destination_mac);
    frame.extend_from_slice(&source_mac);
    frame.extend_from_slice(&[0x08, 0x00]);
    let mut ip = vec![0x45, 0x00];
    ip.extend_from_slice(&ip_length.to_be_bytes());
    ip.extend_from_slice(&[0x29, 0x5a, 0x40, 0x00, 64, 17, 0, 0]);
    ip.extend_from_slice(&source.octets());
    ip.extend_from_slice(&destination.octets());
    let checksum = ipv4_checksum(&ip);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    frame.extend_from_slice(&ip);
    frame.extend_from_slice(&29_510_u16.to_be_bytes());
    frame.extend_from_slice(&29_511_u16.to_be_bytes());
    frame.extend_from_slice(&udp_length.to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(marker);
    frame
}

fn default_drop_packets(inventory: &overdrive_netlink::nft::bridge::BridgeGuardInventory) -> u64 {
    use overdrive_netlink::nft::bridge::{BridgeGuardRuleIdentity, BridgeGuardRuleKind};

    inventory
        .rules
        .iter()
        .find(|rule| {
            rule.fact.identity == BridgeGuardRuleIdentity::Owned(BridgeGuardRuleKind::DefaultDrop)
        })
        .and_then(|rule| rule.counter.as_ref())
        .map(|counter| counter.packets)
        .expect("the exact guard carries its counted DefaultDrop rule")
}

/// The guard inventory without its counters and generation.
fn guard_shape(
    inventory: &overdrive_netlink::nft::bridge::BridgeGuardInventory,
) -> impl PartialEq + std::fmt::Debug {
    (
        inventory.tables.clone(),
        inventory.chains.clone(),
        inventory.sets.clone(),
        inventory
            .rules
            .iter()
            .map(|rule| (rule.chain.clone(), rule.handle, rule.fact.clone()))
            .collect::<Vec<_>>(),
        inventory.members.iter().map(|member| member.identity.clone()).collect::<Vec<_>>(),
        inventory.other_children.clone(),
    )
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-10 — A removed ingress link is still blocked by the guard and condemns only that VM
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::too_many_lines,
    reason = "one example keeps the external mutation, the frame, and all three oracles in order"
)]
#[tokio::test]
#[ignore = "pending DELIVER step 09-01 (S-ND295-10)"]
async fn deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause() {
    use std::io::Write as _;
    use std::os::fd::AsRawFd as _;

    if !is_root() {
        eprintln!("SKIP deliberate_link_loss_reaches_default_drop: root required");
        return;
    }
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let node = VmNode::boot().await;
    let victim = node.deploy("nd295-link-loss").await;
    let peer = node.deploy("nd295-link-peer").await;
    let expected_members = BTreeSet::from([victim.tap().to_owned(), peer.tap().to_owned()]);

    // Test-held queues: the victim's to send as its guest, the peer's so the
    // bridge could deliver to it; a packet socket on the bridge sees anything
    // delivered to the host.
    let mut victim_queue = attach_test_queue(victim.tap()).expect("attach a queue to the victim");
    let peer_queue = attach_test_queue(peer.tap()).expect("attach a queue to the peer");
    let bridge_ifindex = read_attachment(victim.tap())
        .await
        .bridge
        .expect("bridge observation succeeds")
        .expect("the shared bridge exists")
        .ifindex;
    let host_capture = packet_socket(bridge_ifindex).expect("capture on the shared bridge");
    let guard = canonical_guard();
    let guard_before = guard_inventory(
        overdrive_netlink::nft::bridge::observe(&guard, &expected_members)
            .expect("guard observation before the loss"),
    );
    let drops_before = default_drop_packets(&guard_before);

    // External mutation only: detach the victim's exact pinned ingress link.
    let ingress = overdrive_dataplane::guest_tcx::query_attachment(
        victim.tap(),
        overdrive_dataplane::guest_tcx::TcxAttachPoint::Ingress,
    )
    .expect("query the victim's ingress attachment");
    assert_eq!(ingress.program_ids.len(), 1, "the victim's classifier is attached before the loss");
    overdrive_dataplane::guest_tcx::detach_pinned_link(link_pin(victim.tap(), "ingress"))
        .expect("detach the production-pinned ingress link");

    // One valid frame from the victim's guest to the peer's guest.
    let marker = b"nd295-deliberate-link-loss-marker";
    let frame =
        guest_frame(victim.guest_mac(), victim.address, peer.guest_mac(), peer.address, marker);
    victim_queue.write_all(&frame).expect("the victim guest transmits one frame");

    // The guard counts and drops the unmarked frame; read it at once, before
    // the owner's next audit can condemn the victim and start its cleanup.
    let counted_by = tokio::time::Instant::now() + Duration::from_secs(1);
    let (guard_classification, guard_after) = loop {
        let observation = overdrive_netlink::nft::bridge::observe(&guard, &expected_members)
            .expect("guard observation after the loss");
        let exact = matches!(
            observation,
            overdrive_netlink::nft::bridge::BridgeGuardObservation::Exact { .. }
        );
        let inventory = guard_inventory(observation);
        if default_drop_packets(&inventory) > drops_before
            || tokio::time::Instant::now() >= counted_by
        {
            break (exact, inventory);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(guard_classification, "the bridge guard is still exact with both members");
    assert_eq!(guard_shape(&guard_after), guard_shape(&guard_before), "the guard is unchanged");
    assert_eq!(
        default_drop_packets(&guard_after),
        drops_before + 1,
        "the unmarked frame is counted by the guard's DefaultDrop and dropped"
    );
    let escaped_to_peer =
        frames_with_marker(peer_queue.as_raw_fd(), marker, Duration::from_millis(500)).await;
    let escaped_to_host =
        frames_with_marker(host_capture.as_raw_fd(), marker, Duration::from_millis(500)).await;
    assert_eq!(escaped_to_peer, 0, "the frame never reaches the peer guest");
    assert_eq!(escaped_to_host, 0, "the frame never reaches the host");

    // The production audit names only the victim, by a per-allocation
    // ingress cause, and reports no node-level failure.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let killed = loop {
        let events = collector.snapshot();
        if let Some(event) = events.iter().find(|event| {
            event.name == "guest_network.shared_owner_vm_killed"
                && event.fields.get("alloc").map(String::as_str) == Some(victim.alloc.as_str())
        }) {
            break event.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the production audit never reported the victim's damage; events {:?}",
            events.iter().map(|event| &event.name).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    eprintln!("[S-ND295-10] audit report: {:?}", killed.fields);
    assert_eq!(required_field(&killed, "cause"), "attachment_damaged");
    let cause = required_field(&killed, "error");
    assert!(
        cause.contains("TcxQuery") || cause.contains("TcxLinkPin"),
        "the damage names the missing ingress attachment or pin: {cause}"
    );
    let events = collector.snapshot();
    assert!(
        !events.iter().any(|event| event.name == "guest_network.shared_owner_unhealthy"),
        "the link loss is per-allocation damage, never a node failure"
    );
    assert!(
        !events.iter().any(|event| {
            event.name == "guest_network.shared_owner_vm_killed"
                && event.fields.get("alloc").map(String::as_str) == Some(peer.alloc.as_str())
        }),
        "the peer allocation is not condemned"
    );

    drop(victim_queue);
    drop(peer_queue);
    drop(host_capture);
    node.stop(&peer).await;
    node.stop(&victim).await;
    node.shutdown().await;
}
