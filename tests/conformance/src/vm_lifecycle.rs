//! VM lifecycle-completion harness (a disposable guest of a Metal run).
//!
//! One [`VmLifecycleWorld`] is one production composition: `run_server(config,
//! NodeAdapters::host(..))` over the real cgroupfs, the real dataplane and real
//! Cloud Hypervisor microVMs. Only the production KEK is replaced by the
//! hermetic `SimKek`, as every in-process `serve` fixture of this repository
//! does. Workloads enter and are stopped through the server's public HTTPS API,
//! and are observed through the same API, the lifecycle tracing events the
//! production owners emit, and the host's own directories and process table.
//!
//! The guest is the checked-in `guest/lifecycle_guest.rs` staged into the guest
//! kit's workload image. The harness measures no latency: stage targets belong
//! to the `vm-lifecycle` benchmark family.

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, AllocStatusRowBody};
use overdrive_control_plane::{NodeAdapters, ServerConfig, ServerHandle, run_server};
use overdrive_core::TransitionReason;
use overdrive_core::guest_transport::NodeVsockPortConfig;
use overdrive_core::id::AllocationId;
use overdrive_core::transition_reason::StoppedBy;
use overdrive_core::vm::config::{RootfsPlan, VmRunDir, clone_index_dir, clone_staging_dir};
use overdrive_host::RealCgroupFs;
use overdrive_sim::adapters::SimKek;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit};
use overdrive_testing::vm_fixture::default_staging_root;
use tokio::task::JoinSet;

use crate::{
    CleanupObservation, PublicApi, TraceEvent, TraceHistory, assert_kvm_substrate, command_output,
    wait_until,
};

/// The operator command the VM runs; its argument selects the lifecycle shape.
pub const GUEST_COMMAND: &str = "/sbin/lifecycle-guest";

/// The port the cooperative Service listens on.
pub const SERVICE_PORT: u16 = 18_081;

/// The shutdown deadline of the one graceful stop of the server.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

/// How long a workload may take to reach Running.
const RUNNING_BOUND: Duration = Duration::from_secs(90);

/// How long a stopped workload may take to reach a terminal state.
const STOPPED_BOUND: Duration = Duration::from_secs(30);

/// How long a finite Job may take to run to its own terminal state.
const FINITE_BOUND: Duration = Duration::from_secs(90);

/// How long the driver's per-allocation artifacts may outlive the terminal row.
const ARTIFACT_BOUND: Duration = Duration::from_secs(10);

/// The cgroup directory every workload allocation scope lives under.
const WORKLOAD_SCOPES: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";

/// The directory the driver creates each allocation's run directory under.
const VM_RUN_ROOT: &str = "/run/overdrive/vm";

/// The three lifecycle shapes a launch can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// A long-lived Job: reaches Running, then the operator stops it.
    Ready,
    /// A finite, childless Job that exits `0` by itself.
    FiniteJob,
    /// A Service that exits `0` on the platform's stop request.
    CooperativeService,
}

impl Profile {
    /// Every profile, in the order the world exercises them.
    pub const ALL: [Self; 3] = [Self::Ready, Self::FiniteJob, Self::CooperativeService];

    /// A short name used in workload identifiers.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::FiniteJob => "job",
            Self::CooperativeService => "service",
        }
    }

    const fn guest_mode(self) -> &'static str {
        match self {
            Self::Ready => "spin",
            Self::FiniteJob => "exit0",
            Self::CooperativeService => "serve",
        }
    }

    /// Whether the operator stops the workload (a finite Job ends by itself).
    #[must_use]
    pub const fn is_stopped_by_operator(self) -> bool {
        !matches!(self, Self::FiniteJob)
    }
}

/// What one launch did, and every way it departed from a completed lifecycle.
#[derive(Clone, Debug)]
pub struct Launch {
    pub profile: Profile,
    pub ordinal: usize,
    pub workload_id: String,
    /// The allocation, once the public API reported one.
    pub alloc: Option<String>,
    /// Each departure from the lifecycle contract, in the order observed.
    pub failures: Vec<String>,
}

/// The state the world's launches share, cloneable into concurrent launches.
#[derive(Clone)]
struct Launcher {
    api: PublicApi,
    kernel: PathBuf,
    rootfs: PathBuf,
    data_dir: PathBuf,
    submitted: Arc<Mutex<Vec<String>>>,
}

/// One production composition that launches real VMs.
pub struct VmLifecycleWorld {
    _root: tempfile::TempDir,
    handle: Option<ServerHandle>,
    launcher: Launcher,
    trace: TraceHistory,
    cleanup: CleanupObservation,
}

impl VmLifecycleWorld {
    /// Compose the production server over the real adapters, staging the guest
    /// image first.
    pub async fn boot(trace: &TraceHistory) -> Self {
        assert_kvm_substrate();
        overdrive_testing::forwarder_module::expect_forwarder_module();
        // The Metal runner's umask (0027) would create the server's `data/vm`
        // chain `0750 root:root`, closed to the confined VMM gid; the
        // production process runs under the conventional 022.
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o022));
        // The serve data directory lives on the reflink-capable staging
        // filesystem: each per-launch rootfs clone is intra-filesystem.
        let root = tempfile::Builder::new()
            .prefix("vm-lifecycle-conformance-")
            .tempdir_in(default_staging_root())
            .expect("conformance directory on the staging filesystem");
        // The uid-dropped VMM (gid 4200) reaches its rootfs clone beneath this
        // directory, so it carries the traverse grant its staging root has.
        std::os::unix::fs::chown(root.path(), Some(0), Some(4200))
            .expect("grant the confined gid the conformance directory");
        tokio::fs::set_permissions(root.path(), Permissions::from_mode(0o710))
            .await
            .expect("traverse mode on the conformance directory");
        let data_dir = root.path().join("data");
        let config_dir = root.path().join("conf");
        tokio::fs::create_dir_all(&data_dir).await.expect("create the data dir");
        tokio::fs::create_dir_all(&config_dir).await.expect("create the operator config dir");
        let (kernel, rootfs) = stage_guest(root.path());
        // Everything a launch may create on the host, captured before the
        // server starts: none of it may outlive the last launch.
        let cleanup = CleanupObservation::capture(&[
            Path::new(WORKLOAD_SCOPES),
            Path::new(VM_RUN_ROOT),
            &clone_staging_dir(&data_dir),
            &clone_index_dir(&data_dir),
        ]);
        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir: data_dir.clone(),
            operator_config_dir: config_dir.clone(),
            vsock_ports: NodeVsockPortConfig::kernel_assigned(),
            ..ServerConfig::new()
        };
        let adapters = NodeAdapters::host(
            &config,
            Arc::new(RealCgroupFs::new()),
            Arc::new(SimKek::for_boot()),
            1_048_576,
        )
        .expect("the host bundle opens the stores");
        let handle = run_server(config, adapters).await.expect("start the production server");
        let api = PublicApi::from_trust_config(&config_dir.join(".overdrive/config"))
            .expect("load the server trust triple");
        Self {
            _root: root,
            handle: Some(handle),
            launcher: Launcher { api, kernel, rootfs, data_dir, submitted: Arc::default() },
            trace: trace.clone(),
            cleanup,
        }
    }

    /// Launch `count` workloads of `profile` one after another, ordinals
    /// `first..first + count`.
    pub async fn launch_sequentially(
        &self,
        profile: Profile,
        first: usize,
        count: usize,
    ) -> Vec<Launch> {
        let mut launches = Vec::with_capacity(count);
        for ordinal in first..first + count {
            launches.push(self.launcher.clone().launch(profile, ordinal).await);
        }
        launches
    }

    /// Launch `count` workloads of `profile` at once, ordinals
    /// `first..first + count`. Every launch is joined and reported, whatever
    /// the others did: a failure never truncates the result.
    pub async fn launch_concurrently(
        &self,
        profile: Profile,
        first: usize,
        count: usize,
    ) -> Vec<Launch> {
        let mut set = JoinSet::new();
        for ordinal in first..first + count {
            set.spawn(self.launcher.clone().launch(profile, ordinal));
        }
        let mut launches = Vec::with_capacity(count);
        while let Some(joined) = set.join_next().await {
            launches.push(joined.expect("a launch task completes without panicking"));
        }
        launches.sort_by_key(|launch| launch.ordinal);
        launches
    }

    /// The lifecycle events the production owners emitted for `launches`, and
    /// the host's process table, checked against the lifecycle contract.
    ///
    /// Returns each departure: a missing event, a VMM that did not exit
    /// normally, or a VMM process still present.
    #[must_use]
    pub fn audit_events(&self, launches: &[Launch]) -> Vec<String> {
        let events = self.trace.snapshot();
        let mut departures = Vec::new();
        for launch in launches {
            let Some(alloc) = &launch.alloc else { continue };
            audit_launch(&events, launch, alloc, &mut departures);
        }
        departures
    }

    /// Shut the server down gracefully and wait until the host's links,
    /// namespaces, cgroup scopes, run directories and clone staging are as they
    /// were before the boot. Returns the shutdown's typed result.
    pub async fn tear_down(mut self) -> Result<(), String> {
        let handle = self.handle.take().expect("live server handle");
        let result = handle.shutdown(SHUTDOWN_DEADLINE).await.map_err(|error| error.to_string());
        let cleanup = self.cleanup.clone();
        tokio::task::spawn_blocking(move || {
            wait_until(
                Duration::from_secs(15),
                "host links, namespaces, scopes, run directories and clone staging restored",
                || cleanup.is_restored(),
            );
        })
        .await
        .expect("the cleanup observation completes");
        result
    }
}

impl Drop for VmLifecycleWorld {
    fn drop(&mut self) {
        // `ServerHandle` does not stop the server on drop, and a serve-booting
        // body that panics before `tear_down` leaves the DNS responder's
        // blocking thread alive, so the test runtime's teardown never returns
        // (GH #350). A world still holding its handle stops every workload it
        // submitted and shuts the server down here, on dedicated threads with
        // their own runtimes: the test runtime's workers keep running the
        // server's tasks while this thread waits.
        let Some(handle) = self.handle.take() else { return };
        let api = self.launcher.api.clone();
        let submitted = self.launcher.submitted.lock().expect("submitted ids lock").clone();
        let stopped = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            runtime.block_on(async move {
                for id in submitted {
                    let _ = api
                        .post_json(&format!("/v1/workloads/{id}/stop"), &serde_json::json!({}))
                        .await;
                }
                // Terminal rows settle through the production stop path.
                tokio::time::sleep(Duration::from_secs(10)).await;
            });
        })
        .join();
        if stopped.is_err() {
            eprintln!("unwind cleanup: stopping the workloads failed; shutting down anyway");
        }
        let shut = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            runtime
                .block_on(handle.shutdown(SHUTDOWN_DEADLINE))
                .map_err(|error| format!("{error:?}"))
        })
        .join();
        match shut {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("unwind cleanup: server shutdown failed: {error}"),
            Err(_) => eprintln!("unwind cleanup: server shutdown panicked"),
        }
    }
}

/// Stage the guest the workloads boot: the guest kit's kernel and a workload
/// image carrying the production `overdrive-init` as PID 1, the kit's forwarder
/// module and the lifecycle guest.
fn stage_guest(work: &Path) -> (PathBuf, PathBuf) {
    let kit = GuestKit::provision(&default_staging_root())
        .expect("precondition: the guest kit provisions (run `cargo xtask guest-kit-prepare`)");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("guest/lifecycle_guest.rs");
    let binary = work.join("lifecycle-guest");
    let output = command_output(
        "rustc",
        &[
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
            "-C",
            "target-feature=+crt-static",
            "--target",
            "x86_64-unknown-linux-musl",
            "-o",
            &binary.to_string_lossy(),
            &source.to_string_lossy(),
        ],
    );
    assert!(
        output.status.success(),
        "rustc lifecycle guest failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut spec = GuestImageSpec::workload();
    spec.extra.push(GuestFile {
        host: binary,
        guest: PathBuf::from(GUEST_COMMAND),
        with_shared_libraries: false,
    });
    let image = kit.image(work, "vm-lifecycle", &spec).expect("the workload image");
    (kit.kernel_path, image.rootfs)
}

impl Launcher {
    fn spec(&self, profile: Profile, id: &str) -> serde_json::Value {
        let mut spec = serde_json::json!({
            "kind": if profile == Profile::CooperativeService { "service" } else { "job" },
            "id": id,
            "replicas": 1,
            "resources": { "cpu_milli": 500, "memory_bytes": 134_217_728_u64 },
            "vm": {
                "command": GUEST_COMMAND,
                "args": [profile.guest_mode()],
                "kernel": self.kernel,
                "rootfs": self.rootfs
            }
        });
        if profile == Profile::CooperativeService {
            spec["listeners"] = serde_json::json!([{ "port": SERVICE_PORT, "protocol": "tcp" }]);
            spec["startup_probes"] = serde_json::json!([]);
            spec["readiness_probes"] = serde_json::json!([]);
            spec["liveness_probes"] = serde_json::json!([]);
        }
        serde_json::json!({ "spec": spec })
    }

    async fn first_row(&self, id: &str) -> Option<AllocStatusRowBody> {
        let response = self.api.get(&format!("/v1/allocs?job={id}")).await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<AllocStatusResponse>().await.ok()?.rows.into_iter().next()
    }

    /// Poll the workload's allocation row until `reached` accepts it or `bound`
    /// expires. Returns whether it was reached, and the last row observed.
    async fn poll_row(
        &self,
        id: &str,
        bound: Duration,
        reached: impl Fn(&AllocStatusRowBody) -> bool + Send,
    ) -> (bool, Option<AllocStatusRowBody>) {
        let deadline = tokio::time::Instant::now() + bound;
        let mut last = None;
        loop {
            if let Some(row) = self.first_row(id).await {
                let done = reached(&row);
                last = Some(row);
                if done {
                    return (true, last);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return (false, last);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn launch(self, profile: Profile, ordinal: usize) -> Launch {
        let id = format!("lc-{}-{ordinal:03}", profile.slug());
        let mut launch =
            Launch { profile, ordinal, workload_id: id.clone(), alloc: None, failures: Vec::new() };
        let submitted = self.api.post_json("/v1/workloads", &self.spec(profile, &id)).await;
        match submitted {
            Ok(response) if response.status() == reqwest::StatusCode::OK => {
                self.submitted.lock().expect("submitted ids lock").push(id.clone());
            }
            Ok(response) => {
                launch.failures.push(format!("submit answered {}", response.status()));
                return launch;
            }
            Err(error) => {
                launch.failures.push(format!("submit failed: {error}"));
                return launch;
            }
        }

        if profile.is_stopped_by_operator() {
            let (running, last) =
                self.poll_row(&id, RUNNING_BOUND, |row| row.state == AllocStateWire::Running).await;
            launch.alloc = last.as_ref().map(|row| row.alloc_id.clone());
            if !running {
                launch.failures.push(format!("never Running; last row: {last:?}"));
            }
            match self
                .api
                .post_json(&format!("/v1/workloads/{id}/stop"), &serde_json::json!({}))
                .await
            {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => {
                    launch.failures.push(format!("stop answered {}", response.status()));
                }
                Err(error) => launch.failures.push(format!("stop failed: {error}")),
            }
        }

        let bound = if profile.is_stopped_by_operator() { STOPPED_BOUND } else { FINITE_BOUND };
        let (terminal, last) = self
            .poll_row(&id, bound, |row| {
                matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
            })
            .await;
        if launch.alloc.is_none() {
            launch.alloc = last.as_ref().map(|row| row.alloc_id.clone());
        }
        match (&last, terminal) {
            (Some(row), true) => check_terminal(profile, row, &mut launch.failures),
            _ => launch.failures.push(format!("no terminal state; last row: {last:?}")),
        }

        match launch.alloc.as_deref().map(AllocationId::new) {
            Some(Ok(alloc)) => {
                if let Some(remains) = self.wait_for_artifact_absence(&alloc).await {
                    launch.failures.push(remains);
                }
            }
            Some(Err(error)) => launch.failures.push(format!("allocation id is invalid: {error}")),
            None => launch.failures.push("no allocation row to correlate".to_owned()),
        }
        launch
    }

    /// Wait for the driver's per-allocation artifacts to be gone: run directory,
    /// cgroup scope, rootfs clone and its index link.
    async fn wait_for_artifact_absence(&self, alloc: &AllocationId) -> Option<String> {
        let rootfs_len =
            tokio::fs::metadata(&self.rootfs).await.map(|meta| meta.len()).unwrap_or_default();
        let plan = RootfsPlan::for_alloc(
            self.rootfs.clone(),
            rootfs_len,
            alloc,
            &clone_staging_dir(&self.data_dir),
            &clone_index_dir(&self.data_dir),
        );
        let run_dir = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc);
        let cgroup = overdrive_core::cgroup::CgroupPath::for_alloc(alloc)
            .resolve(Path::new(overdrive_control_plane::cgroup_preflight::DEFAULT_CGROUP_ROOT));
        let deadline = tokio::time::Instant::now() + ARTIFACT_BOUND;
        loop {
            let present = [
                run_dir.path().to_path_buf(),
                cgroup.clone(),
                plan.clone_dest().to_path_buf(),
                plan.index_link().to_path_buf(),
            ]
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
            if present.is_empty() {
                return None;
            }
            if tokio::time::Instant::now() >= deadline {
                return Some(format!(
                    "driver artifacts remain after the terminal row: {present:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// The terminal row of a completed lifecycle.
fn check_terminal(profile: Profile, row: &AllocStatusRowBody, failures: &mut Vec<String>) {
    if row.state != AllocStateWire::Terminated {
        failures.push(format!("terminal state {:?}, reason {:?}", row.state, row.reason));
        return;
    }
    let reason_ok = if profile.is_stopped_by_operator() {
        matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::Reconciler })
        )
    } else {
        row.reason == Some(TransitionReason::Stopped { by: StoppedBy::Process })
            && matches!(row.exit_code, None | Some(0))
    };
    if !reason_ok {
        failures.push(format!(
            "Terminated for the wrong reason: {:?}, exit {:?}, terminal {:?}",
            row.reason, row.exit_code, row.terminal
        ));
    }
    if row.restart_count != 0 {
        failures.push(format!("a completed lifecycle restarted {} time(s)", row.restart_count));
    }
}

/// The first event named `name` for `alloc`, or a departure naming it missing.
fn require<'a>(
    events: &'a [TraceEvent],
    alloc: &str,
    label: &str,
    name: &str,
    departures: &mut Vec<String>,
) -> Option<&'a TraceEvent> {
    let event = events.iter().find(|event| {
        event.name == name && event.fields.get("alloc").is_some_and(|value| value == alloc)
    });
    if event.is_none() {
        departures.push(format!("{label}: missing {name}"));
    }
    event
}

fn audit_launch(events: &[TraceEvent], launch: &Launch, alloc: &str, departures: &mut Vec<String>) {
    let label = format!("{} (allocation {alloc})", launch.workload_id);
    let created = require(events, alloc, &label, "vm.lifecycle.created", departures);
    for name in ["vm.lifecycle.create_enter", "vm.lifecycle.cleanup_calls_finished"] {
        require(events, alloc, &label, name, departures);
    }
    if launch.profile.is_stopped_by_operator() {
        require(events, alloc, &label, "vm.lifecycle.stop_enter", departures);
        if let Some(writer) =
            require(events, alloc, &label, "vm.lifecycle.writer_finished", departures)
            && writer.fields.get("disposition").map(String::as_str) != Some("completed")
        {
            departures.push(format!("{label}: the accepted writer did not complete: {writer:?}"));
        }
    } else {
        require(events, alloc, &label, "vm.beacon.exec.released", departures);
    }
    let target = format!("workload/{}", launch.workload_id);
    for name in ["convergence.evaluation.admitted", "convergence.evaluation.completed"] {
        let seen = events.iter().any(|event| {
            event.name == name && event.fields.get("target").is_some_and(|value| *value == target)
        });
        if !seen {
            departures.push(format!("{label}: missing {name} for {target}"));
        }
    }
    let Some(pid) = created.and_then(|event| event.fields.get("pid")) else {
        departures.push(format!("{label}: the created event carries no VMM pid"));
        return;
    };
    let Some(reaped) = events.iter().find(|event| {
        event.name == "vmm.process.reaped" && event.fields.get("pid").is_some_and(|v| v == pid)
    }) else {
        departures.push(format!("{label}: missing vmm.process.reaped for pid {pid}"));
        return;
    };
    if !reaped.fields.get("exit_code").is_some_and(|value| value == "0" || value == "Some(0)") {
        departures.push(format!("{label}: the VMM did not exit normally: {reaped:?}"));
    }
    if reaped.fields.get("signal").is_some_and(|value| value != "None") {
        departures.push(format!("{label}: the VMM was killed by a signal: {reaped:?}"));
    }
    if Path::new("/proc").join(pid).exists() {
        departures.push(format!("{label}: VMM pid {pid} is still in the process table"));
    }
}
