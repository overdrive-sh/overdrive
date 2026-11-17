//! Family `vm-lifecycle` (DDD-11): the host-observable stages of a VM workload's lifecycle,
//! from the operator's deploy call to the cleanup calls after a stop.
//!
//! One replicate boots one in-process `serve` (the operator driving port of `overdrive-cli`)
//! over the production adapters, with three instruments at existing public seams: a
//! timestamping `BeaconIo` decorator (the guest's `READY` line), a `Vmm`
//! decorator (the process id to guest CID join, and the guest console log read when the
//! exit watch fires), and a `tracing` layer for the named
//! lifecycle events; the observation store's subscription supplies the `Stable` row. Every
//! stamp is taken on the host's `CLOCK_MONOTONIC_RAW` (DDD-10). A stage that does not
//! complete is a recorded failure and the stages after it are absent, never zero.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::future::join_all;
use overdrive_bench::{HostClock, HostInstant, Replicate, ReplicateFailure};
use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, run_with_adapters};
use overdrive_control_plane::{NodeAdapters, VmCapability};
use overdrive_core::guest_transport::{GuestCid, NodeVsockPortConfig};
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, SubscriptionEvent,
};
use overdrive_core::traits::vmm::{
    Result as VmmResult, VmControl, VmExitWatch, VmProcess, VmTermination, Vmm, VmmProbeError,
};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::vm::beacon::{BeaconConnKey, BeaconIo, BeaconSocket};
use overdrive_core::vm::config::{VmConfig, VsockPort};
use overdrive_host::{CloudHypervisorVmm, RealCgroupFs, VsockBeaconIo};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::{SimDataplaneComposer, SimKek};
use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::{GuestImage, GuestImageSpec, GuestKit};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing_macros::overdrive_bench;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;

const LISTEN_PORT: u16 = 18081;
const VM_MEMORY_BYTES: u64 = 134_217_728;
const VM_CPU_MILLI: u32 = 500;
const STABLE_BOUND: Duration = Duration::from_secs(90);
const CLEANUP_BOUND: Duration = Duration::from_secs(40);
const POLL: Duration = Duration::from_millis(20);
const STOP_GRACE_PROFILE: &str = "stop-grace-deadline";
const GUEST_RECEIVED_STAGE: &str = "shutdown-received";
const GUEST_COMPLETE_STAGE: &str = "group-complete";
const GUEST_SHUTDOWN_METRIC: &str = "guest_shutdown_handling_ns";

/// The eight host-stage metrics in stage order; the stage's start boundary is the previous
/// stage's end boundary (DDD-11 stage table).
const STAGE_METRICS: [&str; 8] = [
    "admission_ns",
    "vmm_launch_ns",
    "guest_ready_ns",
    "stable_ns",
    "stop_submission_ns",
    "shutdown_delivery_ns",
    "guest_exit_ns",
    "cleanup_calls_ns",
];

#[overdrive_bench(
    family = "vm-lifecycle",
    substrate = native,
    arms = ["lifecycle"],
    replicates = 30,
    profile(name = "healthy-concurrency-1", preparation = "one workload is deployed and stopped per replicate"),
    profile(name = "healthy-concurrency-4", preparation = "four workloads are deployed together and stopped together per replicate"),
    profile(name = "cold-cache", preparation = "page cache dropped before each replicate"),
    profile(name = "warm-cache", preparation = "image pre-read"),
    profile(name = "stop-grace-deadline", preparation = "workload ignores the stop signal and exits only when the guest stop grace ends"),
    metric(name = "admission_ns", unit = ns, table = "host-stages", series),
    metric(name = "vmm_launch_ns", unit = ns, table = "host-stages", series),
    metric(name = "guest_ready_ns", unit = ns, table = "host-stages", series),
    metric(name = "stable_ns", unit = ns, table = "host-stages", series),
    metric(name = "stop_submission_ns", unit = ns, table = "host-stages", series),
    metric(name = "shutdown_delivery_ns", unit = ns, table = "host-stages", series),
    metric(name = "guest_exit_ns", unit = ns, table = "host-stages", series),
    metric(name = "cleanup_calls_ns", unit = ns, table = "host-stages", series),
    metric(name = "guest_shutdown_handling_ns", unit = ns, table = "guest-stages", series),
    smoke(guest = "cgroup-default", profile = "nested"),
)]
fn vm_lifecycle_stages(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let shape = Shape::of(rep.profile())
        .ok_or_else(|| fail(&format!("profile `{}` has no shape", rep.profile())))?;
    install_tracing()?;
    let _umask = UmaskGuard::set(0o022);
    let image = image()?;
    let trace = Arc::new(Trace::default());
    activate_sink(&trace);
    let outcome = replicate(rep, shape, image, &trace);
    deactivate_sink();
    let cycles = outcome?;
    record_cycles(rep, &cycles)
}

fn fail(message: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(message)
}

/// What a profile varies.
#[derive(Clone, Copy)]
struct Shape {
    concurrency: usize,
    cache: Cache,
    ignores_stop: bool,
}

#[derive(Clone, Copy)]
enum Cache {
    Unprepared,
    Cold,
    Warm,
}

impl Shape {
    fn of(profile: &str) -> Option<Self> {
        let (concurrency, cache) = match profile {
            "healthy-concurrency-1" | STOP_GRACE_PROFILE => (1, Cache::Unprepared),
            "healthy-concurrency-4" => (4, Cache::Unprepared),
            "cold-cache" => (1, Cache::Cold),
            "warm-cache" => (1, Cache::Warm),
            _ => return None,
        };
        Some(Self { concurrency, cache, ignores_stop: profile == STOP_GRACE_PROFILE })
    }

    const fn cache_state(self) -> &'static str {
        match self.cache {
            Cache::Unprepared => {
                "unprepared: the image was built or reused with no cache preparation"
            }
            Cache::Cold => "cold: page cache dropped before the replicate",
            Cache::Warm => "warm: image read through before the replicate",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Guest image fixture
// ---------------------------------------------------------------------------------------------

struct Fixture {
    kit: GuestKit,
    image: GuestImage,
}

/// The guest kit's workload image, built once per process under the staging root (the
/// image directory is reused across runs and replaced when rebuilt).
fn image() -> Result<&'static Fixture, ReplicateFailure> {
    static IMAGE: OnceLock<Result<Fixture, String>> = OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let staging = default_staging_root();
            let kit = GuestKit::provision(&staging).map_err(|error| {
                format!(
                    "the guest kit is not ready ({error}); run `cargo xtask kmod-build` and `cargo xtask guest-kit-prepare`"
                )
            })?;
            let directory = staging.join("nbp-vm-lifecycle");
            std::fs::create_dir_all(&directory)
                .map_err(|error| format!("creating {}: {error}", directory.display()))?;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("opening {}: {error}", directory.display()))?;
            let image = kit
                .image(&directory, "workload", &GuestImageSpec::workload())
                .map_err(|error| format!("building the workload image: {error}"))?;
            Ok(Fixture { kit, image })
        })
        .as_ref()
        .map_err(|message| fail(message))
}

/// Restores the process umask on drop. The Metal runner's umask is 027 and `serve` runs under
/// 022; the confined VMM identity traverses every directory it must cross by name.
struct UmaskGuard(libc::mode_t);

impl UmaskGuard {
    fn set(mask: libc::mode_t) -> Self {
        // SAFETY: `umask` has no preconditions and cannot fail.
        Self(unsafe { libc::umask(mask) })
    }
}

impl Drop for UmaskGuard {
    fn drop(&mut self) {
        // SAFETY: `umask` has no preconditions and cannot fail.
        unsafe { libc::umask(self.0) };
    }
}

// ---------------------------------------------------------------------------------------------
// Stamps
// ---------------------------------------------------------------------------------------------

struct Captured {
    name: &'static str,
    at: HostInstant,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct TraceData {
    events: Vec<Captured>,
    cid_of_pid: BTreeMap<u32, u32>,
    cid_of_socket: BTreeMap<u64, u32>,
    ready: BTreeMap<u32, HostInstant>,
    workload_of_alloc: BTreeMap<String, String>,
    stable: BTreeMap<String, HostInstant>,
    guest_shutdown: BTreeMap<u32, Result<f64, String>>,
    lagged: bool,
}

#[derive(Default)]
struct Trace {
    data: Mutex<TraceData>,
}

impl Trace {
    fn lock(&self) -> MutexGuard<'_, TraceData> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn event(
        &self,
        name: &str,
        matching: impl Fn(&BTreeMap<String, String>) -> bool,
    ) -> Option<HostInstant> {
        let data = self.lock();
        data.events
            .iter()
            .find(|event| event.name == name && matching(&event.fields))
            .map(|event| event.at)
    }

    fn alloc_event(&self, name: &str, alloc: &str) -> Option<HostInstant> {
        self.event(name, |fields| fields.get("alloc").is_some_and(|value| value == alloc))
    }

    fn alloc_of(&self, workload: &str) -> Option<String> {
        let data = self.lock();
        data.workload_of_alloc
            .iter()
            .find(|(_, candidate)| candidate.as_str() == workload)
            .map(|(alloc, _)| alloc.clone())
    }

    fn pid_of(&self, alloc: &str) -> Option<u32> {
        let data = self.lock();
        data.events
            .iter()
            .find(|event| {
                event.name == "vm.lifecycle.created"
                    && event.fields.get("alloc").is_some_and(|value| value == alloc)
            })
            .and_then(|event| event.fields.get("pid"))
            .and_then(|pid| pid.parse().ok())
    }

    fn row(
        &self,
        row: &overdrive_core::traits::observation_store::AllocStatusRow,
        now: HostInstant,
    ) {
        let mut data = self.lock();
        let workload = row.workload_id.to_string();
        data.workload_of_alloc.insert(row.alloc_id.to_string(), workload.clone());
        if matches!(row.terminal, Some(TerminalCondition::Stable { .. })) {
            data.stable.entry(workload).or_insert(now);
        }
    }
}

static SINK: Mutex<Option<Arc<Trace>>> = Mutex::new(None);

fn activate_sink(trace: &Arc<Trace>) {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(trace));
}

fn deactivate_sink() {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

const LIFECYCLE_EVENTS: [&str; 7] = [
    "vm.lifecycle.create_enter",
    "vm.lifecycle.created",
    "vm.lifecycle.stop_enter",
    "vm.lifecycle.writer_finished",
    "vmm.process.reaped",
    "vm.lifecycle.cleanup_calls_finished",
    "convergence.evaluation.admitted",
];

/// The process-global layer: it stamps the named lifecycle events into the active sink.
struct LifecycleLayer;

#[derive(Default)]
struct FieldVisitor(BTreeMap<String, String>);

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

impl<S: Subscriber> Layer<S> for LifecycleLayer {
    fn on_event(&self, event: &Event<'_>, _context: tracing_subscriber::layer::Context<'_, S>) {
        let name = event.metadata().name();
        let Some(name) = LIFECYCLE_EVENTS.iter().copied().find(|known| *known == name) else {
            return;
        };
        let at = HostClock.now();
        let sink = SINK.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let Some(trace) = sink else {
            return;
        };
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        trace.lock().events.push(Captured { name, at, fields: visitor.0 });
    }
}

/// Installs the layer once per process (`set_global_default` succeeds once).
fn install_tracing() -> Result<(), ReplicateFailure> {
    static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
    let installed = INSTALLED.get_or_init(|| {
        tracing::subscriber::set_global_default(tracing_subscriber::registry().with(LifecycleLayer))
            .map_err(|error| format!("the lifecycle tracing layer could not be installed: {error}"))
    });
    match installed {
        Ok(()) => Ok(()),
        Err(message) => Err(fail(message)),
    }
}

/// Stamps the guest's `READY` line as the driver receives it. The guest `EXIT` line is not a
/// boundary: on an operator stop the driver closes the beacon after delivering `SHUTDOWN`.
struct StampingBeacon {
    inner: VsockBeaconIo,
    trace: Arc<Trace>,
}

#[async_trait]
impl BeaconIo for StampingBeacon {
    async fn bind(
        &self,
        port: overdrive_core::guest_transport::VsockListenPort,
    ) -> Result<VsockPort, std::io::Error> {
        self.inner.bind(port).await
    }

    async fn accept(&self) -> Result<(BeaconSocket, BeaconConnKey), std::io::Error> {
        let (socket, key) = self.inner.accept().await?;
        self.trace.lock().cid_of_socket.insert(socket.raw(), key.peer_cid.get());
        Ok((socket, key))
    }

    async fn read_line(&self, socket: &BeaconSocket) -> Result<Option<String>, std::io::Error> {
        let line = self.inner.read_line(socket).await?;
        let at = HostClock.now();
        if let Some(text) = line.as_deref() {
            let mut data = self.trace.lock();
            if let Some(cid) = data.cid_of_socket.get(&socket.raw()).copied()
                && text.starts_with("READY")
            {
                data.ready.entry(cid).or_insert(at);
            }
        }
        Ok(line)
    }

    async fn write_line(&self, socket: &BeaconSocket, line: &str) -> Result<(), std::io::Error> {
        self.inner.write_line(socket, line).await
    }

    async fn close(&self, socket: BeaconSocket) {
        self.inner.close(socket).await;
    }
}

/// Joins each VMM process id to its guest CID; otherwise the production VMM.
struct JoiningVmm {
    inner: CloudHypervisorVmm,
    trace: Arc<Trace>,
}

#[async_trait]
impl Vmm for JoiningVmm {
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    async fn probe(&self) -> Result<(), VmmProbeError> {
        self.inner.probe().await
    }

    async fn create(&self, config: &VmConfig, cid: GuestCid) -> VmmResult<VmProcess> {
        let mut process = self.inner.create(config, cid).await?;
        let pid = process.control.pid;
        self.trace.lock().cid_of_pid.insert(pid, cid.get());
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut inner_exit = std::mem::replace(&mut process.exit, VmExitWatch::new(receiver));
        let console = config.run_dir.console_log();
        let trace = Arc::clone(&self.trace);
        // The run directory is removed after the exit is forwarded, so the console log is read
        // first; the exit is then forwarded unchanged.
        tokio::spawn(async move {
            let exit = inner_exit.recv().await;
            let reading = read_guest_shutdown(&console).await;
            trace.lock().guest_shutdown.insert(pid, reading);
            if let Some(exit) = exit
                && sender.send(exit).is_err()
            {
                tracing::debug!("the exit watch receiver was dropped before the exit");
            }
        });
        Ok(process)
    }

    async fn terminate(&self, control: &VmControl, grace: Duration) -> VmmResult<VmTermination> {
        self.inner.terminate(control, grace).await
    }
}

/// The guest's shutdown-handling duration from the console log: `group-complete` minus
/// `shutdown-received`, each a 1 ms-resolution guest-clock reading, so the result is a guest
/// duration and is never combined with a host instant.
async fn read_guest_shutdown(console: &Path) -> Result<f64, String> {
    let text = tokio::fs::read_to_string(console)
        .await
        .map_err(|error| format!("the guest console log could not be read: {error}"))?;
    let received = guest_elapsed_ms(&text, GUEST_RECEIVED_STAGE)
        .ok_or_else(|| format!("the console log has no `{GUEST_RECEIVED_STAGE}` line"))?;
    let complete = guest_elapsed_ms(&text, GUEST_COMPLETE_STAGE)
        .ok_or_else(|| format!("the console log has no `{GUEST_COMPLETE_STAGE}` line"))?;
    match complete.checked_sub(received) {
        Some(milliseconds) if milliseconds > 0 =>
        {
            #[allow(clippy::cast_precision_loss, reason = "a guest duration is far below 2^52 ms")]
            Ok(milliseconds as f64 * 1_000_000.0)
        }
        _ => Err("the guest reported no shutdown handling time at 1 ms resolution".to_owned()),
    }
}

fn guest_elapsed_ms(text: &str, stage: &str) -> Option<u64> {
    let marker = format!("overdrive-init: {stage} elapsed_ms=");
    let at = text.find(&marker)? + marker.len();
    text[at..].split(|character: char| !character.is_ascii_digit()).next()?.parse().ok()
}

// ---------------------------------------------------------------------------------------------
// One replicate
// ---------------------------------------------------------------------------------------------

/// What one workload's deploy and stop produced.
struct Cycle {
    workload: String,
    deployed_at: HostInstant,
    stopped_at: Option<HostInstant>,
    deploy_error: Option<String>,
}

fn replicate(
    rep: &mut Replicate,
    shape: Shape,
    fixture: &Fixture,
    trace: &Arc<Trace>,
) -> Result<Vec<CycleOutcome>, ReplicateFailure> {
    prepare_cache(shape, fixture)?;
    rep.note("cache_state", shape.cache_state());
    rep.note(
        "vm_resources",
        &format!("cpu_milli={VM_CPU_MILLI} memory_bytes={VM_MEMORY_BYTES}, one vCPU-share per VM"),
    );
    rep.note("concurrency", &shape.concurrency.to_string());
    let version =
        rep.exec(&fixture.kit.cloud_hypervisor_bin.display().to_string(), &["--version"])?;
    rep.note("cloud_hypervisor_version", version.trim());
    let kernel = rep.exec("uname", &["-r"])?;
    rep.note("host_kernel", kernel.trim());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| fail(&format!("the benchmark runtime could not start: {error}")))?;
    runtime.block_on(drive(shape, fixture, trace))
}

fn prepare_cache(shape: Shape, fixture: &Fixture) -> Result<(), ReplicateFailure> {
    match shape.cache {
        Cache::Unprepared => Ok(()),
        Cache::Cold => {
            // SAFETY: `sync` has no preconditions and cannot fail.
            unsafe { libc::sync() };
            std::fs::write("/proc/sys/vm/drop_caches", "3")
                .map_err(|error| fail(&format!("dropping the page cache failed: {error}")))
        }
        Cache::Warm => {
            let mut file = std::fs::File::open(&fixture.image.rootfs)
                .map_err(|error| fail(&format!("opening the image failed: {error}")))?;
            std::io::copy(&mut file, &mut std::io::sink())
                .map(drop)
                .map_err(|error| fail(&format!("reading the image through failed: {error}")))
        }
    }
}

async fn drive(
    shape: Shape,
    fixture: &Fixture,
    trace: &Arc<Trace>,
) -> Result<Vec<CycleOutcome>, ReplicateFailure> {
    let staging = default_staging_root();
    let directory = tempfile::Builder::new()
        .prefix("nbp-vm-lifecycle-")
        .tempdir_in(&staging)
        .map_err(|error| fail(&format!("the node directory could not be created: {error}")))?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755))
        .map_err(|error| fail(&format!("opening the node directory failed: {error}")))?;
    let data_dir = directory.path().join("data");
    let config_dir = directory.path().join("conf");
    for path in [&data_dir, &config_dir] {
        std::fs::create_dir_all(path)
            .map_err(|error| fail(&format!("creating {}: {error}", path.display())))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| fail(&format!("opening {}: {error}", path.display())))?;
    }
    let args = ServeArgs {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        data_dir,
        config_dir,
        vsock_ports: NodeVsockPortConfig::kernel_assigned(),
        gateway_address: None,
        gateway_certified_key_id: None,
        gateway_certificate_chain: None,
        gateway_private_key: None,
    };
    let observation: Arc<OnceLock<Arc<dyn ObservationStore>>> = Arc::new(OnceLock::new());
    let (vmm_trace, beacon_trace, shared) =
        (Arc::clone(trace), Arc::clone(trace), Arc::clone(&observation));
    #[allow(
        clippy::result_large_err,
        reason = "the composition callback's error type is fixed by `run_with_adapters`"
    )]
    let server =
        run_with_adapters(args, Arc::new(RealCgroupFs::new()), move |config, filesystem| {
            let mut adapters =
                NodeAdapters::host(config, filesystem, Arc::new(SimKek::for_boot()))?;
            adapters.boot.dataplane = Arc::new(SimDataplaneComposer::new(
                Arc::new(SimDataplane::new()),
                Ipv4Addr::LOCALHOST,
            ));
            adapters.vm.vmm =
                Arc::new(JoiningVmm { inner: CloudHypervisorVmm::new(), trace: vmm_trace });
            adapters.vm.beacon =
                Arc::new(StampingBeacon { inner: VsockBeaconIo::new(), trace: beacon_trace });
            adapters.vm.capability = VmCapability::Required;
            shared.get_or_init(|| Arc::clone(&adapters.boot.observation));
            Ok(adapters)
        })
        .await
        .map_err(|error| fail(&format!("serve did not boot: {error}")))?;
    let store =
        observation.get().cloned().ok_or_else(|| fail("serve built no observation store"))?;
    let mut subscription = store
        .subscribe_all_events()
        .await
        .map_err(|error| fail(&format!("the observation subscription failed: {error}")))?;
    let watching = Arc::clone(trace);
    let watcher = tokio::spawn(async move {
        while let Some(event) = subscription.next().await {
            let at = HostClock.now();
            match event {
                SubscriptionEvent::Row(ObservationRow::AllocStatus(row)) => watching.row(&row, at),
                SubscriptionEvent::Row(_) => {}
                SubscriptionEvent::Lagged { .. } => watching.lock().lagged = true,
            }
        }
    });

    let config_path = directory.path().join("conf").join(".overdrive").join("config");
    let outcome = cycles(shape, fixture, trace, directory.path(), &config_path).await;
    watcher.abort();
    server.shutdown().await.map_err(|error| fail(&format!("serve did not shut down: {error}")))?;
    outcome
}

fn service_toml(id: &str, kernel: &Path, rootfs: &Path, command: &str) -> String {
    format!(
        "[service]\nid = \"{id}\"\nreplicas = 1\n\n[[listener]]\nport = {LISTEN_PORT}\nprotocol = \"tcp\"\n\n\
         [vm]\ncommand = \"{GUEST_PEER_GUEST_PATH}\"\nargs = [\"{command}\", \"{LISTEN_PORT}\", \"bench\"]\n\
         kernel = \"{}\"\nrootfs = \"{}\"\n\n[resources]\ncpu_milli = {VM_CPU_MILLI}\nmemory_bytes = {VM_MEMORY_BYTES}\n\n\
         [[health_check.startup]]\ntype = \"tcp\"\nhost = \"0.0.0.0\"\nport = {LISTEN_PORT}\n\
         interval_seconds = 1\ntimeout_seconds = 1\nmax_attempts = 30\n",
        kernel.display(),
        rootfs.display(),
    )
}

async fn cycles(
    shape: Shape,
    fixture: &Fixture,
    trace: &Arc<Trace>,
    directory: &Path,
    config_path: &Path,
) -> Result<Vec<CycleOutcome>, ReplicateFailure> {
    let command =
        if shape.ignores_stop { subcommand::LISTEN_IGNORING_STOP } else { subcommand::LISTEN };
    let mut specs: Vec<(String, PathBuf)> = Vec::with_capacity(shape.concurrency);
    for index in 0..shape.concurrency {
        let id = format!("nbp-vl-{index}");
        let spec = directory.join(format!("{id}.toml"));
        std::fs::write(
            &spec,
            service_toml(&id, &fixture.kit.kernel_path, &fixture.image.rootfs, command),
        )
        .map_err(|error| fail(&format!("writing {}: {error}", spec.display())))?;
        specs.push((id, spec));
    }
    let mut deployed = join_all(specs.into_iter().map(|(id, spec)| async move {
        let deployed_at = HostClock.now();
        let result = deploy(DeployArgs { spec, config_path: config_path.to_path_buf() }).await;
        Cycle {
            workload: id,
            deployed_at,
            stopped_at: None,
            deploy_error: result.err().map(|error| error.to_string()),
        }
    }))
    .await;

    wait_until(STABLE_BOUND, || {
        deployed
            .iter()
            .filter(|cycle| cycle.deploy_error.is_none())
            .all(|cycle| trace.lock().stable.contains_key(&cycle.workload))
    })
    .await;

    let stops = join_all(deployed.iter().filter(|cycle| cycle.deploy_error.is_none()).map(
        |cycle| async move {
            let stopped_at = HostClock.now();
            let result = stop(StopArgs {
                id: cycle.workload.clone(),
                config_path: config_path.to_path_buf(),
            })
            .await;
            (cycle.workload.clone(), stopped_at, result.err().map(|error| error.to_string()))
        },
    ))
    .await;
    for (workload, stopped_at, error) in stops {
        if let Some(cycle) = deployed.iter_mut().find(|cycle| cycle.workload == workload) {
            match error {
                None => cycle.stopped_at = Some(stopped_at),
                Some(message) => cycle.deploy_error = Some(format!("stop call failed: {message}")),
            }
        }
    }

    wait_until(CLEANUP_BOUND, || {
        deployed.iter().filter(|cycle| cycle.stopped_at.is_some()).all(|cycle| {
            trace.alloc_of(&cycle.workload).is_some_and(|alloc| {
                trace.alloc_event("vm.lifecycle.cleanup_calls_finished", &alloc).is_some()
            })
        })
    })
    .await;

    Ok(deployed.iter().map(|cycle| cycle_outcome(trace, cycle)).collect())
}

async fn wait_until(bound: Duration, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + bound;
    while !done() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(POLL).await;
    }
}

// ---------------------------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------------------------

/// One stage of one workload.
#[derive(Debug, PartialEq)]
enum Stage {
    Measured(f64),
    Failed(String),
    Absent(String),
}

/// The eight host stages of one workload, in stage order.
type StageOutcome = [Stage; 8];

/// What one workload produced: its eight host stages and, apart, the guest's own duration.
struct CycleOutcome {
    stages: StageOutcome,
    guest_shutdown: Result<f64, String>,
}

fn cycle_outcome(trace: &Trace, cycle: &Cycle) -> CycleOutcome {
    let guest_shutdown =
        trace.alloc_of(&cycle.workload).and_then(|alloc| trace.pid_of(&alloc)).map_or_else(
            || Err("the allocation's VMM process was not observed".to_owned()),
            |pid| {
                trace.lock().guest_shutdown.get(&pid).cloned().unwrap_or_else(|| {
                    Err("the VMM exit was not observed, so the console log was not read".to_owned())
                })
            },
        );
    CycleOutcome { stages: stage_outcome(trace, cycle), guest_shutdown }
}

/// The stamps of one workload's boundaries, in stage-table order: deploy entry, admitted,
/// created-enter, created, READY, Stable, stop entry, `stop_enter`, `writer_finished`,
/// reaped, cleanup.
fn boundaries(trace: &Trace, cycle: &Cycle) -> [Result<HostInstant, String>; 11] {
    let missing = |what: &str| Err(format!("{what} was not observed"));
    let workload = cycle.workload.as_str();
    let alloc = trace.alloc_of(workload);
    let alloc_event = |name: &str| {
        alloc.as_deref().map_or_else(
            || missing("the allocation row"),
            |alloc| trace.alloc_event(name, alloc).map_or_else(|| missing(name), Ok),
        )
    };
    let cid = alloc
        .as_deref()
        .and_then(|alloc| trace.pid_of(alloc))
        .and_then(|pid| trace.lock().cid_of_pid.get(&pid).copied());
    let guest_line = |select: fn(&TraceData) -> &BTreeMap<u32, HostInstant>, what: &str| {
        cid.and_then(|cid| select(&trace.lock()).get(&cid).copied())
            .map_or_else(|| missing(what), Ok)
    };
    let reaped = alloc
        .as_deref()
        .and_then(|alloc| trace.pid_of(alloc))
        .and_then(|pid| {
            trace.event("vmm.process.reaped", |fields| {
                fields.get("pid").is_some_and(|value| value == &pid.to_string())
            })
        })
        .map_or_else(|| missing("vmm.process.reaped"), Ok);
    let admitted = trace
        .event("convergence.evaluation.admitted", |fields| {
            fields.get("target").is_some_and(|target| target.ends_with(workload))
        })
        .map_or_else(|| missing("convergence.evaluation.admitted"), Ok);
    let stable = trace.lock().stable.get(workload).copied().map_or_else(
        || {
            Err(if trace.lock().lagged {
                "the Stable row was not observed (the subscription lagged)".to_owned()
            } else {
                "the Stable row was not observed".to_owned()
            })
        },
        Ok,
    );
    [
        Ok(cycle.deployed_at),
        admitted,
        alloc_event("vm.lifecycle.create_enter"),
        alloc_event("vm.lifecycle.created"),
        guest_line(|data| &data.ready, "the READY line"),
        stable,
        cycle.stopped_at.ok_or_else(|| "the stop call did not complete".to_owned()),
        alloc_event("vm.lifecycle.stop_enter"),
        alloc_event("vm.lifecycle.writer_finished"),
        reaped,
        alloc_event("vm.lifecycle.cleanup_calls_finished"),
    ]
}

/// Maps each stage to its (start, end) boundary index in [`boundaries`]. Stage 2 starts at
/// `create_enter` and stage 3 at `created`; stages 5 and 6 start at the stop call and at
/// `stop_enter`; stage 7 starts at `writer_finished`.
const STAGE_BOUNDARIES: [(usize, usize); 8] =
    [(0, 1), (2, 3), (3, 4), (4, 5), (6, 7), (7, 8), (8, 9), (9, 10)];

fn stage_outcome(trace: &Trace, cycle: &Cycle) -> StageOutcome {
    let stamps = boundaries(trace, cycle);
    let mut broken: Option<String> =
        cycle.deploy_error.as_ref().map(|error| format!("deploy or stop failed: {error}"));
    let mut outcome: Vec<Stage> = Vec::with_capacity(STAGE_METRICS.len());
    for (index, (start, end)) in STAGE_BOUNDARIES.iter().copied().enumerate() {
        if let Some(reason) = &broken {
            outcome.push(Stage::Absent(format!("an earlier stage did not complete: {reason}")));
            continue;
        }
        match (&stamps[start], &stamps[end]) {
            (Ok(from), Ok(to)) => {
                let elapsed = to.duration_since(*from);
                if elapsed.is_zero() {
                    let reason =
                        format!("stage `{}` boundaries were out of order", STAGE_METRICS[index]);
                    broken = Some(reason.clone());
                    outcome.push(Stage::Failed(reason));
                } else {
                    #[allow(clippy::cast_precision_loss, reason = "a stage is far below 2^52 ns")]
                    outcome.push(Stage::Measured(elapsed.as_nanos() as f64));
                }
            }
            (Err(reason), _) | (_, Err(reason)) => {
                broken = Some(reason.clone());
                outcome.push(Stage::Failed(reason.clone()));
            }
        }
    }
    outcome.try_into().unwrap_or_else(|_| unreachable!("one stage per stage metric"))
}

/// Records each metric of the replicate: the measured values as a series, or, when any
/// workload's stage failed, each failure; a metric no workload reached is absent.
fn record_cycles(rep: &mut Replicate, cycles: &[CycleOutcome]) -> Result<(), ReplicateFailure> {
    for (index, metric) in STAGE_METRICS.iter().enumerate() {
        let stages: Vec<&Stage> = cycles.iter().map(|cycle| &cycle.stages[index]).collect();
        let failures: Vec<&String> = stages
            .iter()
            .filter_map(|stage| if let Stage::Failed(reason) = stage { Some(reason) } else { None })
            .collect();
        if !failures.is_empty() {
            for reason in failures {
                rep.record_failure(metric, reason)?;
            }
            continue;
        }
        let values: Vec<f64> = stages
            .iter()
            .filter_map(
                |stage| if let Stage::Measured(value) = stage { Some(*value) } else { None },
            )
            .collect();
        if values.is_empty() {
            let reason = stages.iter().find_map(|stage| {
                if let Stage::Absent(reason) = stage { Some(reason.as_str()) } else { None }
            });
            rep.record_absent(metric, reason.unwrap_or("no workload reached this stage"))?;
        } else {
            rep.record_series(metric, &values)?;
        }
    }
    let guest: Vec<f64> =
        cycles.iter().filter_map(|cycle| cycle.guest_shutdown.as_ref().ok().copied()).collect();
    if guest.is_empty() {
        let reason = cycles.iter().find_map(|cycle| cycle.guest_shutdown.as_ref().err());
        rep.record_absent(
            GUEST_SHUTDOWN_METRIC,
            reason.map_or("no workload reached the guest shutdown", String::as_str),
        )
    } else {
        rep.record_series(GUEST_SHUTDOWN_METRIC, &guest)
    }
}
