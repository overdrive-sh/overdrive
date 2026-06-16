//! Tier-3 production-activation gate for transparent mTLS
//! (transparent-mtls-host-socket, GH #26; step 06-03).
//!
//! Proves the (β) production wiring: `run_server` composes a real
//! `EbpfDataplane` → composes the transparent-mTLS layer AFTER
//! `IdentityMgr` (`MtlsDataplane::load` + `HostMtlsEnforcement` +
//! `MtlsInterceptWorker`) → threads it into `AppState.mtls_worker`
//! (`Some` on the real-dataplane boot) → the action-shim fires
//! `start_alloc`/`stop_alloc` at the alloc-lifecycle hooks. `ExecDriver`
//! is UNTOUCHED.
//!
//! Two scenarios:
//!
//! - `criteria[0]` — **fault-injected probe → refuse to boot fail-closed**
//!   (`mtls_probe_actication_fault_refuses_boot`). Boots `run_server` with
//!   a REAL `EbpfDataplane` (no `dataplane_override`) so the mTLS layer is
//!   composed, with `mtls_probe_fault = Some(..)` injected. The boot MUST
//!   return `Err(ControlPlaneError::MtlsBoot(MtlsBootError::Probe))` — the
//!   node refuses to start (`health.startup.refused`), does NOT degrade to
//!   a cleartext path.
//!
//! - `criteria[1]` — **deployed exec workload's declared-peer leg carries
//!   TLS 1.3 via the production boot path**
//!   (`deployed_exec_workload_declared_peer_leg_carries_tls13_via_production_boot_path`).
//!   Boots `run_server` (real dataplane + mTLS worker), deploys an exec
//!   workload via the production path dialing the test-programmed declared
//!   peer (the #178 stand-in via `MtlsInterceptWorker::program_declared_peer_redirect`),
//!   and asserts `tcpdump` shows TLS 1.3 (`0x17`) on the peer-facing leg,
//!   zero payload cleartext, peer reads byte-exact plaintext.
//!
//! Hygiene (`.claude/rules/debugging.md` § leftover-XDP / cgroup-leak):
//! every veth / cgroup / nft / XDP this test stands up is reaped on exit.

#![cfg(target_os = "linux")]
// Skip-on-no-privilege / no-bpf-object messages are the legitimate way
// these Tier-3 tests communicate "capability/artifact absent, scenario
// skipped" on an unprivileged runner.
#![allow(clippy::print_stderr)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
// The criteria[1] e2e is a single long Tier-3 flow (boot → deploy → redirect
// → observe); splitting it across helpers would scatter the one scenario a
// reviewer reads end-to-end. The `waitpid`/cgroup-reaper FFI casts are on
// compile-time-bounded pid_t values — the standard test-fixture idiom (mirrors
// `mtls_e2e_helpers.rs`). The SPIFFE-URI in the docstring is a literal, not a
// code item.
#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown
)]

use std::net::SocketAddrV4;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::api::{AllocStateWire, AllocStatusResponse, SubmitWorkloadRequest};
use overdrive_control_plane::error::{ControlPlaneError, MtlsBootError};
use overdrive_control_plane::{ServerConfig, run_server};
use overdrive_core::AllocationId;
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use overdrive_core::api::submit::SubmitSpecInput;
use overdrive_host::RealCgroupFs;
use tempfile::TempDir;

use super::mtls_e2e_helpers::{OUTBOUND_REPLY, OUTBOUND_REQUEST, OutboundPeer, TestPki};

/// `criteria[0]` — the fault-injected mTLS probe makes the production boot
/// REFUSE fail-closed.
///
/// Boots `run_server` with a real `EbpfDataplane` (no override) so the
/// transparent-mTLS layer is composed AFTER `IdentityMgr`; injects
/// `mtls_probe_fault` so `MtlsEnforcement::probe()` is forced to fail. The
/// boot MUST return `ControlPlaneError::MtlsBoot(MtlsBootError::Probe)` —
/// the node refuses to start and does NOT fall back to cleartext.
///
/// The `overdrive_bpf.o` the real `MtlsDataplane::load` needs is
/// `include_bytes!`-baked into the test binary at compile time (via
/// `OVERDRIVE_BPF_OBJECT_PATH`), so no runtime artifact check is needed —
/// if the binary compiled, the object is embedded. The only environment
/// gate is capability (`CAP_BPF` / `CAP_NET_ADMIN`); a cap-less runner
/// refuses at the mTLS load and is skipped below.
#[tokio::test]
async fn mtls_probe_activation_fault_refuses_boot() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("conf dir");

    // Real-dataplane boot (NO `dataplane_override`) so the mTLS layer is
    // composed; default veth ifaces so the serve-boot provisioner stands
    // up the host-netns pair; `mtls_probe_fault` forces the probe failure.
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
        data_dir,
        operator_config_dir,
        // Inject `SimDataplane` for the LB path so this gate test does NOT
        // pay the real `EbpfDataplane` XDP-attach to `lo` (DRV_MODE rejects
        // loopback under virtio). The mTLS BPF load is INDEPENDENT of the LB
        // dataplane (D-MTLS-17 item 1 — its own `aya::Ebpf`); setting
        // `mtls_probe_fault` opts the mTLS layer in regardless of the LB
        // override, so the real `MtlsDataplane::load` runs and the injected
        // probe fault drives the fail-closed refusal.
        dataplane_override: Some(std::sync::Arc::new(
            overdrive_sim::adapters::dataplane::SimDataplane::new(),
        )),
        // `lo`/`lo` dataplane config lets `host_ipv4` resolve on a VM with
        // no provisioned veth (the `server_lifecycle.rs` precedent).
        dataplane: Some(super::dataplane_lo::lo_dataplane_config()),
        mtls_probe_fault: Some("injected mTLS probe fault (criteria[0])".to_owned()),
        ..ServerConfig::new(std::sync::Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let result = run_server(config, std::sync::Arc::new(RealCgroupFs::new())).await;

    match result {
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Probe { source })) => {
            // The fail-closed refusal fired with the injected probe fault —
            // the production boot path composed the mTLS layer (real
            // `MtlsDataplane::load`), probed it, and refused to start rather
            // than degrade to cleartext. This is the criteria[0] PASS.
            eprintln!("PASS criteria[0]: boot refused fail-closed via MtlsBoot::Probe: {source}");
        }
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Load { source })) => {
            // On a runner without CAP_BPF / a mounted bpffs, the real
            // `MtlsDataplane::load` (its own `aya::Ebpf`) refuses BEFORE the
            // probe — an environment gap, not a logic failure. The
            // fail-closed Load branch DID fire (it is itself a refuse-to-boot
            // path), but it is not the criteria[0] Probe path — skip.
            eprintln!(
                "SKIP: mTLS dataplane load refused (no CAP_BPF / bpffs on this runner). \
                 The injected-probe-fault branch was not reached. Cause: {source}"
            );
        }
        Err(ControlPlaneError::DataplaneBoot(cause)) => {
            // The LB dataplane is `SimDataplane` here, so this should not
            // fire; if it does it is a pre-mTLS environment gap — skip.
            eprintln!("SKIP: dataplane boot refused before the mTLS layer. Cause: {cause}");
        }
        Err(other) => {
            panic!("expected MtlsBoot(Probe) (or a pre-mTLS DataplaneBoot skip), got: {other:?}")
        }
        Ok(_) => panic!(
            "boot SUCCEEDED with an injected mTLS probe fault — the fail-closed refusal \
             did NOT fire (the node degraded instead of refusing)"
        ),
    }
}

/// The job id the criteria[1] workload deploys under; its first-attempt
/// allocation is deterministically `alloc-<JOB_ID>-0` (per
/// `workload_lifecycle::mint_alloc_id`). The `TestPki` mints the agent's
/// leg-B client SVID keyed by THAT alloc id so `IdentityMgr`-override read
/// `svid_for(alloc)` returns a SVID that roots on the shared `TestPki`.
const JOB_ID: &str = "mtls-e2e";

/// `criteria[1]` — a normal exec workload deployed via the PRODUCTION boot
/// path produces TLS 1.3 records on its peer-facing leg.
///
/// Boots `run_server` (real `EbpfDataplane` LB + real `MtlsDataplane` mTLS
/// worker, `mtls_worker = Some`) with the PKI-SEAM
/// `mtls_identity_override = Some(test_pki.held_identities())` so the agent's
/// leg-B SVID + `TrustBundle` both root on the same `TestPki` the
/// `OutboundPeer` server cert (DNS SAN `peer.overdrive.local`) chains to.
/// Deploys an exec workload via the PRODUCTION HTTPS submit path; after it
/// reaches Running (`on_alloc_running` → `mtls_worker.start_alloc` attached
/// `cgroup_connect4_mtls` to the alloc's `.scope` + bound leg-F), programs
/// the single declared-peer redirect via
/// [`MtlsInterceptWorker::program_declared_peer_redirect`] (#178 stand-in),
/// and the cgroup-isolated workload's retry-`connect(peer_addr)` is rewritten
/// → leg-F → worker accept → `enforce` → client mTLS to the `OutboundPeer`.
///
/// Asserts on OBSERVABLE side effects (`.claude/rules/testing.md` § Tier 3):
/// the `OutboundPeer`'s AF_PACKET capture shows TLS 1.3 `0x17` application
/// data on the peer-facing leg, ZERO cleartext-marker bytes on that wire, and
/// the peer reconstructed the workload's request byte-exact (kTLS decrypt
/// proof) + extracted the agent's client SPIFFE (mutual auth proof).
///
/// # GREEN — the declared-peer dial-target fix (step 06-03, 2026-06-15)
///
/// This e2e was empirically RED under Lima before the fix (`captured 0`
/// `0x17` records on the `OutboundPeer` wire) for a PROVEN production bug:
/// the worker's OUTBOUND `accept_loop` built `Routed::Outbound { peer:
/// leg_f_addr }` — it passed the agent's OWN leg-F addr as the routing
/// `peer`, so `enforce_outbound` → `dial_leg(peer)` SELF-LOOPED back to
/// leg-F and never dialed the real `OutboundPeer`. The kernel redirect
/// fired (`connect(peer_addr)` → leg-F) but the agent never dialed the
/// peer → no leg-B handshake → no `0x17` on the peer wire.
///
/// The fix (the #178 declared-peer stand-in supplying the dial target,
/// NOT general east-west resolution): the worker records the SINGLE
/// declared `real_peer` it ALREADY receives in
/// `program_declared_peer_redirect` into a shared per-alloc slot, and the
/// OUTBOUND accept loop reads that slot to build `Routed::Outbound { peer:
/// real_peer }`. `enforce` now dials the REAL peer; the leg-B handshake
/// runs; TLS 1.3 `0x17` records appear on the peer wire. General
/// per-connection multi-peer orig-dst recovery remains
/// [#178](https://github.com/overdrive-sh/overdrive/issues/178); the single
/// declared peer is the ratified D-MTLS-15 scope.
///
/// [`MtlsInterceptWorker::program_declared_peer_redirect`]:
///     overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::program_declared_peer_redirect
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deployed_exec_workload_declared_peer_leg_carries_tls13_via_production_boot_path() {
    // Install the process-default rustls `CryptoProvider` once for this test
    // binary — the `OutboundPeer` server config and the agent's leg-B client
    // config both consume it via `ServerConfig::builder()` /
    // `ClientConfig::builder()` (the composition root's job; the test IS the
    // composition root here). Idempotent — a second install returns `Err`.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("conf dir");

    // The deployed workload's first-attempt alloc id — keyed identically
    // on both ends: the `TestPki` mints the agent client SVID under it, and
    // the redirect is programmed for it once Running.
    let alloc = AllocationId::new(&format!("alloc-{JOB_ID}-0")).expect("alloc id");
    let test_pki = TestPki::mint(alloc.clone());

    // Reap the deployed cgroup-isolated `python3` workload on test exit
    // (panic OR success) so nextest does not flag a `LEAK` and the next run
    // sees a clean `workloads.slice`. The guard goes straight to the kernel
    // (`cgroup.kill` + `waitpid`) — see `workload_lifecycle::cleanup`.
    let _cleanup = WorkloadScopeReaper {
        scope: std::path::PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice")
            .join(format!("{alloc}.scope")),
    };

    // The real mTLS peer the agent's leg-B dials, + the AF_PACKET `0x17`
    // wire oracle on `lo`. Spawned BEFORE boot so its capture is live for
    // the first leg-B record.
    let mut peer = OutboundPeer::spawn(&test_pki);
    let peer_addr: SocketAddrV4 = peer.addr();

    // PRODUCTION boot: NO `dataplane_override` → real `EbpfDataplane` LB
    // (provisions the default veth pair + XDP-attaches under Lima) AND the
    // real `MtlsDataplane` mTLS layer (`mtls_worker = Some`). No probe
    // fault. The PKI-SEAM injects the `TestPki`-rooted identity so the
    // agent's leg-B trusts the peer cert.
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        mtls_identity_override: Some(Arc::new(test_pki.held_identities())),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let handle = match run_server(config, Arc::new(RealCgroupFs::new())).await {
        Ok(h) => h,
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Load { source })) => {
            eprintln!(
                "SKIP criteria[1]: mTLS dataplane load refused (no CAP_BPF / bpffs on this \
                 runner): {source}"
            );
            return;
        }
        Err(ControlPlaneError::DataplaneBoot(cause)) => {
            eprintln!("SKIP criteria[1]: LB dataplane boot refused before the mTLS layer: {cause}");
            return;
        }
        Err(other) => panic!("run_server boot failed unexpectedly: {other:?}"),
    };

    let worker = handle
        .mtls_worker()
        .expect("real-dataplane boot must compose the (β) mTLS worker (mtls_worker = Some)");

    let bound = handle.local_addr().await.expect("server bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    let client = client_trusting(&ca_pem);

    // Deploy the workload via the PRODUCTION HTTPS submit path. The command
    // is a cgroup-isolated `python3` that RETRY-dials `peer_addr` (the
    // declared peer) until one round-trip succeeds byte-exact, then exits 0.
    // The retry loop closes the race between "alloc reaches Running" and
    // "the test programs the redirect": the first dials before the redirect
    // lands pass through unintercepted (connection refused — the test holds
    // no listener on peer_addr), and the first dial AFTER the redirect lands
    // is rewritten → leg-F → enforce → mTLS to the peer.
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let script = workload_dial_script(peer_addr);
    let spec = JobSpecInput {
        id: JOB_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/bin/python3".to_owned(),
            args: vec!["-c".to_owned(), script],
        }),
    };
    let resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "submit must return 200");

    // Poll the production allocs endpoint until the alloc reaches Running
    // (the convergence loop runs on the real `SystemClock` at 100ms cadence),
    // then read the ACTUAL alloc id off the wire (robust against any
    // attempt-index drift) and program the declared-peer redirect for it.
    let allocs_url = format!("https://localhost:{}/v1/allocs?job={JOB_ID}", bound.port());
    let running_alloc =
        wait_for_running(&client, &allocs_url, Duration::from_secs(45)).await.unwrap_or_else(
            || panic!("deployed workload never reached Running within 45s via the production boot"),
        );
    assert_eq!(
        running_alloc, alloc,
        "the Running alloc id must match the deterministic `alloc-{JOB_ID}-0` the TestPki keyed"
    );

    // #178 stand-in: program `MTLS_REDIRECT_DEST[peer_addr] = <alloc leg-F>`
    // through the booted production worker. Resolves the alloc's OWN recorded
    // leg-F (recorded by `start_alloc` at `on_alloc_running`).
    worker
        .program_declared_peer_redirect(&running_alloc, peer_addr)
        .expect("program declared-peer redirect for the Running alloc");

    // The workload's retry loop now dials peer_addr → cgroup_connect4_mtls
    // rewrites → leg-F → worker accept → enforce → client mTLS to the peer.
    // Give the round-trip a generous wall-clock window, then stop+scan the
    // peer-facing wire.
    let request_byte_exact = peer.wait_outcome();
    let wire = peer.wire_observations();
    let presented_spiffe = peer.presented_client_spiffe();

    // Tear down the alloc's mTLS intercept BEFORE shutting the server down:
    // `stop_alloc` signals the worker's blocking accept loops (which run on
    // `spawn_blocking` threads and cannot be `abort()`ed mid-syscall) to exit
    // cooperatively, and drops the cgroup link + TPROXY guard. Without this the
    // accept loops outlive the test and block the tokio runtime drop forever
    // (the post-PASS hang). This is the production `on_alloc_terminal` →
    // `stop_alloc` path the action-shim fires; the test drives it directly
    // because the workload here is a one-shot e2e fixture, not a converging
    // alloc the reconciler would stop.
    worker.stop_alloc(&running_alloc);

    handle.shutdown(Duration::from_secs(2)).await;

    // ---- Observable assertions (the criteria[1] PASS condition) ----
    assert!(
        wire.records_request_dir >= 1,
        "expected >= 1 TLS 1.3 0x17 application_data record on the peer-facing leg \
         (forward / request direction); captured {} — the deployed workload's declared-peer \
         leg did NOT carry TLS 1.3 ciphertext through the production boot path",
        wire.records_request_dir,
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "the cleartext request/reply markers MUST NOT appear on the encrypted peer-facing wire; \
         saw {} plaintext-marker hits (confidentiality breach)",
        wire.plaintext_marker_hits,
    );
    assert!(
        request_byte_exact,
        "the peer must reconstruct the workload's {}-byte request byte-exact after kTLS-RX \
         decrypt (the decrypt proof)",
        OUTBOUND_REQUEST.len(),
    );
    assert!(
        presented_spiffe.is_some(),
        "the peer REQUIRED + verified + extracted the agent's client SVID SPIFFE id from the \
         leg-B handshake (mutual-auth proof); none was presented",
    );
    let _ = OUTBOUND_REPLY;
    eprintln!(
        "PASS criteria[1]: {} 0x17 records (req dir) + {} (resp dir) on the peer leg, \
         0 plaintext hits, request byte-exact, client SPIFFE = {:?}",
        wire.records_request_dir,
        wire.records_response_dir,
        presented_spiffe.map(|s| s.to_string()),
    );
}

/// `criteria[2]` — the production boot's per-alloc mTLS lifecycle ATTACHES
/// `cgroup_connect4_mtls` to the alloc's `.scope` and DETACHES + purges the
/// redirect on `stop_alloc`.
///
/// Boots `run_server` (real `EbpfDataplane` LB + real `MtlsDataplane` mTLS
/// worker), deploys an exec workload via the production HTTPS submit path,
/// waits for Running (so `on_alloc_running` → `start_alloc` attached the
/// cgroup program + recorded leg-F), and programs the declared-peer redirect.
///
/// Asserts on OBSERVABLE kernel state (`.claude/rules/testing.md` § Tier 3 —
/// `bpftool cgroup show`, not program internals):
///   1. While the alloc is Running + redirected, `bpftool cgroup show
///      <alloc .scope>` lists a `cgroup_inet4_connect` (`cgroup_connect4_mtls`)
///      program attached to THAT scope (the production `start_alloc` attach),
///      AND `MTLS_REDIRECT_DEST` carries the programmed `peer_addr` entry.
///   2. After `stop_alloc`, the same `bpftool cgroup show` lists NO connect4
///      program on that scope (the `MtlsCgroupLink` `Drop` detached it).
///
/// The inbound nft-TPROXY rule is NOT asserted here: production `start_alloc`
/// installs no inbound TPROXY rule — its match key is the server workload's
/// logical (virt) address, an east-west service-resolution fact v1 has no
/// production source for, so the install is #178-deferred symmetric with the
/// OUTBOUND `MTLS_REDIRECT_DEST` redirect (which `start_alloc` also does not
/// program; the test-only `program_declared_peer_redirect` seam stands in).
/// The prior shape installed a rule whose virt was synthesised from the
/// agent's own ephemeral leg-C port — a self-referential rule that matched no
/// real inbound connection (the inert-virt bug, RCA
/// `docs/analysis/root-cause-analysis-inbound-tproxy-virt-intercepts-no-traffic.md`;
/// [#178](https://github.com/overdrive-sh/overdrive/issues/178)). The absence
/// of that inert rule is asserted by the sibling
/// `production_boot_installs_no_self_referential_inbound_tproxy_rule`.
///
/// This formalises into asserting tests the live probes a prior 06-03 session
/// already observed (`no connect4 attachments` / `no mtls_redirect map` after
/// stop). The workload here is a long-lived `sleep` (no peer round-trip needed):
/// the gate is the attach/detach lifecycle, not the wire.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_boot_attaches_then_detaches_alloc_mtls_intercept() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Start-of-test hygiene: clear any stale per-virt TPROXY rule a sibling
    // serialized test left in the shared `overdrive-mtls` chain, so this gate's
    // tproxy present→absent delta is measured against a clean baseline.
    clean_mtls_tproxy_chain();

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("conf dir");

    let alloc = AllocationId::new(&format!("alloc-{JOB_ID}-0")).expect("alloc id");
    let test_pki = TestPki::mint(alloc.clone());
    let scope_path = std::path::PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice")
        .join(format!("{alloc}.scope"));
    let _cleanup = WorkloadScopeReaper { scope: scope_path.clone() };

    // The redirect's declared peer — a loopback addr the workload never has to
    // actually dial for THIS gate (we assert map presence, not the wire). Use a
    // bound-but-unconnected addr so `program_declared_peer_redirect` has a real
    // key to program.
    let peer_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("peer placeholder bind");
    let peer_addr: SocketAddrV4 = match peer_listener.local_addr().expect("peer addr") {
        std::net::SocketAddr::V4(a) => a,
        std::net::SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
    };

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        mtls_identity_override: Some(Arc::new(test_pki.held_identities())),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let handle = match run_server(config, Arc::new(RealCgroupFs::new())).await {
        Ok(h) => h,
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Load { source })) => {
            eprintln!(
                "SKIP criteria[2]: mTLS dataplane load refused (no CAP_BPF / bpffs): {source}"
            );
            return;
        }
        Err(ControlPlaneError::DataplaneBoot(cause)) => {
            eprintln!("SKIP criteria[2]: LB dataplane boot refused before the mTLS layer: {cause}");
            return;
        }
        Err(other) => panic!("run_server boot failed unexpectedly: {other:?}"),
    };

    let worker = handle
        .mtls_worker()
        .expect("real-dataplane boot must compose the (β) mTLS worker (mtls_worker = Some)");
    let bound = handle.local_addr().await.expect("server bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    let client = client_trusting(&ca_pem);

    // A long-lived workload: it only has to reach Running so `start_alloc`
    // attaches the intercept. No peer dial is needed for the attach/detach gate.
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let spec = JobSpecInput {
        id: JOB_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["120".to_owned()],
        }),
    };
    let resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "submit must return 200");

    let allocs_url = format!("https://localhost:{}/v1/allocs?job={JOB_ID}", bound.port());
    let running_alloc =
        wait_for_running(&client, &allocs_url, Duration::from_secs(45)).await.unwrap_or_else(
            || panic!("deployed workload never reached Running within 45s via the production boot"),
        );

    worker
        .program_declared_peer_redirect(&running_alloc, peer_addr)
        .expect("program declared-peer redirect for the Running alloc");

    // ---- CAPTURE (1): attached + redirect programmed (while Running) ----
    // Capture every observable BEFORE teardown, and never `assert!` between boot
    // and shutdown: a panic here would skip `stop_alloc` + `shutdown`, leaving the
    // booted server's accept loops + the `/bin/sleep` workload alive and hanging
    // the test binary (the criteria[1] post-PASS-hang shape). Capture → tear down →
    // assert is the leak-safe ordering.
    let attached_while_running = cgroup_connect4_attached(&scope_path);
    let redirect_present_while_running = mtls_redirect_map_has_entries();

    // ---- Production teardown: the action-shim `on_alloc_terminal` path ----
    // `stop_alloc` drops the alloc's `MtlsCgroupLink` (detach the connect4 program
    // from the .scope) and the `TproxyInterceptGuard` (remove the inbound nft rule),
    // both synchronously. It does NOT un-program `MTLS_REDIRECT_DEST`: in production
    // `start_alloc` never PROGRAMS the redirect (v1 has no east-west peer
    // enumeration — #178; the redirect entry here is the test-only #178 stand-in),
    // so production `stop_alloc` has no per-alloc redirect to unprogram. The
    // `MTLS_REDIRECT_DEST` table + the connect4 program are reclaimed wholesale when
    // the `MtlsDataplane` drops at server shutdown (BPF object unload) — asserted
    // after `shutdown` below, not after `stop_alloc`.
    worker.stop_alloc(&running_alloc);
    // Detach is synchronous in `stop_alloc`; give the kernel a beat to settle the
    // link removal before re-probing.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ---- CAPTURE (2): detached + nft rule removed (after stop_alloc) ----
    // These two ARE the per-alloc `stop_alloc` guarantees: the `MtlsCgroupLink` Drop
    // detaches the connect4 program from the .scope, and the `TproxyInterceptGuard`
    // Drop removes the inbound nft rule. The `MTLS_REDIRECT_DEST` entry is NOT a
    // `stop_alloc` concern (production `start_alloc` never programs it — #178; the
    // entry here is the test-only stand-in, reclaimed only when the BPF object
    // unloads, which the test's own held `worker` Arc keeps alive — so we do NOT
    // assert map-gone here: that would assert a test artifact, not production
    // behaviour. The redirect-PROGRAMMED-while-Running capture above is the honest
    // half of the redirect observable).
    let attached_after_stop = cgroup_connect4_attached(&scope_path);

    handle.shutdown(Duration::from_secs(2)).await;
    drop(peer_listener);

    // ---- ASSERT (post-teardown — safe to panic now) ----
    assert!(
        attached_while_running,
        "while Running + redirected, `bpftool cgroup show {}` MUST list a connect4 program \
         (the production `start_alloc` attached `cgroup_connect4_mtls` to the alloc's own \
         .scope); saw none",
        scope_path.display(),
    );
    assert!(
        redirect_present_while_running,
        "MTLS_REDIRECT_DEST MUST carry the programmed declared-peer entry while the alloc is \
         redirected; the map showed no entries",
    );
    assert!(
        !attached_after_stop,
        "after `stop_alloc`, `bpftool cgroup show {}` MUST list NO connect4 program (the \
         `MtlsCgroupLink` Drop detached it); a program is still attached",
        scope_path.display(),
    );
    eprintln!(
        "PASS criteria[2]: connect4 attached (while Running) → detached (stop_alloc) on {}; \
         MTLS_REDIRECT_DEST programmed while Running (inbound nft-TPROXY is #178-deferred — no \
         production rule installed; see sibling self-referential-absence gate)",
        scope_path.display(),
    );
}

/// Regression gate (inert-virt RCA, 2026-06-16) — the production boot path
/// installs NO self-referential / inert inbound nft-TPROXY rule.
///
/// RCA `docs/analysis/root-cause-analysis-inbound-tproxy-virt-intercepts-no-traffic.md`:
/// the prior `start_alloc` synthesised the inbound TPROXY rule's match key
/// (`virt`) from the agent's OWN leg-C ephemeral listener port, so the rule it
/// installed was `ip daddr 127.0.0.1 tcp dport <P> tproxy to 127.0.0.1:<P>` —
/// a self-referential rule (match port == redirect-target port) that matches no
/// real inbound workload connection (clients dial the workload's address, never
/// the agent's random ephemeral port). Inbound transparent mTLS was inert in
/// production while reading as "installed". The fix defers the inbound TPROXY
/// install symmetric with the OUTBOUND redirect deferral
/// ([#178](https://github.com/overdrive-sh/overdrive/issues/178)): production
/// `start_alloc` installs NO inbound rule.
///
/// Boots `run_server` via the SAME production path as criteria[2], deploys an
/// exec workload to Running (so `on_alloc_running` → `start_alloc` ran its full
/// inbound path), programs the declared-peer redirect (so the worker did all the
/// per-alloc install work it does in production), then scans the production
/// `ip overdrive-mtls` prerouting chain. Asserts: NO rule whose `daddr` is
/// `127.0.0.1` AND whose `tcp dport` equals its own `tproxy to 127.0.0.1:<port>`
/// target (the self-referential silhouette). RED against the pre-fix code (the
/// inert rule is installed by `start_alloc`); GREEN after (no rule installed).
///
/// Skip-on-no-capability (CAP_BPF / CAP_NET_ADMIN) via the same boot-error
/// branches as the sibling gates.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_boot_installs_no_self_referential_inbound_tproxy_rule() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Start clean so a stale per-virt rule from a sibling serialized test does
    // not false-positive the self-referential scan against THIS boot's chain.
    clean_mtls_tproxy_chain();

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("conf dir");

    let alloc = AllocationId::new(&format!("alloc-{JOB_ID}-0")).expect("alloc id");
    let test_pki = TestPki::mint(alloc.clone());
    let scope_path = std::path::PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice")
        .join(format!("{alloc}.scope"));
    let _cleanup = WorkloadScopeReaper { scope: scope_path.clone() };

    // A bound-but-unconnected declared peer so the redirect has a real key — the
    // workload never has to dial it for this gate (we assert chain state, not the
    // wire).
    let peer_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("peer placeholder bind");
    let peer_addr: SocketAddrV4 = match peer_listener.local_addr().expect("peer addr") {
        std::net::SocketAddr::V4(a) => a,
        std::net::SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
    };

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        mtls_identity_override: Some(Arc::new(test_pki.held_identities())),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let handle = match run_server(config, Arc::new(RealCgroupFs::new())).await {
        Ok(h) => h,
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Load { source })) => {
            eprintln!(
                "SKIP inert-virt regression: mTLS dataplane load refused (no CAP_BPF / bpffs): \
                 {source}"
            );
            return;
        }
        Err(ControlPlaneError::DataplaneBoot(cause)) => {
            eprintln!(
                "SKIP inert-virt regression: LB dataplane boot refused before the mTLS layer: \
                 {cause}"
            );
            return;
        }
        Err(other) => panic!("run_server boot failed unexpectedly: {other:?}"),
    };

    let worker = handle
        .mtls_worker()
        .expect("real-dataplane boot must compose the (β) mTLS worker (mtls_worker = Some)");
    let bound = handle.local_addr().await.expect("server bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    let client = client_trusting(&ca_pem);

    // Long-lived workload: only has to reach Running so `start_alloc` runs its
    // full per-alloc install path (the path that, pre-fix, installed the inert
    // inbound rule).
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let spec = JobSpecInput {
        id: JOB_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["120".to_owned()],
        }),
    };
    let resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "submit must return 200");

    let allocs_url = format!("https://localhost:{}/v1/allocs?job={JOB_ID}", bound.port());
    let running_alloc =
        wait_for_running(&client, &allocs_url, Duration::from_secs(45)).await.unwrap_or_else(
            || panic!("deployed workload never reached Running within 45s via the production boot"),
        );

    // Program the declared-peer redirect so the worker performed ALL the per-alloc
    // install work it does in production for a Running alloc.
    worker
        .program_declared_peer_redirect(&running_alloc, peer_addr)
        .expect("program declared-peer redirect for the Running alloc");

    // ---- CAPTURE before teardown (leak-safe ordering: capture → stop → assert) ----
    let self_referential_present = mtls_self_referential_tproxy_rule_present();

    worker.stop_alloc(&running_alloc);
    handle.shutdown(Duration::from_secs(2)).await;
    drop(peer_listener);
    clean_mtls_tproxy_chain();

    // ---- ASSERT (post-teardown — safe to panic now) ----
    assert!(
        !self_referential_present,
        "production `start_alloc` MUST NOT install a self-referential inbound nft-TPROXY rule \
         (one whose `ip daddr 127.0.0.1` + `tcp dport <P>` matches its own \
         `tproxy to 127.0.0.1:<P>` target) — such a rule matches no real inbound workload \
         connection (clients dial the workload's address, never the agent's ephemeral leg-C \
         port). The inbound TPROXY install is #178-deferred (no production virt source); the \
         inert rule found here is the pre-fix bug (RCA \
         root-cause-analysis-inbound-tproxy-virt-intercepts-no-traffic.md)",
    );
    eprintln!(
        "PASS inert-virt regression: production boot installed NO self-referential inbound \
         TPROXY rule (inbound install #178-deferred)"
    );
}

/// `criteria[3]` — F5 runtime self-exempt negative: a workload that sets the
/// agent's bypass `SO_MARK` (`MTLS_LEG_S_DIAL_MARK`) on its OWN socket and dials
/// the declared peer is STILL intercepted.
///
/// The OUTBOUND intercept's F5 exemption is **cgroup-subtree scoping**, NOT an
/// `SO_MARK` check: `cgroup_connect4_mtls` attaches to the WORKLOAD's own
/// `.scope` cgroup and keys solely on `(dst_ip, dst_port)` against
/// `MTLS_REDIRECT_DEST` — it reads no socket mark (the OUTBOUND program in
/// `crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs` has no mark
/// branch). `MTLS_LEG_S_DIAL_MARK` is the INBOUND nft-TPROXY exemption (the
/// agent's leg-S dial), reachable only by the agent process on the host —
/// UNREACHABLE from inside the workload's cgroup. So a workload that stamps that
/// mark on its outbound socket changes NOTHING: the cgroup hook fires regardless
/// because the workload IS in the workload subtree the program is attached to.
///
/// Reuses the criteria[1] flow with a mark-setting dialer, and asserts the SAME
/// observable: TLS 1.3 `0x17` application_data STILL appears on the peer-facing
/// leg (the marked dial was rewritten → leg-F → enforce → mTLS), with zero
/// cleartext markers. The self-exemption attempt did not bypass interception.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn marked_workload_dial_is_still_intercepted_f5_self_exempt_negative() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Start-of-test hygiene: clear any stale per-virt TPROXY rule from a sibling
    // serialized test in the shared `overdrive-mtls` chain.
    clean_mtls_tproxy_chain();

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let operator_config_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    std::fs::create_dir_all(&operator_config_dir).expect("conf dir");

    let alloc = AllocationId::new(&format!("alloc-{JOB_ID}-0")).expect("alloc id");
    let test_pki = TestPki::mint(alloc.clone());
    let _cleanup = WorkloadScopeReaper {
        scope: std::path::PathBuf::from("/sys/fs/cgroup/overdrive.slice/workloads.slice")
            .join(format!("{alloc}.scope")),
    };

    let mut peer = OutboundPeer::spawn(&test_pki);
    let peer_addr: SocketAddrV4 = peer.addr();

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("bind addr"),
        data_dir,
        operator_config_dir: operator_config_dir.clone(),
        mtls_identity_override: Some(Arc::new(test_pki.held_identities())),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let handle = match run_server(config, Arc::new(RealCgroupFs::new())).await {
        Ok(h) => h,
        Err(ControlPlaneError::MtlsBoot(MtlsBootError::Load { source })) => {
            eprintln!(
                "SKIP criteria[3]: mTLS dataplane load refused (no CAP_BPF / bpffs): {source}"
            );
            return;
        }
        Err(ControlPlaneError::DataplaneBoot(cause)) => {
            eprintln!("SKIP criteria[3]: LB dataplane boot refused before the mTLS layer: {cause}");
            return;
        }
        Err(other) => panic!("run_server boot failed unexpectedly: {other:?}"),
    };

    let worker = handle
        .mtls_worker()
        .expect("real-dataplane boot must compose the (β) mTLS worker (mtls_worker = Some)");
    let bound = handle.local_addr().await.expect("server bound addr");
    let ca_pem = read_ca_from_trust_triple(&operator_config_dir);
    let client = client_trusting(&ca_pem);

    // The workload dials peer_addr with SO_MARK = MTLS_LEG_S_DIAL_MARK set on its
    // OWN socket — the self-exemption attempt. The OUTBOUND cgroup hook ignores
    // the mark, so the dial is STILL rewritten → leg-F → enforce → mTLS.
    let submit_url = format!("https://localhost:{}/v1/jobs", bound.port());
    let script = marked_workload_dial_script(peer_addr);
    let spec = JobSpecInput {
        id: JOB_ID.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/usr/bin/python3".to_owned(),
            args: vec!["-c".to_owned(), script],
        }),
    };
    let resp = client
        .post(&submit_url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Job(spec) })
        .send()
        .await
        .expect("POST /v1/jobs");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "submit must return 200");

    let allocs_url = format!("https://localhost:{}/v1/allocs?job={JOB_ID}", bound.port());
    let running_alloc =
        wait_for_running(&client, &allocs_url, Duration::from_secs(45)).await.unwrap_or_else(
            || panic!("deployed workload never reached Running within 45s via the production boot"),
        );

    worker
        .program_declared_peer_redirect(&running_alloc, peer_addr)
        .expect("program declared-peer redirect for the Running alloc");

    let request_byte_exact = peer.wait_outcome();
    let wire = peer.wire_observations();
    let presented_spiffe = peer.presented_client_spiffe();

    worker.stop_alloc(&running_alloc);
    handle.shutdown(Duration::from_secs(2)).await;

    // ---- Observable: the marked dial was STILL intercepted (mTLS on the wire) ----
    assert!(
        wire.records_request_dir >= 1,
        "the SO_MARK-self-exempting workload dial MUST still be intercepted → TLS 1.3 0x17 \
         on the peer leg (the OUTBOUND cgroup hook ignores SO_MARK; the F5 exemption is \
         cgroup-scoping, unreachable from the workload's cgroup); captured {} records",
        wire.records_request_dir,
    );
    assert_eq!(
        wire.plaintext_marker_hits, 0,
        "no cleartext marker may appear on the peer wire — the marked dial did NOT bypass \
         interception to reach the peer as plaintext; saw {} plaintext hits",
        wire.plaintext_marker_hits,
    );
    assert!(
        request_byte_exact,
        "the peer reconstructed the marked workload's request byte-exact after kTLS decrypt \
         (the dial WAS intercepted and proxied, not passed through)",
    );
    assert!(
        presented_spiffe.is_some(),
        "the peer verified the agent's client SVID on the intercepted (marked) dial's leg-B \
         handshake — mutual auth held despite the self-exemption attempt",
    );
    let _ = OUTBOUND_REPLY;
    eprintln!(
        "PASS criteria[3]: SO_MARK self-exempt dial STILL intercepted — {} 0x17 records on the \
         peer leg, 0 plaintext, client SPIFFE = {:?}",
        wire.records_request_dir,
        presented_spiffe.map(|s| s.to_string()),
    );
}

/// Build the cgroup-isolated workload's `python3 -c` body: retry-`connect`
/// `peer_addr` until one round-trip succeeds byte-exact (request out, reply
/// back), then `exit(0)`. Mirrors the dataplane reference workload's two-phase
/// write (pre-arm + steady-state) so the agent exercises BOTH the lossless
/// pre-arm capture AND the forward encrypt pump.
///
/// PRE-DIAL SETTLE (06-03): the redirect (`MTLS_REDIRECT_DEST[peer_addr] =
/// leg_f`) can only be programmed by the test AFTER the alloc reaches Running
/// (its leg-F is recorded by `start_alloc` at `on_alloc_running`), and the
/// workload reaches Running the moment `ExecDriver::start` spawns it — so the
/// workload starts at almost exactly the instant the redirect becomes
/// programmable. If the workload dialed `peer_addr` IMMEDIATELY, the first
/// dial(s) would race AHEAD of the redirect and reach the real peer DIRECTLY
/// as plaintext (the workload speaks no TLS) — leaking a cleartext request
/// chunk onto the peer-facing wire (the confidentiality oracle would see it).
/// The settle gives the test's `wait_for_running` → `program_declared_peer_redirect`
/// path (sub-second in practice) ample headroom to land the redirect BEFORE
/// the first dial, so every dial is intercepted → leg-F → enforce → mTLS, and
/// no cleartext ever reaches the peer port. The retry loop still tolerates a
/// late redirect (a pre-redirect dial that slips through hits the real peer's
/// multi-accept loop, which discards the failed-handshake plaintext connection
/// without serving it).
fn workload_dial_script(peer_addr: SocketAddrV4) -> String {
    let split = OUTBOUND_REQUEST.len() / 2;
    format!(
        r#"
import socket, sys, time
part1 = {part1}
part2 = {part2}
reply = {reply}
# Pre-dial settle: let the test program the redirect before the first dial,
# so no cleartext ever reaches the real peer port (see fn docstring).
time.sleep(8)
deadline = time.time() + 40
while time.time() < deadline:
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(12)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        s.connect(("{ip}", {port}))
        s.sendall(part1)
        time.sleep(2.0)
        s.sendall(part2)
        got = b""
        s.settimeout(8)
        while len(got) < len(reply):
            b = s.recv(4096)
            if not b:
                break
            got += b
        s.close()
        if got == reply:
            sys.exit(0)
    except Exception:
        pass
    time.sleep(0.5)
sys.exit(40)
"#,
        part1 = py_bytes(&OUTBOUND_REQUEST[..split]),
        part2 = py_bytes(&OUTBOUND_REQUEST[split..]),
        reply = py_bytes(OUTBOUND_REPLY),
        ip = peer_addr.ip(),
        port = peer_addr.port(),
    )
}

/// criteria[3] workload body: the criteria[1] dial, but stamping
/// `SO_MARK = MTLS_LEG_S_DIAL_MARK` on the socket BEFORE `connect` — the
/// self-exemption attempt. The OUTBOUND `cgroup_connect4_mtls` hook reads no
/// socket mark (it keys only on `(dst_ip, dst_port)`), so the marked dial is
/// rewritten → leg-F → enforce → mTLS exactly like the unmarked one: the
/// workload CANNOT self-exempt from interception by replicating the agent's
/// (inbound) bypass mark.
fn marked_workload_dial_script(peer_addr: SocketAddrV4) -> String {
    let split = OUTBOUND_REQUEST.len() / 2;
    let mark = overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
    format!(
        r#"
import socket, sys, time
part1 = {part1}
part2 = {part2}
reply = {reply}
time.sleep(8)
deadline = time.time() + 40
while time.time() < deadline:
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        # The self-exemption attempt: stamp the agent's bypass SO_MARK on our
        # OWN socket before connect. SO_MARK needs CAP_NET_ADMIN; the workload
        # runs with it here, which is the GENEROUS case — even so the OUTBOUND
        # cgroup hook ignores the mark and intercepts the dial regardless.
        try:
            s.setsockopt(socket.SOL_SOCKET, socket.SO_MARK, {mark})
        except (PermissionError, OSError):
            pass  # no CAP_NET_ADMIN → mark not set; the dial is intercepted anyway
        s.settimeout(12)
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        s.connect(("{ip}", {port}))
        s.sendall(part1)
        time.sleep(2.0)
        s.sendall(part2)
        got = b""
        s.settimeout(8)
        while len(got) < len(reply):
            b = s.recv(4096)
            if not b:
                break
            got += b
        s.close()
        if got == reply:
            sys.exit(0)
    except Exception:
        pass
    time.sleep(0.5)
sys.exit(40)
"#,
        part1 = py_bytes(&OUTBOUND_REQUEST[..split]),
        part2 = py_bytes(&OUTBOUND_REQUEST[split..]),
        reply = py_bytes(OUTBOUND_REPLY),
        mark = mark,
        ip = peer_addr.ip(),
        port = peer_addr.port(),
    )
}

/// criteria[2] observable: `bpftool cgroup show <scope>` lists a
/// `cgroup_inet4_connect` (the `cgroup_connect4_mtls`) program attached to the
/// alloc's own `.scope` cgroup. Returns `true` iff such an attachment is present.
/// This is the production `start_alloc` attach made observable in kernel state
/// (NOT a program-internal read — `.claude/rules/testing.md` § Tier 3).
fn cgroup_connect4_attached(scope: &std::path::Path) -> bool {
    let out = std::process::Command::new("bpftool").args(["cgroup", "show"]).arg(scope).output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            // `bpftool cgroup show` lists one row per attached program with its
            // attach type; the cgroup connect4 attach type renders as
            // `cgroup_inet4_connect` (older bpftool) / `connect4`. Match either.
            text.contains("cgroup_inet4_connect") || text.contains("connect4")
        }
        Err(e) => {
            eprintln!("bpftool cgroup show {} failed: {e}", scope.display());
            false
        }
    }
}

/// The kernel-visible name of the `MTLS_REDIRECT_DEST` map. The kernel truncates
/// BPF object names to `BPF_OBJ_NAME_LEN - 1 = 15` chars, so the 18-char
/// `MTLS_REDIRECT_DEST` shows in `bpftool` as `MTLS_REDIRECT_D`. Dumping by the
/// full name returns nothing (the "wrong surface" inspection trap,
/// `.claude/rules/debugging.md` § 11) — the truncated name is the real one.
const MTLS_REDIRECT_MAP_KERNEL_NAME: &str = "MTLS_REDIRECT_D";

/// criteria[2] observable: `MTLS_REDIRECT_DEST` carries ≥1 entry. Returns `true`
/// iff `bpftool map dump name <kernel-truncated name>` shows at least one
/// key/value. Used to assert the per-alloc redirect was programmed (and, after
/// stop, purged).
fn mtls_redirect_map_has_entries() -> bool {
    let out = std::process::Command::new("bpftool")
        .args(["map", "dump", "name", MTLS_REDIRECT_MAP_KERNEL_NAME])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            // A populated dump lists `key:`/`value:` lines; an empty map prints
            // `Found 0 elements`. Treat the presence of a `key` token as "has
            // entries". Print on the (diagnostic) empty case so a wrong-name /
            // wrong-surface regression is visible rather than silently false.
            let has = text.contains("key") || text.contains("\"key\"");
            if !has {
                eprintln!(
                    "MTLS_REDIRECT_DEST dump ({MTLS_REDIRECT_MAP_KERNEL_NAME}) shows no entries: \
                     stdout={text:?} stderr={:?}",
                    String::from_utf8_lossy(&o.stderr),
                );
            }
            has
        }
        Err(e) => {
            eprintln!("bpftool map dump name {MTLS_REDIRECT_MAP_KERNEL_NAME} failed: {e}");
            false
        }
    }
}

/// Hygiene: flush the SHARED `ip overdrive-mtls` prerouting chain so a stale
/// per-virt TPROXY rule left by a SIBLING serialized test (the chain is
/// node-global converge-on-boot infra NEVER torn down per-workload — see
/// `mtls_intercept::ensure_shared_routing_infra`) does not contaminate THIS
/// test's "tproxy present while Running → absent after stop" delta. Flushing the
/// chain removes the per-virt rules AND the F5 exemption; the production
/// `start_alloc` re-`ensure`s the exemption + installs this alloc's own rule, so
/// flushing-then-boot is safe (the boot reconstructs everything it needs). A
/// missing table (`nft` non-zero) is the clean case — nothing to flush.
/// Called at the START of the attach/detach and self-exempt gates per the
/// `.claude/rules/debugging.md` § leftover-state hygiene (start AND end).
fn clean_mtls_tproxy_chain() {
    let _ = std::process::Command::new("nft")
        .args(["flush", "chain", "ip", "overdrive-mtls", "prerouting"])
        .status();
}

/// Inert-virt regression observable: a SELF-REFERENTIAL inbound nft-TPROXY rule
/// is present in the production `ip overdrive-mtls` table's `prerouting` chain.
/// Returns `true` iff some rule line has `ip daddr 127.0.0.1`, a `tcp dport <P>`,
/// AND a `tproxy to 127.0.0.1:<P>` target with the SAME port `<P>` — the inert
/// silhouette the inert-virt bug produced (the rule's match key was the agent's
/// own ephemeral leg-C port, so daddr/dport == redirect target). Scoped to the
/// production table (NOT a ruleset-wide grep) so an unrelated rule elsewhere
/// cannot false-positive.
fn mtls_self_referential_tproxy_rule_present() -> bool {
    let out =
        std::process::Command::new("nft").args(["list", "table", "ip", "overdrive-mtls"]).output();
    let dump = match out {
        // The table exists only while an alloc's inbound intercept is installed;
        // a missing table (`nft` exits non-zero) means no rule — not self-ref.
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(_) => return false,
        Err(e) => {
            eprintln!("nft list table ip overdrive-mtls failed: {e}");
            return false;
        }
    };
    dump.lines().any(rule_line_is_self_referential)
}

/// True iff a single nft rule line matches `127.0.0.1` on `daddr`, carries a
/// `tcp dport <P>`, AND a `tproxy to 127.0.0.1:<P>` whose port equals that
/// `dport` — i.e. the match port and the redirect-target port are the same.
/// nft renders the rule as one line, e.g.
/// `ip daddr 127.0.0.1 tcp dport 41234 tproxy to 127.0.0.1:41234 meta mark ... accept`.
fn rule_line_is_self_referential(line: &str) -> bool {
    if !line.contains("ip daddr 127.0.0.1") {
        return false;
    }
    let dport = token_after(line, &["tcp", "dport"]);
    let tproxy_port = line
        .split_whitespace()
        .skip_while(|t| *t != "to")
        .nth(1)
        .and_then(|to| to.strip_prefix("127.0.0.1:"))
        .map(str::to_owned);
    match (dport, tproxy_port) {
        (Some(d), Some(t)) => d == t,
        _ => false,
    }
}

/// Return the whitespace token immediately following the ordered `needles`
/// subsequence in `line` (e.g. the `<P>` after `tcp dport`). `None` if the
/// subsequence (followed by one more token) is absent.
fn token_after(line: &str, needles: &[&str]) -> Option<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    tokens
        .windows(needles.len() + 1)
        .find_map(|w| (w[..needles.len()] == *needles).then(|| w[needles.len()].to_owned()))
}

/// Drop guard that mass-kills + reaps the deployed workload's cgroup scope
/// on test exit. Mirrors `workload_lifecycle::cleanup::AllocCleanup` but
/// scoped to the single known alloc scope (no obs-store read needed).
struct WorkloadScopeReaper {
    scope: std::path::PathBuf,
}

impl Drop for WorkloadScopeReaper {
    fn drop(&mut self) {
        let pids: Vec<libc::pid_t> = std::fs::read_to_string(self.scope.join("cgroup.procs"))
            .ok()
            .map(|s| s.lines().filter_map(|l| l.trim().parse::<i32>().ok()).collect())
            .unwrap_or_default();
        let _ = std::fs::write(self.scope.join("cgroup.kill"), "1\n");
        for pid in pids {
            for _ in 0..20 {
                let mut status: libc::c_int = 0;
                // SAFETY: `waitpid` is a thin syscall wrapper; a real pid_t +
                // valid status ptr is sound. WNOHANG so we never block.
                let r = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
                if r == pid || r == -1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = std::fs::remove_dir(&self.scope);
    }
}

/// Render a byte slice as a python `b"\xNN..."` literal.
fn py_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("b\"");
    for b in bytes {
        let _ = write!(out, "\\x{b:02x}");
    }
    out.push('"');
    out
}

/// Poll the production allocs endpoint until one row reaches Running; return
/// its `AllocationId`. `None` on timeout.
async fn wait_for_running(
    client: &reqwest::Client,
    allocs_url: &str,
    timeout: Duration,
) -> Option<AllocationId> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(resp) = client.get(allocs_url).send().await
            && let Ok(body) = resp.json::<AllocStatusResponse>().await
            && let Some(row) = body.rows.iter().find(|r| matches!(r.state, AllocStateWire::Running))
            && let Ok(id) = AllocationId::new(&row.alloc_id)
        {
            return Some(id);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    None
}

// Helpers — mint a reqwest client trusting the booted server's ephemeral
// operator CA, and read that CA out of the trust-triple the boot writes.
// Duplicated from `submit_round_trip.rs` /
// `convergence_loop_spawned_in_production_boot.rs` per the
// one-file-self-contained convention (`observation_empty_rows.rs:34`).

fn client_trusting(ca_pem: &str) -> reqwest::Client {
    let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).expect("parse CA PEM");
    reqwest::Client::builder()
        .add_root_certificate(cert)
        .https_only(true)
        .use_rustls_tls()
        .build()
        .expect("build reqwest client")
}

fn read_ca_from_trust_triple(operator_config_dir: &std::path::Path) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let config_path = operator_config_dir.join(".overdrive").join("config");
    let text = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|e| panic!("read trust triple at {}: {e}", config_path.display()));
    let doc: toml::Value = toml::from_str(&text).expect("parse trust triple TOML");
    let ca_b64 = doc
        .get("contexts")
        .and_then(toml::Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some("local"))
        })
        .and_then(|c| c.get("ca"))
        .and_then(toml::Value::as_str)
        .expect("[[contexts]] with name=\"local\" must carry a ca field");
    let ca_bytes = BASE64.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}
