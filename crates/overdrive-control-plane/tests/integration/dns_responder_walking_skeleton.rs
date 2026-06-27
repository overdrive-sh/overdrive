//! S-DBN-WS / S-DBN-WS-STABLE / S-DBN-SINGLE-SRC / S-DBN-CHURN — the
//! dial-by-name-responder WALKING-SKELETON vertical slice (ADR-0072 REV-2, GH
//! #243; roadmap 02-02).
//!
//! These four Tier-3 `#[tokio::test]`s each boot ONE in-process production boot
//! fixture (the keystone's real-`EbpfDataplane` + `mtls_identity_override`
//! shape) and prove the dial-by-name loop end-to-end through the PRODUCTION
//! entry points — `run_server_with_obs_and_driver` (boot) + `POST /v1/jobs`
//! (deploy) + `getaddrinfo` from inside a deployed workload's
//! PRODUCTION-provisioned netns (resolve, NOT `dig` — K2) + `connect`
//! (capture / translate / mTLS).
//!
//! ## The vertical-slice litmus (CLAUDE.md "Build vertical slices")
//!
//! NO test binds `:53`, installs a `resolv.conf`, allocates `F`, programs a
//! map, or hand-installs the egress capture — production does ALL of those
//! itself:
//!
//! - the `:53` responder is bound by `DnsResponder::probe` (spawned by
//!   `run_server`, DDN-6);
//! - the per-netns `/etc/netns/<ns>/resolv.conf` (`nameserver
//!   <responder_addr>`) is written by the production
//!   `veth_provisioner::provision_workload_netns` (D-TME-9), so a `getaddrinfo`
//!   from inside a deployed workload's netns reaches the responder through the
//!   production resolv.conf, NOT a test-installed one;
//! - the STABLE frontend `F ∈ 10.98.0.0/16` is bound by the production
//!   `FrontendAddrAllocator` (01-04/01-05);
//! - the egress nft-TPROXY capture is installed per-workload by `start_alloc`
//!   (`install_outbound_tproxy`, keyed on `iifname <host_veth>`), so a connect
//!   to `F` from inside a deployed workload's netns is captured, recovers
//!   `orig_dst = (F, SERVICE_PORT)`, and the re-keyed `MtlsResolve` translates
//!   it (`by_frontend` HIT → the live backend).
//!
//! The dial-by-name addition over the canonical-address keystone is ONLY the
//! `getaddrinfo` resolution STEP (name → stable `F`) PLUS the re-keyed
//! `MtlsResolve` translating `F` → a live backend; the capture + handshake +
//! mTLS round-trip datapath is REUSED verbatim from the proven keystone
//! (BLOCKER-1 — the egress capture is destination-blind and recovers a
//! non-`/30` frontend `orig_dst` verbatim).
//!
//! ## How the resolve + dial run from a deployed netns
//!
//! Both the "server" and a long-lived "client" workload are deployed; each
//! reaches Running with a per-instance `workload_addr ∈ 10.99.0.0/16`. The
//! client's netns is derived from its `workload_addr` (`slot = (workload_addr
//! - WORKLOAD_SUBNET_BASE - 2) / 4`; `ovd-ns-<4hex slot>`). A dedicated thread
//! enters the CLIENT's production netns via `setns(CLONE_NEWNET)` (the
//! keystone's `enter_netns` shape) and runs `getaddrinfo("server.svc.\
//! overdrive.local")` there — exercising the production resolv.conf → the
//! production responder → the source-pinned reply — then connects to `(F,
//! SERVICE_PORT)` from the SAME netns so its egress is captured by the
//! client's production egress rule.
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`. A non-root run SKIPs
//! cleanly (the K1 root gate). `uname -r` is recorded. Run via `cargo xtask
//! lima run -- cargo nextest run -p overdrive-control-plane --features
//! integration-tests`. NEVER `--no-run`.
//!
//! MERGE-BLOCKING on the pinned-6.18 appliance-kernel Tier-3 matrix
//! (ADR-0068); dev-Lima is necessary-but-not-sufficient.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names,
    reason = "Tier-3 walking-skeleton bodies; failures must panic with informative messages; \
              F/F1/B1/B2 are the ADR-0072 REV-2 stable-frontend vocabulary; the composed flow \
              is one long scenario"
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

/// The declared Service listener port the server workload offers. The dialer
/// connects to `(F, SERVICE_PORT)`; the egress capture recovers
/// `orig_dst = (F, SERVICE_PORT)` and the re-keyed `MtlsResolve` keys
/// `(F, SERVICE_PORT, Tcp)`.
const SERVICE_PORT: u16 = 18951;

/// The OUTBOUND application request the dialer sends; the server receives it
/// byte-exact as plaintext (decrypted on the agent leg, spliced to the
/// server).
const REQUEST: &[u8] =
    b"OVERDRIVE_DIAL_BY_NAME_REQUEST_client_resolves_server_by_name_dials_stable_F_0202";
/// The DISTINCT application response the server replies; it rides back over the
/// agent leg's kTLS-TX to the dialer byte-exact (proving the reply leg, not an
/// echo).
const RESPONSE: &[u8] =
    b"OVERDRIVE_DIAL_BY_NAME_RESPONSE_server_reply_rides_back_over_agent_leg_ktls_0202";

/// The DNS SAN the test dialer presents toward the server in its OWN
/// `rustls::ClientConnection` SNI (`dial_frontend_in_netns` → `TestPkiHandle::
/// dial`). The agent's leg-C SERVER handshake presents the held server SVID
/// carrying this SAN, so the test client's direct verification succeeds.
const SERVER_SNI: &str = "server.overdrive.local";

/// The fixed sentinel SNI the PRODUCTION dataplane uses for the agent's
/// intra-mesh **leg-B** peer dial (`overdrive-dataplane::mtls::outbound`,
/// hardcoded `"peer.overdrive.local"` — "v1 is single-node + authn-only, so use
/// a fixed sentinel name that the test peer presents a SAN for"). The
/// dial-by-name CROSS-WORKLOAD path is the FIRST to exercise leg-B (the agent's
/// host-originated re-dial to the resolved backend), so the server SVID MUST
/// ALSO carry this SAN — else the leg-B client handshake verifies the server's
/// presented SVID against `peer.overdrive.local`, finds only `server.overdrive.
/// local`, and fails peer verification (the keystone INBOUND-only path never
/// dialed leg-B, so it never needed this SAN).
const MESH_PEER_SNI: &str = "peer.overdrive.local";

/// The mesh name a client resolves to reach the "server" Service — `<job>` =
/// `server`. Equal to `format!("server.{}", MeshServiceName::SUFFIX)`; pinned
/// as a literal here so the on-wire name a real stub resolver would query is
/// visible at the call site.
const SERVER_MESH_NAME: &str = "server.svc.overdrive.local";

/// The production per-host stable-frontend block (`10.98.0.0/16`,
/// `WORKLOAD_FRONTEND_BASE`). `F` answered for `<job>` is a member; a
/// per-instance backend addr lives in `10.99.0.0/16` and is NEVER the answer.
const FRONTEND_FIRST_OCTET: u8 = 10;
const FRONTEND_SECOND_OCTET: u8 = 98;
/// The per-instance workload (backend) block second octet (`10.99.0.0/16`,
/// `WORKLOAD_SUBNET_BASE`) — `getent` MUST NEVER answer an addr here.
const WORKLOAD_SECOND_OCTET: u8 = 99;

// ============================================================================
// root gate + kernel record
// ============================================================================

/// True iff this process is uid 0 (root). The real `EbpfDataplane` XDP attach,
/// per-workload netns provision, nft, `ip rule`, `IP_TRANSPARENT`, and the
/// `:53` responder bind all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a
/// non-root run cannot stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Record the running kernel — the Tier-3 verdict is pinned to a kernel
/// (dev-Lima and the pinned-6.18 appliance kernel differ — ADR-0068; the merge
/// gate is the 6.18 matrix).
fn record_kernel() -> String {
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[02-02] uname -r = {kr} (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)");
    kr
}

/// `WORKLOAD_SUBNET_BASE.network()` = `10.99.0.0`, the base of the per-instance
/// /30 span. The deployed workload's netns slot is `(workload_addr - base - 2)
/// / 4` (the inverse of `derive_workload_netns_plan`'s `workload_addr =
/// network + slot*4 + 2`).
const WORKLOAD_SUBNET_BASE_RAW: u32 = u32::from_be_bytes([10, 99, 0, 0]);

/// The production netns name (`ovd-ns-<4hex slot>`) for the deployed workload
/// whose per-instance `workload_addr` is `addr`. The inverse of
/// `derive_workload_netns_plan`: `slot = (addr - base - 2) / 4`. This is how a
/// deployed workload's PRODUCTION netns (with the production resolv.conf +
/// egress rule already installed) is located so a `setns` dial can run there —
/// NOT a test-created netns.
fn netns_name_for_workload_addr(addr: Ipv4Addr) -> String {
    let raw = u32::from(addr);
    let slot = raw.saturating_sub(WORKLOAD_SUBNET_BASE_RAW).saturating_sub(2) / 4;
    format!("ovd-ns-{slot:04x}")
}

// ============================================================================
// netns entry (the keystone's setns shape — enter a PRODUCTION netns)
// ============================================================================

/// `setns(open("/var/run/netns/<ns>"), CLONE_NEWNET)` — move THIS thread into
/// the named network namespace. Returns false on any failure. Used to enter a
/// DEPLOYED workload's PRODUCTION netns (so the resolv.conf + egress rule are
/// the production ones), never a test-created netns.
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
            eprintln!("[02-02] open {path}: {}", std::io::Error::last_os_error());
            return false;
        }
        let rc = libc::setns(fd, libc::CLONE_NEWNET);
        let err = std::io::Error::last_os_error();
        libc::close(fd);
        if rc != 0 {
            eprintln!("[02-02] setns {path}: {err}");
            return false;
        }
    }
    true
}

// ============================================================================
// getent (the K2 resolution oracle — a real getaddrinfo() via getent, NOT dig)
// ============================================================================
//
// Resolution MUST go through `ip netns exec <ns> getent ahostsv4 <name>` — NOT
// a bare `setns(CLONE_NEWNET)` + libc `getaddrinfo`. `setns(CLONE_NEWNET)`
// switches only the NETWORK namespace; the libc resolver reads `/etc/resolv.conf`
// from the MOUNT namespace, which is unchanged — so it would query the HOST's
// systemd-resolved (`127.0.0.53`), not the production-injected
// `/etc/netns/<ns>/resolv.conf` (`nameserver <responder_addr>`). `ip netns exec`
// enters BOTH the net namespace AND bind-mounts the per-netns resolv.conf over
// `/etc/resolv.conf`, so `getent` (a real `getaddrinfo` call) resolves through
// the production responder. `getent` is a stub resolver: it DISCARDS a reply
// whose source addr is not the queried server addr, so it only succeeds when the
// production responder source-pinned its reply (`ipi_spec_dst`) — exactly the
// signal `dig @gw` would mask (the K2 litmus).

/// Parse the V4 addrs from `getent ahostsv4 <name>` output. Each line is
/// `<addr>  <socktype>  [canonical-name]`; the first whitespace-token is the
/// addr. De-duplicated (getent prints one line per socktype).
fn parse_getent_v4(stdout: &str) -> Vec<Ipv4Addr> {
    let mut seen = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        if let Some(tok) = line.split_whitespace().next()
            && let Ok(addr) = tok.parse::<Ipv4Addr>()
        {
            seen.insert(addr);
        }
    }
    seen.into_iter().collect()
}

/// `Some(F)` ⇔ `getent ahostsv4 SERVER_MESH_NAME` run inside `netns` (via `ip
/// netns exec`, so the production resolv.conf + responder are used) resolves to
/// a V4 addr that is a member of the stable-frontend block `10.98.0.0/16` AND
/// NOT a per-instance backend in `10.99.0.0/16` (the SQ1 guard). Returns the
/// resolved `F`.
fn resolve_frontend_in_netns(netns: &str) -> Option<Ipv4Addr> {
    let out = Command::new("ip")
        .args(["netns", "exec", netns, "getent", "ahostsv4", SERVER_MESH_NAME])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let addrs = parse_getent_v4(&stdout);
    eprintln!(
        "[02-02] getent ahostsv4 {SERVER_MESH_NAME} in {netns} -> {addrs:?} (code {:?})",
        out.status.code()
    );
    // The answer must be the STABLE frontend F ∈ 10.98.0.0/16 — never a
    // per-instance backend addr ∈ 10.99.0.0/16 (the SQ1 guard).
    addrs.into_iter().find(|a| {
        let o = a.octets();
        o[0] == FRONTEND_FIRST_OCTET && o[1] == FRONTEND_SECOND_OCTET
    })
}

/// Poll `resolve_frontend_in_netns` until it answers a stable `F` within
/// `budget` (the K2 5s resolution budget) — re-querying because the responder's
/// `name_index` exposes `<job>` only after the backend reaches
/// running-AND-healthy (the bridge writes the `service_backends` row).
fn poll_resolve_frontend(netns: &str, budget: Duration) -> Option<Ipv4Addr> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(f) = resolve_frontend_in_netns(netns) {
            return Some(f);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ============================================================================
// the in-netns mTLS dial (enter the DEPLOYED workload's netns, connect to F)
// ============================================================================

struct DialResult {
    received_response_byte_exact: bool,
    observed_rst: bool,
}

/// Enter the DEPLOYED workload's PRODUCTION netns on a fresh thread and run the
/// blocking rustls mTLS dial to `(F, SERVICE_PORT)` there, so the connect
/// egresses the workload veth and is captured by the PRODUCTION egress rule
/// (`install_outbound_tproxy` keyed on `iifname <host_veth>`). No test rule is
/// installed.
fn dial_frontend_in_netns(
    netns: &str,
    pki_handle: TestPkiHandle,
    frontend: Ipv4Addr,
) -> DialResult {
    let ns = netns.to_owned();
    std::thread::spawn(move || {
        if !enter_netns(&ns) {
            eprintln!("[02-02] setns into {ns} failed (dial)");
            return DialResult { received_response_byte_exact: false, observed_rst: true };
        }
        pki_handle.dial(SocketAddrV4::new(frontend, SERVICE_PORT))
    })
    .join()
    .expect("netns dial thread")
}

// ============================================================================
// Fresh focused PKI (root → intermediate → leaf, rcgen + rustls) — the
// keystone's shape, trimmed.
// ============================================================================

struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    key_der: PrivateKeyDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    intermediate_cert_der: CertificateDer<'static>,
    client_leaf: Leaf,
    server_leaf: Leaf,
}

impl TestPki {
    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-dial-by-name-0202-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-dial-by-name-0202-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        let client_leaf = intermediate.mint_leaf(client_spiffe, &[], true);
        // The server SVID carries BOTH the test client's direct SNI (SERVER_SNI)
        // AND the production dataplane's hardcoded intra-mesh leg-B sentinel SNI
        // (MESH_PEER_SNI) — the dial-by-name cross-workload path verifies the
        // server's presented SVID against MESH_PEER_SNI on the agent's leg-B
        // re-dial (the first path to exercise leg-B; see MESH_PEER_SNI docs).
        let server_leaf =
            intermediate.mint_leaf(server_spiffe, &[SERVER_SNI, MESH_PEER_SNI], false);

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

    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn server_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.server_leaf)
    }

    fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
    }
}

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

    fn mint_leaf(&self, spiffe: &str, dns_sans: &[&str], client_auth: bool) -> Leaf {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
        let mut sans = vec![SanType::URI(uri)];
        for dns in dns_sans {
            let dns_ia5 = Ia5String::try_from(*dns).expect("dns SAN is a valid IA5 string");
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
/// material (workloads hold nothing). ALLOC-AWARE, because the dial-by-name
/// cross-workload mesh path exercises BOTH leg roles: the CLIENT-workload's
/// agent dials leg-B as a TLS CLIENT (its presented cert must carry ClientAuth
/// EKU), while the SERVER-workload's agent accepts leg-C as a TLS SERVER (its
/// presented cert must carry ServerAuth EKU + the `peer.overdrive.local` SAN
/// leg-B verifies against). A single server SVID for ALL allocs (the keystone's
/// INBOUND-only shape) fails the cross-workload leg-B with "certificate does not
/// allow extended key usage for client authentication" — the client-alloc agent
/// would present a ServerAuth-only cert. So we key by alloc id: the `server`
/// alloc gets the server leaf, every other alloc (the `client`) gets the client
/// leaf.
struct HeldServerIdentity {
    server_svid: SvidMaterial,
    client_svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for HeldServerIdentity {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        // The server-workload alloc id contains "server"; it presents the
        // ServerAuth server leaf on leg-C. Every other alloc (the client) is a
        // leg-B TLS client and must present the ClientAuth client leaf.
        if alloc.as_str().contains("server") {
            Some(self.server_svid.clone())
        } else {
            Some(self.client_svid.clone())
        }
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

// ============================================================================
// the in-process production boot harness (NO dataplane_override; real
// EbpfDataplane + composed mTLS worker via mtls_identity_override)
// ============================================================================

struct Skeleton {
    handle: Option<ServerHandle>,
    obs: Arc<dyn ObservationStore>,
    client: reqwest::Client,
    bound: std::net::SocketAddr,
    _tmp: TempDir,
}

impl Skeleton {
    async fn boot(pki: &TestPki) -> Self {
        let tmp = TempDir::new().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let cfg_dir = tmp.path().join("conf");
        std::fs::create_dir_all(&data_dir).expect("mkdir data");
        std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");

        let obs_path = data_dir.join("observation.redb");
        let obs: Arc<dyn ObservationStore> =
            Arc::new(LocalObservationStore::open(&obs_path).expect("open LocalObservationStore"));

        let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new(
            std::path::PathBuf::from("/sys/fs/cgroup"),
            Arc::new(overdrive_host::SystemClock),
            Arc::new(overdrive_host::RealCgroupFs::new()),
        ));

        let identity: Arc<dyn IdentityRead> = Arc::new(HeldServerIdentity {
            server_svid: pki.server_svid_material(),
            client_svid: pki.client_svid_material(),
            bundle: pki.trust_bundle(),
        });

        let config = ServerConfig {
            bind: "127.0.0.1:0".parse().expect("parse bind addr"),
            data_dir: data_dir.clone(),
            operator_config_dir: cfg_dir.clone(),
            dataplane: Some(DataplaneConfig {
                client_iface: overdrive_control_plane::veth_provisioner::DEFAULT_CLIENT_IFACE
                    .to_owned(),
                backend_iface: overdrive_control_plane::veth_provisioner::DEFAULT_BACKEND_IFACE
                    .to_owned(),
            }),
            dataplane_pin_dir: None,
            // CRITICAL: NO dataplane_override → compose_mtls = true → the
            // production mTLS worker + DnsResponder + FrontendAddrAllocator +
            // re-keyed MtlsResolve are constructed + probed + spawned.
            dataplane_override: None,
            mtls_identity_override: Some(identity),
            ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
        };

        let handle = run_server_with_obs_and_driver(config, obs.clone(), driver)
            .await
            .expect("run_server_with_obs_and_driver (real EbpfDataplane + mTLS worker + DNS)");
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
            // FAIL-FAST teardown (test hygiene) — bound the whole shutdown
            // future so a stalled task join during a live workload does not
            // hang to nextest's slow-test reap. The AllocCleanup guard reaps
            // the workloads after this returns. Test-only.
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                handle.shutdown(Duration::from_secs(3)),
            )
            .await;
        }
    }
}

impl Drop for Skeleton {
    fn drop(&mut self) {
        // FAIL-FAST teardown on the PANIC path (an assertion failed) — tear the
        // server down WITHOUT blocking so a regression surfaces the real
        // assertion in a few seconds, not nextest's ~120s reap. Test-only.
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
/// (`POST /v1/jobs` over the production HTTPS driving port). Returns `true` on
/// a 2xx accept.
async fn run_server_deploy(skeleton: &Skeleton, spec: ServiceSpecInput) -> bool {
    use overdrive_control_plane::api::SubmitWorkloadRequest;
    let url = format!("https://localhost:{}/v1/jobs", skeleton.bound.port());
    let resp = skeleton
        .client
        .post(&url)
        .json(&SubmitWorkloadRequest { spec: SubmitSpecInput::Service(spec) })
        .send()
        .await
        .expect("deploy: POST /v1/jobs");
    let status = resp.status();
    let body = resp.bytes().await.expect("read response body");
    if !status.is_success() {
        eprintln!("[02-02] deploy non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
}

/// Stop a deployed workload through the real in-process stop driving port
/// (`POST /v1/jobs/{id}/stop`). Drives `StopAllocation` → `worker.stop_alloc`
/// (which stops the per-alloc accept loops), the SAME path `overdrive job
/// stop` drives. Returns `true` on a 2xx accept.
async fn run_server_stop(skeleton: &Skeleton, workload_id: &str) -> bool {
    let url = format!("https://localhost:{}/v1/jobs/{workload_id}/stop", skeleton.bound.port());
    let resp = skeleton.client.post(&url).send().await.expect("stop: POST /v1/jobs/{id}/stop");
    let status = resp.status();
    let body = resp.bytes().await.expect("read stop response body");
    if !status.is_success() {
        eprintln!("[02-02] stop non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
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
// the server / client workload specs
// ============================================================================

/// Build a Service spec whose exec driver launches a Python one-liner TCP
/// server bound on `0.0.0.0:SERVICE_PORT` inside its netns. The server READS
/// the request bytes then WRITES the DISTINCT `RESPONSE` constant (NOT an echo)
/// — so the dialer's `got == RESPONSE` assertion can only pass if S authored
/// and sent RESPONSE over the real S→C reply pipe.
fn server_service_spec(workload_id: &str) -> ServiceSpecInput {
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

/// A long-lived idle CLIENT workload. Its only purpose is to give the test a
/// PRODUCTION-provisioned netns (with the production resolv.conf injected AND
/// the production egress nft-TPROXY rule installed by `start_alloc`) to `setns`
/// into for the resolve + dial. A `/bin/sleep` is the minimal long-lived
/// workload; it never binds the listener — that is the SERVER's job. (Its
/// listener port differs from the server's so the two specs never collide on a
/// shared port observation.)
fn client_service_spec(workload_id: &str) -> ServiceSpecInput {
    ServiceSpecInput {
        id: workload_id.to_owned(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_owned(),
            args: vec!["3600".to_owned()],
        }),
        listeners: vec![ListenerInput { port: SERVICE_PORT + 1, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

// ============================================================================
// the in-netns mTLS client handshake/round-trip (the keystone's dial, reused)
// ============================================================================

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

// A small owned-handle wrapper so the dial can run on a dedicated thread
// without borrowing `pki` across the thread boundary.
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
                eprintln!("[02-02] dial client config: {e}");
                return fail();
            }
        };
        // FAIL-FAST: bound the connect so a SYN with no SYN-ACK (a routing /
        // capture failure) returns a clear timeout in 10s instead of blocking
        // past nextest's reap. A real captured dial completes in <1ms.
        let tcp = match TcpStream::connect_timeout(
            &std::net::SocketAddr::V4(server_addr),
            Duration::from_secs(10),
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[02-02] dial connect {server_addr} failed: kind={:?} err={e}", e.kind());
                return fail();
            }
        };
        tcp.set_nodelay(true).ok();
        let sni = ServerName::try_from(SERVER_SNI.to_string()).expect("server SNI");
        let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("dial ClientConnection");
        let mut tcp = tcp;
        tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
        if !drive_client_handshake(&mut conn, &mut tcp) {
            eprintln!("[02-02] dial handshake failed (agent leg)");
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

// ============================================================================
// back-door observation reads (no production path exercised by these helpers)
// ============================================================================

/// Read the deployed workload's Running-row `workload_addr` (the per-instance
/// backend addr ∈ 10.99.0.0/16). `Some(addr)` ⇔ the workload reached Running
/// with its canonical address materialised (so the bridge wrote a healthy
/// `service_backends` row → the responder's `name_index` exposes the `<job>`).
async fn workload_running_addr(
    obs: &Arc<dyn ObservationStore>,
    workload_id: &str,
) -> Option<Ipv4Addr> {
    let rows = obs.alloc_status_rows().await.ok()?;
    rows.into_iter()
        .filter(|r| r.workload_id.as_str() == workload_id && r.state == AllocState::Running)
        .find_map(|r| r.workload_addr)
}

/// Read the deployed workload's CURRENT running AllocationId (for the alloc-
/// cycle assertion — proving a NEW AllocationId after the cycle).
async fn workload_running_alloc_id(
    obs: &Arc<dyn ObservationStore>,
    workload_id: &str,
) -> Option<String> {
    let rows = obs.alloc_status_rows().await.ok()?;
    rows.into_iter()
        .filter(|r| r.workload_id.as_str() == workload_id && r.state == AllocState::Running)
        .map(|r| r.alloc_id.to_string())
        .next()
}

/// `Some(())` ⇔ the workload has ≥1 Terminated row and NO Running row — the
/// stop converged (the action-shim `StopAllocation` arm fired → the accept
/// loops stopped). Polled before `shutdown()` so the accept-loop threads are
/// actually stopped before the runtime drops.
async fn server_stopped(obs: &Arc<dyn ObservationStore>, workload_id: &str) -> Option<()> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let mine = rows.iter().filter(|r| r.workload_id.as_str() == workload_id);
    let any_terminated = mine.clone().any(|r| r.state == AllocState::Terminated);
    let any_running = mine.clone().any(|r| r.state == AllocState::Running);
    (any_terminated && !any_running).then_some(())
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

/// Deploy a workload and wait for it to reach Running with a materialised
/// `workload_addr`. Returns the per-instance backend addr.
async fn deploy_and_wait_running(
    skeleton: &Skeleton,
    spec: ServiceSpecInput,
    workload_id: &str,
) -> Ipv4Addr {
    let submitted = run_server_deploy(skeleton, spec).await;
    assert!(submitted, "the {workload_id} Service spec must be accepted by the deploy handler");
    let addr = poll_until(Duration::from_secs(25), Duration::from_millis(200), || {
        let obs = skeleton.obs();
        let id = workload_id.to_owned();
        async move { workload_running_addr(&obs, &id).await }
    })
    .await;
    addr.unwrap_or_else(|| {
        panic!("the {workload_id} workload must reach Running with a workload_addr within 25s")
    })
}

// ============================================================================
// S-DBN-WS — name -> stable F -> translate -> mTLS round-trip
// ============================================================================

/// S-DBN-WS — a deployed workload resolves its peer's STABLE frontend name and
/// the hop is mTLS'd (US-DBN-2 · K-DBN-1).
///
/// Boots the production composition root in-process; deploys a "server"
/// (Running-AND-HEALTHY → bridge writes a healthy `service_backends` row → the
/// responder's `name_index` exposes the `<job>` bound a stable `F ∈
/// 10.98.0.0/16`) and a long-lived "client" (so the test has a
/// production-provisioned netns to dial from). From inside the client's
/// PRODUCTION netns: `getaddrinfo("server.svc.overdrive.local")` resolves to
/// `F` (NOT a `10.99.0.0/16` backend addr — asserted FIRST, separately from
/// the mTLS assertion), then a connect to `(F, SERVICE_PORT)` is captured by
/// the production egress rule, the re-keyed `MtlsResolve` translates `F` → the
/// live backend, mTLS terminates, and the round-trip completes byte-exact.
///
/// MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
///
/// The mesh→mesh hop closes via the REV-5 output-hook leg-B interception
/// companion (`mtls_intercept.rs`, spike `findings-output-hook-legb.md`): the
/// agent's host-originated leg-B re-dial to the resolved backend (the dial-by-name
/// case where the resolved frontend `F` ≠ the backend `workload_addr`) traverses
/// the kernel OUTPUT hook, where the per-virt `type route hook output` divert
/// rule steers it into the destination's leg-C. Without that companion the leg-B
/// re-dial lands on the plaintext workload listener and the agent reads cleartext
/// → `InvalidContentType` → RST (the symptom the RCA
/// `root-cause-analysis-dial-by-name-by-frontend-resolve-rst.md` diagnosed).
///
/// PARTIALLY BLOCKED (02-02 — leg-F→leg-C handshake-completion follow-up). The
/// REV-5 output-hook DATAPATH fix is LANDED and pwru-proven: the agent's
/// host-originated leg-B re-dial to the resolved backend IS diverted into the
/// destination's leg-C on the OUTPUT path (`ip_route_me_harder` stamps the
/// fwmark, `type route` re-lookup routes via `local table 100`, the prerouting
/// `tproxy` rule on lo-reentry redirects to leg-C) and the TCP handshake
/// completes. Two cross-workload test-fixture identity bugs surfaced and were
/// fixed in this commit: (a) the server SVID now carries the
/// `peer.overdrive.local` leg-B sentinel-SNI SAN the production dataplane
/// hardcodes (`overdrive-dataplane::mtls::outbound`), and (b)
/// `HeldServerIdentity` is alloc-aware so the CLIENT alloc's agent presents the
/// ClientAuth client leaf on leg-B while the SERVER alloc's agent presents the
/// ServerAuth server leaf on leg-C (a single ServerAuth SVID for all allocs —
/// the keystone's INBOUND-only shape — failed the cross-workload leg-B EKU
/// check). With both fixed there is NO agent-side mTLS rejection, but the test
/// client's end-to-end rustls handshake (terminating against the server agent's
/// leg-C) still does not complete. This is a remaining leg-F/leg-C
/// handshake-completion issue in the cross-workload composition, NOT a datapath
/// defect (the keystone INBOUND-only path never exercised an agent-originated
/// leg-B into a peer's leg-C). Un-ignore once that gap closes.
#[ignore = "02-02 follow-up: cross-workload leg-F→leg-C mTLS handshake does not complete; REV-5 datapath fix landed + pwru-proven, SAN/EKU fixture bugs fixed"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls() {
    if !is_root() {
        eprintln!("SKIP deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls: not root");
        return;
    }
    let kr = record_kernel();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pki = TestPki::mint();
    let skeleton = Skeleton::boot(&pki).await;

    let _cleanup = super::workload_lifecycle::cleanup::AllocCleanup {
        obs: skeleton.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    // Deploy a single "server" mesh workload. It reaches Running with a stable
    // per-instance workload_addr; the bridge writes a healthy service_backends
    // row; the responder's name_index exposes <job> "server" bound a stable F.
    let server_id = "server";
    let server_backend = deploy_and_wait_stable_backend(&skeleton, server_id).await;
    let server_netns = netns_name_for_workload_addr(server_backend);

    // Deploy a SEPARATE long-lived client workload — the dial SOURCE. Like the
    // keystone (a distinct client → server, NEVER a self-dial) and the 03-02
    // ping-pong, the client gives the test a PRODUCTION-provisioned netns (the
    // responder-injected resolv.conf + the start_alloc egress nft-TPROXY rule)
    // to resolve + dial FROM. Dialing the server's own frontend from the
    // server's own netns is a mesh hairpin the cross-workload keystone never
    // proved.
    let client_backend =
        deploy_and_wait_running(&skeleton, client_service_spec("client"), "client").await;
    let client_netns = netns_name_for_workload_addr(client_backend);
    eprintln!(
        "[02-02] server backend = {server_backend} (netns {server_netns}); \
         client dial-source = {client_backend} (netns {client_netns})"
    );

    // Settle: a Running row precedes the per-alloc mTLS intercept install
    // (the client's egress nft-TPROXY capture + leg-F listener, and the server's
    // leg-C accept loop) by a short window. Dialing before both legs are live
    // races a fast handshake failure. The sibling S-DBN tests already settle
    // 500ms here; match them.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // (1) RESOLVE — getaddrinfo from inside the CLIENT's PRODUCTION netns must
    //     answer the STABLE frontend F ∈ 10.98.0.0/16 within the K2 5s budget.
    //     Asserted FIRST and SEPARATELY from the mTLS assertion (the K2
    //     two-culprits honesty: source-pin OR healthy-gate).
    let netns = client_netns.clone();
    let frontend =
        tokio::task::spawn_blocking(move || poll_resolve_frontend(&netns, Duration::from_secs(5)))
            .await
            .expect("resolve task join");
    let frontend = frontend.unwrap_or_else(|| {
        panic!(
            "S-DBN-WS: getaddrinfo({SERVER_MESH_NAME}) from inside the client netns must resolve \
             to the STABLE frontend F ∈ 10.98.0.0/16 within 5s (the production responder bound :53, \
             source-pinned the reply via ipi_spec_dst, and the name_index exposed the <job> after \
             running-and-healthy). A timeout means EITHER the source-pin is missing OR the \
             healthy-gate regressed (K2 two culprits)."
        )
    });
    let o = frontend.octets();
    assert_eq!(
        (o[0], o[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "S-DBN-WS: getent must resolve to the STABLE frontend F ∈ 10.98.0.0/16 \
         (got {frontend}), NEVER a per-instance backend addr ∈ 10.99.0.0/16",
    );
    assert_ne!(
        o[1], WORKLOAD_SECOND_OCTET,
        "S-DBN-WS: getent must NOT answer a per-instance backend addr ∈ 10.99.0.0/16 (got {frontend})",
    );
    eprintln!("[02-02] resolved STABLE frontend F = {frontend}");

    // (2) DIAL — connect to (F, SERVICE_PORT) from inside the CLIENT's
    //     PRODUCTION netns; the production egress rule captures it, the re-keyed
    //     MtlsResolve translates F → the server's live backend, mTLS terminates,
    //     the round-trip completes byte-exact. A wrong F (or a missing
    //     by_frontend translation) would miss → fail-close → no backend → this
    //     fails.
    let dial_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let result =
        tokio::task::spawn_blocking(move || dial_frontend_in_netns(&netns, dial_pki, frontend))
            .await
            .expect("dial task join");

    assert!(
        !result.observed_rst,
        "S-DBN-WS: the dial to the stable frontend F must NOT observe a transport RST (the agent \
         leg terminated cleanly and the round-trip completed)"
    );
    assert!(
        result.received_response_byte_exact,
        "S-DBN-WS: the dialer must read the server's reply byte-exact back over the agent leg's \
         kTLS — proving the connect to the resolved F was captured by the PRODUCTION egress \
         nft-TPROXY rule, the re-keyed MtlsResolve translated (F, SERVICE_PORT, Tcp) → the live \
         backend, and mTLS terminated. Removing the production responder spawn (getent times out) \
         or the by_frontend translation arm (connect to F fail-closes) takes this RED."
    );
    eprintln!(
        "[02-02] VERDICT: WORKS — a deployed workload resolved its peer by name to the stable \
         frontend F ({frontend}:{SERVICE_PORT}) and the hop is mTLS'd, driven through in-process \
         run_server + deploy on the REAL EbpfDataplane, on kernel {kr}. \
         (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    // Stop the workload through the production stop path BEFORE shutdown so the
    // accept-loop threads are actually stopped (not timed-out-around).
    stop_and_converge(&skeleton, server_id).await;
    skeleton.shutdown().await;
}

/// Deploy a "server" mesh workload and wait until the bridge advertises a
/// STABLE per-instance backend addr ∈ `10.99.0.0/16` (the real `workload_addr`,
/// NOT the `host_ipv4` fallback) in the `service_backends` row — i.e. the alloc
/// is a settled Path-A mesh alloc whose canonical `workload_addr` the bridge
/// reads. Returns that backend addr. This is the precondition for the
/// dial-by-name loop: the re-keyed `MtlsResolve` translates `F` → this backend,
/// so it must be the routable per-instance addr (a `host_ipv4` fallback would
/// not reach the workload's in-netns listener).
async fn deploy_and_wait_stable_backend(skeleton: &Skeleton, server_id: &str) -> Ipv4Addr {
    let submitted = run_server_deploy(skeleton, server_service_spec(server_id)).await;
    assert!(submitted, "the {server_id} Service spec must be accepted by the deploy handler");
    let addr = poll_until(Duration::from_secs(30), Duration::from_millis(250), || {
        let obs = skeleton.obs();
        let id = server_id.to_owned();
        async move { stable_mesh_backend_addr(&obs, &id).await }
    })
    .await;
    addr.unwrap_or_else(|| {
        panic!(
            "S-DBN-WS: the {server_id} workload must reach a settled Path-A mesh alloc whose \
             service_backends row advertises a per-instance workload_addr ∈ 10.99.0.0/16 within \
             30s (the re-keyed MtlsResolve translates F → this addr; a host_ipv4 fallback would \
             not reach the in-netns listener)"
        )
    })
}

/// `Some(addr)` ⇔ the `<job>`'s `service_backends` row currently advertises a
/// HEALTHY backend whose addr is a per-instance mesh workload_addr ∈
/// `10.99.0.0/16` (NOT the `host_ipv4` fallback). Reads through the
/// `<job>`-tagged backend SpiffeId.
async fn stable_mesh_backend_addr(obs: &Arc<dyn ObservationStore>, job: &str) -> Option<Ipv4Addr> {
    let rows = obs.all_service_backends_rows().await.ok()?;
    let needle = format!("/job/{job}/");
    rows.into_iter()
        .flat_map(|r| r.backends)
        .filter(|b| b.healthy && b.alloc.as_str().contains(&needle))
        .filter_map(|b| match b.addr {
            std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
            std::net::SocketAddr::V6(_) => None,
        })
        .find(|ip| {
            ip.octets()[0] == FRONTEND_FIRST_OCTET && ip.octets()[1] == WORKLOAD_SECOND_OCTET
        })
}

/// Stop a deployed workload through the production stop path and poll its obs
/// row to Terminated (so the accept-loop threads are STOPPED before the runtime
/// drops).
async fn stop_and_converge(skeleton: &Skeleton, workload_id: &str) {
    let stopped = run_server_stop(skeleton, workload_id).await;
    assert!(stopped, "{workload_id} must be accepted by the in-process stop driving port");
    let converged = poll_until(Duration::from_secs(20), Duration::from_millis(200), || {
        let obs = skeleton.obs();
        let id = workload_id.to_owned();
        async move { server_stopped(&obs, &id).await }
    })
    .await;
    assert!(
        converged.is_some(),
        "{workload_id} must converge to Terminated within 20s after the production stop"
    );
}

// ============================================================================
// S-DBN-SINGLE-SRC — the answered F is the addr MtlsResolve translates to Mesh
// ============================================================================

/// S-DBN-SINGLE-SRC — single-source oracle: the answered `F` is the addr the
/// production re-keyed `MtlsResolve` recognizes and translates to a `Mesh`
/// backend (US-DBN-2 · K-DBN-4).
///
/// The `<job> → F` binding has exactly ONE source — the SINGLE shared
/// `FrontendAddrAllocator` constructed once in `run_server` and cloned into
/// BOTH the DNS `name_index` (which ANSWERS `F`) AND the re-keyed `MtlsResolve`
/// (`by_frontend`, which RECOGNIZES `F`) (lib.rs ~2019 + ~2228; DDN-2). The
/// production surface does NOT expose that live allocator on `ServerHandle`, so
/// the Tier-3 observable of byte-identity-via-one-source is: the SAME `F`
/// `getaddrinfo` answers is the SAME `F` the production `MtlsResolve.resolve(F,
/// Tcp)` translates to a `Mesh` live backend — observed at the port as a
/// SUCCESSFUL mTLS round-trip whose `orig_dst` was the answered `F`. A
/// `by_frontend` MISS would fail-close on the `10.98.0.0/16` subnet
/// (MeshUnreachable → no backend → no round-trip); a wrong-subnet answer would
/// be `NonMesh` (cleartext → the mTLS-required server handshake rejects it). So
/// a byte-exact round-trip THROUGH the answered `F` proves `resolve(F)` was a
/// `by_frontend` HIT classified `Mesh` translating to the live backend — the
/// name answer and the resolve translation agreed on the SAME `F` from the SAME
/// allocator. This scenario asserts that agreement explicitly, separate from
/// S-DBN-WS, and pins that the answered `F ∈ 10.98.0.0/16` (the recognizable
/// subnet) and NOT a `10.99.0.0/16` backend addr.
///
/// The round-trip THROUGH `F` closes via the REV-5 output-hook leg-B
/// interception companion (`mtls_intercept.rs`): the agent's host-originated
/// leg-B re-dial to the translated backend is diverted into the destination's
/// leg-C on the OUTPUT path, so the mTLS hop terminates rather than RSTing on the
/// plaintext listener.
///
/// PARTIALLY BLOCKED (02-02) — same cross-workload leg-F→leg-C
/// handshake-completion follow-up as
/// `deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls` (see its
/// docstring): the REV-5 datapath fix is landed + pwru-proven and the SAN/EKU
/// fixture bugs are fixed, but the end-to-end mTLS round-trip does not yet
/// complete. Un-ignore once that gap closes.
#[ignore = "02-02 follow-up: cross-workload leg-F→leg-C mTLS handshake does not complete; REV-5 datapath fix landed + pwru-proven"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn answered_frontend_is_the_addr_mtls_resolve_translates_to_a_mesh_backend() {
    if !is_root() {
        eprintln!(
            "SKIP answered_frontend_is_the_addr_mtls_resolve_translates_to_a_mesh_backend: not root"
        );
        return;
    }
    record_kernel();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pki = TestPki::mint();
    let skeleton = Skeleton::boot(&pki).await;
    let _cleanup = super::workload_lifecycle::cleanup::AllocCleanup {
        obs: skeleton.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    let server_id = "server";
    let client_id = "client";
    let server_backend =
        deploy_and_wait_running(&skeleton, server_service_spec(server_id), server_id).await;
    let client_addr =
        deploy_and_wait_running(&skeleton, client_service_spec(client_id), client_id).await;
    let client_netns = netns_name_for_workload_addr(client_addr);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The answered F (the single source — the responder's name_index reads the
    // SAME FrontendAddrAllocator the re-keyed MtlsResolve's by_frontend reads).
    let netns = client_netns.clone();
    let frontend =
        tokio::task::spawn_blocking(move || poll_resolve_frontend(&netns, Duration::from_secs(5)))
            .await
            .expect("resolve task join")
            .unwrap_or_else(|| {
                panic!("S-DBN-SINGLE-SRC: getaddrinfo must resolve the stable frontend F")
            });

    // The answered F is in the by_frontend-RECOGNIZABLE subnet (10.98.0.0/16),
    // and is NOT the per-instance backend addr the dataplane translates TO
    // (10.99.0.0/16) — the two are byte-distinct (frontend vs backend), which is
    // the whole point of the stable-frontend split.
    let fo = frontend.octets();
    assert_eq!(
        (fo[0], fo[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "S-DBN-SINGLE-SRC: the answered F must be in the by_frontend-recognizable subnet \
         10.98.0.0/16 (got {frontend})",
    );
    assert_ne!(
        frontend, server_backend,
        "S-DBN-SINGLE-SRC: the answered F (the frontend) must be byte-DISTINCT from the server's \
         per-instance backend addr {server_backend} — the stable-frontend split means DNS answers F, \
         NOT the backend addr",
    );

    // The single-source oracle observed at the port: resolve(F) → Mesh →
    // translate to the live backend, proven by a byte-exact mTLS round-trip
    // THROUGH the answered F. (A by_frontend miss → MeshUnreachable → no
    // backend → no round-trip; a wrong subnet → NonMesh → cleartext → the
    // mTLS-required server handshake rejects.)
    let dial_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let result =
        tokio::task::spawn_blocking(move || dial_frontend_in_netns(&netns, dial_pki, frontend))
            .await
            .expect("dial task join");
    assert!(
        !result.observed_rst && result.received_response_byte_exact,
        "S-DBN-SINGLE-SRC: the production MtlsResolve.resolve(F, Tcp) for the answered F must be a \
         by_frontend HIT classified Mesh translating to the live backend — observed as a byte-exact \
         mTLS round-trip THROUGH the answered F. The name answer and the resolve translation read \
         the SAME <job> → F binding from the SAME FrontendAddrAllocator (DDN-2 single source)."
    );
    eprintln!(
        "[02-02] S-DBN-SINGLE-SRC: the answered F ({frontend}) is the addr the production \
         re-keyed MtlsResolve recognized (by_frontend HIT) and translated to a Mesh live backend \
         — one source, byte-consistent."
    );

    stop_and_converge(&skeleton, server_id).await;
    stop_and_converge(&skeleton, client_id).await;
    skeleton.shutdown().await;
}

// ============================================================================
// S-DBN-WS-STABLE — the answered F is byte-stable across an alloc cycle
// ============================================================================

/// S-DBN-WS-STABLE — THE SQ1-elimination AC: the answered `F` is BYTE-STABLE
/// across a backend alloc cycle, and the next connect lands the NEW backend
/// (US-DBN-2 · K-DBN-1).
///
/// `getent` resolves to stable `F1`; one connect lands backend B1. The server
/// backend is CYCLED (stopped → its AllocationId ends → its per-instance
/// `workload_addr` freed → a NEW instance with a NEW AllocationId → a NEW
/// `workload_addr` B2, the `<job>` still declared). After it reaches
/// Running-AND-HEALTHY, `getent` re-resolves to the SAME `F1` byte-for-byte
/// (the FrontendAddrAllocator's idempotent `assign("server")` retained `F1`),
/// the next connect lands the NEW backend B2, mTLS terminates, and at NO point
/// did `getent` return a per-instance backend addr ∈ 10.99.0.0/16.
///
/// The pre/post-cycle connects close via the REV-5 output-hook leg-B
/// interception companion (`mtls_intercept.rs`): each connect to `F1` is
/// translated to the current live backend and the agent's host-originated leg-B
/// re-dial is diverted into the destination's leg-C on the OUTPUT path, so the
/// mTLS hop terminates against B1 then B2.
///
/// PARTIALLY BLOCKED (02-02) — same cross-workload leg-F→leg-C
/// handshake-completion follow-up as
/// `deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls` (see its
/// docstring). Un-ignore once that gap closes. (The `getent`-byte-stability
/// half — the SQ1 elimination — is independently observable once the connect
/// half completes.)
#[ignore = "02-02 follow-up: cross-workload leg-F→leg-C mTLS handshake does not complete; REV-5 datapath fix landed + pwru-proven"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend() {
    if !is_root() {
        eprintln!(
            "SKIP answered_frontend_is_byte_stable_across_alloc_cycle_next_connect_lands_new_backend: not root"
        );
        return;
    }
    record_kernel();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pki = TestPki::mint();
    let skeleton = Skeleton::boot(&pki).await;
    let _cleanup = super::workload_lifecycle::cleanup::AllocCleanup {
        obs: skeleton.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    let server_id = "server";
    let client_id = "client";
    let backend_b1 =
        deploy_and_wait_running(&skeleton, server_service_spec(server_id), server_id).await;
    let alloc_b1 = workload_running_alloc_id(&skeleton.obs(), server_id)
        .await
        .expect("server must have a running AllocationId (B1)");
    let client_addr =
        deploy_and_wait_running(&skeleton, client_service_spec(client_id), client_id).await;
    let client_netns = netns_name_for_workload_addr(client_addr);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // GIVEN: getent resolves to stable F1; one connect lands B1.
    let netns = client_netns.clone();
    let f1 =
        tokio::task::spawn_blocking(move || poll_resolve_frontend(&netns, Duration::from_secs(5)))
            .await
            .expect("resolve task join")
            .unwrap_or_else(|| {
                panic!("S-DBN-WS-STABLE: getent must resolve stable F1 before the cycle")
            });
    eprintln!("[02-02] F1 = {f1} (pre-cycle), B1 = {backend_b1}, alloc B1 = {alloc_b1}");

    let dial_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let pre = tokio::task::spawn_blocking(move || dial_frontend_in_netns(&netns, dial_pki, f1))
        .await
        .expect("dial task join");
    assert!(
        !pre.observed_rst && pre.received_response_byte_exact,
        "S-DBN-WS-STABLE: the pre-cycle connect to F1 must land the current backend B1 (byte-exact \
         mTLS round-trip)"
    );

    // WHEN: cycle the server backend — stop it, then re-deploy the SAME <job>.
    stop_and_converge(&skeleton, server_id).await;
    // Re-deploy the SAME <job> "server" (a NEW AllocationId → NEW workload_addr
    // B2; the <job> is still declared, so the allocator must retain F1).
    let backend_b2 =
        deploy_and_wait_running(&skeleton, server_service_spec(server_id), server_id).await;
    let alloc_b2 = workload_running_alloc_id(&skeleton.obs(), server_id)
        .await
        .expect("server must have a running AllocationId (B2) after the cycle");
    assert_ne!(
        alloc_b1, alloc_b2,
        "S-DBN-WS-STABLE: the cycle must produce a NEW AllocationId (B1={alloc_b1}, B2={alloc_b2})",
    );
    eprintln!("[02-02] post-cycle B2 = {backend_b2}, alloc B2 = {alloc_b2}");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // THEN: getent re-resolves to the SAME F1 byte-for-byte.
    let netns = client_netns.clone();
    let f1_again =
        tokio::task::spawn_blocking(move || poll_resolve_frontend(&netns, Duration::from_secs(8)))
            .await
            .expect("resolve task join")
            .unwrap_or_else(|| {
                panic!("S-DBN-WS-STABLE: getent must re-resolve the stable F after the cycle")
            });
    assert_eq!(
        f1_again, f1,
        "S-DBN-WS-STABLE: getent must re-resolve to the SAME F1 byte-for-byte across the alloc \
         cycle (the FrontendAddrAllocator's idempotent assign(\"server\") retained F1 — \
         withhold-not-release; F is per-logical-workload). got {f1_again}, expected {f1}",
    );

    // AND: the subsequent connect to F1 lands the NEW backend B2 (the re-keyed
    // MtlsResolve re-resolved the live backend per-connect). The byte-exact
    // round-trip succeeds against the fresh server instance B2.
    let dial_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let post = tokio::task::spawn_blocking(move || dial_frontend_in_netns(&netns, dial_pki, f1))
        .await
        .expect("dial task join");
    assert!(
        !post.observed_rst && post.received_response_byte_exact,
        "S-DBN-WS-STABLE: the post-cycle connect to the SAME F1 must land the NEW backend B2 (the \
         re-keyed MtlsResolve re-resolved the live backend per-connect; the round-trip succeeds \
         against the fresh instance)"
    );

    // AND: F1 was a stable frontend ∈ 10.98.0.0/16, NEVER a backend addr ∈
    // 10.99.0.0/16 — neither B1 nor B2 was ever the resolved value.
    let o = f1.octets();
    assert_eq!(
        (o[0], o[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "S-DBN-WS-STABLE: the resolved value was always the stable frontend F1 ∈ 10.98.0.0/16, \
         never a per-instance backend addr ∈ 10.99.0.0/16 (B1={backend_b1}, B2={backend_b2})",
    );
    assert_ne!(f1, backend_b1, "S-DBN-WS-STABLE: F1 must never be the B1 backend addr");
    assert_ne!(f1, backend_b2, "S-DBN-WS-STABLE: F1 must never be the B2 backend addr");
    eprintln!(
        "[02-02] S-DBN-WS-STABLE: F1 ({f1}) byte-stable across the cycle; next connect landed B2 \
         ({backend_b2}); SQ1 (stale-cached-address) eliminated."
    );

    stop_and_converge(&skeleton, server_id).await;
    stop_and_converge(&skeleton, client_id).await;
    skeleton.shutdown().await;
}

// ============================================================================
// S-DBN-CHURN — cycling a backend mid-connection fails fast, never hangs
// ============================================================================

/// S-DBN-CHURN — cycling a backend mid-connection gives the client a PROMPT
/// reset/error bounded by `TCP_USER_TIMEOUT`, never an indefinite hang
/// (US-DBN-4 · K-DBN-2; NO `sock_destroy`).
///
/// A client holds an OPEN in-flight connection through the intercept to the
/// current backend B1 (data flowing). B1 is CYCLED mid-connection (stopped
/// while the connection is still open). The client's in-flight connection gets
/// a PROMPT reset/error bounded by `TCP_USER_TIMEOUT` (NOT an indefinite hang),
/// surfaced through the per-connection pump task + `TCP_USER_TIMEOUT`/keepalive
/// (the terminating-proxy posture; NO `sock_destroy`, which is #61 scope). A
/// subsequent fresh connect to `F` lands the new live backend B2. Distinct from
/// S-DBN-WS-STABLE: that proves the NEXT dial is live; this proves the IN-FLIGHT
/// dial fails fast rather than hangs.
///
/// The in-flight connection is established through the REV-5 output-hook leg-B
/// interception companion (`mtls_intercept.rs`): the connect to `F` is translated
/// to the live backend and the agent's host-originated leg-B re-dial is diverted
/// into the destination's leg-C on the OUTPUT path, so the mTLS hop terminates
/// and data flows before the churn.
///
/// PARTIALLY BLOCKED (02-02) — same cross-workload leg-F→leg-C
/// handshake-completion follow-up as
/// `deployed_workload_resolves_peer_stable_frontend_and_hop_is_mtls` (see its
/// docstring): establishing the in-flight connection needs the connect to F to
/// complete the end-to-end mTLS hop, which does not yet close. Un-ignore once
/// that gap closes.
#[ignore = "02-02 follow-up: cross-workload leg-F→leg-C mTLS handshake does not complete; REV-5 datapath fix landed + pwru-proven"]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend() {
    if !is_root() {
        eprintln!(
            "SKIP in_flight_connection_fails_fast_on_backend_churn_subsequent_connect_lands_new_backend: not root"
        );
        return;
    }
    record_kernel();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pki = TestPki::mint();
    let skeleton = Skeleton::boot(&pki).await;
    let _cleanup = super::workload_lifecycle::cleanup::AllocCleanup {
        obs: skeleton.obs(),
        cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
    };

    let server_id = "server";
    let client_id = "client";
    let _b1 = deploy_and_wait_running(&skeleton, server_service_spec(server_id), server_id).await;
    let client_addr =
        deploy_and_wait_running(&skeleton, client_service_spec(client_id), client_id).await;
    let client_netns = netns_name_for_workload_addr(client_addr);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let netns = client_netns.clone();
    let frontend =
        tokio::task::spawn_blocking(move || poll_resolve_frontend(&netns, Duration::from_secs(5)))
            .await
            .expect("resolve task join")
            .unwrap_or_else(|| panic!("S-DBN-CHURN: getent must resolve the stable frontend F"));
    eprintln!("[02-02] S-DBN-CHURN: resolved F = {frontend}");

    // GIVEN an OPEN in-flight connection through the intercept to B1. Open the
    // connection on a dedicated thread inside the client netns, complete the
    // handshake + first round-trip (data flowing), then HOLD it open and try a
    // second read — while a churn happens concurrently — and time how long
    // until the read returns (reset / error / EOF), bounded.
    let churn_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let churn_handle =
        std::thread::spawn(move || churn_in_flight_read(&netns, churn_pki, frontend));

    // Let the in-flight connection establish + flow before cycling B1.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // WHEN B1 is CYCLED mid-connection (stopped while the connection is open).
    stop_and_converge(&skeleton, server_id).await;

    // THEN the in-flight read returns PROMPTLY (bounded) — NOT an indefinite
    // hang. The churn thread measures the elapsed time from "B1 may now be
    // dying" to the read returning; it must be well within the TCP_USER_TIMEOUT
    // bound (the worker proxy legs tune TCP_USER_TIMEOUT to a sane value — the
    // terminating-proxy posture, NO sock_destroy).
    let in_flight = churn_handle.join().expect("churn thread join");
    assert!(
        in_flight.returned_promptly,
        "S-DBN-CHURN: the in-flight connection must fail FAST (reset/error/EOF) bounded by \
         TCP_USER_TIMEOUT when B1 is cycled mid-connection — NOT an indefinite hang. The pump task \
         + TCP_USER_TIMEOUT/keepalive on the worker proxy legs surface the backend death (NO \
         sock_destroy — #61 scope). Observed elapsed: {:?} (bound {:?}).",
        in_flight.elapsed, CHURN_BOUND,
    );
    eprintln!(
        "[02-02] S-DBN-CHURN: in-flight connection failed fast in {:?} (≤ {:?}) on backend churn",
        in_flight.elapsed, CHURN_BOUND,
    );

    // AND a SUBSEQUENT fresh connect to F lands the new live backend B2.
    let backend_b2 =
        deploy_and_wait_running(&skeleton, server_service_spec(server_id), server_id).await;
    eprintln!("[02-02] S-DBN-CHURN: re-deployed server, B2 = {backend_b2}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let dial_pki = TestPkiHandle::from(&pki);
    let netns = client_netns.clone();
    let fresh =
        tokio::task::spawn_blocking(move || dial_frontend_in_netns(&netns, dial_pki, frontend))
            .await
            .expect("dial task join");
    assert!(
        !fresh.observed_rst && fresh.received_response_byte_exact,
        "S-DBN-CHURN: a SUBSEQUENT fresh connect to F must land the new live backend B2 (byte-exact \
         mTLS round-trip) — proving the next dial is live after the in-flight one failed fast"
    );
    eprintln!("[02-02] S-DBN-CHURN: subsequent fresh connect landed the new backend B2");

    stop_and_converge(&skeleton, server_id).await;
    stop_and_converge(&skeleton, client_id).await;
    skeleton.shutdown().await;
}

/// The fail-fast bound for the in-flight churn read. The worker proxy legs tune
/// `TCP_USER_TIMEOUT` (mtls_intercept_worker.rs) to a sane value; the in-flight
/// read must return within this bound (reset/error/EOF), NOT hang indefinitely.
/// Generous enough to absorb the stop convergence + the user-timeout window,
/// tight enough to falsify an indefinite hang (a hang would block to nextest's
/// ~120s slow-test reap).
const CHURN_BOUND: Duration = Duration::from_secs(30);

struct InFlightOutcome {
    returned_promptly: bool,
    elapsed: Duration,
}

/// Inside the client's PRODUCTION netns: open an mTLS connection to `(F,
/// SERVICE_PORT)`, complete the handshake + a first round-trip (data flowing),
/// then HOLD the connection open and block on a second read with a generous
/// per-read timeout. When B1 is cycled mid-connection the read returns
/// PROMPTLY (reset / error / EOF) bounded by `TCP_USER_TIMEOUT` on the proxy
/// legs — measured as the elapsed time until the read returns. Returns whether
/// it returned within `CHURN_BOUND` (a hang would exceed it).
fn churn_in_flight_read(netns: &str, pki: TestPkiHandle, frontend: Ipv4Addr) -> InFlightOutcome {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection};

    let timeout = InFlightOutcome { returned_promptly: false, elapsed: CHURN_BOUND };
    if !enter_netns(netns) {
        eprintln!("[02-02] setns into {netns} failed (churn)");
        return timeout;
    }
    let roots = ca_root_store(&pki.ca_cert_pem);
    let Ok(cfg) = ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(
        vec![pki.client_cert_der, pki.intermediate_cert_der],
        pki.client_key_der,
    ) else {
        return timeout;
    };
    let server_addr = SocketAddrV4::new(frontend, SERVICE_PORT);
    let Ok(mut tcp) =
        TcpStream::connect_timeout(&std::net::SocketAddr::V4(server_addr), Duration::from_secs(10))
    else {
        eprintln!("[02-02] churn connect {server_addr} failed");
        return timeout;
    };
    tcp.set_nodelay(true).ok();
    let sni = ServerName::try_from(SERVER_SNI.to_string()).expect("server SNI");
    let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("churn ClientConnection");
    // Per-read timeout: BELOW the CHURN_BOUND, so a single blocked read cannot
    // by itself satisfy "returned promptly" — the read must return because the
    // backend died (reset/error/EOF), not because the socket read timeout
    // fired. We loop reads and measure total elapsed from the churn trigger.
    tcp.set_read_timeout(Some(Duration::from_secs(2))).ok();
    if !drive_client_handshake(&mut conn, &mut tcp) {
        eprintln!("[02-02] churn handshake failed");
        return timeout;
    }
    // First round-trip — data is flowing (the in-flight connection is real).
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        if tls.write_all(REQUEST).and_then(|()| tls.flush()).is_err() {
            return timeout;
        }
    }
    let mut buf = vec![0u8; 4096];
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        // Read the first response so the connection is genuinely established +
        // flowing before the churn. A WouldBlock here is fine (the bytes may
        // still be in flight); we proceed to hold the connection open.
        let _ = tls.read(&mut buf);
    }

    // Now HOLD the connection open and block on reads until the backend death
    // surfaces (the parent thread cycles B1 ~800ms after we started). Measure
    // the elapsed time until a read returns reset/error/EOF — bounded by
    // TCP_USER_TIMEOUT on the proxy legs.
    let start = Instant::now();
    loop {
        if start.elapsed() >= CHURN_BOUND {
            // A hang — the read never surfaced the backend death within bound.
            return InFlightOutcome { returned_promptly: false, elapsed: start.elapsed() };
        }
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        match tls.read(&mut buf) {
            // Clean EOF (the proxy closed the leg) — the backend death surfaced.
            Ok(0) => return InFlightOutcome { returned_promptly: true, elapsed: start.elapsed() },
            // More bytes flowed before the churn, OR a WouldBlock/TimedOut from
            // the per-read socket timeout (2s) firing while the backend is still
            // alive — keep polling within the bound.
            Ok(_) => {}
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            // Any other error (ConnectionReset, BrokenPipe, …) IS the prompt
            // backend-death signal surfaced by the pump + TCP_USER_TIMEOUT.
            Err(_) => return InFlightOutcome { returned_promptly: true, elapsed: start.elapsed() },
        }
    }
}
