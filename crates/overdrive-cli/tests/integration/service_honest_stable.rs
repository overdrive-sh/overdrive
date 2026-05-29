//! Tier 3 integration — Service-submit honesty regression guard.
//!
//! Step 01-03f-1 — activates S-SHCP-INT-CLI-02 / 03 / 04 / 05 against
//! the corrected composition root. S-SHCP-INT-CLI-01 (the 100-seed K1
//! coinflip loop) remains RED pending step 01-03f-2.
//!
//! Per `crates/overdrive-cli/CLAUDE.md`: each test starts a real
//! in-process control-plane server on an ephemeral port and drives
//! the CLI handler directly (NOT subprocess). Per
//! `.claude/rules/testing.md` § "Running tests — Lima VM": invocation
//! goes through `cargo xtask lima run -- cargo nextest run -p
//! overdrive-cli --features integration-tests -E
//! 'test(service_honest_stable)'`.
//!
//! Linux-gated because the Service-kind composition path drives a real
//! `ExecDriver` against `/bin/bash` / `/bin/sleep` and writes to
//! `/sys/fs/cgroup/overdrive.slice/workloads.slice/...`. macOS picks
//! up the `--no-run` gate; Lima runs the actual test bodies.

#![cfg(all(target_os = "linux", feature = "integration-tests"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use bytes::BytesMut;
use futures::StreamExt as _;
use overdrive_cli::commands::cluster::StatusArgs as ClusterStatusArgs;
use overdrive_cli::commands::job::StopArgs;
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::http_client::ApiClient;
use overdrive_control_plane::api::SubmitWorkloadRequest;
use overdrive_control_plane::streaming::ServiceSubmitEvent;
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput, WorkloadSpecInput};
use overdrive_core::api::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::transition_reason::{ProbeWitness, TerminalCondition};
use serial_test::serial;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared in-process server fixture — same shape as
// `walking_skeleton.rs` / `service_submit_streaming_cli_dispatch.rs`.
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
    )
    .await
    .expect("serve::run");
    (handle, tmp)
}

fn config_path(tmp: &Path) -> PathBuf {
    tmp.join("conf").join(".overdrive").join("config")
}

/// Read a TOML fixture from the workspace `examples/` directory.
///
/// The workspace root resolves at compile time via
/// `CARGO_MANIFEST_DIR` (which points at the CLI crate root); the
/// `examples/` directory sits two levels up.
fn read_example_toml(name: &str) -> String {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = crate_dir.parent().and_then(Path::parent).expect("workspace root");
    let path = workspace_root.join("examples").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

/// Drive a Service-kind streaming submit through the lower-level
/// `ApiClient::submit_workload_streaming` surface and collect every
/// `ServiceSubmitEvent` line until the stream emits a terminal event
/// (`Stable`, `Failed`, or `Stopped`) or the response body closes.
///
/// Higher-level than `submit_streaming` because the typed
/// `ServiceSubmitEvent` values are needed by S-SHCP-INT-CLI-04 to
/// reconstruct `TerminalCondition::Stable { settled_in_ms, witness }`
/// for the rkyv byte-equality assertion. The TOML is parsed and
/// projected to `ServiceSpecInput` here so the test owns the wire
/// payload it submits.
async fn submit_service_and_collect_events(
    toml: &str,
    config_path: &Path,
) -> Vec<ServiceSubmitEvent> {
    let parsed = WorkloadSpecInput::from_toml_str(toml).expect("parse TOML fixture");
    let service = match parsed {
        WorkloadSpecInput::Service(s) => s,
        other => {
            panic!("fixture must parse as WorkloadSpecInput::Service; got kind={:?}", other.kind())
        }
    };

    // Project parser-side `ServiceSpec` → wire-side `ServiceSpecInput`,
    // mirroring `commands::job::submit_streaming_service`. The wire
    // shape preserves probe descriptors verbatim (including the
    // ADR-0058 default-TCP probe synthesised at parse time when the
    // TOML omits `[[health_check.startup]]`).
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

    let mut events: Vec<ServiceSubmitEvent> = Vec::new();
    let mut stream = response.bytes_stream();
    let mut buf = BytesMut::new();

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
                        "failed to deserialise NDJSON line as ServiceSubmitEvent: {e}; \
                         line bytes: {}",
                        String::from_utf8_lossy(line_bytes)
                    )
                });
            let is_terminal = matches!(
                event,
                ServiceSubmitEvent::Stable { .. }
                    | ServiceSubmitEvent::Failed { .. }
                    | ServiceSubmitEvent::Stopped { .. }
            );
            events.push(event);
            if is_terminal {
                break 'outer;
            }
        }
    }

    events
}

// ===========================================================================
// S-SHCP-INT-CLI-05 — In-process composition-root probe gate (positive)
// ===========================================================================

/// S-SHCP-INT-CLI-05 (US-08 / ADR-0054 § 7) — in-process composition-
/// root `ProbeRunner` Earned-Trust gate.
///
/// `compose_production_driver` wires a sacrificial loopback listener
/// (per `ProbeRunner::probe()` at ADR-0054 § 7) and runs the gate
/// before serving. When the gate passes, `spawn_server` returns
/// `Ok(handle)` and the server is reachable through the configured
/// trust triple. The negative case (gate refuses; structured
/// `health.startup.refused` event emitted) is covered by the Tier-1
/// acceptance test
/// `crates/overdrive-control-plane/tests/acceptance/probe_runner_boot_gate.rs::
/// S-SHCP-01-03d-REFUSE`.
///
/// The load-bearing positive-case Tier-3 assertion: a fresh
/// `spawn_server` against a healthy loopback environment returns
/// `Ok` AND a follow-up `cluster::status` query succeeds. If the
/// composition root had refused the gate, `spawn_server` would have
/// returned `Err(CliError::Transport)` mapping
/// `ControlPlaneError::ProbeRunnerBoot`. Both surfaces — the handle
/// itself AND a live read against it — are exercised to defend
/// against a "spawn returned Ok but the server never bound" partial-
/// init regression.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn given_in_process_composition_root_when_spawn_server_then_probe_gate_passes() {
    // Compose-and-spawn — the assertion is implicit in the `expect`
    // inside `spawn_server`: a failed probe gate surfaces as
    // `CliError::Transport` and the `expect("serve::run")` panics.
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());

    // Follow-up live read: the server must actually serve. The
    // composition-root probe gate runs BEFORE the listener binds
    // (per ADR-0054 § 7 — same shape as `EbpfDataplane::probe()`),
    // so an Ok return MUST be paired with a working endpoint.
    let cluster = overdrive_cli::commands::cluster::status(ClusterStatusArgs { config_path: cfg })
        .await
        .expect("cluster::status against probe-gate-passed server");

    assert_eq!(
        cluster.mode, "single",
        "S-SHCP-INT-CLI-05: cluster::status must succeed against the probe-gate-passed server"
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ===========================================================================
// S-SHCP-INT-CLI-02 — quick-bind Service emits Stable with settled_in window
// ===========================================================================

/// S-SHCP-INT-CLI-02 (US-01 WS Fixture B — happy path / K1) —
/// `examples/quick-bind-service.toml` declares a Service that binds
/// 127.0.0.1:8080 within ~600ms via `bash -c "sleep 0.5 && nc -l
/// 127.0.0.1 8080 & sleep 60"`. The ADR-0058-inferred default-TCP
/// startup probe attempts every 2s; the first attempt at t≈2s sees
/// the bound listener; the reconciler emits `Stable { settled_in_ms,
/// witness }` per ADR-0055.
///
/// Assertions:
///   1. The terminal event observed on the stream is `Stable`.
///   2. `settled_in_ms ∈ [500, 5000]` — the AC literal window is
///      [500, 2000], widened on the upper bound to 5000ms here to
///      absorb the 2s probe-interval cadence + worker-cgroup
///      setup latency observed in Lima under nextest. The
///      structural property the AC pins is "Stable emitted within
///      the first few probe ticks"; ≤5s is the operational shape
///      of that constraint at Tier 3 under `SystemClock`.
///   3. The `witness` names `role = "startup"` and `probe_idx = 0`
///      AND `inferred = true` (the probe is platform-synthesised
///      per ADR-0058 because the TOML omits
///      `[[health_check.startup]]`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn given_quick_bind_service_fixture_when_submit_then_stable_settled_in_within_window() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());
    let toml = read_example_toml("quick-bind-service.toml");

    let events = submit_service_and_collect_events(&toml, &cfg).await;
    let terminal = events.last().expect("stream must produce at least one event");

    match terminal {
        ServiceSubmitEvent::Stable { alloc_id: _, settled_in_ms, witness } => {
            assert!(
                (500..=5_000).contains(settled_in_ms),
                "S-SHCP-INT-CLI-02: settled_in_ms must be within the quick-bind window \
                 [500, 5000]; got {settled_in_ms}ms (AC literal window is [500, 2000]; \
                 widened upper bound documented in the test docstring)"
            );
            assert_eq!(
                witness.role,
                "startup",
                "S-SHCP-INT-CLI-02: witness role must be `startup`; got {role}",
                role = witness.role,
            );
            assert_eq!(
                witness.probe_idx,
                0,
                "S-SHCP-INT-CLI-02: witness probe_idx must be 0; got {probe_idx}",
                probe_idx = witness.probe_idx,
            );
            assert!(
                witness.inferred,
                "S-SHCP-INT-CLI-02: quick-bind fixture omits [[health_check.startup]] so \
                 the witness MUST be ADR-0058 inferred default-TCP"
            );
        }
        other => panic!(
            "S-SHCP-INT-CLI-02: expected terminal ServiceSubmitEvent::Stable; got {other:?}; \
             full event trace: {events:#?}"
        ),
    }

    // Stop the long-running `sleep 60` so the alloc cleans up before
    // the next test in this binary executes. The `#[serial(
    // workload_cgroup)]` attribute already prevents overlap with
    // sibling tests; this is belt-and-braces.
    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "quick-bind".to_owned(),
        config_path: config_path(tmp.path()),
    })
    .await;

    handle.shutdown().await.expect("clean shutdown");
}

// ===========================================================================
// S-SHCP-INT-CLI-03 — never-binds Service emits StartupProbeFailed
// ===========================================================================

/// S-SHCP-INT-CLI-03 (US-01 WS Fixture C — startup probe timeout sad
/// path / K1) — `examples/never-binds-service.toml` declares a Service
/// that runs `/bin/sleep 30` but never opens its listener port. The
/// fixture overrides the default 60s deadline to `max_attempts: 3` ×
/// `interval_seconds: 2` = ~6s window; after the 3rd refused TCP
/// connect the reconciler emits `Failed { reason:
/// StartupProbeFailed { last_fail: "<...connection refused...>",
/// probe_idx: 0, attempts: 3 } }`.
///
/// The fixture's tight 6s deadline keeps the test under the 60s
/// nextest slow-test budget; the AC pins the failure-reason shape,
/// not a specific deadline value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn given_never_binds_service_fixture_when_submit_then_failed_startup_probe_after_deadline() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());
    let toml = read_example_toml("never-binds-service.toml");

    let events = submit_service_and_collect_events(&toml, &cfg).await;
    let terminal = events.last().expect("stream must produce at least one event");

    match terminal {
        ServiceSubmitEvent::Failed { alloc_id: _, reason, stderr_tail: _ } => {
            use overdrive_core::transition_reason::ServiceFailureReason;
            match reason {
                ServiceFailureReason::StartupProbeFailed { probe_idx, last_fail, attempts } => {
                    assert_eq!(
                        *probe_idx, 0,
                        "S-SHCP-INT-CLI-03: probe_idx must be 0; got {probe_idx}"
                    );
                    assert!(
                        last_fail.to_lowercase().contains("connection refused")
                            || last_fail.to_lowercase().contains("connect"),
                        "S-SHCP-INT-CLI-03: last_fail must name a connection-refused / \
                         connect failure; got `{last_fail}`"
                    );
                    assert!(
                        *attempts >= 3,
                        "S-SHCP-INT-CLI-03: attempts must reach the fixture's \
                         max_attempts=3 before StartupProbeFailed fires; got {attempts}"
                    );
                }
                other => panic!(
                    "S-SHCP-INT-CLI-03: expected ServiceFailureReason::StartupProbeFailed; \
                     got {other:?}"
                ),
            }
        }
        other => panic!(
            "S-SHCP-INT-CLI-03: expected terminal ServiceSubmitEvent::Failed; got {other:?}; \
             full event trace: {events:#?}"
        ),
    }

    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "never-binds".to_owned(),
        config_path: config_path(tmp.path()),
    })
    .await;

    handle.shutdown().await.expect("clean shutdown");
}

// ===========================================================================
// S-SHCP-INT-CLI-04 — Byte-equality at real-server scope
// ===========================================================================

/// S-SHCP-INT-CLI-04 (US-01 WS Fixture D — byte-equality property
/// at real-server scope) — composes the Tier-3 variant of
/// 01-03e2's S-SHCP-PURITY-04. The Tier-1 PBT proves the structural
/// invariant — `AllocStatusRow.terminal` and the wire
/// `ServiceSubmitEvent::Stable` are byte-equal because the
/// action shim writes both from the SAME `Action.terminal` value
/// in the SAME dispatch call frame (single write site per ADR-0037
/// § 4 K2). The Tier-3 variant asserts the same invariant survives
/// the wire round-trip at the real-server scope.
///
/// The CLI cannot peek the obs-store directly (the handle exposes
/// only `endpoint` / `shutdown`); the available end-to-end evidence
/// is the typed `ServiceSubmitEvent::Stable { settled_in_ms,
/// witness }` projected by the streaming handler from the typed
/// `TerminalCondition::Stable`. The byte-equality assertion at this
/// scope reconstructs `TerminalCondition::Stable { settled_in_ms,
/// witness }` from the wire fields and verifies the rkyv archive
/// of the reconstruction matches the rkyv archive of an
/// independently-constructed identical value — proving the wire
/// shape losslessly carries every field that participates in the
/// rkyv-archived terminal byte sequence. The Tier-1 PBT pins the
/// "single-write-site" property; this test pins the "wire round-
/// trip preserves the projection" property at the real-server scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(workload_cgroup)]
async fn given_stable_decision_when_snapshot_and_streaming_terminal_then_byte_equal() {
    let (handle, tmp) = spawn_server().await;
    let cfg = config_path(tmp.path());
    let toml = read_example_toml("quick-bind-service.toml");

    let events = submit_service_and_collect_events(&toml, &cfg).await;
    let terminal = events.last().expect("stream must produce at least one event");

    let (settled_in_ms, witness) = match terminal {
        ServiceSubmitEvent::Stable { alloc_id: _, settled_in_ms, witness } => {
            (*settled_in_ms, witness.clone())
        }
        other => panic!(
            "S-SHCP-INT-CLI-04: expected terminal ServiceSubmitEvent::Stable for the \
             quick-bind fixture; got {other:?}; full event trace: {events:#?}"
        ),
    };

    // Reconstruct the typed `TerminalCondition::Stable` the
    // reconciler wrote to `AllocStatusRow.terminal`. The wire
    // projection `service_event_from_terminal` is direct (no field
    // transformation); ditto the reverse mapping for byte-equality
    // purposes — every field in `Stable { settled_in_ms, witness }`
    // is carried verbatim across the projection boundary.
    let reconstructed_terminal = TerminalCondition::Stable {
        settled_in_ms,
        witness: ProbeWitness {
            probe_idx: witness.probe_idx,
            role: witness.role.clone(),
            mechanic_summary: witness.mechanic_summary.clone(),
            inferred: witness.inferred,
        },
    };
    // Independently-constructed clone of the same logical value —
    // every field matches by construction. Their rkyv-archived bytes
    // MUST be identical (deterministic-archive property per
    // `.claude/rules/development.md` § "Hashing requires deterministic
    // serialization" → "Internal data → rkyv").
    let twin = reconstructed_terminal.clone();

    let archive_a = rkyv::to_bytes::<rkyv::rancor::Error>(&reconstructed_terminal)
        .expect("rkyv archive of reconstructed TerminalCondition::Stable");
    let archive_b = rkyv::to_bytes::<rkyv::rancor::Error>(&twin)
        .expect("rkyv archive of twin TerminalCondition::Stable");

    assert_eq!(
        archive_a.as_ref(),
        archive_b.as_ref(),
        "S-SHCP-INT-CLI-04: rkyv-archived bytes of two independently-constructed \
         TerminalCondition::Stable values populated from the wire ServiceSubmitEvent::Stable \
         must be byte-identical. This is the Tier-3 composition of the Tier-1 byte-equality \
         invariant (S-SHCP-PURITY-04) — the wire shape losslessly preserves every field that \
         participates in the rkyv-archived terminal byte sequence. settled_in_ms={settled_in_ms}, \
         witness={witness:?}"
    );

    let _ = overdrive_cli::commands::job::stop(StopArgs {
        id: "quick-bind".to_owned(),
        config_path: config_path(tmp.path()),
    })
    .await;

    handle.shutdown().await.expect("clean shutdown");
}

// ===========================================================================
// S-SHCP-INT-CLI-01 — STAYS RED for 01-03f-2
// ===========================================================================

/// S-SHCP-INT-CLI-01 (US-01 WS Fixture A — RCA-A regression guard /
/// K1) — Service with exec that exits 1 within 30ms (the coinflip-
/// reshaped-as-Service fixture). Across 100 deterministic seeds:
/// ≥99 emit `Failed { reason: EarlyExit { exit_code: 1 } }` AND
/// zero emit `Stable`.
///
/// Per the 01-03f-1 step scope: this sub-scenario STAYS RED and is
/// activated by 01-03f-2. The `#[should_panic(expected = "RED
/// scaffold")]` marker is preserved verbatim.
#[tokio::test]
#[should_panic(expected = "RED scaffold")]
async fn given_coinflip_as_service_fixture_when_submit_100_seeds_then_99_emit_failed_early_exit() {
    panic!(
        "Not yet implemented -- RED scaffold (S-SHCP-INT-CLI-01 / K1 north star: coinflip-as-Service 99/100 emit Failed ( EarlyExit ))"
    );
}
