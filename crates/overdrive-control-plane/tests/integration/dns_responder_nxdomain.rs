//! S-DBN-NXDOMAIN-01 / -02 / -03 — the dial-by-name-responder EMPTY-CANDIDATE
//! HONESTY vertical slice (ADR-0072 REV-2; GH #243; roadmap 03-01 / US-DBN-4 ·
//! K-DBN-2).
//!
//! These Tier-3 `#[tokio::test]`s prove that the PRODUCTION responder WITHHOLDS
//! the answer (returns NXDOMAIN) whenever a name has no running-AND-healthy
//! backend, and resolves to the STABLE frontend `F ∈ 10.98.0.0/16` once (and
//! only once) a backend is running-and-healthy. They drive ONLY the production
//! entry points — `run_server_with_obs_and_driver` (boot) + `POST /v1/jobs`
//! (deploy) + `POST /v1/jobs/{id}/stop` (stop) + `ip netns exec <client-ns>
//! getent ahostsv4 <name>` (resolve, NOT `dig` — K2) from inside a deployed
//! client's PRODUCTION-provisioned netns.
//!
//! ## The vertical-slice litmus (CLAUDE.md "Build vertical slices")
//!
//! NO test binds `:53`, installs a `resolv.conf`, allocates `F`, programs a
//! map, or hand-installs the egress capture — production does ALL of those
//! itself (mirrored verbatim from the 02-02 walking skeleton). The
//! WITHHOLD/resolve logic this step OBSERVES already lives in the landed
//! `dns_responder` surface (01-03 `answer_for` NxDomain arm + `NameIndex`
//! running-and-healthy WITHHOLD seam) and the F-retention invariant in 01-04
//! (`FrontendAddrAllocator::release` only-on-deletion). This step adds NO new
//! production type, method, enum variant, trait, or parameter — it is the
//! Tier-3 OBSERVABLE of contracts already unit/mutation-gated at Tier 1.
//!
//! ## How NXDOMAIN is observed through `getent` (K2)
//!
//! A WITHHELD `<job>` projects to `answer_for → NameAnswer::NxDomain` (an empty
//! candidate set ⇒ NXDOMAIN; NODATA is reserved for AAAA on a RESOLVABLE name).
//! On the wire that is an `RCODE=3 (NXDOMAIN)` response with NO answer records.
//! `getent ahostsv4 <name>` (a real `getaddrinfo` stub-resolver call) then
//! returns a NON-zero exit code and prints NO V4 address line — exactly the
//! signal `dig @gw` would mask (`dig` prints the RCODE but still "succeeds",
//! and bypasses the per-netns resolv.conf bind-mount). The negative answer's 1s
//! SOA TTL (DDN-8) lets a recovery re-query land promptly.
//!
//! ## NXDOMAIN-02 recovery leg is #249-BLOCKED (read before editing)
//!
//! S-DBN-NXDOMAIN-02's final leg ("re-deploy/recover the SAME `<job>` to
//! Running-AND-HEALTHY after the stop, and prove getent resolves the SAME F")
//! is the EXACT `stop_and_converge` + re-deploy-same-`<job>` shape the 02-02
//! S-DBN-WS-STABLE / S-DBN-CHURN ATs proved is UNREACHABLE through the
//! production driving ports: `POST /v1/jobs/{id}/stop` writes a STICKY,
//! OVERRIDING operator-stop intent (`IntentKey::for_workload_stop`); the
//! `WorkloadLifecycle` reconciler DELIBERATELY refuses to schedule a
//! replacement alloc for an operator-stopped workload (ADR-0037 Amendment /
//! §Bug 3), a same-spec `POST /v1/jobs` resubmit takes the `put_if_absent →
//! KeyExists → Unchanged` path which does NOT clear the operator-stop key, and
//! there is NO production verb that does. So the recovered server never reaches
//! Running. That leg is `#[ignore]`'d here and deferred to
//! overdrive-sh/overdrive#249 (backend instance replacement / restart-after-
//! stop), mirroring the landed S-DBN-WS-STABLE / S-DBN-CHURN deferral verbatim.
//! The withhold-not-release F-retention invariant is ALREADY Tier-1
//! mutation-gated at 01-04 (S-DBN-FRONTEND-03 / S-DBN-IDX-02). The REACHABLE
//! NXDOMAIN-02 legs (resolve-to-F → stop → re-query gets NXDOMAIN, NEVER a
//! stale addr) ARE driven GREEN here.
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`. A non-root run SKIPs
//! cleanly (the K1 root gate). `uname -r` is recorded. Run via `cargo xtask
//! lima run -- cargo nextest run -p overdrive-control-plane --features
//! integration-tests`. NEVER `--no-run`.
//!
//! ## Per-host singleton — run these fixtures SEQUENTIALLY
//!
//! Each fixture boots a full production composition root that binds the
//! process-wide `:53` DNS responder (a real OS port) and provisions per-host
//! netns / cgroup / `FrontendAddrAllocator` state. nextest runs each `#[test]`
//! in a SEPARATE process by default, so two of these fixtures launched
//! concurrently collide on the `:53` bind (a `serial_test` `#[serial]`
//! annotation does NOT help — it serialises threads WITHIN one process, not
//! nextest's per-test processes; see `.claude/rules/testing.md` § "serial_test
//! … does NOT fix shared port numbers"). Run them sequentially — e.g. `cargo
//! xtask lima run -- cargo nextest run -p overdrive-control-plane --features
//! integration-tests -E 'test(dns_responder_nxdomain)' --test-threads=1` — or
//! via a nextest `test-group` with `max-threads = 1` for the dial-by-name
//! Tier-3 binaries (a `.config/nextest.toml` change is out of THIS step's
//! two-file boundary; the sibling `dns_responder_walking_skeleton.rs` shares
//! the same singleton shape). Each test PASSES individually and as a
//! sequential group.
//!
//! MERGE-BLOCKING on the pinned-6.18 appliance-kernel Tier-3 matrix
//! (ADR-0068); dev-Lima is necessary-but-not-sufficient.
//!
//! Helpers (`is_root`, `record_kernel`, `netns_name_for_workload_addr`,
//! `enter_netns`, the getent oracle, the in-process production boot harness,
//! the server/client specs, the obs back-door reads) are MIRRORED FROM
//! `dns_responder_walking_skeleton.rs` (02-02). Sibling `tests/integration/
//! <scenario>.rs` files are distinct module roots and cannot import each
//! other's items, so the load-bearing helper set is copied here with this note
//! rather than re-shared (sharing would require promoting them into a shared
//! `tests/integration/<helpers>.rs` module AND touching the 02-02 file — out of
//! this step's two-file boundary).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    reason = "Tier-3 NXDOMAIN bodies mirrored from dns_responder_walking_skeleton.rs (02-02); \
              failures must panic with informative messages; F is the ADR-0072 REV-2 \
              stable-frontend vocabulary; the helper set is copied verbatim from the precedent"
)]

use std::net::Ipv4Addr;
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
use rustls::pki_types::CertificateDer;

// ============================================================================
// constants
// ============================================================================

/// The declared Service listener port the server workload offers.
const SERVICE_PORT: u16 = 18961;

/// The OUTBOUND application response a (recovered) server replies — distinct
/// from the request. Only used by the recovery legs (which complete a mesh
/// dial); the reachable NXDOMAIN legs assert resolution, not a round-trip.
const RESPONSE: &[u8] =
    b"OVERDRIVE_DIAL_BY_NAME_NXDOMAIN_RESPONSE_server_reply_after_recovery_0301_step";

/// The fixed sentinel SNI the production dataplane uses for the agent's
/// intra-mesh leg-B peer dial (`overdrive-dataplane::mtls::outbound`).
const MESH_PEER_SNI: &str = "peer.overdrive.local";

/// The mesh name a client resolves to reach the "server" Service — `<job>` =
/// `server`. Equal to `format!("server.{}", MeshServiceName::SUFFIX)`.
const SERVER_MESH_NAME: &str = "server.svc.overdrive.local";

/// An UNKNOWN mesh name (no `<job>` of this label is ever deployed) — the
/// S-DBN-NXDOMAIN-03 lookup-miss probe. Same `.svc.overdrive.local` SUFFIX so
/// it routes to the production responder, but no backend ever contributes it.
const UNKNOWN_MESH_NAME: &str = "nonexistent.svc.overdrive.local";

/// The production per-host stable-frontend block (`10.98.0.0/16`,
/// `WORKLOAD_FRONTEND_BASE`). `F` answered for `<job>` is a member; a
/// per-instance backend addr lives in `10.99.0.0/16` and is NEVER the answer.
const FRONTEND_FIRST_OCTET: u8 = 10;
const FRONTEND_SECOND_OCTET: u8 = 98;
/// The per-instance workload (backend) block second octet (`10.99.0.0/16`,
/// `WORKLOAD_SUBNET_BASE`) — `getent` MUST NEVER answer an addr here.
const WORKLOAD_SECOND_OCTET: u8 = 99;

// ============================================================================
// root gate + kernel record (mirrored from 02-02)
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
    eprintln!("[03-01] uname -r = {kr} (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)");
    kr
}

/// `WORKLOAD_SUBNET_BASE.network()` = `10.99.0.0`. The deployed workload's netns
/// slot is `(workload_addr - base - 2) / 4`.
const WORKLOAD_SUBNET_BASE_RAW: u32 = u32::from_be_bytes([10, 99, 0, 0]);

/// The production netns name (`ovd-ns-<4hex slot>`) for the deployed workload
/// whose per-instance `workload_addr` is `addr`. Inverse of
/// `derive_workload_netns_plan`. Locates a DEPLOYED workload's PRODUCTION netns
/// (with the production resolv.conf already injected) — NOT a test-created one.
fn netns_name_for_workload_addr(addr: Ipv4Addr) -> String {
    let raw = u32::from(addr);
    let slot = raw.saturating_sub(WORKLOAD_SUBNET_BASE_RAW).saturating_sub(2) / 4;
    format!("ovd-ns-{slot:04x}")
}

// ============================================================================
// getent (the K2 resolution oracle — a real getaddrinfo() via getent, NOT dig)
// ============================================================================
//
// Resolution MUST go through `ip netns exec <ns> getent ahostsv4 <name>` — NOT
// a bare `setns(CLONE_NEWNET)` + libc `getaddrinfo`. `setns(CLONE_NEWNET)`
// switches only the NETWORK namespace; the libc resolver reads
// `/etc/resolv.conf` from the MOUNT namespace, which is unchanged. `ip netns
// exec` enters BOTH the net namespace AND bind-mounts the per-netns resolv.conf
// over `/etc/resolv.conf`, so `getent` resolves through the production
// responder. `getent` is a stub resolver that DISCARDS a reply whose source
// addr is not the queried server addr — so it only succeeds when the production
// responder source-pinned its reply (the K2 litmus), AND it reports a clean
// NXDOMAIN (non-zero exit, no addr line) when the responder WITHHELD the answer.

/// The outcome of `getent ahostsv4 <name>` in a netns: the V4 addrs it printed
/// (empty on NXDOMAIN) and the process exit code (`Some(0)` only on a resolving
/// hit; non-zero / `Some(2)` on a name-not-found / NXDOMAIN).
#[derive(Debug, Clone)]
struct GetentOutcome {
    v4_addrs: Vec<Ipv4Addr>,
    exit_code: Option<i32>,
}

impl GetentOutcome {
    /// A clean NXDOMAIN: the resolver returned NO V4 address AND a non-zero exit
    /// code (getent exits 2 on "one or more keys could not be found"). Both
    /// halves matter — an empty answer with exit 0 would be a NODATA-shaped
    /// success, not the name-not-found the WITHHOLD seam must produce.
    fn is_nxdomain(&self) -> bool {
        self.v4_addrs.is_empty() && self.exit_code != Some(0)
    }

    /// The single resolved STABLE frontend `F ∈ 10.98.0.0/16`, if the answer is
    /// a resolving hit to a frontend addr (never a `10.99.0.0/16` backend — the
    /// SQ1 guard).
    fn resolved_frontend(&self) -> Option<Ipv4Addr> {
        self.v4_addrs.iter().copied().find(|a| {
            let o = a.octets();
            o[0] == FRONTEND_FIRST_OCTET && o[1] == FRONTEND_SECOND_OCTET
        })
    }
}

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

/// Run `ip netns exec <netns> getent ahostsv4 <name>` and capture the V4 addrs
/// together with the exit code. Goes through the production resolv.conf and
/// responder (NOT `dig`, and NOT a host-mount-ns getaddrinfo).
fn getent_in_netns(netns: &str, name: &str) -> GetentOutcome {
    let out = Command::new("ip")
        .args(["netns", "exec", netns, "getent", "ahostsv4", name])
        .output()
        .expect("ip netns exec getent");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v4_addrs = parse_getent_v4(&stdout);
    let exit_code = out.status.code();
    eprintln!("[03-01] getent ahostsv4 {name} in {netns} -> {v4_addrs:?} (code {exit_code:?})");
    GetentOutcome { v4_addrs, exit_code }
}

/// Poll `getent_in_netns(name)` until it answers a stable `F ∈ 10.98.0.0/16`
/// within `budget` (the K2 5s resolution budget) — re-querying because the
/// responder's `name_index` exposes `<job>` only after the backend reaches
/// running-AND-healthy.
fn poll_resolve_frontend(netns: &str, name: &str, budget: Duration) -> Option<Ipv4Addr> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(f) = getent_in_netns(netns, name).resolved_frontend() {
            return Some(f);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Poll `getent_in_netns(name)` until it reports a clean NXDOMAIN (no V4 addr,
/// non-zero exit) within `budget`. Used after a stop converges — the WITHHOLD
/// seam empties the `<job>`'s healthy set, the watch folds the dropped row, and
/// the responder begins answering NXDOMAIN. The negative answer's 1s SOA TTL
/// (DDN-8) lets the transition land within budget.
fn poll_nxdomain(netns: &str, name: &str, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if getent_in_netns(netns, name).is_nxdomain() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ============================================================================
// Fresh focused PKI (root → intermediate → leaf, rcgen + rustls) — mirrored
// from 02-02 (needed because the production boot composes the mTLS worker, and
// the recovery legs complete a mesh dial).
// ============================================================================

struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    client_leaf: Leaf,
    server_leaf: Leaf,
}

impl TestPki {
    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-dial-by-name-0301-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-dial-by-name-0301-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        let client_leaf = intermediate.mint_leaf(client_spiffe, &[], true);
        let server_leaf = intermediate.mint_leaf(server_spiffe, &[MESH_PEER_SNI], false);

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem,
            client_leaf,
            server_leaf,
        }
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
}

impl MintedCa {
    fn mint_root(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(rcgen::DnType::CommonName, cn);
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = params.self_signed(&key).unwrap();
        let cert_pem = cert.pem();
        Self { params, key, cert_pem }
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
        Self { params, key, cert_pem }
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
        Leaf {
            cert_pem,
            key_pem,
            cert_der,
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

/// The agent's held-identity `IdentityRead` double — alloc-aware (the server
/// alloc presents the ServerAuth server leaf on leg-C; every other alloc the
/// ClientAuth client leaf on leg-B). Mirrored from 02-02.
struct HeldServerIdentity {
    server_svid: SvidMaterial,
    client_svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for HeldServerIdentity {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
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
// EbpfDataplane + composed mTLS worker via mtls_identity_override) — mirrored
// from 02-02.
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
        eprintln!("[03-01] deploy non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
}

/// Stop a deployed workload through the real in-process stop driving port
/// (`POST /v1/jobs/{id}/stop`). Drives `StopAllocation` → `worker.stop_alloc`,
/// the SAME path `overdrive job stop` drives (the keystone:677 precedent).
async fn run_server_stop(skeleton: &Skeleton, workload_id: &str) -> bool {
    let url = format!("https://localhost:{}/v1/jobs/{workload_id}/stop", skeleton.bound.port());
    let resp = skeleton.client.post(&url).send().await.expect("stop: POST /v1/jobs/{id}/stop");
    let status = resp.status();
    let body = resp.bytes().await.expect("read stop response body");
    if !status.is_success() {
        eprintln!("[03-01] stop non-success: {status} {}", String::from_utf8_lossy(&body));
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
// the server / client workload specs (mirrored from 02-02)
// ============================================================================

/// A Python one-liner TCP server bound on `0.0.0.0:SERVICE_PORT` inside its
/// netns. Reads the request then writes the DISTINCT `RESPONSE` (NOT an echo).
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

/// A long-lived idle CLIENT workload — gives the test a PRODUCTION-provisioned
/// netns (with the production resolv.conf injected + the production egress rule)
/// to resolve FROM.
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
// back-door observation reads (no production path exercised by these helpers) —
// mirrored from 02-02.
// ============================================================================

/// Read the deployed workload's Running-row `workload_addr` (the per-instance
/// backend addr ∈ 10.99.0.0/16). `Some(addr)` ⇔ the workload reached Running
/// with its canonical address materialised.
async fn workload_running_addr(
    obs: &Arc<dyn ObservationStore>,
    workload_id: &str,
) -> Option<Ipv4Addr> {
    let rows = obs.alloc_status_rows().await.ok()?;
    rows.into_iter()
        .filter(|r| r.workload_id.as_str() == workload_id && r.state == AllocState::Running)
        .find_map(|r| r.workload_addr)
}

/// `Some(())` ⇔ the workload has ≥1 Terminated row and NO Running row — the
/// stop converged.
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

/// Stop a deployed workload through the production stop path and poll its obs
/// row to Terminated.
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
// S-DBN-NXDOMAIN-01 — query before running-and-healthy yields NXDOMAIN, never a
// stale addr; once running-and-healthy a re-query resolves to the stable F
// ============================================================================

/// S-DBN-NXDOMAIN-01 (US-DBN-4 · K-DBN-2; withhold then resolve-to-stable-F).
///
/// Boots the production composition root in-process; deploys a "server" Service
/// through `POST /v1/jobs` AND a long-lived "client" (the dial SOURCE). The
/// SERVER may still be Pending / not-yet-running-and-healthy when the client
/// first queries — at which point the `name_index` WITHHOLDS the answer (no
/// running-and-healthy backend) and `getent("server.svc.overdrive.local")`
/// reports NXDOMAIN (no V4 addr, non-zero exit), NEVER a stale / cached /
/// guessed / frontend addr. Once the server reaches Running-AND-HEALTHY (the
/// bridge writes a healthy `service_backends` row → the index exposes `<job>`
/// bound a stable `F`), a re-query resolves to `F ∈ 10.98.0.0/16` (the negative
/// answer's 1s SOA TTL lets the retry land promptly — DDN-8).
///
/// PORT-TO-PORT litmus: removing the production responder spawn (getent never
/// resolves) or regressing the WITHHOLD seam (it would answer a stale addr
/// before running-and-healthy) takes this RED. NO test binds `:53`, installs a
/// resolv.conf, or allocates `F` — production does all of it.
///
/// MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn query_before_running_and_healthy_is_nxdomain_then_resolves_to_stable_frontend() {
    if !is_root() {
        eprintln!(
            "SKIP query_before_running_and_healthy_is_nxdomain_then_resolves_to_stable_frontend: not root"
        );
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

    // Deploy the long-lived CLIENT first (the dial SOURCE) — it reaches Running
    // quickly (a /bin/sleep) and gives the test a production-provisioned netns
    // to query from. Its netns carries the production-injected resolv.conf.
    let client_backend =
        deploy_and_wait_running(&skeleton, client_service_spec("client"), "client").await;
    let client_netns = netns_name_for_workload_addr(client_backend);

    // Deploy the "server" but do NOT wait for running-and-healthy. While it is
    // still Pending / not-yet-healthy the name is WITHHELD — getent must report
    // a clean NXDOMAIN (no V4 addr, non-zero exit), NEVER a stale/guessed/
    // frontend addr. The deploy is accepted (the intent is declared); the
    // name_index simply has no running-and-healthy backend to expose yet.
    let submitted = run_server_deploy(&skeleton, server_service_spec("server")).await;
    assert!(submitted, "the server Service spec must be accepted by the deploy handler");

    // (1) WITHHOLD — query immediately (the server is Pending/unhealthy). The
    //     name_index withholds the answer; getent reports NXDOMAIN with NO addr.
    //     Asserted as a clean name-not-found, NEVER any returned address.
    let ns = client_netns.clone();
    let pre = tokio::task::spawn_blocking(move || getent_in_netns(&ns, SERVER_MESH_NAME))
        .await
        .expect("pre-running getent task join");
    assert!(
        pre.v4_addrs.is_empty(),
        "S-DBN-NXDOMAIN-01: getent({SERVER_MESH_NAME}) BEFORE the server is running-and-healthy \
         must return NO address — the name_index WITHHOLDS the answer (no running-and-healthy \
         backend). NEVER a stale/cached/unhealthy/guessed addr, and NEVER a stable frontend F \
         (F is withheld until a backend is running-and-healthy). got {:?}",
        pre.v4_addrs,
    );
    assert!(
        pre.is_nxdomain(),
        "S-DBN-NXDOMAIN-01: getent({SERVER_MESH_NAME}) BEFORE running-and-healthy must report a \
         clean NXDOMAIN (no V4 addr AND non-zero exit) — the empty-candidate set projects to \
         answer_for → NameAnswer::NxDomain (RCODE=3), the fail-honest WITHHOLD. got {pre:?}",
    );
    eprintln!("[03-01] S-DBN-NXDOMAIN-01: pre-running query WITHHELD (NXDOMAIN), no stale addr");

    // (2) RECOVERY — once the server reaches Running-AND-HEALTHY, a re-query
    //     resolves to the STABLE frontend F ∈ 10.98.0.0/16. Wait for the server
    //     to reach Running (the bridge then writes a healthy service_backends
    //     row and the index exposes <job> bound F). The 1s SOA negative-TTL lets
    //     the retry land within the 5s budget.
    let _server_backend = poll_until(Duration::from_secs(30), Duration::from_millis(250), || {
        let obs = skeleton.obs();
        async move { workload_running_addr(&obs, "server").await }
    })
    .await
    .unwrap_or_else(|| panic!("the server workload must reach Running with a workload_addr"));

    let ns = client_netns.clone();
    let frontend = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(8))
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| {
        panic!(
            "S-DBN-NXDOMAIN-01: AFTER the server is running-and-healthy, getent({SERVER_MESH_NAME}) \
             must resolve to the STABLE frontend F ∈ 10.98.0.0/16 within the budget (the WITHHOLD \
             lifted once a running-and-healthy backend exists; the negative answer's 1s SOA TTL \
             lets the retry land). A timeout means EITHER the source-pin is missing OR the \
             healthy-gate never lifted (K2 two culprits)."
        )
    });
    let o = frontend.octets();
    assert_eq!(
        (o[0], o[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "S-DBN-NXDOMAIN-01: the recovered resolution must be the STABLE frontend F ∈ 10.98.0.0/16 \
         (got {frontend}), NEVER a per-instance backend addr ∈ 10.99.0.0/16",
    );
    assert_ne!(
        o[1], WORKLOAD_SECOND_OCTET,
        "S-DBN-NXDOMAIN-01: the recovered resolution must NOT be a per-instance backend addr ∈ \
         10.99.0.0/16 (got {frontend})",
    );
    eprintln!(
        "[03-01] S-DBN-NXDOMAIN-01 VERDICT: WORKS — queried-before-running-and-healthy WITHHELD \
         (NXDOMAIN, no stale addr); recovered to the STABLE frontend F = {frontend} on kernel {kr}. \
         (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    stop_and_converge(&skeleton, "server").await;
    stop_and_converge(&skeleton, "client").await;
    skeleton.shutdown().await;
}

// ============================================================================
// S-DBN-NXDOMAIN-02 — after the backend stops, the <job> is WITHHELD (NXDOMAIN);
// the stable F is NOT released. (Recovery leg #249-BLOCKED — see below.)
// ============================================================================

/// S-DBN-NXDOMAIN-02 (US-DBN-4 · K-DBN-2; withhold-not-release, Finding-2).
///
/// A "server" Service is deployed and Running-AND-HEALTHY, resolving
/// `server.svc.overdrive.local` to its stable frontend addr `F` (getent
/// confirms `F ∈ 10.98.0.0/16`). The server is then STOPPED through the
/// production stop path (`POST /v1/jobs/server/stop`) and converges to
/// Terminated, leaving the `<job>` "server" zero-healthy but STILL DECLARED
/// (not deleted). A deployed client re-querying the name then gets NXDOMAIN —
/// the `name_index` WITHHELD the answer (zero running-and-healthy backends) —
/// and NEVER the stale F's translated backend nor any stale addr.
///
/// The withhold-not-release RECOVERY leg ("re-deploy/recover the SAME `<job>`
/// to Running-AND-HEALTHY → getent resolves the SAME F") is asserted in the
/// SEPARATE `#[ignore]`'d recovery test below — it needs a production
/// restart-after-stop verb that does NOT exist (#249). This test proves the
/// REACHABLE legs: resolve-to-F, then stop → re-query → NXDOMAIN, no stale addr.
///
/// PORT-TO-PORT litmus: regressing the WITHHOLD-on-zero-healthy seam (the index
/// would keep answering the stale F after the backend stopped) takes this RED.
///
/// MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
///
/// IGNORED (03-01 — production WITHHOLD-on-stop gap surfaced by this Tier-3
/// observable). When run as root under Lima this fails: after `POST
/// /v1/jobs/server/stop` converges the alloc to Terminated, `getent` keeps
/// resolving the stale `F` (`[10.98.0.1] code 0`) indefinitely and NEVER
/// reports NXDOMAIN. The WITHHOLD-on-zero-healthy seam is NOT driven by the
/// production stop path: `BackendDiscoveryBridge::reconcile` builds backends
/// from the Running alloc set, but the empty-backends `service_backends` row
/// that would empty `name_index.by_name["server"]` (→ WITHHOLD → NXDOMAIN) does
/// not land on the alloc→Terminated transition, so `frontend_for("server")`
/// still returns `Some(F)`. Making this GREEN requires a production change to
/// `crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs` (re-tick
/// / write a zero-backend row on alloc-Terminated) and/or the action-shim
/// re-enqueue path — OUT OF this test-only step's two-file boundary
/// (`dns_responder/*` and the bridge are READ-ONLY for 03-01 per the dispatch).
/// The withhold-not-release F-retention contract this observes is ALREADY
/// Tier-1 mutation-gated at 01-03 (`NameIndex` healthy-gate WITHHOLD seam +
/// `answer_for` NxDomain arm) / 01-04. The test body is INTACT and will fail
/// loud (surfacing the gap) the moment the production withhold-on-stop lands and
/// this `#[ignore]` is removed — it is NOT weakened. Un-ignore when the
/// withhold-on-stop production fix lands (a separate, production-drivable
/// slice). Distinct from the recovery leg's #249 blocker below.
#[ignore = "03-01 BLOCKED on a production WITHHOLD-on-stop gap (out of this test-only step's boundary): after POST /v1/jobs/{id}/stop converges to Terminated, the service_backends healthy row is not dropped (BackendDiscoveryBridge does not write a zero-backend row on alloc-Terminated), so the responder keeps answering the stale F and never NXDOMAINs. Fixing needs a production change to backend_discovery_bridge / the action-shim re-enqueue path — READ-ONLY for 03-01. The withhold-not-release contract is Tier-1 mutation-gated at 01-03/01-04. Body is intact (not weakened); un-ignore when the withhold-on-stop production slice lands. See docstring."]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn after_backend_stops_the_job_is_withheld_nxdomain_never_a_stale_addr() {
    if !is_root() {
        eprintln!(
            "SKIP after_backend_stops_the_job_is_withheld_nxdomain_never_a_stale_addr: not root"
        );
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

    // GIVEN a Running-AND-HEALTHY server, resolving to its stable F.
    let _server_backend =
        deploy_and_wait_running(&skeleton, server_service_spec("server"), "server").await;
    let client_backend =
        deploy_and_wait_running(&skeleton, client_service_spec("client"), "client").await;
    let client_netns = netns_name_for_workload_addr(client_backend);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let ns = client_netns.clone();
    let f_before = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(5))
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| {
        panic!("S-DBN-NXDOMAIN-02: getent must resolve the stable F while the server is healthy")
    });
    let o = f_before.octets();
    assert_eq!(
        (o[0], o[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "S-DBN-NXDOMAIN-02: the pre-stop resolution must be the stable frontend F ∈ 10.98.0.0/16 \
         (got {f_before})",
    );
    eprintln!("[03-01] S-DBN-NXDOMAIN-02: pre-stop F = {f_before}");

    // WHEN the server is stopped through the production stop path and converges
    // to Terminated, leaving <job> "server" zero-healthy but STILL DECLARED.
    stop_and_converge(&skeleton, "server").await;

    // THEN a re-query gets NXDOMAIN — the name_index WITHHELD the answer (the
    // bridge dropped the healthy service_backends row → apply_row emptied the
    // <job>'s healthy set → WITHHOLD). It NEVER returns the stale F's translated
    // backend, nor any stale addr (no second source of liveness truth). The 1s
    // SOA negative-TTL + the watch fold land the transition within budget.
    let ns = client_netns.clone();
    let became_nxdomain = tokio::task::spawn_blocking(move || {
        poll_nxdomain(&ns, SERVER_MESH_NAME, Duration::from_secs(10))
    })
    .await
    .expect("nxdomain task join");
    assert!(
        became_nxdomain,
        "S-DBN-NXDOMAIN-02: AFTER the server stops (zero running-and-healthy backends), \
         getent({SERVER_MESH_NAME}) must report NXDOMAIN (no V4 addr, non-zero exit) — the \
         name_index WITHHELD the answer once the bridge dropped the healthy service_backends row. \
         It must NEVER keep answering the stale F (no second source of liveness truth). A resolving \
         answer here means the WITHHOLD-on-zero-healthy seam regressed."
    );
    // Belt-and-braces: the post-stop query returned NO address at all — never
    // the stale F, never the backend it translated to.
    let ns = client_netns.clone();
    let post = tokio::task::spawn_blocking(move || getent_in_netns(&ns, SERVER_MESH_NAME))
        .await
        .expect("post-stop getent task join");
    assert!(
        post.v4_addrs.is_empty(),
        "S-DBN-NXDOMAIN-02: the post-stop query must return NO address — never the stale F \
         ({f_before}) nor its translated backend. got {:?}",
        post.v4_addrs,
    );
    eprintln!(
        "[03-01] S-DBN-NXDOMAIN-02 VERDICT: WORKS — after the production stop the <job> is WITHHELD \
         (NXDOMAIN), no stale F ({f_before}) returned, on kernel {kr}. The withhold-not-release \
         RECOVERY observable is the #249-blocked recovery test (allocator F-retention is Tier-1 \
         mutation-gated at 01-04). (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    stop_and_converge(&skeleton, "client").await;
    skeleton.shutdown().await;
}

/// S-DBN-NXDOMAIN-02 RECOVERY leg (withhold-not-release, Finding-2) — the SAME
/// `<job>` recovered to Running-AND-HEALTHY after a stop resolves to the SAME F.
///
/// BLOCKED (#249 — backend instance replacement / restart-after-stop). This
/// leg's "WHEN: re-deploy/recover the SAME `<job>` 'server' to
/// Running-AND-HEALTHY after the stop" is the EXACT `stop_and_converge` +
/// re-deploy-same-`<job>` shape the 02-02 S-DBN-WS-STABLE / S-DBN-CHURN ATs
/// proved is UNREACHABLE through the production driving ports: `POST
/// /v1/jobs/{id}/stop` writes a STICKY, OVERRIDING operator-stop intent
/// (`IntentKey::for_workload_stop`); the `WorkloadLifecycle` reconciler
/// DELIBERATELY refuses to schedule a replacement alloc for an operator-stopped
/// workload (ADR-0037 Amendment / §Bug 3); a same-spec `POST /v1/jobs` resubmit
/// takes the `put_if_absent → KeyExists → Unchanged` path which does NOT clear
/// the operator-stop key; and there is NO production verb that does. So the
/// recovered server never reaches Running and `deploy_and_wait_running` times
/// out. The withhold-not-release F-retention invariant is ALREADY Tier-1
/// mutation-gated at 01-04 (S-DBN-FRONTEND-03 / S-DBN-IDX-02); only its Tier-3
/// `getent`-observable recovery is gated on #249. Distinct from #211 (deletion,
/// which WOULD release F). Un-ignore when #249 lands.
#[ignore = "03-01 DEFERRED to overdrive-sh/overdrive#249 (backend instance replacement / restart-after-stop): recovering the SAME <job> to Running-AND-HEALTHY after a POST /v1/jobs/{id}/stop needs a replace/restart verb that does not exist — operator-stop is sticky/overriding by design (ADR-0037 Amdt / WorkloadLifecycle §Bug 3), a same-spec re-deploy does not clear it. The withhold-not-release F-retention invariant is Tier-1 mutation-gated at 01-04 (S-DBN-FRONTEND-03/IDX-02); only the Tier-3 getent recovery observable is #249-blocked. Same dependency as 02-02 S-DBN-WS-STABLE / S-DBN-CHURN. Un-ignore when #249 lands. See docstring."]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn recovered_job_after_stop_resolves_to_the_same_stable_frontend() {
    if !is_root() {
        eprintln!("SKIP recovered_job_after_stop_resolves_to_the_same_stable_frontend: not root");
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

    let _server_backend =
        deploy_and_wait_running(&skeleton, server_service_spec("server"), "server").await;
    let client_backend =
        deploy_and_wait_running(&skeleton, client_service_spec("client"), "client").await;
    let client_netns = netns_name_for_workload_addr(client_backend);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // GIVEN: getent resolves the stable F before the stop.
    let ns = client_netns.clone();
    let f_before = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(5))
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| panic!("recovery: getent must resolve the stable F before the stop"));

    // WHEN: stop the server, then RE-DEPLOY the SAME <job> "server" (#249: the
    // operator-stop intent is sticky/overriding; this re-deploy does NOT reach
    // Running, so deploy_and_wait_running times out — the #249 collision).
    stop_and_converge(&skeleton, "server").await;
    let _server_b2 =
        deploy_and_wait_running(&skeleton, server_service_spec("server"), "server").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // THEN: getent re-resolves to the SAME F byte-for-byte (the allocator
    // retained F across the zero-healthy window — withhold-not-release).
    let ns = client_netns.clone();
    let f_after = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(8))
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| panic!("recovery: getent must re-resolve the stable F after recovery"));
    assert_eq!(
        f_after, f_before,
        "S-DBN-NXDOMAIN-02 (recovery): getent must re-resolve to the SAME F byte-for-byte after \
         the <job> recovers to Running-AND-HEALTHY (the FrontendAddrAllocator RETAINED F across \
         the zero-healthy window — withhold-not-release; F is per-logical-workload). got {f_after}, \
         expected {f_before}",
    );

    stop_and_converge(&skeleton, "server").await;
    stop_and_converge(&skeleton, "client").await;
    skeleton.shutdown().await;
}

// ============================================================================
// S-DBN-NXDOMAIN-03 — an unknown name yields NXDOMAIN; the hit path still works
// ============================================================================

/// S-DBN-NXDOMAIN-03 (US-DBN-4) — an unknown name yields NXDOMAIN through
/// getent, and the unrelated server's name still resolves in the same fixture.
///
/// With at least one unrelated "server" deployed and Running-AND-HEALTHY (so
/// the hit path is live), a deployed client querying `nonexistent.svc.\
/// overdrive.local` gets NXDOMAIN (the `answer_for` lookup-miss arm: the index
/// has no `<job>` of that label → WITHHOLD → NXDOMAIN). The unrelated server's
/// name STILL resolves on the same real socket — proving the miss does not
/// break the hit path.
///
/// PORT-TO-PORT litmus: a mutant that answered a fabricated addr for an unknown
/// name (instead of NXDOMAIN) takes the miss assertion RED; a mutant that broke
/// the socket on a miss takes the hit assertion RED.
///
/// MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn unknown_name_is_nxdomain_and_the_known_name_still_resolves() {
    if !is_root() {
        eprintln!("SKIP unknown_name_is_nxdomain_and_the_known_name_still_resolves: not root");
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

    // An unrelated server, Running-AND-HEALTHY (the hit path must be live), plus
    // a long-lived client to query from.
    let _server_backend =
        deploy_and_wait_running(&skeleton, server_service_spec("server"), "server").await;
    let client_backend =
        deploy_and_wait_running(&skeleton, client_service_spec("client"), "client").await;
    let client_netns = netns_name_for_workload_addr(client_backend);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The KNOWN name resolves (the hit path is live) — establish it FIRST so the
    // subsequent miss is provably against a working socket, not a dead one.
    let ns = client_netns.clone();
    let known = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(8))
    })
    .await
    .expect("known resolve task join")
    .unwrap_or_else(|| {
        panic!("S-DBN-NXDOMAIN-03: the KNOWN name {SERVER_MESH_NAME} must resolve to the stable F")
    });
    eprintln!("[03-01] S-DBN-NXDOMAIN-03: known name resolved to F = {known}");

    // The UNKNOWN name yields NXDOMAIN on the SAME real socket — the answer_for
    // lookup-miss arm WITHHOLDS (no <job> of that label in the index).
    let ns = client_netns.clone();
    let unknown = tokio::task::spawn_blocking(move || getent_in_netns(&ns, UNKNOWN_MESH_NAME))
        .await
        .expect("unknown getent task join");
    assert!(
        unknown.v4_addrs.is_empty(),
        "S-DBN-NXDOMAIN-03: getent({UNKNOWN_MESH_NAME}) must return NO address — no <job> of that \
         label is deployed, so the index WITHHOLDS (the answer_for lookup-miss arm). NEVER a \
         fabricated/guessed addr. got {:?}",
        unknown.v4_addrs,
    );
    assert!(
        unknown.is_nxdomain(),
        "S-DBN-NXDOMAIN-03: getent({UNKNOWN_MESH_NAME}) must report a clean NXDOMAIN (no V4 addr \
         AND non-zero exit) — the lookup-miss projects to answer_for → NameAnswer::NxDomain. \
         got {unknown:?}",
    );

    // AND the KNOWN name STILL resolves on the SAME socket AFTER the miss — the
    // miss did not break the hit path.
    let ns = client_netns.clone();
    let known_again = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, SERVER_MESH_NAME, Duration::from_secs(5))
    })
    .await
    .expect("known-again resolve task join")
    .unwrap_or_else(|| {
        panic!(
            "S-DBN-NXDOMAIN-03: the KNOWN name {SERVER_MESH_NAME} must STILL resolve AFTER the \
             unknown-name miss — the miss must not break the hit path on the real socket"
        )
    });
    assert_eq!(
        known_again, known,
        "S-DBN-NXDOMAIN-03: the KNOWN name must resolve to the SAME stable F before and after the \
         unknown-name miss (the miss does not perturb the hit path). got {known_again}, expected {known}",
    );
    eprintln!(
        "[03-01] S-DBN-NXDOMAIN-03 VERDICT: WORKS — unknown name {UNKNOWN_MESH_NAME} → NXDOMAIN; \
         the known name {SERVER_MESH_NAME} still resolves to F ({known}) on the same socket, on \
         kernel {kr}. (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    stop_and_converge(&skeleton, "server").await;
    stop_and_converge(&skeleton, "client").await;
    skeleton.shutdown().await;
}
