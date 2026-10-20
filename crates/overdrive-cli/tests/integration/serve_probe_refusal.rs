//! E2 — composition-root probe gate walking-skeleton.
//!
//! Step 01-06 of the `cgroup-fs-port` migration per ADR-0054
//! § Composition root wiring; re-pointed by GH #295 PROBE-ROOT-ENV (user
//! ruling 2026-10-19). Drives the CLI's one entry function
//! `serve::run_with_adapters` — production `run` is a call of it — with a
//! REAL `RealCgroupFs` whose probe root lies under a regular file
//! (`RealCgroupFs::new().with_probe_root(..)`, the real adapter's own
//! probe-root constructor), so the refusal comes from the real probe's
//! `create_dir_all` failure. It asserts:
//!   1. `run_with_adapters` returns `Err(CliError::ProbeRefused { .. })`
//!      (the typed variant, not a `Display`-grep on `Transport`).
//!   2. A `health.startup.refused` structured tracing event fires
//!      with the `ProbeError::Substrate` cause threaded as a typed
//!      field — NOT collapsed to `Internal(String)` per
//!      `.claude/rules/development.md` § "Never flatten a typed error
//!      to `Internal(String)` at a composition boundary".
//!   3. The control-plane server NEVER reached the listener-bind step:
//!      no `cgroup_preflight` event fired.
//!   4. The adapters builder was never called: a refused probe opens no
//!      store and creates no file (probe before the bundle, round 13).
//!
//! No environment variable is read or written (the
//! `OVERDRIVE_TEST_PROBE_ROOT` seam is removed), so the test needs no
//! `#[serial(env)]`. The tracing capture is a thread-local default
//! subscriber held for the test body on a current-thread runtime, so the
//! probe's event (emitted on the calling task) is captured and no other
//! test's events are.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use overdrive_cli::commands::serve;
use overdrive_cli::http_client::CliError;
use overdrive_host::RealCgroupFs;
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

/// Drive `serve::run_with_adapters` with a real `RealCgroupFs` whose probe
/// root lies under a regular file. The probe MUST fail with
/// `ProbeError::Substrate { source }` (`NotADirectory`), the structured
/// `health.startup.refused` event MUST fire with the typed cause,
/// `run_with_adapters` MUST return `Err(CliError::ProbeRefused { .. })`, and
/// the adapters builder MUST NOT have been called.
#[tokio::test(flavor = "current_thread")]
#[expect(
    clippy::result_large_err,
    reason = "the bundle builder returns the boot's typed `ControlPlaneError` unboxed, as `run_server` does"
)]
async fn serve_refuses_to_start_when_cgroup_fs_probe_fails() {
    // `create_dir_all` under a regular FILE (not a directory) fails with
    // NotADirectory — the reliable unwritable probe root (chmod-based
    // fixtures are flaky under Lima).
    let tmp = TempDir::new().expect("tempdir");
    let blocking_file = tmp.path().join("not-a-dir");
    std::fs::write(&blocking_file, b"this is a file, not a directory")
        .expect("create blocking file");
    let probe_root = blocking_file.join("probe-root");

    // Thread-local default subscriber, installed before the probe runs; the
    // current-thread runtime keeps the probe on this thread.
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _trace_guard = tracing::subscriber::set_default(subscriber);

    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&config_dir).expect("mkdir conf");

    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind");
    let args = super::serve_fixture::serve_args(bind, data_dir, config_dir);

    let builder_called = Arc::new(AtomicBool::new(false));
    let called = Arc::clone(&builder_called);
    let result = serve::run_with_adapters(
        args,
        Arc::new(RealCgroupFs::new().with_probe_root(probe_root.clone())),
        move |_config, _fs| {
            called.store(true, Ordering::SeqCst);
            unreachable!("the adapters builder runs only after the cgroupfs probe passed")
        },
    )
    .await;

    // 1. Typed error variant — `ProbeRefused`, not `Transport`.
    let err = result.expect_err("expected run_with_adapters to refuse when the probe fails");
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

    // 4. Probe before the bundle: the builder never ran, so no store was
    //    opened and the data directory holds nothing.
    assert!(
        !builder_called.load(Ordering::SeqCst),
        "a refused probe must not call the adapters builder"
    );
    let data_entries = std::fs::read_dir(tmp.path().join("data")).expect("read data dir").count();
    assert_eq!(data_entries, 0, "a refused probe opens no store under data_dir");
    assert!(!probe_root.exists(), "the probe root under a regular file never came into existence");
}
