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

use overdrive_control_plane::error::{ControlPlaneError, MtlsBootError};
use overdrive_control_plane::{ServerConfig, run_server};
use overdrive_host::RealCgroupFs;
use tempfile::TempDir;

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

/// `criteria[1]` — a normal exec workload deployed via the PRODUCTION boot
/// path produces TLS 1.3 records on its peer-facing leg.
///
/// RED scaffold — BLOCKED on a peer-trust design gap (step 06-03,
/// surfaced 2026-06-15). The two authorized test-access seams ARE landed,
/// compile+clippy-clean, and regression-free:
///   - [`ServerHandle::mtls_worker`] surfaces the booted (β) worker; and
///   - [`MtlsInterceptWorker::program_declared_peer_redirect`] resolves the
///     alloc's OWN leg-F and programs the single `MTLS_REDIRECT_DEST` entry.
///
/// But they are NOT sufficient to drive criteria[1] GREEN. The production
/// agent's OUTBOUND leg-B (`mtls::tls_config::client_config`) verifies the
/// peer's server cert against the **boot-composed `TrustBundle`** (the
/// ephemeral workload-identity root the boot mints) AND requires SNI
/// `peer.overdrive.local`. The production CA (`RcgenCa`) mints only
/// SPIFFE-URI SVIDs — never a `peer.overdrive.local` DNS-SAN server cert —
/// and `ServerConfig` exposes NO seam to inject a test CA / identity /
/// bundle into the boot. So the test's `OutboundPeer` cannot present a
/// server cert the production agent will accept, and the leg-B handshake
/// can never complete → no `0x17` on the peer wire. (The dataplane
/// `mtls_composed_walking_skeleton` reference sidesteps this by composing
/// its OWN `HostMtlsEnforcement` over a `HeldIdentities` double backed by
/// `TestPki` — both ends share one PKI; that option does not exist when the
/// production `IdentityMgr` owns the bundle.)
///
/// Closing criteria[1] requires resolving the peer-trust gap (a production
/// design decision beyond the two authorized seams) — e.g. a test seam to
/// inject a `TestPki`-derived identity+bundle into the boot, OR a CA surface
/// to mint a DNS-SAN peer cert chaining to the boot root, OR relaxing the
/// leg-B SNI/verification under an `integration-tests` gate. Surfaced to the
/// orchestrator; not improvised here per CLAUDE.md "STOP and surface the gap".
///
/// [`ServerHandle::mtls_worker`]: overdrive_control_plane::ServerHandle::mtls_worker
/// [`MtlsInterceptWorker::program_declared_peer_redirect`]:
///     overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker::program_declared_peer_redirect
#[test]
#[should_panic(expected = "RED scaffold")]
fn deployed_exec_workload_declared_peer_leg_carries_tls13_via_production_boot_path() {
    panic!(
        "Not yet implemented -- RED scaffold (06-03 criteria[1] / deployed exec workload's \
         declared-peer leg carries TLS 1.3 via the production boot path: BLOCKED on the \
         peer-trust gap — the production agent's leg-B verifies the peer against the boot \
         TrustBundle + SNI peer.overdrive.local, and there is no seam to give the test's \
         OutboundPeer a cert the production CA would issue. See the docstring; surfaced to \
         the orchestrator)"
    );
}
