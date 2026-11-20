//! Node-local persistent volume (GH #348, ADR-0313, ADR-0314): the volume pool
//! boot gate of a real in-process `overdrive serve`, in a guest.
//!
//! Driving path: `serve::run_with_adapters` over the `NodeAdapters` bundle, whose
//! `VmAdapters.volume_pool` (the driven probe port) is the only thing a test
//! replaces. A present-but-broken pool refuses startup whatever the node's VM
//! capability is (the bundle's `Optional`, as `overdrive serve` runs); a node with
//! no labelled partition boots and says so; a healthy pool boots without a refusal.
//! Every observation is the boot's result and its structured events.
//!
//! Evidence tier: Tier 3, a real in-process serve in a guest, example-only
//! (Mandate 11); one named test per broken-pool scenario of ADR-0314.

#![allow(clippy::doc_markdown, reason = "the CONTRACT_SHAPE line is machine-read verbatim")]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
#[cfg(feature = "kvm-tests")]
use std::time::Duration;

#[cfg(feature = "kvm-tests")]
use overdrive_cli::commands::deploy::{DeployArgs, deploy};
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::volume_backing::{
    VolumePoolProbe, VolumeProbeDetail, VolumeProbeError, VolumeProbeScenario,
};
#[cfg(feature = "kvm-tests")]
use overdrive_sim::SimVmm;
use overdrive_sim::{SimVolumeBacking, SimVolumePoolProbe};
use overdrive_testing::overdrive_test;
#[cfg(feature = "kvm-tests")]
use overdrive_testing::vm_fixture::VmFixture;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

// ---- tracing event capture (local copy of the canonical shape) ----

#[derive(Default)]
struct CapturedFields {
    map: std::collections::BTreeMap<String, String>,
}

impl Visit for CapturedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let val = format!("{value:?}");
        self.map.insert(field.name().to_owned(), val.trim_matches('"').to_owned());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.map.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Debug, Clone)]
struct EventRow {
    name: String,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct EventCollector {
    inner: Arc<Mutex<Vec<EventRow>>>,
}

impl EventCollector {
    fn snapshot(&self) -> Vec<EventRow> {
        self.inner.lock().expect("collector lock").clone()
    }

    fn named(&self, name: &str) -> Vec<EventRow> {
        self.snapshot().into_iter().filter(|event| event.name == name).collect()
    }
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = CapturedFields::default();
        event.record(&mut fields);
        self.inner
            .lock()
            .expect("collector lock")
            .push(EventRow { name: event.metadata().name().to_owned(), fields: fields.map });
    }
}

/// Boots `serve` over `probe`; the events are those of this thread's boot.
async fn boot(
    probe: Arc<dyn VolumePoolProbe>,
) -> (
    Result<overdrive_cli::commands::serve::ServeHandle, overdrive_cli::http_client::CliError>,
    EventCollector,
    TempDir,
) {
    let tmp = TempDir::new().expect("tempdir");
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);
    let result = super::serve_fixture::serve_with_volume_pool(args, probe).await;
    (result, collector, tmp)
}

fn broken(scenario: VolumeProbeScenario, detail: VolumeProbeDetail) -> Arc<dyn VolumePoolProbe> {
    Arc::new(SimVolumePoolProbe::failing(VolumeProbeError { scenario, detail }))
}

/// The boot refuses, naming the volume pool probe: the error the operator reads and the
/// `health.startup.refused` event (reason `volume_backing.probe`, cause = the probe
/// error's own words) both carry the words of the scenario's typed detail.
/// `row` names the table row in every failure message.
async fn assert_refused(row: &str, scenario: VolumeProbeScenario, typed_detail: VolumeProbeDetail) {
    let detail = typed_detail.to_string();
    let detail = detail.as_str();
    let (result, events, _tmp) = boot(broken(scenario, typed_detail)).await;
    let Err(err) = result else {
        panic!("a present but broken volume pool must refuse startup ({row})")
    };
    let rendered = err.to_string();
    assert!(
        rendered.contains("volume pool probe failed") && rendered.contains(detail),
        "row {row}: the refusal must name the volume pool probe and its cause: {rendered}"
    );
    let refused = events.named("health.startup.refused");
    assert!(
        refused.iter().any(|event| {
            event.fields.get("reason").is_some_and(|reason| reason == "volume_backing.probe")
                && event.fields.get("cause").is_some_and(|cause| cause.contains(detail))
        }),
        "row {row}: a health.startup.refused event with reason volume_backing.probe and the cause \
         is required; events: {refused:?}"
    );
}

/// ADR-0314 scenarios 1 to 4, one table, each row one real in-process serve run in turn: the
/// `lvm` tools or device-mapper are unusable (exit 4); the volume group is missing (exit 5); the
/// scratch volume could not be created or its node never appeared; the scratch volume could not
/// be removed (no exit status). Every row's failure message names the row.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "current_thread")]
#[should_panic(
    expected = "a present but broken volume pool must refuse startup (tools_or_device_mapper_unusable)"
)]
async fn sad_a_broken_volume_pool_refuses_startup() {
    let rows = [
        (
            "tools_or_device_mapper_unusable",
            VolumeProbeScenario::ToolsOrDeviceMapperUnusable,
            VolumeProbeDetail::CommandFailed {
                exit_status: Some(4),
                stderr: "no system dir".to_owned(),
            },
        ),
        (
            "volume_group_missing",
            VolumeProbeScenario::VolumeGroupMissing,
            VolumeProbeDetail::CommandFailed {
                exit_status: Some(5),
                stderr: "volume group overdrive not found".to_owned(),
            },
        ),
        (
            "scratch_create_or_node_failed",
            VolumeProbeScenario::ScratchCreateOrNodeFailed,
            VolumeProbeDetail::NodeMissing {
                path: std::path::PathBuf::from("/dev/mapper/overdrive-ovd--probe--scratch"),
            },
        ),
        (
            "scratch_remove_failed",
            VolumeProbeScenario::ScratchRemoveFailed,
            VolumeProbeDetail::CommandFailed {
                exit_status: None,
                stderr: "lvremove killed by a signal".to_owned(),
            },
        ),
    ];
    for (row, scenario, detail) in rows {
        assert_refused(row, scenario, detail).await;
    }
}

/// Boundary 6 (feature disabled): a node with no labelled partition boots, composing no
/// pool, and says so with the soft `volume_backing.not_composed` event, not a refusal.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "the boot must log volume_backing.not_composed")]
async fn a_node_with_no_labelled_partition_boots_and_logs_not_composed() {
    let (result, events, _tmp) = boot(Arc::new(SimVolumePoolProbe::absent())).await;
    let handle = result.expect("an absent pool is the soft case, not a refusal");
    assert!(
        !events.named("volume_backing.not_composed").is_empty(),
        "the boot must log volume_backing.not_composed"
    );
    assert!(events.named("health.startup.refused").is_empty());
    drop(handle);
}

/// A healthy pool boots with no refusal and no `not_composed` event. A GUARD: the boot scaffold
/// already passes a probed pool through without either event, and the boot gate must keep doing
/// so; that the pool reaches the VM driver exactly once is the launch tests' observation, not
/// this boot's.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "current_thread")]
async fn guard_a_healthy_pool_boots_without_a_refusal_or_a_not_composed_event() {
    let backing = SimVolumeBacking::with_capacity(volume_size_gib(1024));
    let (result, events, _tmp) = boot(Arc::new(SimVolumePoolProbe::composed(backing))).await;
    let handle = result.expect("a healthy pool boots");
    assert!(events.named("health.startup.refused").is_empty());
    assert!(events.named("volume_backing.not_composed").is_empty());
    drop(handle);
}

/// A `Vmm` over `SimVmm` that records the volume attachment of every `create`, so the
/// composition test observes what production handed the VMM (`VmConfig.volume`) without
/// installing anything production does not.
#[cfg(feature = "kvm-tests")]
struct VolumeRecordingVmm {
    inner: SimVmm,
    attached: Arc<Mutex<Vec<Option<overdrive_core::vm::config::VolumeAttachment>>>>,
}

#[cfg(feature = "kvm-tests")]
#[async_trait::async_trait]
impl overdrive_core::traits::vmm::Vmm for VolumeRecordingVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), overdrive_core::traits::vmm::VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(
        &self,
        config: &overdrive_core::vm::config::VmConfig,
        cid: overdrive_core::guest_transport::GuestCid,
    ) -> overdrive_core::traits::vmm::Result<overdrive_core::traits::vmm::VmProcess> {
        self.attached.lock().expect("attached lock").push(config.volume.clone());
        self.inner.create(config, cid).await
    }

    async fn terminate(
        &self,
        control: &overdrive_core::traits::vmm::VmControl,
        grace: Duration,
    ) -> overdrive_core::traits::vmm::Result<overdrive_core::traits::vmm::VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// The production composition end to end (H2): `run_server` composes the probed pool
/// (`compose_volume_backing`), hands it to the VM driver (`compose_vm_driver`, `VmDriver::new`),
/// and a launch of a `[vm.volume]` workload ensures its volume through that same pool. Nothing is
/// installed that production does not install: the pool is the probe's, the workload arrives
/// through `deploy`. Observed: the VMM's `create` carried the volume attachment, the device the
/// composed backing ensured, with the fixed serial `ovd-volume`.
/// The pool is the probe's own (simulated), so no LVM pool is needed; the VM fixture needs a
/// nested-launch guest, so the test is `kvm-tests`-gated like every test that provisions it.
/// CONTRACT_SHAPE: bounded-change.
#[cfg(feature = "kvm-tests")]
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "one workload, one volume")]
async fn a_deployed_volume_workload_launches_through_the_composed_volume_pool() {
    let fixture = VmFixture::provision(&overdrive_testing::vm_fixture::default_staging_root())
        .expect("provision the shared VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("vm-volume-compose-")
        .tempdir_in(overdrive_testing::vm_fixture::default_staging_root())
        .expect("tempdir on the staging root");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let backing = SimVolumeBacking::with_capacity(volume_size_gib(1024));
    let attached = Arc::new(Mutex::new(Vec::new()));
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir.clone());
    let handle = super::serve_fixture::serve_with_vm_and_volume_pool(
        args,
        super::serve_fixture::VmPorts {
            vmm: Arc::new(VolumeRecordingVmm {
                inner: SimVmm::new(),
                attached: Arc::clone(&attached),
            }),
            beacon: Arc::new(overdrive_host::VsockBeaconIo::new()),
        },
        Arc::new(SimVolumePoolProbe::composed(backing.clone())),
    )
    .await
    .expect("serve must start with a composed volume pool");

    let spec = tmp.path().join("vol-compose.toml");
    std::fs::write(
        &spec,
        format!(
            "[job]\nid = \"vol-compose\"\n\n[vm]\ncommand = \"/sbin/true\"\nargs = []\nkernel = \"{}\"\nrootfs = \"{}\"\n\n[vm.volume]\nsize = \"1GiB\"\n\n[resources]\ncpu_milli = 500\nmemory_bytes = 134217728\n",
            fixture.kernel_path.display(),
            fixture.rootfs_path.display(),
        ),
    )
    .expect("write spec");
    let cfg = config_dir.join(".overdrive").join("config");
    deploy(DeployArgs { spec, config_path: cfg }).await.unwrap_or_else(|error| {
        panic!("a VM workload that declares [vm.volume] must deploy (vol-compose): {error}")
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while attached.lock().expect("attached lock").is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the launch must create the VMM through the composed pool"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(backing.volumes_created(), 1, "one workload, one volume");
    let attachments = attached.lock().expect("attached lock").clone();
    let volume = attachments
        .first()
        .and_then(Option::as_ref)
        .expect("the VMM must be created with the volume attachment the pool ensured");
    assert!(
        volume.device.starts_with("/dev/mapper/overdrive-ovd--"),
        "the attachment is the ensured volume's device node, got {}",
        volume.device.display()
    );
    handle.shutdown().await.expect("shutdown");
}
