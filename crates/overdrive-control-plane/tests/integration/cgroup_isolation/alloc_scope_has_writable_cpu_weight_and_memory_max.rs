//! Step 01-02 — GREEN regression for the cgroup `subtree_control`
//! delegation production fix.
//!
//! Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § "Regression test": after both inits run against real
//! `/sys/fs/cgroup`, an alloc started by `ExecDriver` MUST see
//! writable `cpu.weight` and `memory.max` under its scope directory.
//!
//! Pre-fix (RED, step 01-01) this file held two `#[should_panic
//! (expected = "RED scaffold")]` test scaffolds; step 01-02 lands the
//! production fix in `crates/overdrive-control-plane/src/cgroup_manager.rs`
//! (`create_and_enrol_control_plane_slice_at` extended with the
//! `subtree_control` write) and `crates/overdrive-worker/src/cgroup_manager.rs`
//! (new `create_workloads_slice_with_controllers`), and this file
//! transitions GREEN by removing the scaffolds and exercising the
//! real production path.
//!
//! The companion regression
//! [`alloc_start_does_not_emit_resource_limit_warning`] captures the
//! WARN line absence on the success path — the ADR-0026 D9
//! warn-and-continue disposition was the structural amplifier that
//! masked the original bug for so long; now that the production fix
//! lands, the warning MUST NOT fire.
//!
//! Runs against real `/sys/fs/cgroup`, so it requires root + cgroup
//! delegation; gated `integration-tests` and invoked through
//! `cargo xtask lima run --` per
//! `.claude/rules/testing.md` § "Cgroup writes need root or
//! delegation".

#![cfg(target_os = "linux")]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::cgroup_manager::create_and_enrol_control_plane_slice_at;
use overdrive_core::id::{AllocationId, SpiffeId};
use overdrive_core::traits::CgroupFs;
use overdrive_core::traits::driver::{AllocationSpec, Driver, Resources};
use overdrive_host::SystemClock;
use overdrive_worker::ExecDriver;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

/// Captures the metadata `name` and rendered `message` of every event
/// the subscriber sees. Adapted from
/// `crates/overdrive-dataplane/tests/integration/veth_attach.rs:307-310`
/// (per the RCA implementation note); inlined here because there is
/// no second call site yet — promote into a shared helper if a third
/// arrives. We deliberately do NOT take a `tracing-test` dev-dep —
/// the project has standardised on the custom-`Layer` shape so we can
/// assert on the metadata `name` (set via `tracing::warn!(name:
/// "...", ...)`), not just the rendered message text.
#[derive(Clone, Debug)]
struct CapturedEvent {
    #[allow(dead_code)] // read via Debug; explicit field for future name-based assertions
    name: &'static str,
    message: Option<String>,
}

#[derive(Default, Clone)]
struct CaptureLayer {
    events: Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct MessageVisitor {
            message: Option<String>,
        }
        impl tracing::field::Visit for MessageVisitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "message" {
                    self.message = Some(value.to_string());
                }
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.message = Some(format!("{value:?}"));
                }
            }
        }
        let mut v = MessageVisitor { message: None };
        event.record(&mut v);
        self.events
            .lock()
            .expect("events mutex")
            .push(CapturedEvent { name: event.metadata().name(), message: v.message });
    }
}

/// Best-effort cleanup of the alloc scope this test creates. Mirrors
/// the `AllocCleanup` pattern from `tests/integration/workload_lifecycle/cleanup.rs`,
/// but inlined because we don't have an obs store handle to enumerate
/// (we know the single `AllocationId` we care about).
struct ScopeCleanup {
    cgroup_root: std::path::PathBuf,
    alloc: AllocationId,
}

impl Drop for ScopeCleanup {
    fn drop(&mut self) {
        let scope = self
            .cgroup_root
            .join("overdrive.slice/workloads.slice")
            .join(format!("{}.scope", self.alloc));

        let pids: Vec<libc::pid_t> = std::fs::read_to_string(scope.join("cgroup.procs"))
            .ok()
            .map(|s| s.lines().filter_map(|l| l.trim().parse::<i32>().ok()).collect())
            .unwrap_or_default();
        let _ = std::fs::write(scope.join("cgroup.kill"), "1\n");
        for pid in pids {
            for _ in 0..20 {
                let mut status: libc::c_int = 0;
                // SAFETY: `waitpid` is a thin syscall wrapper. We pass
                // a real pid_t and a valid status pointer; ignoring
                // the return is sound because the loop bails on the
                // next read or after 20×10ms.
                let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                if r == pid || r == -1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = std::fs::remove_dir(&scope);
    }
}

fn build_spec(alloc: &AllocationId) -> AllocationSpec {
    AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/regression/alloc/0")
            .expect("valid SpiffeId"),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 2_000, memory_bytes: 128 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        // transparent-mtls-enrollment step 04-01 (JOIN-4/JOIN-6): off the mTLS-composed boot gate.
        netns: None,
        host_veth: None,
        service_ports: Vec::new(),
        workload_addr: None,
    }
}

/// AC2 — boots both inits against real `/sys/fs/cgroup`, drives
/// `ExecDriver::start` for an alloc with `cpu_milli=2000,
/// memory_bytes=128MiB`, asserts `cpu.weight=200` AND `memory.max=
/// 134217728`. With the production fix in place, the per-alloc
/// resource-limit writes succeed because `overdrive.slice` and
/// `workloads.slice` both have the relevant controllers delegated.
#[tokio::test]
#[serial(cgroup)]
async fn alloc_scope_has_writable_cpu_weight_and_memory_max() {
    let cgroup_root = Path::new("/sys/fs/cgroup");

    create_and_enrol_control_plane_slice_at(cgroup_root, std::process::id())
        .expect("control-plane bootstrap succeeds");
    let fs: Arc<dyn CgroupFs> = Arc::new(overdrive_host::RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let driver = Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SystemClock), fs));
    let alloc = AllocationId::new("alloc-subtree-control-regression").expect("valid AllocationId");
    let spec = build_spec(&alloc);
    let _cleanup = ScopeCleanup { cgroup_root: cgroup_root.to_path_buf(), alloc: alloc.clone() };

    let handle =
        driver.start(&spec).await.expect("ExecDriver::start succeeds against real cgroupfs");

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    let cpu_weight = std::fs::read_to_string(scope_dir.join("cpu.weight"))
        .expect("cpu.weight readable")
        .trim()
        .to_owned();
    assert_eq!(cpu_weight, "200", "cpu_milli=2000 must produce cpu.weight=200");
    let memory_max = std::fs::read_to_string(scope_dir.join("memory.max"))
        .expect("memory.max readable")
        .trim()
        .to_owned();
    assert_eq!(
        memory_max,
        format!("{}", 128_u64 * 1024 * 1024),
        "memory.max must match the spec's memory_bytes",
    );

    driver.stop(&handle).await.expect("ExecDriver::stop succeeds");
}

/// AC3 — companion regression: the WARN log line "cgroup
/// resource-limit write failed" (emitted from
/// `crates/overdrive-worker/src/driver.rs:299-305` per the RCA) MUST
/// NOT fire on the success path with the production fix landed.
///
/// Captures `tracing` events via a `CaptureLayer` scoped to this test
/// only (per the RCA implementation note, the custom-`Layer` shape
/// rather than `tracing-test`).
#[tokio::test]
#[serial(cgroup)]
async fn alloc_start_does_not_emit_resource_limit_warning() {
    let cgroup_root = Path::new("/sys/fs/cgroup");

    create_and_enrol_control_plane_slice_at(cgroup_root, std::process::id())
        .expect("control-plane bootstrap succeeds");
    let fs: Arc<dyn CgroupFs> = Arc::new(overdrive_host::RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let layer = CaptureLayer::default();
    let events = layer.events.clone();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let driver = Arc::new(ExecDriver::new(cgroup_root.to_path_buf(), Arc::new(SystemClock), fs));
    let alloc =
        AllocationId::new("alloc-subtree-control-warn-regression").expect("valid AllocationId");
    let spec = build_spec(&alloc);
    let _cleanup = ScopeCleanup { cgroup_root: cgroup_root.to_path_buf(), alloc: alloc.clone() };

    let handle = driver.start(&spec).await.expect("ExecDriver::start succeeds");
    driver.stop(&handle).await.expect("ExecDriver::stop succeeds");

    let captured = events.lock().expect("events mutex").clone();
    assert!(
        !captured.iter().any(|ev| ev
            .message
            .as_deref()
            .is_some_and(|m| m.contains("cgroup resource-limit write failed"))),
        "WARN line `cgroup resource-limit write failed` MUST NOT fire on the success path; \
         observed events: {captured:?}",
    );
}
