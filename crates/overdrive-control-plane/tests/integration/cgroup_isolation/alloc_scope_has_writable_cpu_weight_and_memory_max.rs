//! Step 01-01 — RED scaffold for the cgroup `subtree_control`
//! delegation regression.
//!
//! Per `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
//! § "Regression test": after both inits run against real
//! `/sys/fs/cgroup`, an alloc started by `ExecDriver` MUST see
//! writable `cpu.weight` and `memory.max` under its scope directory.
//!
//! Today (RED) the production `create_and_enrol_control_plane_slice_at`
//! never writes `+cpu +memory +io +pids` to
//! `overdrive.slice/cgroup.subtree_control`, so the workload-bearing
//! child slice has no `cpu.*` / `memory.*` interface files and the
//! resource-limit writes return EACCES — silently logged via the
//! ADR-0026 D9 warn-and-continue disposition. The companion
//! [`alloc_start_does_not_emit_resource_limit_warning`] regression
//! captures the WARN line absence on the success path.
//!
//! The full assertion body is documented inline below; landing it as
//! live code requires the production fixes from step 01-02 (extend
//! `create_and_enrol_control_plane_slice_at` with the
//! `subtree_control` write + introduce
//! `create_workloads_slice_with_controllers` in
//! `overdrive-worker::cgroup_manager`). Until then the test asserts
//! its RED state via `#[should_panic(expected = "RED scaffold")]` per
//! `.claude/rules/testing.md` § "Test-side scaffolds — `#[should_panic
//! (expected = \"RED scaffold\")]`".
//!
//! Runs against real `/sys/fs/cgroup`, so it requires root + cgroup
//! delegation; gated `integration-tests` and invoked through
//! `cargo xtask lima run --` per
//! `.claude/rules/testing.md` § "Cgroup writes need root or
//! delegation".

#![cfg(target_os = "linux")]
// Step 01-01 RED-scaffold scope: the per-test bodies are unreachable
// behind a leading `panic!` line per `.claude/rules/testing.md` §
// "Test-side scaffolds". Step 01-02 lands the GREEN body and removes
// these allows along with the `panic!` lines.
#![allow(dead_code, unused_imports, unreachable_code, clippy::diverging_sub_expression)]

use std::sync::Arc;

use serial_test::serial;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Captures the metadata `name` of every event the subscriber sees.
/// Adapted from
/// `crates/overdrive-dataplane/tests/integration/veth_attach.rs:307-310`
/// (per the RCA implementation note); inlined here because there is
/// no second call site yet — promote into a shared helper if a third
/// arrives. We deliberately do NOT take a `tracing-test` dev-dep —
/// the project has standardised on the custom-`Layer` shape so we can
/// assert on the metadata `name` (set via `tracing::warn!(name:
/// "...", ...)`), not just the rendered message text.
#[derive(Default)]
struct CapturedEvent {
    /// Event metadata `name` — the field set by `tracing::warn!(name:
    /// "...", ...)`. The RCA pins the WARN we MUST NOT see on the
    /// success path to the rendered text "cgroup resource-limit write
    /// failed"; production emits it under
    /// `name: "worker.cgroup.resource_limit_write_failed"` (see
    /// `crates/overdrive-worker/src/driver.rs:299-305`). Either
    /// surface is acceptable as the negative-assertion key — the body
    /// below pins the rendered-message substring per RCA.
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

/// AC2 — boots both inits against real `/sys/fs/cgroup`, drives
/// `ExecDriver::start` for an alloc with `cpu_milli=2000,
/// memory_bytes=128MiB`, asserts `cpu.weight=200` AND `memory.max=
/// 134217728`. Marked RED scaffold per `.claude/rules/testing.md` §
/// "Test-side scaffolds"; transitions GREEN in step 01-02 by removing
/// the `should_panic` attribute and the leading `panic!` line, and
/// landing the production `create_workloads_slice_with_controllers`
/// init the body needs to call.
#[tokio::test]
#[serial(cgroup)]
#[should_panic(expected = "RED scaffold")]
async fn alloc_scope_has_writable_cpu_weight_and_memory_max() {
    panic!("Not yet implemented -- RED scaffold (subtree_control delegation regression)");

    // ----- Body documented for step 01-02; unreachable in this commit.
    //
    // Stays out of compile reach behind the `panic!` above so this
    // file builds today without depending on the future
    // `create_workloads_slice_with_controllers` init. Step 01-02 turns
    // the scaffold GREEN by:
    //   1. Adding `create_workloads_slice_with_controllers` to
    //      `overdrive-worker::cgroup_manager`.
    //   2. Extending `create_and_enrol_control_plane_slice_at` with
    //      the `+cpu +memory +io +pids` write to
    //      `overdrive.slice/cgroup.subtree_control` BEFORE enrolling
    //      the server PID.
    //   3. Replacing this comment block with the assertion body
    //      sketched in the bugfix RCA § "Regression test".
    //   4. Removing both the `panic!` line above and the
    //      `#[should_panic(expected = "RED scaffold")]` attribute on
    //      this fn.
    //
    // Sketch:
    //   let cgroup_root = std::path::Path::new("/sys/fs/cgroup");
    //   create_and_enrol_control_plane_slice_at(
    //       cgroup_root, std::process::id())
    //       .expect("control-plane bootstrap succeeds");
    //   create_workloads_slice_with_controllers(cgroup_root)
    //       .expect("workloads.slice bootstrap succeeds");
    //   let driver = std::sync::Arc::new(ExecDriver::new(
    //       cgroup_root.to_path_buf(),
    //       std::sync::Arc::new(SystemClock),
    //   ));
    //   let alloc = AllocationId::new("alloc-subtree-control-regression")
    //       .expect("valid");
    //   let spec = AllocationSpec {
    //       alloc: alloc.clone(),
    //       identity: SpiffeId::new(
    //           "spiffe://overdrive.local/job/regression/alloc/0").unwrap(),
    //       command: "/bin/sleep".to_owned(),
    //       args: vec!["60".to_owned()],
    //       resources: Resources {
    //           cpu_milli: 2_000, memory_bytes: 128 * 1024 * 1024 },
    //   };
    //   let _cleanup = AllocCleanup { obs: ..., cgroup_root:
    //       cgroup_root.to_path_buf() };
    //   let handle = driver.start(&spec).await
    //       .expect("start succeeds against real cgroupfs");
    //   let scope_dir = cgroup_root.join(format!(
    //       "overdrive.slice/workloads.slice/{alloc}.scope"));
    //   let cpu_weight = std::fs::read_to_string(
    //       scope_dir.join("cpu.weight"))
    //       .expect("cpu.weight readable").trim().to_owned();
    //   assert_eq!(cpu_weight, "200",
    //       "cpu_milli=2000 must produce cpu.weight=200");
    //   let memory_max = std::fs::read_to_string(
    //       scope_dir.join("memory.max"))
    //       .expect("memory.max readable").trim().to_owned();
    //   assert_eq!(memory_max,
    //       format!("{}", 128_u64 * 1024 * 1024));
    //   driver.stop(&handle).await.expect("stop succeeds");
}

/// AC3 — companion regression: the WARN log line "cgroup
/// resource-limit write failed" (emitted from
/// `crates/overdrive-worker/src/driver.rs:299-305` per the RCA) MUST
/// NOT fire on the success path once the production fix lands. Same
/// RED-scaffold shape and same GREEN-transition mechanics as AC2.
///
/// The `CaptureLayer` machinery above is wired as a `tracing` layer
/// for the duration of this test (per the RCA implementation note,
/// the custom-`Layer` shape rather than `tracing-test` so the
/// assertion can reach the event metadata `name` if required, even
/// though the canonical assertion below is on the rendered message
/// substring per RCA wording).
#[tokio::test]
#[serial(cgroup)]
#[should_panic(expected = "RED scaffold")]
async fn alloc_start_does_not_emit_resource_limit_warning() {
    panic!("Not yet implemented -- RED scaffold (subtree_control delegation regression)");

    // ----- Body documented for step 01-02; unreachable in this commit.
    //
    // GREEN-transition is the same diff as AC2: remove the panic
    // line, remove the `should_panic` attribute, expand this comment
    // block into:
    //
    //   let layer = CaptureLayer::default();
    //   let events = layer.events.clone();
    //   let _guard = tracing_subscriber::registry()
    //       .with(layer)
    //       .set_default();
    //
    //   // ... boot both inits, drive ExecDriver::start as in AC2 ...
    //
    //   let captured = events.lock().expect("events mutex").clone();
    //   assert!(
    //       !captured.iter().any(|ev|
    //           ev.message.as_deref()
    //               .is_some_and(|m| m.contains(
    //                   "cgroup resource-limit write failed"))),
    //       "WARN line `cgroup resource-limit write failed` MUST NOT \
    //        fire on the success path; observed events: {captured:?}",
    //   );
}
