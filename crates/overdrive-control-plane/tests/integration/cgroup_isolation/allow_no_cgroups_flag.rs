//! Step 03-01 / Slice 4 scenario 4.7 —
//! `allow_no_cgroups_bypasses_preflight_with_warning_banner`.
//!
//! Per ADR-0028 §6: setting `allow_no_cgroups: true` bypasses the
//! cgroup pre-flight, skips control-plane slice enrolment, and
//! ALSO emits a structured warning. The banner is the operator
//! signal that the dev escape hatch is active — production
//! deployments grep for it.
//!
//! Linux-gated like the rest of `cgroup_isolation/`. The bypass
//! itself works on every host (it short-circuits before any cgroup
//! syscall), but the WARNING-banner contract is part of the cgroup
//! safety surface and conceptually belongs in this group.

#![cfg(target_os = "linux")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::{ServerConfig, run_server};
use tempfile::TempDir;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// In-memory tracing layer that captures every emitted event's
/// formatted message body. Used to assert the boot path emits the
/// `tracing::warn!` banner when `allow_no_cgroups: true`.
#[derive(Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<String>>>,
}

impl CaptureLayer {
    fn new() -> (Self, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Self { events: Arc::clone(&events) }, events)
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                let _ = write!(self.0, "{}={value:?} ", field.name());
            }
        }
        let mut buf = String::new();
        event.record(&mut Visit(&mut buf));
        self.events.lock().expect("capture mutex").push(buf);
    }
}

/// Sync `#[test]` rather than `#[tokio::test]` because the body
/// installs a thread-local tracing subscriber via
/// `tracing::subscriber::with_default` and then drives `run_server`
/// to completion via a dedicated current-thread Tokio runtime — the
/// double-runtime panic Lima surfaced was the symptom of nesting a
/// `block_on` inside a `#[tokio::test]` multi-thread runtime. One
/// runtime, one subscriber scope, no nesting.
#[test]
fn allow_no_cgroups_bypasses_preflight_with_warning_banner() {
    let (capture, events) = CaptureLayer::new();
    let subscriber = tracing_subscriber::registry().with(capture);

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("create operator config dir");

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir,
        operator_config_dir,
        allow_no_cgroups: true,
        // `tick_cadence` + `clock` default per
        // `fix-convergence-loop-not-spawned` Step 01-02.
        ..Default::default()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");

    // Drive the boot path under our scoped subscriber — the banner
    // must land in the captured events buffer. `with_default` only
    // affects the calling thread, which is the same thread the
    // current-thread runtime drives, so the subscriber is visible to
    // the boot path's `tracing::warn!`.
    let handle = tracing::subscriber::with_default(subscriber, || {
        runtime
            .block_on(async { run_server(config).await.expect("run_server with allow_no_cgroups") })
    });

    let bound = runtime.block_on(handle.local_addr()).expect("listener must bind");
    assert!(bound.port() > 0, "ephemeral port must be assigned");

    // The boot path must have emitted at least one event whose body
    // mentions `--allow-no-cgroups`. The exact wording is the
    // production banner — see `lib.rs::run_server_with_obs_and_driver`.
    let captured = events.lock().expect("events mutex").clone();
    let banner = captured.iter().find(|e| e.contains("--allow-no-cgroups")).unwrap_or_else(|| {
        panic!("WARNING banner must mention --allow-no-cgroups; captured={captured:?}")
    });
    assert!(
        banner.contains("WARNING") || banner.contains("Workloads run without cgroup"),
        "banner must announce the dev-escape-hatch posture: {banner}"
    );

    runtime.block_on(handle.shutdown(Duration::from_secs(2)));
}
