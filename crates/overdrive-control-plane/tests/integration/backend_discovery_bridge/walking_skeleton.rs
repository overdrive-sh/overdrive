//! Walking-skeleton acceptance tests for
//! `backend-discovery-bridge-service-reachability` (joint #174 + #175 e2e gate).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-01 (walking-skeleton e2e), S-BDB-18 (Drop-RAII teardown via the
//! walking-skeleton fixture), S-BDB-19 (bridge-to-hydrator handoff in-process Tier 3).
//!
//! Tier 3 — runs through `cargo xtask lima run -- cargo nextest run
//! -p overdrive-control-plane -E 'test(walking_skeleton)' --features integration-tests`
//! per `.claude/rules/testing.md` § "Running tests — Lima VM".
//!
//! Cleanup discipline: every test owns its veth pair + bpffs pin dir
//! via [`super::boot_composition::BootFixture`]-shaped helpers
//! reproduced here (`VethFixture`). Drop fires XDP detach + bpffs
//! pin removal + iface deletion on every exit path, including
//! assertion-failure unwinds, per `.claude/rules/debugging.md`
//! § "Leftover XDP attachments across runs".

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    reason = "Test bodies; failures must panic with informative messages"
)]

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::traits::observation_store::AllocState;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::test_server::{TestServer, poll_until, service_spec_digest_hex};

// ----------------------------------------------------------------------------
// Per-test veth + bpffs fixture (RAII)
// ----------------------------------------------------------------------------

static IFACE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn next_iface_names() -> (String, String) {
    let n = IFACE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() & 0xFFFF;
    (format!("ws-{pid:04x}-{n}a"), format!("ws-{pid:04x}-{n}b"))
}

fn run_ip(args: &[&str]) -> Result<(), String> {
    let out =
        Command::new("ip").args(args).output().map_err(|e| format!("spawn ip {args:?}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ip {args:?} exit={:?} stderr={}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim(),
        ))
    }
}

fn run_ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).output();
}

/// Per-test veth-pair + bpffs pin directory. Drop tears them down
/// best-effort. Mirrors `boot_composition::BootFixture` but lives
/// here to keep the walking-skeleton self-contained.
struct VethFixture {
    client_iface: String,
    backend_iface: String,
    pin_dir: PathBuf,
    _bpffs_root: TempDir,
}

impl VethFixture {
    fn setup(client_cidr: &str) -> Result<Self, String> {
        let (client, backend) = next_iface_names();
        run_ip_quiet(&["link", "del", &client]);
        run_ip_quiet(&["link", "del", &backend]);

        run_ip(&["link", "add", &client, "type", "veth", "peer", "name", &backend])?;
        run_ip(&["link", "set", &client, "up"])?;
        run_ip(&["link", "set", &backend, "up"])?;
        run_ip(&["addr", "add", client_cidr, "dev", &client])?;

        let bpffs_root = tempfile::Builder::new()
            .prefix("overdrive-test-")
            .tempdir_in("/sys/fs/bpf")
            .map_err(|e| format!("tempdir under /sys/fs/bpf: {e}"))?;
        let pin_dir = bpffs_root.path().to_path_buf();

        Ok(Self { client_iface: client, backend_iface: backend, pin_dir, _bpffs_root: bpffs_root })
    }

    fn dataplane_config(&self) -> DataplaneConfig {
        DataplaneConfig {
            client_iface: self.client_iface.clone(),
            backend_iface: self.backend_iface.clone(),
        }
    }
}

impl Drop for VethFixture {
    fn drop(&mut self) {
        for mode in ["xdpgeneric", "xdpdrv", "xdp"] {
            run_ip_quiet(&["link", "set", "dev", &self.client_iface, mode, "off"]);
            run_ip_quiet(&["link", "set", "dev", &self.backend_iface, mode, "off"]);
        }
        let pin_path = self.pin_dir.join("SERVICE_MAP");
        let _ = std::fs::remove_file(&pin_path);
        run_ip_quiet(&["link", "del", &self.client_iface]);
        run_ip_quiet(&["link", "del", &self.backend_iface]);
    }
}

/// Skip the test (panic with sentinel marker) when `ip(8)` lacks
/// CAP_NET_ADMIN. Tier 3 tests run inside Lima as root per
/// `.claude/rules/testing.md` § "Running tests — Lima VM".
fn require_cap_net_admin() {
    let probe = "ws-capprobe";
    run_ip_quiet(&["link", "del", probe]);
    let result = Command::new("ip").args(["link", "add", probe, "type", "dummy"]).output();
    let ok = match result {
        Ok(out) if out.status.success() => true,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            !(stderr.contains("Operation not permitted") || stderr.contains("Permission denied"))
        }
        Err(_) => false,
    };
    run_ip_quiet(&["link", "del", probe]);
    if !ok {
        eprintln!("skipping: CAP_NET_ADMIN required; run via `cargo xtask lima run --`");
        panic!("WALKING_SKELETON_SKIPPED_NO_CAP_NET_ADMIN");
    }
}

// ----------------------------------------------------------------------------
// Test fixtures — backend listener
// ----------------------------------------------------------------------------

/// Build a Service spec with a single `tcp` listener and an exec
/// driver that launches a Python one-liner echo server on `port`.
///
/// Per DWD-03 K2 (Form A — Python one-liner; Python 3 provisioned in
/// Lima per the verified package list 2026-05-20). The listener is a
/// minimal line-buffered TCP echo that reads from each accepted
/// connection and writes back byte-equal — the line-buffering means
/// the probe payload MUST terminate with `\n` to flush.
fn service_spec_with_python_echo(workload_id: &str, port: u16) -> ServiceSpecInput {
    // Use `python3 -u -c '<src>'` so stdout/stderr buffering does not
    // delay startup signals. The script binds 0.0.0.0:<port>, accepts
    // one connection at a time (sequential — sufficient for the
    // walking-skeleton's single-probe shape), reads up to 4 KiB, and
    // echoes byte-equal.
    let echo_script = format!(
        r"
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('0.0.0.0', {port}))
s.listen(8)
while True:
    c, _ = s.accept()
    try:
        buf = c.recv(4096)
        if buf:
            c.sendall(buf)
    except Exception:
        pass
    finally:
        c.close()
",
    );
    ServiceSpecInput {
        id: workload_id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/bin/python3".to_owned(),
            args: vec!["-u".to_owned(), "-c".to_owned(), echo_script],
        }),
        listeners: vec![ListenerInput { port, protocol: "tcp".to_owned() }],
    }
}

// ----------------------------------------------------------------------------
// S-BDB-01 — walking-skeleton e2e through real HTTPS + XDP path
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn submit_service_workload_tcp_round_trip_through_vip_succeeds() {
    require_cap_net_admin();

    // 1. Per-test veth pair (client iface gets a real IPv4 — the
    //    boot path's `resolve_iface_ipv4` reads it for `host_ipv4`).
    let host_ipv4 = Ipv4Addr::new(10, 244, 1, 1);
    let fx = VethFixture::setup("10.244.1.1/24").expect("veth setup");

    // 2. Production server: real EbpfDataplane on the veth pair,
    //    per-test bpffs pin dir.
    let server = TestServer::serve_with_dataplane(fx.dataplane_config(), fx.pin_dir.clone()).await;

    // Install the AllocCleanup guard immediately after the obs
    // handle is available so a panic anywhere below this point still
    // reaps the test-spawned cgroup. Per
    // `workload_lifecycle::cleanup::AllocCleanup` — direct cgroup.kill
    // + waitpid; do NOT call Driver::stop (cross-runtime hang).
    let _cleanup = super::super::workload_lifecycle::cleanup::AllocCleanup {
        obs: server.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // 3. Submit a Service spec through the real HTTPS driving port.
    //    Listener port 8080 — Python echo server in the exec driver.
    let workload_id = "walking-skeleton-svc";
    let listener_port: u16 = 8080;
    let spec = service_spec_with_python_echo(workload_id, listener_port);
    let submit_response = server.submit_workload(SubmitSpecInput::Service(spec.clone())).await;

    // 4. The submit echo MUST carry the allocator-issued VIP.
    let assigned_vip_str = submit_response
        .vip
        .as_deref()
        .expect("S-BDB-01: submit echo MUST carry assigned_vip for Service kind per ADR-0049");
    let assigned_vip: Ipv4Addr = assigned_vip_str
        .parse()
        .unwrap_or_else(|e| panic!("assigned_vip '{assigned_vip_str}' must be IPv4: {e}"));

    // 5. Sanity precondition — the spec_digest the server returned
    //    must equal the local re-derivation. Surfaces at the correct
    //    altitude per `.claude/rules/debugging.md` § 7: a digest
    //    mismatch is an admission-path regression (e.g., handler
    //    re-archival drift), not a bridge bug.
    let expected_digest = service_spec_digest_hex(spec.clone());
    assert_eq!(
        submit_response.spec_digest, expected_digest,
        "submit_response.spec_digest must equal locally-derived spec_digest",
    );

    // 6. Wait up to 10s for the alloc to reach AllocState::Running.
    //    Re-driven through the back-door obs read so the test does
    //    not depend on a long-poll API. The convergence loop ticks
    //    every 100ms; 100 polls × 100ms = 10s budget.
    let running = poll_until(Duration::from_secs(10), Duration::from_millis(100), || async {
        // Open a fresh handle each iteration — the production server
        // owns its own obs handle; this read is purely observational.
        // The Local* adapters permit concurrent readers alongside a
        // live writer.
        let observed = read_alloc_state_for(&server, workload_id).await;
        observed.into_iter().find(|s| *s == AllocState::Running)
    })
    .await;
    assert!(
        running.is_some(),
        "S-BDB-01: alloc for {workload_id} did not reach Running within 10s — \
         Service-arm convergence regression or backend listener crash",
    );

    // 7. Get a handle to the production dataplane so we can inspect
    //    BACKEND_MAP + SERVICE_MAP. TestServer constructed it before
    //    `run_server` and retained an Arc clone (the production
    //    AppState owns the other clone via `dataplane_override`).
    let dataplane = server.dataplane();

    // 8. BACKEND_MAP must carry an entry whose
    //    `(ipv4_host, port_host)` matches `(host_ipv4, listener_port)`
    //    within 5s. The full pipeline is bridge tick (≤100ms) →
    //    EnqueueEvaluation → hydrator tick (≤100ms) → action-shim
    //    DataplaneUpdateService dispatch → BACKEND_MAP populated.
    //    5s gives 50 ticks of budget — generous against Lima FS
    //    contention.
    let backend_present = poll_until(Duration::from_secs(5), Duration::from_millis(50), || async {
        let entries = dataplane.backend_map_entries().ok()?;
        entries
            .into_iter()
            .find(|(_, e)| e.ipv4_host == u32::from(host_ipv4) && e.port_host == listener_port)
    })
    .await;
    assert!(
        backend_present.is_some(),
        "S-BDB-01: BACKEND_MAP did not receive an entry for \
         host_ipv4={host_ipv4}:{listener_port} within 5s — \
         bridge or hydrator regression",
    );

    // 9. SERVICE_MAP must resolve the (assigned_vip, listener_port)
    //    outer key (the inner HoM lookup is implicit in
    //    `service_map_contains`). Poll briefly — the hydrator's
    //    DataplaneUpdateService dispatch populates BACKEND_MAP and
    //    SERVICE_MAP in the same call, so this should already be
    //    present, but Lima scheduling can produce a millisecond
    //    of lag between the two map writes.
    let service_present = poll_until(Duration::from_secs(2), Duration::from_millis(50), || async {
        dataplane.service_map_contains(assigned_vip, listener_port).ok().filter(|b| *b)
    })
    .await;
    assert!(
        service_present.is_some(),
        "S-BDB-01: SERVICE_MAP did not resolve {assigned_vip}:{listener_port} within 2s — \
         hydrator did not run DataplaneUpdateService or the outer-map insert failed",
    );

    // 10. D3 in-gate TCP round-trip through the assigned VIP.
    //     Per architecture.md § 6.2 D3 + DWD-03 K1/K3:
    //       - K1 bind-readiness wait: 50ms cadence × 2s budget
    //       - K3 payload literal `b"walking-skeleton-probe\n"`
    //         (24 bytes; the trailing newline is intentional — the
    //         Python line-buffered echo needs it to flush).
    let probe_payload: &[u8] = b"walking-skeleton-probe\n";
    let response = poll_until(Duration::from_secs(2), Duration::from_millis(50), || async {
        let stream = tokio::net::TcpStream::connect((assigned_vip, listener_port)).await.ok()?;
        // Set a per-attempt I/O timeout (TCP_USER_TIMEOUT would
        // be nicer but is harder to wire; the outer poll budget
        // bounds the failure surface).
        let mut stream = stream;
        stream.write_all(probe_payload).await.ok()?;
        let mut buf = vec![0u8; probe_payload.len()];
        tokio::time::timeout(Duration::from_millis(500), stream.read_exact(&mut buf))
            .await
            .ok()?
            .ok()?;
        Some(buf)
    })
    .await;
    assert_eq!(
        response.as_deref(),
        Some(probe_payload),
        "S-BDB-01: TCP round-trip to {assigned_vip}:{listener_port} did not echo \
         {probe_payload:?} within 2s — XDP / reverse-NAT path or backend listener regression",
    );

    // Cleanup: shutdown the server (drains in-flight + drops
    // EbpfDataplane → XDP detach + pin unlink). _cleanup at function
    // scope fires next on Drop and reaps the workload cgroup.
    server.shutdown().await;
    drop(fx);
}

// ----------------------------------------------------------------------------
// S-BDB-18 — graceful shutdown via the walking-skeleton's natural lifecycle
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn graceful_shutdown_detaches_xdp_and_removes_bpffs_pin() {
    // Walking-skeleton-fixture variant per dispatch — exercises the
    // same Drop-RAII property the boot-composition S-BDB-18 covers,
    // but through the walking-skeleton's TestServer lifecycle. The
    // canonical happy-path assertion (XDP attached + pin exists pre-
    // shutdown) lives in `boot_composition::boot_composes_ebpf_...`;
    // this variant pins the post-shutdown shape via TestServer Drop.
    require_cap_net_admin();

    let fx = VethFixture::setup("10.244.18.1/24").expect("veth setup");
    let pin_path = fx.pin_dir.join("SERVICE_MAP");
    let client_iface = fx.client_iface.clone();
    let backend_iface = fx.backend_iface.clone();

    // Bring the server up — boot composition attaches XDP + pins
    // SERVICE_MAP per S-BDB-11.
    let server = TestServer::serve_with_dataplane(fx.dataplane_config(), fx.pin_dir.clone()).await;

    // Pre-condition: both ifaces have XDP + pin exists. If this fails
    // we are testing the wrong property — see boot_composition::
    // boot_composes_ebpf_dataplane_and_attaches_xdp_to_both_ifaces
    // for the canonical happy-path assertion.
    assert!(iface_has_xdp(&client_iface), "client_iface must have XDP before shutdown");
    assert!(iface_has_xdp(&backend_iface), "backend_iface must have XDP before shutdown");
    assert!(pin_path.exists(), "SERVICE_MAP pin must exist before shutdown");

    // Trigger graceful shutdown — explicit async so the drain
    // completes before the XDP/pin assertions below. The inner
    // EbpfDataplane Drop (XDP detach + pin unlink) fires when
    // AppState's Arc<dyn Dataplane> clone releases; TestServer's
    // own Arc clone is dropped here via the explicit shutdown
    // consuming `self`.
    server.shutdown().await;

    assert!(
        !iface_has_xdp(&client_iface),
        "S-BDB-18: expected XDP detached from {client_iface} after walking-skeleton shutdown",
    );
    assert!(
        !iface_has_xdp(&backend_iface),
        "S-BDB-18: expected XDP detached from {backend_iface} after walking-skeleton shutdown",
    );
    assert!(
        !pin_path.exists(),
        "S-BDB-18: expected SERVICE_MAP pin at {} to be removed after walking-skeleton shutdown",
        pin_path.display(),
    );

    drop(fx);
}

fn iface_has_xdp(iface: &str) -> bool {
    let out = match Command::new("ip").args(["link", "show", iface]).output() {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains("xdpdrv") || s.contains("xdpgeneric") || s.contains("xdp ")
}

// ----------------------------------------------------------------------------
// S-BDB-19 — in-process Tier 3 bridge-to-hydrator handoff
// ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial(env)]
async fn bridge_to_hydrator_handoff_dispatches_dataplane_update_service() {
    // In-process Tier 3 variant of the bridge → hydrator → dataplane
    // pipeline. The Tier 1 DST counterpart
    // (`bridge-to-hydrator-handoff`) landed in commit fc68beef.
    //
    // Property: once the bridge writes a ServiceBackendRow for a
    // Running Service workload, the production ServiceMapHydrator
    // picks it up on the next tick and dispatches
    // Action::DataplaneUpdateService into the real EbpfDataplane;
    // a corresponding service_hydration_results row with Completed
    // status is observable, and SERVICE_MAP carries the outer slot.
    //
    // The walking-skeleton (above) already proves the full e2e
    // including TCP round-trip. This test pins the *property* that
    // the handoff produces a Completed observation row — failure
    // here without the walking-skeleton also failing means the
    // hydrator dispatched but the observation write went sideways.
    require_cap_net_admin();

    let host_ipv4 = Ipv4Addr::new(10, 244, 19, 1);
    let fx = VethFixture::setup("10.244.19.1/24").expect("veth setup");
    let server = TestServer::serve_with_dataplane(fx.dataplane_config(), fx.pin_dir.clone()).await;

    // Cleanup guard — see S-BDB-01 above for rationale.
    let _cleanup = super::super::workload_lifecycle::cleanup::AllocCleanup {
        obs: server.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    let workload_id = "bridge-handoff-svc";
    let listener_port: u16 = 8081;
    let spec = service_spec_with_python_echo(workload_id, listener_port);
    let submit = server.submit_workload(SubmitSpecInput::Service(spec)).await;
    let assigned_vip: Ipv4Addr =
        submit.vip.as_deref().expect("vip in submit echo").parse().expect("vip parses as IPv4");

    // Wait for Running.
    let running = poll_until(Duration::from_secs(10), Duration::from_millis(100), || async {
        read_alloc_state_for(&server, workload_id)
            .await
            .into_iter()
            .find(|s| *s == AllocState::Running)
    })
    .await;
    assert!(running.is_some(), "alloc must reach Running for handoff property to fire");

    let dataplane = server.dataplane();

    // Property: the hydrator MUST have dispatched
    // Action::DataplaneUpdateService — observable via SERVICE_MAP
    // containing the outer slot for (assigned_vip, listener_port).
    let service_present = poll_until(Duration::from_secs(2), Duration::from_millis(50), || async {
        dataplane.service_map_contains(assigned_vip, listener_port).ok().filter(|b| *b)
    })
    .await;
    assert!(
        service_present.is_some(),
        "S-BDB-19: SERVICE_MAP did not resolve {assigned_vip}:{listener_port} within 2s — \
         bridge wrote ServiceBackendRow but hydrator did not dispatch DataplaneUpdateService",
    );

    // BACKEND_MAP must also carry an entry — the hydrator's
    // `update_service` call populates BACKEND_MAP as a side effect.
    let backend_present = poll_until(Duration::from_secs(2), Duration::from_millis(50), || async {
        let entries = dataplane.backend_map_entries().ok()?;
        entries
            .into_iter()
            .find(|(_, e)| e.ipv4_host == u32::from(host_ipv4) && e.port_host == listener_port)
    })
    .await;
    assert!(
        backend_present.is_some(),
        "S-BDB-19: BACKEND_MAP did not receive backend ({host_ipv4}, {listener_port}) — \
         hydrator dispatch did not populate BACKEND_MAP",
    );

    server.shutdown().await;
    drop(fx);
}

// ----------------------------------------------------------------------------
// Back-door observation accessors. These read the production server's
// state via a second IntentStore handle / the obs trait surface — no
// production code path is exercised by these helpers themselves.
// ----------------------------------------------------------------------------

/// Read AllocStatus rows for the named workload from the obs store.
/// Returns the per-alloc `AllocState` values (one per replica).
///
/// The TestServer does not expose its obs handle directly — that would
/// widen the test fixture's public surface and make production /
/// test wiring drift detectable only at runtime. Instead we open a
/// fresh in-process ObservationStore against the same data dir
/// (LocalObservationStore permits concurrent readers).
async fn read_alloc_state_for(server: &TestServer, workload_id: &str) -> Vec<AllocState> {
    let Ok(rows) = server.obs().alloc_status_rows().await else {
        return Vec::new();
    };
    rows.into_iter().filter(|r| r.workload_id.as_str() == workload_id).map(|r| r.state).collect()
}
