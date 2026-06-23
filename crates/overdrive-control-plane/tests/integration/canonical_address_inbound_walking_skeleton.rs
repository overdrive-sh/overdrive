//! S-WS — KEYSTONE WALKING SKELETON (GH #241), relocated to the
//! `overdrive-control-plane` test tree (R1).
//!
//! The mandatory #241 acceptance gate: an in-process **real `run_server` boot**
//! (the production composition root) + a server mesh workload deployed through
//! the real in-process deploy submit handler, where a client dialing the server
//! at its canonical `workload_addr:service_port` **directly** (no DNS) is
//! captured by the **PRODUCTION-installed** inbound nft-TPROXY rule — the rule
//! `start_alloc` installs from `spec.{workload_addr, service_ports}` (step
//! 03-01), NOT by the test — mTLS terminates on the production leg-C, and the
//! client's request bytes reach the server workload byte-for-byte and its reply
//! returns byte-for-byte.
//!
//! THE vertical-slice gate (CLAUDE.md § "Build vertical slices through production
//! entry points"): **NO test-installed `install_inbound_tproxy`, NO synthetic
//! loopback virt.** The C3 seam supplies `workload_addr`,
//! `WorkloadLifecycle::project_service_listen_ports` supplies `service_ports`,
//! and `start_alloc` installs the rule. **Litmus:** delete the 03-01 production
//! install and this test goes RED — the dial is not captured and the round-trip
//! fails.
//!
//! ## Why this lives in `overdrive-control-plane`, not `overdrive-worker` (R1)
//!
//! In-process `run_server` / `ServerConfig` / `mtls_identity_override` all live
//! in `overdrive-control-plane` (`src/lib.rs`), and `overdrive-control-plane`
//! depends-on `overdrive-worker` — a reverse edge is a Cargo-rejected cycle, so
//! a worker-crate test physically **cannot** reach in-process `run_server`. The
//! direct precedent (`backend_discovery_bridge/walking_skeleton.rs`) lives in
//! this same control-plane test tree.
//!
//! ## Why the REAL `EbpfDataplane`, NO `dataplane_override` (R2)
//!
//! The production boot gate `compose_mtls = config.dataplane_override.is_none()
//! || config.mtls_probe_fault.is_some()` (`overdrive-control-plane/src/lib.rs`)
//! switches the **mTLS worker OFF** whenever a `dataplane_override` is present,
//! and the only override-compatible compose path (`mtls_probe_fault`) forces a
//! fail-closed boot refusal — so a `dataplane_override` and a working mTLS worker
//! cannot coexist. The keystone needs the composed mTLS worker for the leg-C
//! server handshake, so it MUST run the real dataplane (`dataplane_override =
//! None`). The **only** test seam injected at the in-process composition boundary
//! is `mtls_identity_override = Some(TestPki)` (read on the `compose_mtls = true`
//! path for the leg-C/leg-B handshakes); everything else is production wiring.
//!
//! ## Litmus is the TRANSITIVE round-trip — no map inspection (R2)
//!
//! A successful canonical-address mTLS round-trip IS the proof that the LB gate
//! fell through to nft-TPROXY: a gated mesh backend MISSES `LOCAL_BACKEND_MAP`
//! (so the dial is not short-circuited by the cgroup LB path) and the
//! production-installed inbound rule diverts it. The keystone asserts on the
//! round-trip, NOT on `LOCAL_BACKEND_MAP` contents.
//!
//! ## Merge gate (DELIVER obligation #3 — MERGE-BLOCKING)
//!
//! The spike verdicts are dev-Lima kernel 7.0; the authoritative signal is the
//! **pinned-6.18 appliance-kernel Tier-3 matrix** (ADR-0068). The DELIVER roadmap
//! AC asserts the bidirectional mesh loop passes the **pinned-6.18 Tier-3
//! matrix**, not merely "tests pass." Dev-Lima 7.0 is necessary-but-not-sufficient
//! (the built-in-ca-operator-composition cold-boot regression is the precedent
//! for an "expected to work on 6.18" change that did not).
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN` (nft, `ip netns`, `ip rule`,
//! `IP_TRANSPARENT`, real `EbpfDataplane` XDP attach + per-workload netns
//! provision). A non-root run SKIPs cleanly. `uname -r` is recorded. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
//! --features integration-tests`. NEVER `--no-run` (a compile-only gate is green
//! even when every fixture refuses at boot).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names,
    reason = "Tier-3 keystone test body; failures must panic with informative messages; \
              leg F/B/C/S are the ADR-0069 contract vocabulary; the composed flow is one \
              long scenario"
)]

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_control_plane::dataplane_config::DataplaneConfig;
use overdrive_control_plane::{ServerConfig, ServerHandle, run_server_with_obs_and_driver};
use overdrive_core::AllocationId;
use overdrive_core::CertSerial;
use overdrive_core::aggregate::{DriverInput, ExecInput, ResourcesInput};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput, SubmitSpecInput};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::observation_store::{AllocState, ObservationStore};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

// ============================================================================
// constants
// ============================================================================

/// The declared Service listener port the server workload offers. The inbound
/// rule keys on `ip daddr <workload_addr> tcp dport <SERVICE_PORT>` (D-BLOCKER1
/// one-source/two-readers) and the client dials `workload_addr:SERVICE_PORT`.
const SERVICE_PORT: u16 = 18941;

/// The OUTBOUND application request the client sends; the server must receive it
/// byte-exact as plaintext (decrypted on leg-C, spliced to leg-S).
const REQUEST: &[u8] =
    b"OVERDRIVE_CANONICAL_ADDR_REQUEST_client_to_server_must_arrive_plaintext_at_S_0302";
/// The application response the server replies; it rides back over leg-C's
/// kTLS-TX to the client byte-exact.
const RESPONSE: &[u8] =
    b"OVERDRIVE_CANONICAL_ADDR_RESPONSE_server_reply_rides_back_over_legC_ktls_to_client_0302";

/// The DNS SAN the inbound client presents toward the server's canonical addr
/// (matches the SNI; the inbound server handshake presents the held server SVID
/// carrying this SAN).
const SERVER_SNI: &str = "server.overdrive.local";

// ----------------------------------------------------------------------------
// Client-source netns plumbing (the TRAFFIC SOURCE — NOT a production-install
// stand-in). The production inbound nft-TPROXY rule fires at the host PREROUTING
// hook (priority mangle), which only sees packets INGRESSING the host. A
// host-local connect traverses OUTPUT, not PREROUTING, so the dial must
// originate from inside a netns whose egress ingresses the host veth — exactly
// the spike's netns-B → host PREROUTING → TPROXY → leg-C path (findings.md
// sub-probe 2). This fixture stands up that client netns + its routed veth; the
// INBOUND capture under test stays the PRODUCTION-installed rule (the litmus).
// ----------------------------------------------------------------------------

/// The client-source netns. A /30 OUTSIDE the production `10.99.0.0/18` workload
/// span (so it never collides with a deployed workload's slot) but inside the
/// `10.99.0.0/16` the host forwards within.
const CLIENT_NS: &str = "ovd-ks-cli-ns";
/// Host-side veth of the client netns (host end).
const CLIENT_HOST_VETH: &str = "ovd-ks-cli-hv";
/// Workload-side veth of the client netns (inside-netns end).
const CLIENT_WL_VETH: &str = "ovd-ks-cli-wv";
/// Host-side gateway addr on the client netns /30 (network+1).
const CLIENT_GW: &str = "10.99.200.1";
/// The client's address inside its netns (network+2) — the unmasqueraded source
/// the server's leg-C path sees, proving a genuine routed path (spike L82).
const CLIENT_ADDR: &str = "10.99.200.2";
const CLIENT_PREFIX: &str = "30";

// ============================================================================
// root gate
// ============================================================================

/// True iff this process is uid 0 (root). The real `EbpfDataplane` XDP attach,
/// per-workload netns provision, nft, `ip rule`, and `IP_TRANSPARENT` all need
/// root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run cannot stand up the
/// fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

// ============================================================================
// client-source netns fixture (RAII)
// ============================================================================

fn ip(args: &[&str]) {
    let out = Command::new("ip").args(args).output().expect("spawn ip");
    assert!(
        out.status.success(),
        "ip {args:?} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

fn ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).output();
}

/// The client-source netns + its routed veth pair. Drop tears it down. The host
/// gets a kernel route to the client /30 (via `ip addr add <gw>/30 dev
/// <host_veth>`), and the in-netns side gets `default via <gw>` — so a connect
/// from inside the netns to the server's /30 egresses the workload veth,
/// ingresses the host (PREROUTING fires), and the production inbound rule
/// captures it (the spike's netns-B → host PREROUTING shape).
struct ClientNetns;

impl ClientNetns {
    fn setup() -> Self {
        Self::teardown_best_effort();
        ip(&["netns", "add", CLIENT_NS]);
        ip(&["link", "add", CLIENT_WL_VETH, "type", "veth", "peer", "name", CLIENT_HOST_VETH]);
        ip(&["link", "set", CLIENT_WL_VETH, "netns", CLIENT_NS]);
        // Host side: gateway addr + up (this auto-installs the host route to the
        // client /30).
        ip(&["addr", "add", &format!("{CLIENT_GW}/{CLIENT_PREFIX}"), "dev", CLIENT_HOST_VETH]);
        ip(&["link", "set", CLIENT_HOST_VETH, "up"]);
        // Netns side: lo up + client addr + up + default route via the gateway.
        ip(&["netns", "exec", CLIENT_NS, "ip", "link", "set", "lo", "up"]);
        ip(&[
            "netns",
            "exec",
            CLIENT_NS,
            "ip",
            "addr",
            "add",
            &format!("{CLIENT_ADDR}/{CLIENT_PREFIX}"),
            "dev",
            CLIENT_WL_VETH,
        ]);
        ip(&["netns", "exec", CLIENT_NS, "ip", "link", "set", CLIENT_WL_VETH, "up"]);
        ip(&["netns", "exec", CLIENT_NS, "ip", "route", "add", "default", "via", CLIENT_GW]);
        // rp_filter relaxation on the client host-veth (asymmetric path: the
        // reply from leg-C returns on `lo`, not this veth).
        let _ = Command::new("sysctl")
            .args(["-w", &format!("net.ipv4.conf.{CLIENT_HOST_VETH}.rp_filter=0")])
            .output();
        Self
    }

    fn teardown_best_effort() {
        ip_quiet(&["link", "del", CLIENT_HOST_VETH]);
        ip_quiet(&["netns", "del", CLIENT_NS]);
    }
}

impl Drop for ClientNetns {
    fn drop(&mut self) {
        Self::teardown_best_effort();
    }
}

/// Enter the client netns on a fresh thread (via `setns(CLONE_NEWNET)`) and run
/// the blocking rustls mTLS dial there, so the socket originates inside the
/// netns and its egress ingresses the host (PREROUTING → production inbound
/// rule). Restores nothing — the thread is dedicated and exits after the dial.
fn dial_in_netns(pki_handle: TestPkiHandle, server_addr: SocketAddrV4) -> DialResult {
    let join = std::thread::spawn(move || {
        if !enter_netns(CLIENT_NS) {
            eprintln!("[03-02] setns into {CLIENT_NS} failed");
            return DialResult { received_response_byte_exact: false, observed_rst: true };
        }
        pki_handle.dial(server_addr)
    });
    join.join().expect("netns dial thread")
}

/// `setns(open("/var/run/netns/<ns>"), CLONE_NEWNET)` — move THIS thread into the
/// named network namespace. Returns false on any failure.
fn enter_netns(ns: &str) -> bool {
    let path = format!("/var/run/netns/{ns}");
    let Ok(cpath) = std::ffi::CString::new(path.clone()) else {
        return false;
    };
    // SAFETY: open the netns handle O_RDONLY|O_CLOEXEC, setns it onto this
    // thread's net namespace, then close the fd. All args are valid for the
    // duration of the calls.
    unsafe {
        let fd = libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
        if fd < 0 {
            eprintln!("[03-02] open {path}: {}", std::io::Error::last_os_error());
            return false;
        }
        let rc = libc::setns(fd, libc::CLONE_NEWNET);
        let err = std::io::Error::last_os_error();
        libc::close(fd);
        if rc != 0 {
            eprintln!("[03-02] setns {path}: {err}");
            return false;
        }
    }
    true
}

// ============================================================================
// Fresh focused PKI (re-authored — root → intermediate → leaf, rcgen + rustls)
// ============================================================================

/// A minted leaf — the PEM cert + key + the SPIFFE SAN, plus the DER forms.
struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

/// The shared test PKI: root self-signs; intermediate signed by root; every leaf
/// signed by the intermediate (production issuance shape).
struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    /// The CLIENT SVID — the agent presents it on the client's outbound leg-B
    /// (not used by this keystone's test-driven client, which presents
    /// `client_leaf` directly as the mTLS client; carried for parity).
    client_leaf: Leaf,
    /// The SERVER SVID — production's inbound `enforce` selects it (via the
    /// `IdentityRead` double, which returns it for ANY alloc id) for the leg-C
    /// SERVER handshake.
    server_leaf: Leaf,
}

impl TestPki {
    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-canonical-addr-0302-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-canonical-addr-0302-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        let client_leaf = intermediate.mint_leaf(client_spiffe, None, true);
        let server_leaf = intermediate.mint_leaf(server_spiffe, Some(SERVER_SNI), false);

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem.clone(),
            intermediate_cert_der: CertificateDer::from(intermediate.cert_der),
            client_leaf,
            server_leaf,
        }
    }

    fn intermediate_cert_der(&self) -> CertificateDer<'static> {
        self.intermediate_cert_der.clone()
    }

    /// The shared trust bundle: root anchor = the ROOT; intermediate chain
    /// material = the INTERMEDIATE (the agent reads this via `IdentityRead`).
    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn server_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.server_leaf)
    }
}

/// A minted signing authority (root OR intermediate) retaining its
/// `CertificateParams` + `KeyPair` so it can build a reusable rcgen 0.14 `Issuer`.
struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
    cert_der: Vec<u8>,
}

impl MintedCa {
    fn mint_root(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_intermediate(&self, cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        params.use_authority_key_identifier_extension = true;
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let root_issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&key, &root_issuer).unwrap();
        let cert_pem = cert.pem();
        let cert_der = cert.der().to_vec();
        Self { params, key, cert_pem, cert_der }
    }

    fn mint_leaf(&self, spiffe: &str, dns_san: Option<&str>, client_auth: bool) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
        let mut sans = vec![SanType::URI(uri)];
        if let Some(dns) = dns_san {
            let dns_ia5 = Ia5String::try_from(dns).expect("dns SAN is a valid IA5 string");
            sans.push(SanType::DnsName(dns_ia5));
        }
        params.subject_alt_names = sans;
        params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
        params.use_authority_key_identifier_extension = true;
        params.extended_key_usages = if client_auth {
            vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth]
        } else {
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]
        };
        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&self.params, &self.key);
        let cert = params.signed_by(&leaf_key, &issuer).unwrap();
        let cert_pem = cert.pem();
        let key_pem = leaf_key.serialize_pem();
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        Leaf {
            cert_pem,
            key_pem,
            cert_der,
            key_der,
            spiffe: spiffe.parse().expect("valid spiffe id"),
            serial: CertSerial::new("0a0b0c0d").expect("valid serial"),
        }
    }
}

/// Assemble `SvidMaterial` from a minted leaf (cert PEM/DER + leaf key PEM +
/// far-future `not_after`).
fn svid_from_leaf(leaf: &Leaf) -> SvidMaterial {
    let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)); // 2100
    SvidMaterial::new(
        CaCertPem::new(leaf.cert_pem.clone()),
        CaCertDer::new(leaf.cert_der.as_ref().to_vec()),
        leaf.serial.clone(),
        leaf.spiffe.clone(),
        CaKeyPem::new(leaf.key_pem.clone()),
        not_after,
    )
}

/// The agent's held-identity `IdentityRead` double — the ONLY holder of SVID
/// material (workloads hold nothing). Because production's inbound `enforce`
/// selects the SVID by the SERVER workload's PRODUCTION-assigned alloc id (which
/// the test cannot know in advance), this double returns the SAME held server
/// SVID for ANY alloc id. This is legitimate test-double behaviour — it models
/// "the agent holds the server's SVID"; the production `IdentityMgr` keys by
/// alloc id, but the keystone's transitive litmus does not depend on which
/// alloc id selects which SVID, only that the leg-C server handshake completes.
struct HeldServerIdentity {
    server_svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for HeldServerIdentity {
    fn svid_for(&self, _alloc: &AllocationId) -> Option<SvidMaterial> {
        Some(self.server_svid.clone())
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

// ============================================================================
// the in-process production boot harness (NO dataplane_override; real
// EbpfDataplane + composed mTLS worker via mtls_identity_override)
// ============================================================================

/// In-process production server: a real `run_server` boot on the real
/// `EbpfDataplane` (the default `ovd-veth-cli`/`ovd-veth-bk` veth pair the boot
/// auto-provisions per ADR-0061) with `mtls_identity_override = Some(TestPki)`.
/// Drop tears down the server task + tempdir.
struct Keystone {
    handle: Option<ServerHandle>,
    obs: Arc<dyn ObservationStore>,
    /// The HTTPS client trusting the server's ephemeral operator CA — the real
    /// in-process deploy submit handler is `POST /v1/jobs` through this client.
    client: reqwest::Client,
    bound: std::net::SocketAddr,
    _tmp: TempDir,
}

impl Keystone {
    async fn boot(pki: &TestPki) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let cfg_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("mkdir data");
        std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");

        // Retain an obs handle the test reads through (concurrent readers
        // permitted alongside the production writer).
        let obs_path = data_dir.join("observation.redb");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(LocalObservationStore::open(&obs_path).expect("open LocalObservationStore"));

        // Production driver: ExecDriver rooted at /sys/fs/cgroup.
        let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new(
            std::path::PathBuf::from("/sys/fs/cgroup"),
            Arc::new(overdrive_host::SystemClock),
            Arc::new(overdrive_host::RealCgroupFs::new()),
        ));

        // The composed mTLS worker reads this on the `compose_mtls = true` path
        // for the leg-C SERVER handshake. Returns the held server SVID for any
        // alloc id (see HeldServerIdentity).
        let identity: Arc<dyn IdentityRead> = Arc::new(HeldServerIdentity {
            server_svid: pki.server_svid_material(),
            bundle: pki.trust_bundle(),
        });

        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("parse bind addr"),
            data_dir: data_dir.clone(),
            operator_config_dir: cfg_dir.clone(),
            // Default veth names → the boot path auto-provisions the host-netns
            // veth pair + the real EbpfDataplane (ADR-0061 § 3).
            dataplane: Some(DataplaneConfig {
                client_iface: overdrive_control_plane::veth_provisioner::DEFAULT_CLIENT_IFACE
                    .to_owned(),
                backend_iface: overdrive_control_plane::veth_provisioner::DEFAULT_BACKEND_IFACE
                    .to_owned(),
            }),
            dataplane_pin_dir: None,
            // CRITICAL (R2): NO dataplane_override → compose_mtls = true →
            // the production mTLS worker is composed. The real EbpfDataplane is
            // the only adapter on this path.
            dataplane_override: None,
            // The ONLY injected composition seam: the leg-C/leg-B test PKI.
            mtls_identity_override: Some(identity),
            // Step 02-02 (C1-AMEND) — hermetic in-process boot KEK.
            ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
        };

        let handle = run_server_with_obs_and_driver(config, obs.clone(), driver)
            .await
            .expect("run_server_with_obs_and_driver (real EbpfDataplane + mTLS worker)");
        let bound = handle.local_addr().await.expect("bound addr");
        let ca_pem = read_ca_from_trust_triple(&cfg_dir);
        let client = client_trusting(&ca_pem);

        Self { handle: Some(handle), obs, client, bound, _tmp: tmp }
    }

    fn obs(&self) -> Arc<dyn ObservationStore> {
        Arc::clone(&self.obs)
    }

    async fn shutdown(mut self) {
        if let Some(handle) = self.handle.take() {
            // FAIL-FAST teardown (test hygiene). `ServerHandle::shutdown` awaits
            // four task joins (convergence, emit-drain, server_task,
            // exit-observer); the `drain_deadline` arg bounds ONLY the axum step.
            // The keystone's server workload is a `while True` Python listener
            // that never exits, so its `ExecDriver` exit-watcher is still
            // awaiting `child.wait()` and the convergence/observer joins can stall
            // while it is live. The workload is reaped *after* this returns by the
            // `AllocCleanup` guard (direct cgroupfs `cgroup.kill`), so bounding
            // the whole shutdown future here caps teardown at a few seconds and
            // lets cleanup do the kill — instead of blocking to nextest's ~120s
            // slow-test reap. Test-only — production `shutdown` is untouched.
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                handle.shutdown(Duration::from_secs(3)),
            )
            .await;
        }
    }
}

impl Drop for Keystone {
    fn drop(&mut self) {
        // FAIL-FAST teardown (test hygiene). On the happy path the test calls
        // `keystone.shutdown().await` explicitly and `self.handle` is already
        // `None` here — this Drop is a no-op. On the PANIC path (an assertion
        // failed), the unwind reaches this Drop with `self.handle` still
        // `Some`; we must tear the server down WITHOUT blocking, so a future
        // regression surfaces the real assertion failure in a few seconds
        // instead of hanging to nextest's ~120s slow-test reap.
        //
        // `ServerHandle::shutdown` awaits four task joins (convergence,
        // emit-drain, server_task, exit-observer) plus the axum graceful drain;
        // the `drain_deadline` arg bounds ONLY the axum step, so a stalled join
        // during unwind could block `block_on` indefinitely. Bounding the WHOLE
        // shutdown future with a hard `timeout` here caps the Drop at a few
        // seconds on every path. This is test-only — production `shutdown` is
        // untouched.
        if let Some(handle) = self.handle.take()
            && let Ok(rt) = tokio::runtime::Handle::try_current()
        {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tokio::task::block_in_place(|| {
                    let _ = rt.block_on(tokio::time::timeout(
                        Duration::from_secs(3),
                        handle.shutdown(Duration::from_secs(2)),
                    ));
                });
            }));
        }
    }
}

/// Deploy a Service spec through the real in-process deploy submit handler
/// (`POST /v1/jobs` over the production HTTPS driving port — no subprocess).
/// Returns `true` on a 2xx accept. This is the `overdrive deploy <SPEC>` handler
/// called in-process per `overdrive-cli/CLAUDE.md` § "Integration tests — no
/// subprocess".
async fn run_server_deploy(keystone: &Keystone, spec: ServiceSpecInput) -> bool {
    use overdrive_control_plane::api::SubmitWorkloadRequest;
    let url = format!("https://localhost:{}/v1/jobs", keystone.bound.port());
    let resp = keystone
        .client
        .post(&url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
        .send()
        .await
        .expect("deploy: POST /v1/jobs");
    let status = resp.status();
    let body = resp.bytes().await.expect("read response body");
    if !status.is_success() {
        eprintln!("[03-02] deploy non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
}

/// Stop a deployed workload through the real in-process stop driving port
/// (`POST /v1/jobs/{id}/stop` over the production HTTPS client — no subprocess).
/// This is the SAME path an operator's `overdrive job stop` drives: the handler
/// writes the stop-intent key and enqueues a `WorkloadLifecycle` eval; the
/// convergence loop then drives the running allocation to Terminated, dispatching
/// `Action::StopAllocation` through the action-shim, which calls
/// `MtlsInterceptWorker::stop_alloc` — signalling the per-alloc inbound
/// `accept_loop`'s cooperative `stop` flag so its `spawn_blocking` thread exits
/// (within its ~200ms bounded poll). Without this, the accept-loop thread
/// survives the in-process `Runtime::drop` and teardown blocks ~120s on it.
/// Returns `true` on a 2xx accept.
async fn run_server_stop(keystone: &Keystone, workload_id: &str) -> bool {
    let url = format!("https://localhost:{}/v1/jobs/{workload_id}/stop", keystone.bound.port());
    let resp = keystone.client.post(&url).send().await.expect("stop: POST /v1/jobs/{id}/stop");
    let status = resp.status();
    let body = resp.bytes().await.expect("read stop response body");
    if !status.is_success() {
        eprintln!("[03-02] stop non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
}

/// `Some(())` ⇔ the workload has at least one Terminated row and NO Running row
/// — i.e. the convergence loop has driven the stop to completion and the
/// action-shim's `StopAllocation` arm has fired (which is what calls
/// `worker.stop_alloc`, stopping the inbound accept loop). The keystone polls on
/// this BEFORE `shutdown()` cancels the convergence loop, so the accept-loop
/// thread is actually stopped (not merely timed-out-around) before the runtime
/// drops.
async fn server_stopped(obs: &Arc<dyn ObservationStore>, workload_id: &str) -> Option<()> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let mine = rows.iter().filter(|r| r.workload_id.as_str() == workload_id);
    let any_terminated = mine.clone().any(|r| r.state == AllocState::Terminated);
    let any_running = mine.clone().any(|r| r.state == AllocState::Running);
    (any_terminated && !any_running).then_some(())
}

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
    let ca_bytes =
        base64::engine::general_purpose::STANDARD.decode(ca_b64).expect("base64 decode ca");
    String::from_utf8(ca_bytes).expect("ca PEM is UTF-8")
}

// ============================================================================
// the server workload spec (a real TCP server bound on 0.0.0.0:port inside its
// production-provisioned netns — workload_addr covered by 0.0.0.0 — that reads
// the request then replies with the DISTINCT `RESPONSE` constant)
// ============================================================================

/// Build a Service spec whose exec driver launches a Python one-liner TCP server
/// bound on `0.0.0.0:SERVICE_PORT` inside the workload's netns. The inbound mTLS
/// path's leg-S dial reaches it at `workload_addr:SERVICE_PORT`
/// (`server_dial_addr(orig_dst) == orig_dst`).
///
/// The server READS the request bytes, then WRITES the DISTINCT `RESPONSE`
/// constant — it is NOT an echo. This mirrors the proven-working reply-leg
/// baseline (`cb7d8d09` `inbound_server_run`, which read `INBOUND_REQUEST` then
/// wrote a distinct `INBOUND_RESPONSE`) and preserves the keystone's
/// two-distinct-constants litmus: because `REQUEST != RESPONSE`, the client's
/// `got == RESPONSE` assertion can only pass if S authored and sent the distinct
/// RESPONSE over the real S→C reply pipe (leg-S read → leg-C kTLS-TX encrypt →
/// client decrypt) — an echo could not distinguish "the reply leg works" from
/// "the request was looped back at some layer", which is why the assertion
/// stays `got == RESPONSE`, never `got == REQUEST`.
fn server_service_spec(workload_id: &str) -> ServiceSpecInput {
    // Interpolate the distinct RESPONSE constant into the Python script as a
    // bytes literal, the same way SERVICE_PORT is interpolated. RESPONSE is an
    // ASCII constant (see :120), so each byte renders as a printable char.
    let response_py = String::from_utf8(RESPONSE.to_vec())
        .expect("RESPONSE is ASCII — renders as a Python bytes literal");
    let server_script = format!(
        r"
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('0.0.0.0', {SERVICE_PORT}))
s.listen(8)
while True:
    c, _ = s.accept()
    try:
        _ = c.recv(4096)
        c.sendall(b'{response_py}')
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
            args: vec!["-u".to_owned(), "-c".to_owned(), server_script],
        }),
        listeners: vec![ListenerInput { port: SERVICE_PORT, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

// ============================================================================
// the inbound mTLS client — dials workload_addr:SERVICE_PORT, presents the
// CLIENT SVID, verifies the agent's leg-C server cert chains to the bundle.
// This stands in for the originating mesh peer's agent-mediated leg-B; the
// PRODUCTION inbound install + leg-C + server handshake + leg-S are all
// production.
// ============================================================================

struct DialResult {
    received_response_byte_exact: bool,
    observed_rst: bool,
}

fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

fn drive_client_handshake(conn: &mut rustls::ClientConnection, tcp: &mut TcpStream) -> bool {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            if conn.write_tls(tcp).is_err() {
                return false;
            }
        }
        if !conn.is_handshaking() {
            return true;
        }
        match conn.read_tls(tcp) {
            Ok(0) => return false,
            Ok(_) => {
                if conn.process_new_packets().is_err() {
                    return false;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return false,
        }
    }
}

// ============================================================================
// back-door observation reads (no production path exercised by these helpers)
// ============================================================================

/// Read the server workload's Running-row `workload_addr` (the canonical address
/// the C3 seam materialised at provision time and the row write copied onto the
/// V2 `AllocStatusRow`). `Some(addr)` ⇔ the server reached Running with its
/// canonical address materialised.
async fn server_workload_addr(
    obs: &Arc<dyn ObservationStore>,
    workload_id: &str,
) -> Option<Ipv4Addr> {
    let rows = obs.alloc_status_rows().await.ok()?;
    rows.into_iter()
        .filter(|r| r.workload_id.as_str() == workload_id && r.state == AllocState::Running)
        .find_map(|r| r.workload_addr)
}

async fn poll_until<F, Fut, T>(budget: Duration, cadence: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + budget;
    loop {
        if let Some(v) = probe().await {
            return Some(v);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(cadence).await;
    }
}

// ============================================================================
// THE keystone scenario (S-WS)
// ============================================================================

/// S-WS — a workload reached at its canonical address terminates mTLS end to end.
///
/// Boots the production composition root in-process (real `run_server` + real
/// `EbpfDataplane`, NO `dataplane_override`, `mtls_identity_override =
/// Some(TestPki)`), deploys a server mesh workload through the real in-process
/// deploy submit handler, discovers its canonical `workload_addr`, dials
/// `workload_addr:SERVICE_PORT` directly (no name lookup), and asserts the
/// PRODUCTION-installed (03-01) inbound nft-TPROXY rule captures the dial, mTLS
/// terminates on leg-C, and the application round-trip completes byte-exact.
///
/// MERGE-BLOCKING on the pinned-6.18 appliance-kernel Tier-3 matrix (ADR-0068);
/// dev-Lima 7.0 is necessary-but-not-sufficient.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn workload_reached_at_canonical_address_terminates_mtls_end_to_end() {
    if !is_root() {
        eprintln!(
            "SKIP workload_reached_at_canonical_address_terminates_mtls_end_to_end: not root \
             (real EbpfDataplane XDP attach + per-workload netns provision + nft need \
             CAP_NET_ADMIN/CAP_SYS_ADMIN)"
        );
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline). The merge-blocking
    // signal is the pinned-6.18 Tier-3 matrix; dev-Lima is the inner loop.
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[03-02] uname -r = {kr} (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)");

    // The composition-root rustls CryptoProvider (installed once per process).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pki = TestPki::mint();

    // 1. Boot the production composition root in-process on the REAL dataplane.
    let keystone = Keystone::boot(&pki).await;

    // Reap the server workload's cgroup on any exit path (panic or clean).
    let _cleanup = super::workload_lifecycle::cleanup::AllocCleanup {
        obs: keystone.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // 2. Deploy the server mesh workload through the real in-process deploy
    //    submit handler. The production path provisions its per-workload netns +
    //    veth + canonical `workload_addr`, and `start_alloc` installs the inbound
    //    nft-TPROXY rule keyed on `ip daddr <workload_addr> tcp dport
    //    <SERVICE_PORT>` (03-01) — the production call site the litmus protects.
    let server_id = "canonical-addr-server";
    let submitted = run_server_deploy(&keystone, server_service_spec(server_id)).await;
    assert!(
        submitted,
        "S-WS: the server Service spec must be accepted by the in-process deploy submit handler"
    );

    // 3. Wait for the server to reach Running AND its canonical workload_addr to
    //    be materialised on the V2 AllocStatusRow (the C3 seam → row write).
    let workload_addr = poll_until(Duration::from_secs(20), Duration::from_millis(200), || {
        let obs = keystone.obs();
        let id = server_id.to_owned();
        async move { server_workload_addr(&obs, &id).await }
    })
    .await;
    let workload_addr = workload_addr.unwrap_or_else(|| {
        panic!(
            "S-WS: the server workload must reach Running with a materialised canonical \
             workload_addr within 20s (C3 seam → AllocStatusRowV2.workload_addr)"
        )
    });
    eprintln!("[03-02] server canonical workload_addr = {workload_addr}:{SERVICE_PORT}");

    // Give the server's exec echo a moment to bind inside its netns before the
    // agent's leg-S dial reaches it.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3b. Stand up the client-source netns (the TRAFFIC SOURCE — not a
    //     production-install stand-in). The dial MUST originate inside a netns so
    //     its egress ingresses the host and hits PREROUTING (where the production
    //     inbound rule lives); a host-local connect bypasses PREROUTING entirely.
    let _client_ns = ClientNetns::setup();

    // 4. Dial the server's canonical address DIRECTLY (no name lookup) FROM INSIDE
    //    the client netns. The dial egresses the client veth → ingresses the host
    //    → PREROUTING; the PRODUCTION-installed inbound rule diverts it to the
    //    agent's leg-C IP_TRANSPARENT listener; mTLS terminates; leg-S dials the
    //    server at workload_addr:SERVICE_PORT; bytes round-trip. NO test-installed
    //    rule — if the 03-01 install is absent the dial is not captured and this
    //    round-trip fails (the litmus).
    let server_addr = SocketAddrV4::new(workload_addr, SERVICE_PORT);
    let dial_pki = TestPkiHandle::from(&pki);

    let result = tokio::task::spawn_blocking(move || dial_in_netns(dial_pki, server_addr))
        .await
        .expect("dial task join");

    assert!(
        !result.observed_rst,
        "S-WS: the canonical-address mTLS dial must NOT observe a transport RST (leg-C terminated \
         cleanly and the round-trip completed)"
    );
    assert!(
        result.received_response_byte_exact,
        "S-WS: the client must read the server's reply byte-exact back over the production leg-C \
         kTLS — proving the canonical-address dial was captured by the PRODUCTION-installed inbound \
         nft-TPROXY rule (03-01), mTLS terminated, and leg-S reached the server. If the 03-01 \
         production install is absent the dial is not captured and this fails (the litmus)."
    );

    eprintln!(
        "[03-02] VERDICT: WORKS — a mesh workload reachable at its canonical address \
         ({workload_addr}:{SERVICE_PORT}) over mTLS via the PRODUCTION-installed inbound nft-TPROXY \
         rule, driven through in-process run_server + deploy on the REAL EbpfDataplane, on kernel \
         {kr}. (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    // 5. STOP the server workload through the PRODUCTION stop path — the SAME
    //    path an operator's `overdrive job stop` drives. The handler writes the
    //    stop-intent key and enqueues a `WorkloadLifecycle` eval; the
    //    convergence loop drives the alloc to Terminated, dispatching
    //    `Action::StopAllocation` → action-shim → `worker.stop_alloc`, which
    //    signals the inbound `accept_loop`'s cooperative `stop` flag so its
    //    `spawn_blocking` thread exits within its ~200ms bounded poll.
    //
    //    ORDERING IS LOAD-BEARING: this MUST happen BEFORE `shutdown()` cancels
    //    the convergence loop — otherwise the enqueued stop is never processed,
    //    `StopAllocation` never dispatches, and the accept-loop thread survives
    //    the in-process `Runtime::drop` (the ~120s teardown hang). We poll the
    //    obs row to Terminated to CONFIRM the stop converged (and thus
    //    `stop_alloc` actually fired) — we do not merely fire-and-forget.
    let stopped = run_server_stop(&keystone, server_id).await;
    assert!(
        stopped,
        "S-WS: the server workload must be accepted by the in-process stop driving port \
         (POST /v1/jobs/{{id}}/stop) — this is the production path that drives \
         StopAllocation → worker.stop_alloc, stopping the inbound accept loop"
    );
    let converged = poll_until(Duration::from_secs(20), Duration::from_millis(200), || {
        let obs = keystone.obs();
        let id = server_id.to_owned();
        async move { server_stopped(&obs, &id).await }
    })
    .await;
    assert!(
        converged.is_some(),
        "S-WS: the server workload must converge to Terminated within 20s after the production \
         stop — proving the convergence loop dispatched Action::StopAllocation (→ \
         worker.stop_alloc → the inbound accept loop's cooperative stop), so the spawn_blocking \
         accept-loop thread is actually STOPPED before the in-process runtime drops (not merely \
         timed-out-around)"
    );
    eprintln!(
        "[03-02] server workload {server_id} converged to Terminated via production stop \
         (StopAllocation → worker.stop_alloc → inbound accept loop stopped)"
    );

    keystone.shutdown().await;
}

// A small owned-handle wrapper so the dial can run on a `spawn_blocking` thread
// without borrowing `pki` across the await boundary.
struct TestPkiHandle {
    ca_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    client_cert_der: CertificateDer<'static>,
    client_key_der: PrivateKeyDer<'static>,
}

impl TestPkiHandle {
    fn from(pki: &TestPki) -> Self {
        Self {
            ca_cert_pem: pki.ca_cert_pem.clone(),
            intermediate_cert_der: pki.intermediate_cert_der(),
            client_cert_der: pki.client_leaf.cert_der.clone(),
            client_key_der: pki.client_leaf.key_der.clone_key(),
        }
    }

    fn dial(self, server_addr: SocketAddrV4) -> DialResult {
        use rustls::pki_types::ServerName;
        use rustls::{ClientConfig, ClientConnection};

        let fail = || DialResult { received_response_byte_exact: false, observed_rst: true };
        let roots = ca_root_store(&self.ca_cert_pem);
        let cfg = match ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(
            vec![self.client_cert_der, self.intermediate_cert_der],
            self.client_key_der,
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[03-02] inbound client config: {e}");
                return fail();
            }
        };
        // FAIL-FAST: bound the connect so a SYN with no SYN-ACK (a routing /
        // capture failure) returns a clear timeout in 10s instead of blocking
        // ~127s past nextest's reap and hanging the harness. A real captured dial
        // completes the TCP handshake in <1ms (loopback leg-C).
        let tcp = match TcpStream::connect_timeout(
            &std::net::SocketAddr::V4(server_addr),
            Duration::from_secs(10),
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "[03-02] inbound client connect {server_addr} failed: kind={:?} err={e}",
                    e.kind()
                );
                return fail();
            }
        };
        tcp.set_nodelay(true).ok();
        let sni = ServerName::try_from(SERVER_SNI.to_string()).expect("server SNI");
        let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("inbound ClientConnection");
        let mut tcp = tcp;
        tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
        if !drive_client_handshake(&mut conn, &mut tcp) {
            eprintln!("[03-02] inbound client handshake failed (leg-C)");
            return fail();
        }
        std::thread::sleep(Duration::from_millis(400));

        let mut observed_rst = false;
        {
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            if tls.write_all(REQUEST).and_then(|()| tls.flush()).is_err() {
                observed_rst = true;
            }
        }
        let mut got = Vec::new();
        if !observed_rst {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut buf = vec![0u8; 4096];
            while got.len() < RESPONSE.len() && Instant::now() < deadline {
                let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
                match tls.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => got.extend_from_slice(&buf[..n]),
                    Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                        observed_rst = true;
                        break;
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        DialResult { received_response_byte_exact: got == RESPONSE, observed_rst }
    }
}
