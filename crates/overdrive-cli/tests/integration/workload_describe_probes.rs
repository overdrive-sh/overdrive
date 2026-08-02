//! Tier 3 acceptance — probe state is operator-observable through
//! `overdrive workload describe`.
//!
//! Closes the store → API type → handler → CLI chain for the probe
//! surface (US-06 / K4; EDD expectation
//! `verification/expectations/O02-alloc-status-probes-section`). The
//! renderer `render::probes_section` and its unit tests existed before
//! this file; what did NOT exist was any production caller — the exact
//! `CLAUDE.md` § "Build vertical slices through production entry
//! points" failure mode ("dead code wearing a green test suite").
//!
//! **A renderer unit test cannot catch that, by construction** — it
//! calls the renderer itself, so it is green whether or not anything
//! else does. These tests instead drive the *production* path:
//!
//!   real `serve` composition root
//!     → real `ExecDriver` workload + real `ProbeRunner` tick
//!     → durable `ProbeResultRow` in the `ObservationStore`
//!     → `GET /v1/allocs` (`handlers::alloc_status`)
//!     → `commands::workload::describe`
//!     → `render::workload_describe` (the single live renderer
//!        `main.rs` dispatches `overdrive workload describe` through)
//!
//! and assert on the rendered operator output. Delete any link in that
//! chain and these tests go red.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: real in-process server on an
//! ephemeral port, CLI handlers called directly (NOT subprocess). Per
//! `.claude/rules/testing.md` § "Running tests — Lima VM":
//! `cargo xtask lima run -- cargo nextest run -p overdrive-cli
//! --features integration-tests -E 'test(workload_describe_probes)'`.
//!
//! Linux-gated for the same reason as `service_honest_stable.rs`: the
//! Service path drives a real `ExecDriver` against `/bin/bash` and
//! writes under `/sys/fs/cgroup/overdrive.slice/workloads.slice/`.

#![cfg(all(target_os = "linux", feature = "integration-tests"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use bytes::BytesMut;
use futures::StreamExt as _;
use overdrive_cli::commands::deploy::{DeployArgs, StopArgs};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::workload::{DescribeArgs, describe, describe_snapshot};
use overdrive_cli::http_client::ApiClient;
use overdrive_cli::render::workload_describe;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput, WorkloadSpecInput};
use overdrive_core::api::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::observation::probe_result_row::ProbeRole;
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared in-process server fixture — same shape as
// `service_honest_stable.rs`.
// ---------------------------------------------------------------------------

async fn spawn_server() -> (ServeHandle, TempDir) {
    let tmp = TempDir::new().expect("tempdir");
    let bind: SocketAddr = "127.0.0.1:0".parse().expect("parse bind addr");
    let data_dir = tmp.path().join("data");
    let config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&config_dir).expect("create operator config dir");
    let args = ServeArgs { bind, data_dir, config_dir };
    let handle = overdrive_cli::commands::serve::run_with_dataplane(
        args,
        std::sync::Arc::new(overdrive_sim::adapters::dataplane::SimDataplane::new()),
        std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .expect("serve::run");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

fn example_path(name: &str) -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir.parent().and_then(Path::parent).expect("workspace root");
    workspace_root.join("examples").join(name)
}

fn read_example_toml(name: &str) -> String {
    let path = example_path(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// Submit a Service-kind TOML through the streaming lane and drain
/// until a terminal `ServiceSubmitEvent`. Mirrors
/// `service_honest_stable::submit_service_and_collect_events`; kept
/// local because that module is private to its own test binary file.
async fn submit_service_until_terminal(toml: &str, config_path: &Path) -> ServiceSubmitEvent {
    let parsed = WorkloadSpecInput::from_toml_str(toml).expect("parse TOML fixture");
    let service = match parsed {
        WorkloadSpecInput::Service(s) => s,
        other => panic!("fixture must parse as Service; got kind={:?}", other.kind()),
    };

    let listeners: Vec<ListenerInput> = service
        .listeners
        .iter()
        .map(|l| ListenerInput { port: l.port.get(), protocol: l.protocol.as_str().to_owned() })
        .collect();
    let spec_input = ServiceSpecInput {
        id: service.id,
        replicas: service.replicas,
        resources: ResourcesInput {
            cpu_milli: service.resources.cpu_milli,
            memory_bytes: service.resources.memory_bytes,
        },
        driver: DriverInput::Exec(ExecInput {
            command: service.exec.command,
            args: service.exec.args,
        }),
        listeners,
        startup_probes: service.startup_probes,
        readiness_probes: service.readiness_probes,
        liveness_probes: service.liveness_probes,
    };

    let client = ApiClient::from_config(config_path).expect("ApiClient::from_config");
    let request = SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec_input) };
    let response =
        client.submit_workload_streaming(request).await.expect("submit_workload_streaming");

    let mut stream = response.bytes_stream();
    let mut buf = BytesMut::new();
    let mut last: Option<ServiceSubmitEvent> = None;

    'outer: while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.expect("read NDJSON chunk");
        buf.extend_from_slice(&chunk);
        while let Some(newline_pos) = buf.iter().position(|&b| b == b'\n') {
            let line = buf.split_to(newline_pos + 1);
            let line_bytes = &line[..line.len() - 1];
            if line_bytes.is_empty() {
                continue;
            }
            let event: ServiceSubmitEvent =
                serde_json::from_slice(line_bytes).unwrap_or_else(|e| {
                    panic!(
                        "NDJSON line is not a ServiceSubmitEvent: {e}; line: {}",
                        String::from_utf8_lossy(line_bytes)
                    )
                });
            let is_terminal = matches!(
                event,
                ServiceSubmitEvent::Stable { .. }
                    | ServiceSubmitEvent::Failed { .. }
                    | ServiceSubmitEvent::Stopped { .. }
            );
            last = Some(event);
            if is_terminal {
                break 'outer;
            }
        }
    }

    last.expect("stream must produce at least one event")
}

// ===========================================================================
// The load-bearing scenario — a deployed Service's probe state reaches
// the operator through `overdrive workload describe`.
// ===========================================================================

/// GIVEN a Service deployed through the production `serve` + submit
/// path that has converged to Stable (so its ADR-0058-inferred
/// default-TCP startup probe has ticked and written a durable
/// `ProbeResultRow`),
/// WHEN the operator runs `overdrive workload describe quick-bind`,
/// THEN the rendered output carries a `Probes:` section naming the
/// probe's role, per-role index, mechanic, and its observed outcome.
///
/// The assertions are deliberately layered so a failure localises to
/// one link of the chain rather than "output didn't match":
///
///   1. **Wire — declaration side.** `snapshot.probes` is non-empty and
///      carries the inferred startup descriptor. Fails if the handler's
///      intent projection is missing.
///   2. **Wire — observation side.** `snapshot.probe_results` carries a
///      row for that probe. Fails if the handler's
///      `list_probe_results_for_alloc` read is missing — the link whose
///      absence is the whole reason this feature was invisible.
///   3. **Render.** The operator string contains the section, the row,
///      the mechanic summary, and `last=pass`.
///
/// `last=pass` (not merely "a Probes section exists") is the assertion
/// that makes this test unfakeable by a renderer that emits declared
/// probes with no observation join: `Pass` can only come from a row the
/// `ProbeRunner` actually wrote.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn given_stable_service_when_workload_describe_then_probe_state_is_rendered() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());
    let toml = read_example_toml("quick-bind-service.toml");

    let terminal = submit_service_until_terminal(&toml, &cfg).await;
    assert!(
        matches!(terminal, ServiceSubmitEvent::Stable { .. }),
        "fixture must converge to Stable before describe is meaningful; got {terminal:?}",
    );

    let args = || DescribeArgs { id: "quick-bind".to_owned(), config_path: cfg.clone() };

    // (1) Wire — declaration side.
    let snapshot = describe_snapshot(args()).await.expect("describe_snapshot");
    assert!(
        !snapshot.probes.is_empty(),
        "AllocStatusResponse.probes must carry the Service's declared (here: \
         ADR-0058-inferred) probe descriptors; got an empty vector, which means the \
         handler's intent-side projection is not wired",
    );
    let startup = snapshot
        .probes
        .iter()
        .find(|p| p.role == ProbeRole::Startup)
        .expect("quick-bind declares a listener, so ADR-0058 infers a startup probe");
    assert!(
        startup.inferred,
        "quick-bind-service.toml omits [[health_check.startup]], so the descriptor MUST be \
         the platform-synthesised default per ADR-0058",
    );

    // (2) Wire — observation side. This is the link whose absence made
    // probe state unobservable: the rows were durable all along, the
    // read was never wired.
    assert!(
        !snapshot.probe_results.is_empty(),
        "AllocStatusResponse.probe_results must carry the observed ProbeResultRow(s) the \
         ProbeRunner wrote — the workload reached Stable, which REQUIRES an observed \
         startup Pass, so an empty vector means the handler's \
         list_probe_results_for_alloc read is not wired",
    );
    let observed = snapshot
        .probe_results
        .iter()
        .find(|r| r.role == ProbeRole::Startup.as_str() && r.probe_idx == startup.idx.get())
        .expect("an observed row for the startup probe the Stable verdict was witnessed on");
    assert!(
        snapshot.rows.iter().any(|row| row.alloc_id == observed.alloc_id),
        "the observed probe row must belong to one of this workload's allocations; \
         probe row alloc_id={} vs alloc rows {:?}",
        observed.alloc_id,
        snapshot.rows.iter().map(|r| &r.alloc_id).collect::<Vec<_>>(),
    );

    // (3) Render — the operator-facing string.
    let out = describe(args()).await.expect("workload describe");
    let rendered = workload_describe(&out);

    assert!(
        rendered.contains("Probes:"),
        "`overdrive workload describe` must render a Probes section for a Service; got:\n{rendered}",
    );
    assert!(
        rendered.contains("startup probe[0]"),
        "the Probes row must name the role and the PER-ROLE probe_idx (ADR-0080 § D1); \
         got:\n{rendered}",
    );
    assert!(
        rendered.contains("tcp 0.0.0.0:8080"),
        "the Probes row must carry the mechanic summary from the spec descriptor; got:\n{rendered}",
    );
    assert!(
        rendered.contains("last=pass"),
        "the Probes row must carry the OBSERVED outcome — `last=pass` can only come from a \
         durable ProbeResultRow, so this is what distinguishes a real join from a renderer \
         echoing declarations; got:\n{rendered}",
    );
    assert!(
        rendered.contains("(inferred)"),
        "an ADR-0058-synthesised probe must be marked `(inferred)` so the operator can tell \
         it apart from one they declared; got:\n{rendered}",
    );
    assert!(
        !rendered.contains("last=pending"),
        "a Stable workload's startup probe has ticked, so nothing may render as pending; \
         got:\n{rendered}",
    );

    let _ = overdrive_cli::commands::deploy::stop(StopArgs {
        id: "quick-bind".to_owned(),
        config_path: cfg.clone(),
    })
    .await;
    handle.shutdown().await.expect("clean shutdown");
}

// ===========================================================================
// The empty case, honestly — a workload with no probes.
// ===========================================================================

/// GIVEN a Job deployed through the production path (a kind that
/// CANNOT declare probes — `JOB_PROBES_GUIDANCE`: "Job has no readiness
/// question; on completion is enough"),
/// WHEN the operator runs `overdrive workload describe coinflip`,
/// THEN the describe output carries NO Probes section at all — not an
/// empty one, not a `Probes:` header with nothing under it — and the
/// rest of the Job render is unaffected.
///
/// This is the negative half of the kind-guard, driven end to end
/// rather than at the renderer. It also pins the wire contract: both
/// probe fields stay empty for a Job, so `skip_serializing_if` keeps
/// them off the JSON entirely and non-Service consumers see no change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn given_job_with_no_probes_when_workload_describe_then_no_probes_section() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    overdrive_cli::commands::deploy::deploy(DeployArgs {
        spec: example_path("coinflip.toml"),
        config_path: cfg.clone(),
    })
    .await
    .expect("deploy Job fixture");

    let args = || DescribeArgs { id: "coinflip".to_owned(), config_path: cfg.clone() };

    let snapshot = describe_snapshot(args()).await.expect("describe_snapshot");
    assert!(
        snapshot.probes.is_empty(),
        "a Job cannot declare probes (the parser rejects [[health_check.*]] on this kind), \
         so the wire declaration side MUST stay empty; got {:?}",
        snapshot.probes,
    );
    assert!(
        snapshot.probe_results.is_empty(),
        "a Job has no probe surface, so no observed rows may be projected; got {:?}",
        snapshot.probe_results,
    );

    let out = describe(args()).await.expect("workload describe");
    let rendered = workload_describe(&out);

    assert!(
        !rendered.contains("Probes"),
        "a Job describe must contain no Probes section and no bare `Probes` header \
         (US-06 kind-guard); got:\n{rendered}",
    );
    // The rest of the Job render must be untouched by the new section —
    // an additive change that silently swallowed the kind body would
    // otherwise pass the negative assertion above.
    assert!(
        rendered.contains("(kind: Job)"),
        "the Job kind-aware body must still render; got:\n{rendered}",
    );

    handle.shutdown().await.expect("clean shutdown");
}
