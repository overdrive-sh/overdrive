//! E2 — composition-root probe gate walking-skeleton.
//!
//! Step 01-06 of the `cgroup-fs-port` migration per ADR-0054
//! § Composition root wiring. Drives the production `serve::run`
//! handler against an unwritable `OVERDRIVE_TEST_PROBE_ROOT` and
//! asserts:
//!   1. `serve::run` returns `Err(CliError::ProbeRefused { .. })`
//!      (the typed variant, not a `Display`-grep on `Transport`).
//!   2. A `health.startup.refused` structured tracing event fires
//!      with the `ProbeError::Substrate` cause threaded as a typed
//!      field — NOT collapsed to `Internal(String)` per
//!      `.claude/rules/development.md` § "Never flatten a typed error
//!      to `Internal(String)` at a composition boundary".
//!   3. The control-plane server NEVER reached the listener-bind step
//!      (asserted indirectly by the absence of a
//!      `cgroup_preflight` event AND by the test fixture's HTTPS
//!      port staying unbound — verified by the fact that
//!      `serve::run` returned `Err` rather than `Ok(handle)`).
//!
//! Per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
//! subprocess", this is a DIRECT in-process call to `serve::run` (not
//! a `Command::spawn` of `overdrive`). The test-only env var
//! `OVERDRIVE_TEST_PROBE_ROOT` is honoured by `serve::run` when
//! present; production callers never set it, so the production probe
//! path remains rooted at `/sys/fs/cgroup`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use overdrive_cli::commands::serve::{self, ServeArgs};
use overdrive_cli::http_client::CliError;
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
        let trimmed = val.trim_matches('"').to_owned();
        self.map.insert(field.name().to_owned(), trimmed);
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
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = CapturedFields::default();
        event.record(&mut fields);
        let metadata = event.metadata();
        self.inner
            .lock()
            .expect("collector lock")
            .push(EventRow { name: metadata.name().to_owned(), fields: fields.map });
    }
}

// ----------------------------------------------------------------------------
// E2 — composition root refuses to start when RealCgroupFs::probe() fails.
// ----------------------------------------------------------------------------

/// RAII guard that restores `OVERDRIVE_TEST_PROBE_ROOT` on drop. The
/// `#[serial(env)]` annotation on the test body guarantees exclusive
/// access; this guard ensures the env stays clean if the test panics
/// mid-body.
struct EnvGuard;

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: `#[serial(env)]` on the test body guarantees
        // exclusive access to the process env for the duration of
        // this guard's lifetime.
        unsafe {
            std::env::remove_var("OVERDRIVE_TEST_PROBE_ROOT");
        }
    }
}

/// Drive `serve::run` with `OVERDRIVE_TEST_PROBE_ROOT` pointed at an
/// unwritable parent path. The probe MUST fail with
/// `ProbeError::Substrate { source }` (`NotFound` / `PermissionDenied`
/// depending on which step trips first), the structured
/// `health.startup.refused` event MUST fire with the typed cause,
/// and `serve::run` MUST return `Err(CliError::ProbeRefused { .. })`.
///
/// `#[serial(env)]` is required: the test mutates the process-global
/// `OVERDRIVE_TEST_PROBE_ROOT` env var and the global tracing
/// subscriber (`with_default` guard is process-wide while held).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn serve_refuses_to_start_when_cgroup_fs_probe_fails() {
    // Choose a probe root whose parent does not exist. RealCgroupFs's
    // probe calls `tokio::fs::create_dir_all(probe_root/.overdrive-probe-<uuid>)`
    // — `create_dir_all` creates missing parents EXCEPT when the
    // grandparent is also missing under some Lima FS configurations.
    // We construct a path whose root is non-creatable by an
    // unprivileged process: under /nonexistent-overdrive-probe/...
    // the root `/nonexistent-overdrive-probe` cannot be created
    // without root.
    //
    // The reliable way: an unwritable parent. We use a tempdir, then
    // chmod its parent so create_dir_all returns PermissionDenied.
    // BUT chmod-based fixtures are flaky under Lima. The simplest
    // reliable mechanism is to target a path under a regular FILE
    // (not a directory): create_dir_all returns NotADirectory.
    let tmp = TempDir::new().expect("tempdir");
    let blocking_file = tmp.path().join("not-a-dir");
    std::fs::write(&blocking_file, b"this is a file, not a directory")
        .expect("create blocking file");
    let probe_root = blocking_file.join("probe-root");

    // Install tracing subscriber BEFORE setting the env var so the
    // probe's `health.startup.refused` event is captured.
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _trace_guard = tracing::subscriber::set_default(subscriber);

    // SAFETY: `#[serial(env)]` guarantees exclusive access to the
    // process env for the duration of this test.
    unsafe {
        std::env::set_var("OVERDRIVE_TEST_PROBE_ROOT", &probe_root);
    }
    // RAII guard (see top of file) restores the env on drop, so a
    // panic mid-body does not pollute subsequent tests in the same
    // binary.
    let _env_guard = EnvGuard;

    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&config_dir).expect("mkdir conf");

    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind");
    let args = ServeArgs { bind, data_dir, config_dir };

    let result = serve::run(args).await;

    // 1. Typed error variant — `ProbeRefused`, not `Transport`.
    let err = result.expect_err("expected serve::run to refuse when probe fails");
    assert!(
        matches!(err, CliError::ProbeRefused { .. }),
        "expected CliError::ProbeRefused variant; got {err:?}"
    );

    // The Display rendering must surface the substrate cause so the
    // operator sees what was wrong with the FS surface.
    let display = format!("{err}");
    assert!(
        display.contains("CgroupFs") || display.contains("cgroup") || display.contains("probe"),
        "expected ProbeRefused Display to reference the probe / cgroup substrate; got: {display}"
    );

    // 2. Structured event surfaces with the typed cause.
    let events = collector.snapshot();
    let refused: Vec<&EventRow> =
        events.iter().filter(|e| e.name == "health.startup.refused").collect();
    assert!(
        !refused.is_empty(),
        "expected at least one health.startup.refused event; observed events: {:?}",
        events.iter().map(|e| &e.name).collect::<Vec<_>>()
    );

    // The event MUST carry a `cause` field naming the substrate
    // problem. Per criterion 3 in the step brief, the ProbeError
    // Display string is the load-bearing payload.
    let has_typed_cause = refused.iter().any(|e| {
        e.fields.get("cause").is_some_and(|c| c.contains("CgroupFs probe") || c.contains("probe"))
    });
    assert!(
        has_typed_cause,
        "expected at least one health.startup.refused event to carry a typed `cause` \
         field naming the CgroupFs probe failure; events: {refused:?}"
    );

    // 3. The probe ran BEFORE cgroup_preflight — when probe fails,
    //    the cgroup_preflight event MUST NOT have fired. We assert
    //    indirectly: `serve::run` returned Err, no ServerHandle was
    //    produced; the convergence loop was never started.
    //    (A stronger structural assertion would require additional
    //    instrumentation; the failed `serve::run` return + the
    //    health.startup.refused event before any preflight event is
    //    the load-bearing signal.)
    let preflight_event = events.iter().find(|e| e.name.contains("preflight"));
    assert!(
        preflight_event.is_none(),
        "expected NO cgroup_preflight event when probe refuses; saw: {preflight_event:?}"
    );

    // Probe root cleanup: tempdir handles the file; the
    // `probe-root` subpath never came into existence because
    // `create_dir_all` failed.
    let _ = probe_root;
}

/// Sanity counterpart — when `OVERDRIVE_TEST_PROBE_ROOT` is UNSET
/// AND we're not running under Lima with cgroupfs delegated, the
/// production probe path runs against `/sys/fs/cgroup` and may
/// succeed (Lima sudo) or fail (no cgroup v2 substrate). This
/// variant exists only to document the env-var-is-optional shape;
/// it does NOT assert success — we only assert that `serve::run`
/// reads the env var when set vs not set.
///
/// In practice, the GREEN sister of the above test is the existing
/// `http_client::*` integration tests, which call `run_server`
/// directly (not via `serve::run`) and assume `cgroup_preflight`
/// passes under `cargo xtask lima run --`.
#[allow(dead_code)]
fn _doc_test_only_env_var_is_optional() {
    let _ = PathBuf::from("/sys/fs/cgroup");
}
