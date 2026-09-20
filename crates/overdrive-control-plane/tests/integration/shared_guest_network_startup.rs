//! GH #295 S-ND295-00 — shared guest-network startup proof is a hard gate.
//!
//! These bodies enter through the production `run_server_with_obs_and_driver`
//! composition. The simulation adapter scripts only the accepted owner-port
//! result; it does not install a consequence or mutate the EXEC state.

#![allow(clippy::doc_markdown)]

use std::path::Path;
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
        self.inner
            .lock()
            .expect("event collector lock")
            .push(EventRow { name: event.metadata().name().to_owned(), fields: visitor.fields });
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
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
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

    let result = run_server_with_obs_and_driver(config, obs, driver, owner_port, wiring).await;
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
#[ignore = "pending DELIVER step for GH #295 composed shared-network startup gate"]
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
#[ignore = "pending DELIVER step for GH #295 composed shared-network startup gate"]
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
#[ignore = "pending DELIVER step for GH #295 real host-owner startup and D9 bridge binding"]
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
#[ignore = "pending DELIVER step for GH #295 typed stale-attachment boot sweep"]
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
