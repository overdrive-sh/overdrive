//! Tier-3 OUTBOUND enforce-substrate per-direction agent-light ASYMMETRY (step
//! 05-03) — RE-ESTABLISHED FRESH on the Path-A egress nft-TPROXY mechanism.
//!
//! ## What this re-establishes, and why FRESH (not salvaged)
//!
//! The DELETED `overdrive-dataplane/tests/integration/mtls_outbound_enforce.rs`
//! (deleted whole in 04-01) carried the load-bearing ADR-0069 substrate coverage
//! this test re-establishes: the OUTBOUND **per-direction agent-light asymmetry**.
//! That test's connection SETUP was the now-deleted cgroup-rewrite
//! `OutboundWorkload` / `program_redirect_dest` mechanism, so per CLAUDE.md
//! § "Deletion discipline" the test was DELETED, not salvaged-by-rewrite. The
//! ASSERTION it carried — the directional copy strategy — is UNCHANGED by Path A
//! (ADR-0069 carries the enforcement substrate forward VERBATIM; the mechanism
//! swap from cgroup-rewrite to egress nft-TPROXY changes only HOW the connection
//! is captured, NEVER the substrate that enforces it). So this step re-writes that
//! assertion FRESH against the new Path-A egress nft-TPROXY setup, structurally
//! symmetric with the SURVIVING
//! `overdrive-dataplane/tests/integration/mtls_inbound_enforce.rs`.
//!
//! ## The directional asymmetry this proves (ADR-0069, carried forward by Path A)
//!
//! The agent-light substrate is ASYMMETRIC by direction, and the OUTBOUND
//! asymmetry is the INVERSE of inbound (mtls/outbound.rs § module docstring,
//! verbatim):
//!   - **FORWARD** (plaintext workload → ciphertext backend, `legF → legB`) is an
//!     AGENT-LIGHT `read → write_all` **COPY** pump. leg B is kTLS-TX-armed, so the
//!     kernel `tls_sw_sendmsg` encrypts each `write_all`ed record SYNCHRONOUSLY; the
//!     agent does ZERO crypto but DOES copy each forward byte through a userspace
//!     buffer (`splice` INTO a kTLS-TX socket loses records, so the forward is a
//!     copy, not a splice — `PumpHandle::spawn_encrypt`).
//!   - **RETURN** (ciphertext backend → plaintext workload, `legB → legF`) is an
//!     AGENT-LIGHT zero-copy `splice(legB → pipe → legF)` out of leg B's kTLS-RX
//!     (the kernel `tls_sw_splice_read` decrypts each record on splice-out; the
//!     agent issues only `splice`/`poll`, NO per-byte plaintext copy of the
//!     response — `PumpHandle::spawn_decrypt`).
//!
//! The request-carrying OUTBOUND primary is the COPY; the request-carrying INBOUND
//! primary is the zero-copy SPLICE — the exact inverse mtls_inbound_enforce.rs
//! pins for the other direction.
//!
//! ## How this is OBSERVABLE (syscall side effects only — testing.md Tier-3 rules)
//!
//! The directional copy strategy is observable via `strace` on the agent's own
//! pump threads. The test process runs the production accept loop in-process, so the
//! pump threads (`PumpHandle::spawn_encrypt`/`spawn_decrypt` → `std::thread::spawn`)
//! are CLONE_THREAD threads of THIS process — they share the test's thread group
//! (tgid). The netns workload client, by contrast, is a SEPARATE process
//! (`ip netns exec … python3`, a distinct tgid). Rust `TcpStream` `read`/`write_all`
//! lower to `recvfrom`/`sendto` (or `read`/`write`); the return decrypt pump issues
//! `splice(2)`. So:
//!   - the FORWARD COPY surfaces as the request plaintext appearing in a `write(2)`/
//!     `sendto(2)` buffer INTO leg B (the kTLS-TX leg), issued BY A THREAD OF THE TEST
//!     PROCESS (the agent's forward pump) — and NOT riding a `splice` (a copy through
//!     userspace is exactly what the forward is); and
//!   - the RETURN SPLICE surfaces as ≥1 `splice(2)` call (the response decrypt pump,
//!     `splice(legB → legF)`).
//! These are REAL captured syscalls, never the adapter's own bookkeeping.
//!
//! ### Thread-group isolation — the FORWARD oracle MUST attribute to the agent
//!
//! `strace -f` follows the netns client's forked `python3` descendant, whose own
//! `s.sendall(OUTBOUND_REQUEST)` lowers to a `sendto(<plaintext incl. marker>)` —
//! so the request marker appears in the trace on BOTH the agent's forward-pump write
//! AND the workload client's send. The two are distinguished by the leading TID
//! `strace -f` prefixes on every line: the agent's pump threads' TIDs are members of
//! this process's thread group (enumerated from `/proc/self/task`, sampled WHILE the
//! pumps are live); the netns `python3`'s TID is not. The forward-copy oracle counts
//! a marker-carrying write ONLY when its owning TID belongs to the test's thread
//! group — so the workload's identical plaintext send CANNOT satisfy it. Without this
//! filter the oracle is confounded (the client's send alone flips the flag regardless
//! of the agent's pump strategy); WITH it the oracle proves the AGENT copied.
//!
//! ## Driven through the PRODUCTION composition root (port-to-port / TBU defense)
//!
//! The connection is driven END-TO-END through the SHIPPING production seams —
//! `MtlsInterceptWorker::start_alloc` → the spawned outbound `accept_loop`
//! (getsockname → resolve(Mesh) → the real `HostMtlsEnforcement::enforce`) — NOT a
//! hand-rolled replica. The ONLY injected double is the `resolve` port (a
//! `ScriptedResolve`; the production resolve index 01-03 is its own DST's job). The
//! enforce substrate is the REAL `HostMtlsEnforcement` (ADR-0069, UNCHANGED). If the
//! production wiring that drives the outbound enforce substrate were removed, this
//! test goes RED: the netns workload's round-trip would not complete, the `splice`
//! evidence would vanish, and the forward-copy marker would never appear in a
//! `write`/`sendto` into a kTLS-TX leg.
//!
//! ## Authn-only boundary (Q4 / #178)
//!
//! `expected_peer` stays `None` for the enforced connection (v1 authn-only; the
//! intended-peer pinning is #178). This AT asserts encryption + the substrate
//! asymmetry — it MUST NOT assert intended-peer "protection". Identical authn-only
//! discipline to mtls_inbound_enforce.rs and 05-01's last criterion.
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, nft, ip netns, ip
//! rule) AND `strace` (the syscall oracle is load-bearing — present in the canonical
//! Lima VM). A non-root run SKIPs. Run via `cargo xtask lima run -- cargo nextest
//! run -p overdrive-worker --features integration-tests -E
//! 'test(outbound_enforce_substrate_forward_copy_return_splice_asymmetry)'`. NEVER
//! `--no-run` (a compile-only gate is green even when every fixture refuses at
//! boot). `uname -r` is recorded (spike.md: the verdict is pinned to a kernel).
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design
//! (node-global converge-on-boot), so the test scrubs ALL `overdrive-mtls` nft state
//! + the fwmark rule/route + the test netns/veth/lo-addr at START (tolerate
//! pre-existing) AND END. A cross-PROCESS `flock(2)` lock (`KernelStateLock`, on the
//! SAME path the sibling kernel-touching suites use) serialises the kernel-touching
//! tests — nextest runs each `#[test]` in a separate process, so an in-process lock
//! cannot serialise node-global state.

#![allow(
    clippy::doc_markdown,
    clippy::doc_lazy_continuation,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::match_wildcard_for_single_variants,
    clippy::missing_panics_doc,
    clippy::missing_const_for_fn,
    clippy::format_collect,
    reason = "Tier-3 outbound-substrate-asymmetry test body; the directional-asymmetry narrative in the module docstring is prose; skip messages + strace diagnostics go to stderr; failures must panic with informative messages; the libc FFI casts are width conversions on compile-time constants (ETH_P_ALL.to_be() as i32 mirrors traffic.rs); leg F/B are the ADR-0069 contract vocabulary; the single composed Tier-3 scenario drives the round-trip under one strace attach; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the per-byte \\xNN python-literal fold reads clearer than a write! accumulator in a test fixture; const-fn-ability on test constructors is not load-bearing"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddrV4, TcpListener};
use std::os::fd::AsRawFd as _;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use async_trait::async_trait;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

// ============================================================================
// topology constants (mirror the increment-b egress spike + the 05-01/05-02 harness)
// ============================================================================

const NS_W: &str = "nsW-asym0503";
const VETH_W: &str = "vethW-asym05";
const VETH_H: &str = "vethH-asym05";
const HOST_GW: &str = "10.99.0.1";
const WL_ADDR: &str = "10.99.0.2";
const SUBNET_LEN: &str = "24";

/// The mesh backend the OUTBOUND workload dials — a host-side lo-bound address it
/// routes to via the gateway, so its egress genuinely INGRESSES vethH and hits
/// PREROUTING. This is the dialed `orig_dst` the resolve consumer classifies
/// `Mesh`, and the address the real mesh mTLS server (leg-B's peer) binds.
const MESH_BACKEND_IP: &str = "10.200.0.1";
const MESH_BACKEND_PORT: u16 = 18831;

/// `lo` — where leg-B's TLS records (agent → the lo-bound mesh backend) physically
/// carry their bytes, so the AF_PACKET 0x17 confidentiality oracle captures there.
const LOOPBACK_IFACE: &str = "lo";

/// The OUTBOUND application request the workload sends through leg-F → (mTLS leg-B)
/// → the mesh server. Its distinctive interior bytes are the FORWARD-COPY marker:
/// because the forward pump is a `read(legF) → write_all(legB)` COPY, this plaintext
/// MUST appear in a userspace `write`/`sendto` buffer INTO leg B (the kTLS-TX leg),
/// issued by a thread of the TEST process (the agent's forward pump) — proving the
/// forward direction copies through userspace and is NOT a splice. NOTE: the netns
/// workload client ALSO sends this same plaintext (it is the application request),
/// so the marker appears in the trace on the client's `sendto` too; the forward
/// oracle's thread-group filter (see `TraceFindings::parse`) is what attributes the
/// flip to the AGENT and excludes the client's identical send.
const OUTBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_0503_OUTBOUND_REQUEST_forward_copy_marker_workload_to_mesh_legF_to_legB_writeall";
/// The OUTBOUND application response the mesh server replies; it rides back over
/// leg-B's kTLS-RX via the RETURN `splice(legB -> legF)` pump (zero-copy, decrypted
/// on splice-out) to the workload byte-exact.
const OUTBOUND_RESPONSE: &[u8] =
    b"OVERDRIVE_0503_OUTBOUND_RESPONSE_return_splice_mesh_reply_rides_back_over_legB_ktls_rx";

// ============================================================================
// Cross-process kernel-state exclusion (shared path with the sibling suites)
// ============================================================================

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL. nextest runs each `#[test]` in a SEPARATE PROCESS, so an
/// in-process lock cannot serialise them — an `flock(2)` on the fixed path (shared
/// with `egress_tproxy_capture.rs` / `bidirectional_walking_skeleton.rs`) spans
/// processes.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the returned fd is
        // adopted by OwnedFd. flock blocks until the exclusive lock is held.
        let fd = unsafe {
            let raw = libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(raw >= 0, "open kernel-state lock file: {}", std::io::Error::last_os_error());
            let rc = libc::flock(raw, libc::LOCK_EX);
            assert!(rc == 0, "flock LOCK_EX: {}", std::io::Error::last_os_error());
            std::os::fd::OwnedFd::from_raw_fd(raw)
        };
        Self { fd }
    }
}

impl Drop for KernelStateLock {
    fn drop(&mut self) {
        // SAFETY: fd is the live lock fd; LOCK_UN releases the advisory lock.
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// True iff this process is uid 0 (root). IP_TRANSPARENT, nft, `ip netns`, and
/// `ip rule` all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run cannot
/// stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

// ============================================================================
// command shims (mirror egress_tproxy_capture.rs / bidirectional_walking_skeleton.rs)
// ============================================================================

fn ip(args: &[&str]) {
    let out = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip");
    assert!(
        out.status.success(),
        "ip {args:?} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

fn ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn sysctl_w(kv: &str) {
    let _ = Command::new("sysctl")
        .args(["-w", kv])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn nft_dump_table() -> String {
    Command::new("nft")
        .args(["list", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing) AND END. Best-effort: every failure is "nothing to clean".
fn clean_shared_infra() {
    for _ in 0..64 {
        let ok = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            break;
        }
    }
    ip_quiet(&["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Tear down the per-test netns + veth pair + the lo-bound mesh backend addr. The
/// shared `overdrive-mtls` infra is handled by `clean_shared_infra`.
fn teardown_topology() {
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
}

/// Stand up the netns + veth pair + addresses + host routing hygiene EXACTLY as the
/// increment-b egress spike does, plus the lo-bound mesh backend the OUTBOUND dial
/// targets.
fn setup_topology() {
    teardown_topology();

    ip(&["netns", "add", NS_W]);
    ip(&["link", "add", VETH_W, "type", "veth", "peer", "name", VETH_H]);
    ip(&["link", "set", VETH_W, "netns", NS_W]);

    // Host side: address + up.
    ip(&["addr", "add", &format!("{HOST_GW}/{SUBNET_LEN}"), "dev", VETH_H]);
    ip(&["link", "set", VETH_H, "up"]);

    // Workload side (inside netns): lo up + address + up + default route.
    ip(&["netns", "exec", NS_W, "ip", "link", "set", "lo", "up"]);
    ip(&[
        "netns",
        "exec",
        NS_W,
        "ip",
        "addr",
        "add",
        &format!("{WL_ADDR}/{SUBNET_LEN}"),
        "dev",
        VETH_W,
    ]);
    ip(&["netns", "exec", NS_W, "ip", "link", "set", VETH_W, "up"]);
    ip(&["netns", "exec", NS_W, "ip", "route", "add", "default", "via", HOST_GW]);

    // The OUTBOUND dial target lives on host lo (the host binds+listens on it; the
    // workload routes to it via the gateway).
    ip(&["addr", "add", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);

    // Host-side routing hygiene (NOT a TPROXY concession; spike § Edge cases):
    // forwarding + rp_filter relaxation so the asymmetric ingress is not dropped.
    sysctl_w("net.ipv4.ip_forward=1");
    sysctl_w(&format!("net.ipv4.conf.{VETH_H}.rp_filter=0"));
    sysctl_w("net.ipv4.conf.all.rp_filter=0");
    sysctl_w("net.ipv4.conf.lo.rp_filter=0");

    // bpf.md Rule 2 / spike: disable TX-checksum-offload on the host veth.
    let _ = Command::new("ethtool")
        .args(["-K", VETH_H, "tx", "off"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ============================================================================
// Fresh focused PKI (re-authored — replicates the dataplane `mtls_pki.rs` reference
// + the 05-01 walking-skeleton: a real root → intermediate → leaf chain)
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
    /// The OUTBOUND client SVID (workload-as-client; the agent presents on leg-B).
    client_leaf: Leaf,
    /// The OUTBOUND real mesh peer leaf: a SERVER cert with a DNS SAN matching the
    /// fixed leg-B SNI (`peer.overdrive.local`, per mtls/outbound.rs) so the agent's
    /// leg-B client handshake verifies the mesh server's cert.
    peer_leaf: Leaf,
    client_alloc: AllocationId,
}

impl TestPki {
    /// The DNS SAN the OUTBOUND mesh peer presents (matches the FIXED SNI the
    /// adapter's leg-B client handshake uses in `mtls::outbound::client_handshake` —
    /// `peer.overdrive.local`).
    const PEER_SNI: &'static str = "peer.overdrive.local";

    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-mtls-05-03-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-mtls-05-03-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let client_leaf = intermediate.mint_leaf(client_spiffe, None, true);
        let peer_leaf = intermediate.mint_leaf(
            "spiffe://overdrive.local/ns/default/sa/peer",
            Some(Self::PEER_SNI),
            false,
        );

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem.clone(),
            intermediate_cert_der: CertificateDer::from(intermediate.cert_der),
            client_leaf,
            peer_leaf,
            client_alloc: AllocationId::new("alloc-asym-client").expect("valid alloc"),
        }
    }

    fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
    }

    fn intermediate_cert_der(&self) -> CertificateDer<'static> {
        self.intermediate_cert_der.clone()
    }

    /// The shared trust bundle: root anchor = the ROOT; intermediate chain material
    /// = the INTERMEDIATE (the agent reads this via `IdentityRead`).
    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
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

/// The agent's held-identity store — the ONLY holder of SVID material (workloads
/// hold nothing; the agent reads through THIS `IdentityRead` port and NEVER mints,
/// #26 is a reader). `None` is explicit absence.
struct HeldIdentities {
    svids: BTreeMap<AllocationId, SvidMaterial>,
    bundle: TrustBundle,
}

impl IdentityRead for HeldIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.svids.get(alloc).cloned()
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

fn held_identities(pki: &TestPki) -> HeldIdentities {
    let mut svids = BTreeMap::new();
    svids.insert(pki.client_alloc.clone(), pki.client_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The `AllocationSpec` the OUTBOUND production `start_alloc` consumes: keyed on the
/// CLIENT alloc id (so production's `enforce` selects the held client SVID for the
/// leg-B handshake) with `host_veth = Some(VETH_H)` (the channel the action-shim C3
/// provision seam sets in production — drives the egress nft-TPROXY install matching
/// `iifname VETH_H`).
fn build_client_spec(pki: &TestPki, host_veth: Option<String>) -> AllocationSpec {
    AllocationSpec {
        alloc: pki.client_alloc.clone(),
        identity: pki.client_leaf.spiffe.clone(),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth,
    }
}

// ============================================================================
// Scriptable resolve double (replicates SimMtlsResolve's role — maps a fixed
// orig_dst → MtlsResolution arm so the OUTBOUND accept loop drives the Mesh arm)
// ============================================================================

/// A scripted [`MtlsResolve`]: each `orig_dst` maps to a pre-programmed
/// [`MtlsResolution`] arm. `Mesh` carries the RESOLVED backend addr (the agent's
/// leg-B dial target — the real mesh mTLS server). `expected_svid` is `None` (v1
/// authn-only). An unscripted addr resolves `NonMesh` (the conservative pass-through
/// default).
struct ScriptedResolve {
    table: BTreeMap<SocketAddrV4, MtlsResolution>,
}

impl ScriptedResolve {
    fn new(table: BTreeMap<SocketAddrV4, MtlsResolution>) -> Self {
        Self { table }
    }
}

#[async_trait]
impl MtlsResolve for ScriptedResolve {
    async fn probe(&self) -> Result<(), MtlsResolveError> {
        Ok(())
    }

    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution, MtlsResolveError> {
        Ok(self.table.get(&orig_dst).cloned().unwrap_or(MtlsResolution::NonMesh))
    }
}

// ============================================================================
// 0x17 wire scan (re-authored — replicates the dataplane `traffic.rs` technique:
// AF_PACKET capture on `lo`, walk TLS record framing, count 0x17 app-data records
// per direction, scan for cleartext markers)
// ============================================================================

const TLS_CONTENT_TYPE_APPLICATION_DATA: u8 = 0x17;
const TLS_LEGACY_RECORD_VERSION_TLS12: [u8; 2] = [0x03, 0x03];
const TLS_LEGACY_RECORD_VERSION_TLS10: [u8; 2] = [0x03, 0x01];
const TLS_RECORD_HEADER_LEN: usize = 5;
const ETH_HDR_LEN: usize = 14;
const IPV4_HDR_LEN: usize = 20;
const ETH_P_ALL: std::os::raw::c_int = 0x0003;

fn is_tls_record_version(version: [u8; 2]) -> bool {
    version == TLS_LEGACY_RECORD_VERSION_TLS12 || version == TLS_LEGACY_RECORD_VERSION_TLS10
}

/// The result of scanning a captured wire on `wire_port`: how many genuine `0x17`
/// application_data records crossed in each direction, and how many times EITHER
/// cleartext marker appeared (MUST be 0 on the encrypted leg-B wire).
#[derive(Debug, Clone, Copy, Default)]
struct WireScan {
    records_to_wire_port: u64,
    records_from_wire_port: u64,
    plaintext_marker_hits: u64,
}

impl WireScan {
    /// 0x17 records present in EITHER direction.
    fn has_app_data(&self) -> bool {
        self.records_to_wire_port > 0 || self.records_from_wire_port > 0
    }
}

/// A live AF_PACKET/SOCK_RAW capture on `iface` that records every frame into a
/// buffer on a background thread until `stop_and_scan`. Filtered (at scan time) to
/// TCP frames touching `wire_port` (as src OR dst).
struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    wire_port: u16,
}

impl WireCapture {
    fn start(iface: &str, wire_port: u16) -> Self {
        let ifindex = if_nametoindex(iface).expect("wire-capture: if_nametoindex");
        // SAFETY: AF_PACKET / SOCK_RAW socket on the bound iface.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32) };
        assert!(fd >= 0, "wire-capture: socket: {}", std::io::Error::last_os_error());

        let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = ifindex as i32;
        // SAFETY: bind an AF_PACKET socket to the resolved ifindex.
        let rc = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&sll).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert!(rc == 0, "wire-capture: bind {iface}: {}", std::io::Error::last_os_error());
        // SAFETY: fcntl on our own fd; non-blocking so the loop can poll `stop`.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || -> Vec<Vec<u8>> {
            let mut frames: Vec<Vec<u8>> = Vec::new();
            let mut buf = vec![0u8; 65536];
            while !stop_thread.load(Ordering::SeqCst) {
                // SAFETY: recv into our owned buffer on the bound AF_PACKET fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            // Final drain so records written right before `stop` are not lost.
            loop {
                // SAFETY: same bounded recv on our fd.
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                if n > 0 {
                    frames.push(buf[..n as usize].to_vec());
                } else {
                    break;
                }
            }
            // SAFETY: fd created above; close on capture-thread exit.
            unsafe { libc::close(fd) };
            frames
        });
        Self { stop, handle: Some(handle), wire_port }
    }

    fn stop_and_scan(mut self, request_marker: &[u8], response_marker: &[u8]) -> WireScan {
        self.stop.store(true, Ordering::SeqCst);
        let frames = self.handle.take().expect("wire-capture handle").join().expect("capture join");
        scan_frames(&frames, self.wire_port, request_marker, response_marker)
    }
}

fn scan_frames(
    frames: &[Vec<u8>],
    wire_port: u16,
    request_marker: &[u8],
    response_marker: &[u8],
) -> WireScan {
    let mut streams: BTreeMap<(u16, u16), Vec<u8>> = BTreeMap::new();
    for frame in frames {
        let Some((src_port, dst_port, payload)) = parse_tcp_payload(frame) else {
            continue;
        };
        if src_port != wire_port && dst_port != wire_port {
            continue;
        }
        if payload.is_empty() {
            continue;
        }
        streams.entry((src_port, dst_port)).or_default().extend_from_slice(payload);
    }
    let mut records_to_wire_port: u64 = 0;
    let mut records_from_wire_port: u64 = 0;
    let mut plaintext_marker_hits: u64 = 0;
    for (&(src_port, dst_port), stream) in &streams {
        let records = count_tls_app_data_records(stream);
        if dst_port == wire_port {
            records_to_wire_port += records;
        } else if src_port == wire_port {
            records_from_wire_port += records;
        }
        // The leg-B wire (toward/from MESH_BACKEND_PORT) is ENCRYPTED end-to-end, so
        // a cleartext request/response marker on it WOULD be a breach. The DIRECTIONAL
        // 0x17 counts are the load-bearing confidentiality oracle; the marker counter
        // is the belt-and-braces "no plaintext leaked onto the encrypted wire" check.
        plaintext_marker_hits += count_subslices(stream, request_marker);
        plaintext_marker_hits += count_subslices(stream, response_marker);
    }
    WireScan { records_to_wire_port, records_from_wire_port, plaintext_marker_hits }
}

fn parse_tcp_payload(frame: &[u8]) -> Option<(u16, u16, &[u8])> {
    if frame.len() < ETH_HDR_LEN + IPV4_HDR_LEN {
        return None;
    }
    if frame.get(12).copied()? != 0x08 || frame.get(13).copied()? != 0x00 {
        return None;
    }
    let ip = ETH_HDR_LEN;
    let vihl = frame.get(ip).copied()?;
    if vihl >> 4 != 4 {
        return None;
    }
    let ihl = ((vihl & 0x0f) as usize) * 4;
    if ihl < IPV4_HDR_LEN {
        return None;
    }
    if frame.get(ip + 9).copied()? != 0x06 {
        return None; // not TCP
    }
    let tcp = ip + ihl;
    if frame.len() < tcp + 20 {
        return None;
    }
    let src_port = u16::from_be_bytes([frame.get(tcp).copied()?, frame.get(tcp + 1).copied()?]);
    let dst_port = u16::from_be_bytes([frame.get(tcp + 2).copied()?, frame.get(tcp + 3).copied()?]);
    let data_off = ((frame.get(tcp + 12).copied()? >> 4) as usize) * 4;
    if data_off < 20 {
        return None;
    }
    let payload_start = tcp + data_off;
    if payload_start > frame.len() {
        return None;
    }
    Some((src_port, dst_port, &frame[payload_start..]))
}

fn count_tls_app_data_records(stream: &[u8]) -> u64 {
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + TLS_RECORD_HEADER_LEN <= stream.len() {
        let content_type = stream[i];
        let version = [stream[i + 1], stream[i + 2]];
        let length = u16::from_be_bytes([stream[i + 3], stream[i + 4]]) as usize;
        if !is_tls_record_version(version) {
            break;
        }
        if content_type == TLS_CONTENT_TYPE_APPLICATION_DATA {
            count += 1;
        }
        let next = i + TLS_RECORD_HEADER_LEN + length;
        if next <= i {
            break;
        }
        i = next;
    }
    count
}

fn count_subslices(haystack: &[u8], needle: &[u8]) -> u64 {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

fn if_nametoindex(iface: &str) -> std::io::Result<u32> {
    let cstr = std::ffi::CString::new(iface).expect("iface name has no NUL");
    // SAFETY: thin syscall wrapper; pointer not retained past call.
    let idx = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(idx)
}

// ============================================================================
// real mTLS mesh peer — the agent's leg-B dial target (re-authored fresh from the
// 05-01 walking skeleton's spawn_mesh_peer)
// ============================================================================

/// Spawn the OUTBOUND mesh peer: a real rustls TLS-1.3 SERVER on
/// `MESH_BACKEND_IP:MESH_BACKEND_PORT` (host lo) presenting the PEER SVID and
/// REQUIRE+VERIFYing the client SVID chains to the bundle. This is the real backend
/// the agent's leg-B client handshake reaches. Reads `OUTBOUND_REQUEST` byte-exact
/// (decrypted), replies `OUTBOUND_RESPONSE`. Returns a join handle whose `bool`
/// reports the byte-exact request receipt.
fn spawn_mesh_peer(pki: &TestPki) -> std::thread::JoinHandle<bool> {
    let bind = SocketAddrV4::new(MESH_BACKEND_IP.parse().expect("mesh ip"), MESH_BACKEND_PORT);
    let peer_cert = pki.peer_leaf.cert_der.clone();
    let intermediate = pki.intermediate_cert_der();
    let peer_key = pki.peer_leaf.key_der.clone_key();
    let ca_pem = pki.ca_cert_pem().to_string();
    std::thread::spawn(move || mesh_peer_run(bind, peer_cert, intermediate, peer_key, &ca_pem))
}

fn mesh_peer_run(
    bind: SocketAddrV4,
    cert: CertificateDer<'static>,
    intermediate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    ca_pem: &str,
) -> bool {
    use rustls::server::WebPkiClientVerifier;
    let roots = Arc::new(ca_root_store(ca_pem));
    let verifier = match WebPkiClientVerifier::builder(roots).build() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[05-03] mesh peer client verifier: {e}");
            return false;
        }
    };
    // Present [peer_leaf, intermediate] so the agent's root-anchor-only client
    // verifier can build leaf → intermediate → root.
    let mut cfg = match rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![cert, intermediate], key)
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-03] mesh peer server config: {e}");
            return false;
        }
    };
    // Suppress the TLS 1.3 NewSessionTicket: the agent's leg-B is kTLS-RX-armed
    // immediately after the handshake, and a raw kTLS-RX hits EIO on a post-handshake
    // ticket record (mtls/outbound.rs sentinel_peer_recv sets the same
    // `send_tls13_tickets = 0` for exactly this reason). Without this the return
    // splice pump errors on the ticket and the workload sees an EOF with no response.
    cfg.send_tls13_tickets = 0;
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[05-03] mesh peer bind {bind}: {e}");
            return false;
        }
    };
    let (tcp, _peer) = match accept_with_timeout(&listener, Duration::from_secs(12)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[05-03] mesh peer accept: {e}");
            return false;
        }
    };
    tcp.set_nodelay(true).ok();
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut conn = match rustls::ServerConnection::new(Arc::new(cfg)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-03] mesh peer ServerConnection: {e}");
            return false;
        }
    };
    if !drive_server_handshake(&mut conn, &mut tcp) {
        eprintln!("[05-03] mesh peer handshake failed");
        return false;
    }
    // Read the workload's request (decrypted) byte-exact, then reply.
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut buf = vec![0u8; 4096];
    while got.len() < OUTBOUND_REQUEST.len() && Instant::now() < deadline {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    let request_ok = got == OUTBOUND_REQUEST;
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        let _ = tls.write_all(OUTBOUND_RESPONSE).and_then(|()| tls.flush());
    }
    std::thread::sleep(Duration::from_millis(300));
    request_ok
}

// ---- shared TLS + socket helpers (re-authored from the 05-01 skeleton) ----

fn ca_root_store(ca_cert_pem: &str) -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    let mut rd = std::io::BufReader::new(ca_cert_pem.as_bytes());
    for c in rustls_pemfile::certs(&mut rd) {
        roots.add(c.expect("ca cert")).expect("add ca cert");
    }
    roots
}

fn drive_server_handshake(
    conn: &mut rustls::ServerConnection,
    tcp: &mut std::net::TcpStream,
) -> bool {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            if conn.write_tls(tcp).is_err() {
                return false;
            }
        }
        if !conn.is_handshaking() {
            while conn.wants_write() {
                if conn.write_tls(tcp).is_err() {
                    return false;
                }
            }
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

/// Accept one connection within `timeout` by polling a non-blocking accept.
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(std::net::TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let result = loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false).ok();
                break Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no connection within timeout",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => break Err(e),
        }
    };
    listener.set_nonblocking(false).ok();
    result
}

/// Run a `/dev/tcp`-style client INSIDE the workload netns: connect to `dst`, send
/// `request`, read back `want` bytes. Returns the captured process output (stdout =
/// the bytes read back, stderr = `CLIENT-FAIL:...` on any error).
fn run_netns_client(dst: SocketAddrV4, request: &[u8], want: usize) -> std::process::Output {
    let req_literal: String = request.iter().map(|b| format!("\\x{b:02x}")).collect();
    let script = format!(
        "\
import socket,sys
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
s.settimeout(12)
try:
    s.connect(('{ip}',{port}))
    s.sendall(b'{req}')
    got=b''
    while len(got)<{want}:
        b=s.recv(65536)
        if not b: break
        got+=b
    sys.stdout.buffer.write(got)
    sys.stdout.flush()
except Exception as e:
    sys.stderr.write('CLIENT-FAIL:'+str(e))
",
        ip = dst.ip(),
        port = dst.port(),
        req = req_literal,
        want = want,
    );
    Command::new("ip")
        .args(["netns", "exec", NS_W, "python3", "-c", &script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn netns client")
}

// ============================================================================
// THE deliverable scenario (ADR-0071 / ADR-0069 OUTBOUND substrate asymmetry)
// ============================================================================

/// THE OUTBOUND enforce-substrate per-direction asymmetry (re-established FRESH on
/// the Path-A egress nft-TPROXY mechanism). Drives ONE outbound flow through
/// PRODUCTION `start_alloc` → `accept_loop` (getsockname → resolve(Mesh) → the real
/// `HostMtlsEnforcement::enforce`) on the real netns/veth + egress nft-TPROXY
/// topology while a `strace` attaches to the agent's pump threads, then asserts the
/// ADR-0069 directional asymmetry UNCHANGED by Path A: the FORWARD direction
/// (plaintext workload → ciphertext backend, `legF → legB`) is a `write_all` COPY,
/// the RETURN direction (ciphertext backend → plaintext workload, `legB → legF`) is
/// a `splice`. Plus encryption on the leg-B wire and the authn-only boundary
/// (`expected_peer` None — never asserted here because production owns the enforced
/// connection internally; the authn-only discipline is honoured by NOT asserting any
/// intended-peer protection claim).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn outbound_enforce_substrate_forward_copy_return_splice_asymmetry() {
    if !is_root() {
        eprintln!("SKIP outbound_enforce_substrate_forward_copy_return_splice_asymmetry: not root");
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[05-03] uname -r = {kr}");

    // strace must be present (the syscall oracle is load-bearing); its absence is a
    // gate FAILURE, not a skip — the canonical Lima VM ships it.
    assert!(
        Command::new("strace").arg("-V").output().is_ok_and(|o| o.status.success()),
        "strace is required for the outbound-substrate syscall oracle (forward copy / return \
         splice); it is present in the canonical Lima VM — its absence is a gate failure, not a \
         skip"
    );

    // The composition root rustls CryptoProvider (installed once per process, as
    // overdrive-control-plane's serve boot does — a library must not mutate
    // process-global crypto state; the test IS the composition root here).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Cross-process exclusion + clean baseline.
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_topology();

    let pki = TestPki::mint();
    let identity: Arc<dyn IdentityRead> = Arc::new(held_identities(&pki));
    let adapter = Arc::new(HostMtlsEnforcement::new(identity, MtlsLimits::default()));

    // Earned-Trust probe BEFORE any enforce (the wire→probe→use invariant). On the
    // real 6.18/7.0 kernel this MUST pass.
    adapter
        .probe()
        .await
        .expect("Earned-Trust probe must pass on the real kernel before any enforce");

    let mesh_backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().unwrap(), MESH_BACKEND_PORT);

    // The scripted resolve table the PRODUCTION accept_loop consumes:
    // mesh_backend → Mesh(backend.addr = mesh_backend, expected_svid = None). The
    // `expected_svid` None is the authn-only boundary (Q4 / #178) carried into the
    // resolved arm — production enforces with `expected_peer = None`.
    let mut table = BTreeMap::new();
    table.insert(
        mesh_backend,
        MtlsResolution::Mesh(ResolvedBackend { addr: mesh_backend, expected_svid: None }),
    );
    let resolve: Arc<dyn MtlsResolve> = Arc::new(ScriptedResolve::new(table));

    // Build the PRODUCTION worker over the REAL enforce substrate + the injected
    // resolve double, then drive `start_alloc` — this binds the PRODUCTION leg-F,
    // installs the egress rule on VETH_H, and spawns the PRODUCTION outbound
    // accept_loop. `spec.alloc = client_alloc` so production's `enforce` selects the
    // held CLIENT SVID for the leg-B handshake.
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(&adapter) as Arc<dyn MtlsEnforcement>;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::new(SimClock::new()),
    ));
    let spec = build_client_spec(&pki, Some(VETH_H.to_owned()));
    worker.start_alloc(&spec).expect(
        "PRODUCTION start_alloc must bind leg-F + install the egress rule + spawn accept_loop",
    );

    // The PRODUCTION install appended the `iifname VETH_H` egress rule (observable
    // kernel side effect; the worker — not the fixture — installed it).
    let dump = nft_dump_table();
    assert!(
        dump.contains(&format!("iifname \"{VETH_H}\"")) && dump.contains("tproxy to"),
        "start_alloc must install the iifname egress rule in the shared chain, got:\n{dump}"
    );

    // Start the leg-B wire capture (filtered to the mesh-backend port) BEFORE the
    // workload dials so the first leg-B record is on the captured wire (encryption
    // oracle). The leg-B records carry src/dst = mesh_backend_port.
    let outbound_wire = WireCapture::start(LOOPBACK_IFACE, MESH_BACKEND_PORT);
    let mesh_peer = spawn_mesh_peer(&pki);
    // Give the mesh peer a moment to bind before the workload dials / the agent
    // dials leg-B.
    std::thread::sleep(Duration::from_millis(200));

    // Attach strace to THIS test process (and its threads, `-f`) BEFORE the workload
    // dials, so every pump syscall on the steady-state forward COPY + return SPLICE
    // is captured. Trace `splice` (the return decrypt pump signature) +
    // `sendto`/`write` (the forward COPY's write side INTO leg B's kTLS-TX — where
    // the request plaintext appears) + `recvfrom`/`read` (so the forward read off
    // leg F is visible and the splice sources can be isolated). `-s 512 -xx` dumps
    // the read/write buffers so the request plaintext can be located in a `write`/
    // `sendto` buffer (the forward-copy signature).
    let mut syscalls = StraceProbe::attach_self(&["splice", "sendto", "write", "recvfrom", "read"]);

    // Sample the test process's own thread group WHILE the agent's pumps run, so the
    // forward-copy oracle can attribute a marker-carrying write to the AGENT (an
    // in-tgid TID) and EXCLUDE the netns client's identical plaintext send (a
    // separate-process TID). Started before the dial so every pump thread's TID is
    // observed (S1).
    let tid_sampler = TidSampler::start();

    // The workload (inside the netns) dials the mesh backend, sends the request,
    // reads the response. Its egress ingresses vethH → PREROUTING → TPROXY →
    // PRODUCTION leg-F → PRODUCTION accept_loop → getsockname → resolve(Mesh) →
    // enforce. NO test code touches the accept path — production owns it.
    let req = OUTBOUND_REQUEST.to_vec();
    let want_resp = OUTBOUND_RESPONSE.len();
    let mesh_client = std::thread::spawn(move || run_netns_client(mesh_backend, &req, want_resp));

    // Drive the round-trip to completion (the workload reads the mesh server's
    // response byte-exact; the mesh server received the workload's request
    // byte-exact), then collect the strace trace + the leg-B wire scan.
    let client_out = mesh_client.join().expect("outbound mesh client thread");
    let client_read = client_out.stdout.clone();
    let mesh_request_ok = mesh_peer.join().expect("mesh peer thread");
    // Collect the test's thread group BEFORE detaching strace so the pump threads are
    // still captured, then parse the trace with that attribution oracle.
    let test_thread_group = tid_sampler.stop_and_collect();
    let (trace, raw_trace) = syscalls.detach_and_read(&test_thread_group);
    let scan = outbound_wire.stop_and_scan(OUTBOUND_REQUEST, OUTBOUND_RESPONSE);

    eprintln!(
        "[05-03] netns client exit={:?} stdout_len={} stderr={} | mesh_request_ok={}",
        client_out.status.code(),
        client_read.len(),
        String::from_utf8_lossy(&client_out.stderr).trim(),
        mesh_request_ok,
    );
    eprintln!("[05-03] leg-B wire scan = {scan:?}");
    eprintln!(
        "[05-03] test thread group (size {}) = {:?}",
        test_thread_group.len(),
        test_thread_group
    );
    eprintln!("[05-03] strace summary = {}", trace.summary());

    // The round-trip completed through the PRODUCTION accept_loop's Mesh arm — the
    // substrate genuinely ran end-to-end (a wrong getsockname/resolve/enforce would
    // never complete this round-trip). This is the precondition that makes the
    // syscall-asymmetry assertions below meaningful: the pumps actually pumped.
    assert!(
        client_out.status.success(),
        "the netns workload client must exit cleanly (got {:?}, stderr={}) — the substrate must \
         have run for the asymmetry to be observable",
        client_out.status.code(),
        String::from_utf8_lossy(&client_out.stderr).trim()
    );
    assert_eq!(
        client_read,
        OUTBOUND_RESPONSE,
        "the workload must read the mesh server's response byte-exact back over the RETURN splice \
         pump (leg-B kTLS-RX → leg-F) — through the PRODUCTION accept_loop Mesh arm (got {} bytes)",
        client_read.len()
    );
    assert!(
        mesh_request_ok,
        "the mesh server must receive the workload's request byte-exact (decrypted) — it rode the \
         FORWARD write_all COPY pump (leg-F → leg-B kTLS-TX)"
    );

    // Encryption oracle: the leg-B wire shows TLS-1.3 application_data records in
    // BOTH directions and NO cleartext marker. The DIRECTIONAL 0x17 counts are the
    // load-bearing confidentiality proof (a cleartext leg-B would have zero records
    // in at least one direction).
    assert!(
        scan.has_app_data(),
        "the leg-B wire must carry TLS-1.3 0x17 application_data records (encryption), got {scan:?}"
    );
    assert!(
        scan.records_to_wire_port > 0,
        "the request direction (toward the mesh backend) must carry 0x17 records"
    );
    assert!(
        scan.records_from_wire_port > 0,
        "the response direction (from the mesh backend) must carry 0x17 records"
    );
    assert_eq!(
        scan.plaintext_marker_hits, 0,
        "NO cleartext request/response marker may appear on the encrypted leg-B wire"
    );

    // ----------------------------------------------------------------
    // THE asymmetry assertions (ADR-0069, carried forward VERBATIM by Path A).
    // ----------------------------------------------------------------

    // RETURN = zero-copy SPLICE: the agent used `splice` on the leg-B → leg-F return
    // path. At least one `splice(2)` must be traced (the return decrypt pump runs ~1
    // splice per record out of leg-B's kTLS-RX). This is the RETURN half of the
    // asymmetry — the inverse of the inbound deliver's splice.
    assert!(
        trace.splice_calls > 0,
        "ASYMMETRY (return = splice): the RETURN path (ciphertext backend → plaintext workload, \
         leg-B → leg-F) must be a zero-copy splice out of leg-B's kTLS-RX — at least one splice(2) \
         must be traced; strace summary:\n{}",
        trace.summary()
    );
    // S3: PIN the return splice to `legB → legF`. Leg B is a single TX+RX kTLS fd, so
    // the agent's forward-write DESTINATION fd == the return-splice SOURCE fd. A
    // recovered splice source that equals an agent forward-write dst is genuinely the
    // leg-B → leg-F return pump, not an incidental splice elsewhere in the process.
    assert!(
        trace.return_splice_source_is_legb(),
        "ASYMMETRY (return = splice on leg B): at least one traced splice(2) must source from the \
         leg-B kTLS fd (== the agent forward-write destination fd, since leg B is one TX+RX fd). \
         No recovered splice source matched an agent forward-write dst, so the splice cannot be \
         pinned to the legB → legF return path. strace summary:\n{}",
        trace.summary()
    );

    // FORWARD = write_all COPY: the request plaintext rode a `read(legF) →
    // write_all(legB)` COPY, so the request marker MUST appear in a traced
    // `write(2)`/`sendto(2)` buffer INTO leg B (the kTLS-TX leg) issued BY THE AGENT
    // (a thread of THIS process). The kernel tls_sw_sendmsg encrypts on write; the
    // marker in a userspace write buffer is the copy-through-userspace signature. The
    // AGENT-attribution (thread-group filter) is what makes this load-bearing: the
    // netns client also sends the same plaintext, but from a separate process —
    // excluded.
    assert!(
        trace.request_forwarded_through_io_copy,
        "ASYMMETRY (forward = write_all COPY): the FORWARD path (plaintext workload → ciphertext \
         backend, leg-F → leg-B) must COPY the request through a userspace write_all into leg-B's \
         kTLS-TX — the request plaintext marker MUST appear in a traced write(2)/sendto(2) buffer \
         issued by a THREAD OF THE TEST PROCESS (the agent's forward pump). It did NOT (agent \
         marker writes = {}), which means the forward rode a splice (the inbound shape) instead of \
         the outbound copy. strace summary:\n{}",
        trace.agent_marker_writes,
        trace.summary()
    );

    // ----------------------------------------------------------------
    // FALSIFICATION of the FORWARD oracle (the load-bearing S1 re-validation).
    //
    // The prior cycle's "invert assert!(flag)" litmus only proved the flag toggles —
    // NOT that the AGENT sets it (the netns client's plaintext send sets it too). The
    // genuine falsification HOLDS the netns client's plaintext send CONSTANT and shows
    // the oracle now tracks the AGENT:
    //
    //   (a) the netns client's marker-carrying sendto DOES exist in the trace — it is
    //       captured under `strace -f` and is the EXCLUDED population; and
    //   (b) re-running the SAME parse with the test thread group REMOVED (i.e. as if
    //       the agent's pump threads were not in the test process) drops the
    //       agent-attributed forward-copy count to ZERO — while the client's
    //       (excluded) marker send still exists. So it is the AGENT's write, not the
    //       client's, that flips the flag.
    //
    // (a) is satisfied because the round-trip completed (the client necessarily sent
    // the request plaintext); (b) is demonstrated by re-parsing against the EMPTY
    // thread group: with no TID in the set, every marker write is excluded and the
    // agent count is zero.
    let empty_tg: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
    let agent_excluded = TraceFindings::parse(&raw_trace, &empty_tg);
    assert_eq!(
        agent_excluded.agent_marker_writes,
        0,
        "FALSIFICATION (forward oracle tracks the AGENT): with the test thread group REMOVED, the \
         agent-attributed forward-copy count MUST drop to zero — proving the live flag was set by \
         an in-tgid (agent) TID, not by the netns client. Got {} agent marker writes against the \
         empty thread group. summary:\n{}",
        agent_excluded.agent_marker_writes,
        agent_excluded.summary()
    );
    assert!(
        agent_excluded.excluded_marker_writes > 0,
        "FALSIFICATION (the netns client's plaintext send is held CONSTANT and PRESENT): the \
         workload client's own marker-carrying sendto MUST exist in the trace (it sent the request \
         plaintext) — yet it is EXCLUDED from the forward oracle. Got {} excluded marker writes; if \
         zero, the client's send was not captured and the held-constant premise of the \
         falsification is unmet. summary:\n{}",
        agent_excluded.excluded_marker_writes,
        agent_excluded.summary()
    );
    eprintln!(
        "[05-03] FALSIFICATION OK: forward oracle tracks the AGENT — live agent_marker_writes={} \
         (flag set by an in-tgid TID); with the thread group removed agent_marker_writes drops to \
         0 while the client's excluded marker send persists (excluded_marker_writes={}). The \
         client's identical plaintext send canNOT satisfy the oracle.",
        trace.agent_marker_writes, agent_excluded.excluded_marker_writes
    );

    eprintln!(
        "[05-03] VERDICT: WORKS — OUTBOUND enforce-substrate per-direction asymmetry validated on \
         kernel {kr}: FORWARD (workload → backend, leg-F → leg-B) is a write_all COPY (request \
         plaintext seen in a write/sendto into the kTLS-TX leg, ATTRIBUTED TO THE AGENT's pump \
         thread — the netns client's identical send is excluded), RETURN (backend → workload, \
         leg-B → leg-F) is a splice pinned to the leg-B source fd (>=1 splice(2) out of leg-B's \
         kTLS-RX). Encryption asserted (0x17 both directions, no cleartext on the leg-B wire). \
         Authn-only honoured (expected_svid None on the resolved arm; no intended-peer protection \
         claim, #178)."
    );

    // Teardown: drop the production outbound intercept (removes the egress rule),
    // then scrub the shared infra + topology so a re-run reproduces.
    worker.stop_alloc(&pki.client_alloc);
    teardown_topology();
    clean_shared_infra();
}

// =====================================================================
// strace syscall oracle — attach `strace -f -p <self>` to the running test process
// so the agent's own pump threads' syscalls are captured, then parse the trace for
// the OUTBOUND substrate asymmetry (forward `write_all` COPY of the request into a
// kTLS-TX leg; return zero-copy `splice` out of the kTLS-RX leg).
// =====================================================================

/// A live `strace` attached to this test process (and its threads). Captures the raw
/// syscall log to a temp file; `detach_and_read` stops it and parses.
struct StraceProbe {
    child: Option<Child>,
    out_path: std::path::PathBuf,
}

impl StraceProbe {
    /// Attach `strace -f -p <self_pid>` filtered to `syscalls`, dumping read/write
    /// buffers (`-s 512 -xx`) so the request plaintext can be located in a
    /// `write`/`sendto` buffer (the forward-copy signature). Blocks briefly until
    /// strace has attached (so the pump syscalls that follow are captured).
    fn attach_self(syscalls: &[&str]) -> Self {
        let pid = std::process::id();
        let out_path = std::env::temp_dir().join(format!("mtls-outbound-strace-{pid}.log"));
        let _ = std::fs::remove_file(&out_path);
        let trace_arg = format!("trace={}", syscalls.join(","));
        let child = Command::new("strace")
            .args(["-f", "-q", "-qq"])
            .args(["-e", &trace_arg])
            .args(["-s", "512", "-xx"])
            .args(["-o", out_path.to_str().expect("utf8 path")])
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn strace -p self");
        // Give strace a moment to attach to every thread before the pumps spawn; a
        // few hundred ms is ample on the Lima VM.
        std::thread::sleep(Duration::from_millis(400));
        Self { child: Some(child), out_path }
    }

    /// Stop strace (SIGTERM → it detaches cleanly and flushes the log), read the
    /// captured trace, and parse it for the substrate asymmetry evidence.
    ///
    /// `test_thread_group` is the union of the test process's own thread TIDs sampled
    /// WHILE the pumps were live — it is what attributes a marker-carrying write to
    /// the agent vs the netns client (S1). See `TidSampler`.
    /// Returns `(findings, raw_trace)`. The raw trace is returned so the caller can
    /// RE-PARSE it against a different thread group for the S1 falsification (proving
    /// the live flag was set by an in-tgid agent TID, not the netns client) without
    /// re-reading the on-disk file (which `Drop` removes).
    fn detach_and_read(
        &mut self,
        test_thread_group: &std::collections::BTreeSet<i32>,
    ) -> (TraceFindings, String) {
        // Let the steady-state round-trip's last records flush, then detach.
        std::thread::sleep(Duration::from_millis(300));
        if let Some(mut child) = self.child.take() {
            // SIGTERM makes strace detach (PTRACE_DETACH) and flush its output file.
            let pid = child.id();
            let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
            let _ = child.wait();
        }
        // strace flushes on detach; a brief settle covers the file write.
        std::thread::sleep(Duration::from_millis(150));
        let raw = std::fs::read_to_string(&self.out_path).unwrap_or_default();
        // Diagnostic dump of the agent's splice lines so a return-mechanism mismatch
        // is debuggable from the captured nextest output.
        for line in raw.lines() {
            let (_tid, body) = split_strace_tid_prefix(line);
            if body.starts_with("splice(") {
                let head: String = body.chars().take(80).collect();
                eprintln!("STRACE: {head}");
            }
        }
        let findings = TraceFindings::parse(&raw, test_thread_group);
        (findings, raw)
    }
}

/// Background sampler that unions the test process's own thread TIDs
/// (`/proc/self/task`) on a tight loop WHILE the agent's pumps are live, so even a
/// short-lived pump thread that has already exited by `detach_and_read` is captured.
/// The agent's `std::thread::spawn` pump threads are CLONE_THREAD threads of the test
/// process — their TIDs land in this set; the netns `python3` client is a separate
/// process whose TID never does. This set is the attribution oracle for the forward
/// copy (S1): a marker-carrying write from a TID NOT in this set is the workload's
/// own send, excluded.
struct TidSampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<std::collections::BTreeSet<i32>>>,
}

impl TidSampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || -> std::collections::BTreeSet<i32> {
            let mut tids: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
            // Always include the sampler's own snapshot at least once, even if the
            // round-trip is fast.
            loop {
                if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
                    for e in entries.flatten() {
                        if let Some(t) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok())
                        {
                            tids.insert(t);
                        }
                    }
                }
                if stop_thread.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            tids
        });
        Self { stop, handle: Some(handle) }
    }

    /// Stop sampling and return the union of every TID observed in the test's thread
    /// group while the pumps ran.
    fn stop_and_collect(mut self) -> std::collections::BTreeSet<i32> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().expect("tid-sampler handle").join().expect("tid-sampler join")
    }
}

impl Drop for StraceProbe {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_file(&self.out_path);
    }
}

/// The OUTBOUND substrate-mechanism evidence parsed from the strace log.
struct TraceFindings {
    /// `splice(2)` was used (the RETURN zero-copy decrypt pump, leg-B → leg-F).
    splice_calls: usize,
    /// The set of recovered `splice(2)` SOURCE fds (the return pump splices OUT of
    /// leg-B's kTLS-RX). Used to PIN the return-half assertion to a real leg-B fd
    /// (S3) rather than just "some splice happened".
    splice_src_fds: std::collections::BTreeSet<i32>,
    /// The request plaintext appeared in a traced `write(2)`/`sendto(2)` buffer INTO
    /// leg B (the kTLS-TX leg) issued BY A THREAD OF THE TEST PROCESS — the AGENT's
    /// forward pump. MUST be true (the FORWARD is a `read → write_all` COPY; the
    /// copied request surfaces in the agent's own write buffer). The thread-group
    /// filter is what makes this attribute to the agent and NOT to the netns client.
    request_forwarded_through_io_copy: bool,
    /// The count of marker-carrying writes attributed to the AGENT (a TID in the test
    /// thread group). ≥1 is the positive forward-copy signal.
    agent_marker_writes: usize,
    /// The count of marker-carrying writes attributed to a NON-agent TID (the netns
    /// workload client's own `s.sendall(request)`, captured under `strace -f`). This
    /// is the EXCLUDED population — it exists in the trace but does NOT flip the
    /// forward oracle. Tracked so the falsification can prove the filter works: the
    /// client's send is present yet excluded, and it is the agent's write that flips
    /// the flag.
    excluded_marker_writes: usize,
    /// The DESTINATION fds of the agent's marker-carrying forward writes (leg B, the
    /// kTLS-TX leg). Leg B is a SINGLE kTLS fd (TX+RX armed), so this is the SAME fd
    /// the return pump splices OUT of — used to PIN the return-splice source to leg B
    /// (S3): a return splice whose source is one of these fds is genuinely
    /// `legB → legF`, not an incidental splice elsewhere.
    agent_forward_write_dst_fds: std::collections::BTreeSet<i32>,
    write_calls: usize,
    read_calls: usize,
}

impl TraceFindings {
    /// A distinctive interior substring of the OUTBOUND request. Because the FORWARD
    /// is a userspace COPY into leg-B's kTLS-TX, this plaintext appears in a
    /// `write`/`sendto` buffer off the agent's forward pump (the forward is a copy,
    /// not a splice). Derived as a real sub-slice of `OUTBOUND_REQUEST` (S4: a
    /// `debug_assert!` pins it as an actual substring so silent drift of either the
    /// request or the marker cannot go unnoticed).
    fn request_marker() -> &'static [u8] {
        // The interior bytes after the `OVERDRIVE_0503_OUTBOUND_REQUEST_` prefix
        // (32 bytes) through end — a real sub-slice of OUTBOUND_REQUEST
        // (`forward_copy_marker_..._writeall`).
        let marker = &OUTBOUND_REQUEST[32..];
        debug_assert!(
            OUTBOUND_REQUEST.windows(marker.len()).any(|w| w == marker),
            "request_marker MUST be an actual sub-slice of OUTBOUND_REQUEST (S4 drift guard)"
        );
        marker
    }

    /// Parse the strace log, attributing each marker-carrying write to the AGENT (a
    /// TID in `test_thread_group`) or to the excluded netns client (any other TID).
    ///
    /// `test_thread_group` is the union of `/proc/self/task` TIDs sampled WHILE the
    /// agent's pumps were live — the agent's `std::thread::spawn` pump threads are
    /// CLONE_THREAD threads of the test process, so their TIDs are in this set; the
    /// netns `python3` client is a separate process whose TID is not.
    fn parse(raw: &str, test_thread_group: &std::collections::BTreeSet<i32>) -> Self {
        let mut splice_calls = 0usize;
        let mut write_calls = 0usize;
        let mut read_calls = 0usize;
        let mut agent_marker_writes = 0usize;
        let mut excluded_marker_writes = 0usize;
        let mut agent_forward_write_dst_fds: std::collections::BTreeSet<i32> =
            std::collections::BTreeSet::new();

        // `-xx` renders buffers as `\xHH\xHH...`; convert the marker to that hex form
        // so a substring match against the raw line finds the plaintext regardless of
        // where strace truncated the buffer or split it across records.
        let req_hex = to_strace_hex(Self::request_marker());

        // The agent's pumps' splice SOURCE fds — `splice(SRC, NULL, DST, NULL, len,
        // flags)`. Leg B is a SINGLE kTLS fd (TX+RX armed on the same fd,
        // mtls/outbound.rs:111): it is the return-`splice` SOURCE and ALSO the
        // forward-`write_all` DESTINATION. Collecting splice sources serves two ends:
        // (a) S3 — PIN the return-half assertion to a real recovered leg-B source fd;
        // (b) the forward-copy parse-order invariant below.
        let mut splice_src_fds: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();

        for line in raw.lines() {
            // strace `-f` prefixes each line with the traced thread's TID then a
            // space: `<tid> syscall(args) = ret`, with blocking calls split as
            // `... <unfinished ...>` / `<... syscall resumed> ...`. Recover the TID
            // (for thread-group attribution) AND the body (syscall + args). Classify
            // by the leading syscall-name token.
            //
            // The agent's FORWARD pump COPIES the request via `read(legF) →
            // write_all(legB)`; leg B is kTLS-TX-armed, so the request plaintext
            // surfaces in a `write`/`sendto(legB, <request-plaintext>)` buffer — the
            // copy-through-userspace SIGNATURE of the forward direction. The RETURN
            // pump is `splice` out of leg-B's kTLS-RX. The netns client ALSO sends the
            // request plaintext, but from a non-agent TID — excluded below.
            let (tid, body) = split_strace_tid_prefix(line);
            let is_resume = body.starts_with("<...");
            let names = |n: &str| body.starts_with(n) || (is_resume && body.contains(n));
            let carries_req = body.contains(&req_hex);

            if names("splice(") {
                splice_calls += 1;
                if let Some(src) = splice_source_fd(body) {
                    splice_src_fds.insert(src);
                }
            } else if names("sendto(") || names("write(") {
                write_calls += 1;
                if carries_req {
                    // ATTRIBUTION (S1): a marker-carrying write counts as the FORWARD
                    // COPY only when its owning TID belongs to the TEST process's
                    // thread group (the agent's pump threads). The netns workload
                    // client sends the same plaintext from a SEPARATE process whose
                    // TID is not in the set — that send is EXCLUDED, so it cannot
                    // satisfy the forward oracle. This is the structural defense
                    // against the netns-client confound: the oracle now tracks the
                    // AGENT, not whoever happens to put the marker on the wire.
                    //
                    // (Parse-order note, S2: leg B is a single kTLS fd that is both
                    // the forward-write dst AND the return-splice source, so in the
                    // FULL trace leg B *is* a splice source; the forward write into it
                    // is parsed BEFORE any return splice inserts leg B into
                    // `splice_src_fds` — causally the request is forwarded before the
                    // response returns. We therefore do NOT gate the forward write on
                    // "non-splice-source fd"; the thread-group filter is the real and
                    // sufficient isolator. The in-process mesh-peer thread writes
                    // CIPHERTEXT and never carries the plaintext marker, so it cannot
                    // false-positive here regardless.)
                    match tid {
                        Some(t) if test_thread_group.contains(&t) => {
                            agent_marker_writes += 1;
                            if let Some(fd) = syscall_fd(body) {
                                agent_forward_write_dst_fds.insert(fd);
                            }
                        }
                        _ => excluded_marker_writes += 1,
                    }
                }
            } else if names("recvfrom(") || names("read(") {
                read_calls += 1;
            }
        }

        Self {
            splice_calls,
            splice_src_fds,
            request_forwarded_through_io_copy: agent_marker_writes > 0,
            agent_marker_writes,
            excluded_marker_writes,
            agent_forward_write_dst_fds,
            write_calls,
            read_calls,
        }
    }

    /// True iff ≥1 recovered `splice` SOURCE fd is a leg-B fd the agent's forward pump
    /// wrote the request into (leg B is a single TX+RX kTLS fd, so forward-write-dst
    /// == return-splice-source). PINS the return half to `legB → legF` (S3) rather
    /// than admitting any incidental splice. `None`-safe: empty when neither set was
    /// populated.
    fn return_splice_source_is_legb(&self) -> bool {
        self.splice_src_fds.intersection(&self.agent_forward_write_dst_fds).next().is_some()
    }

    fn summary(&self) -> String {
        format!(
            "splice={} splice_srcs={:?} fwd_write_dsts={:?} write={} read={} \
             agent_marker_writes={} excluded_marker_writes={} request_copy_seen={} \
             return_splice_src_is_legb={}",
            self.splice_calls,
            self.splice_src_fds,
            self.agent_forward_write_dst_fds,
            self.write_calls,
            self.read_calls,
            self.agent_marker_writes,
            self.excluded_marker_writes,
            self.request_forwarded_through_io_copy,
            self.return_splice_source_is_legb(),
        )
    }
}

/// Split strace's leading `<tid> ` prefix (present under `-f`) into `(Some(tid),
/// body)` where `body` begins at the syscall name. A line with no leading-digit
/// prefix returns `(None, trimmed_line)`. The TID is the traced THREAD's id — for a
/// CLONE_THREAD thread it equals neither the leader pid nor a child process pid, so
/// it cleanly distinguishes the agent's in-process pump threads (members of
/// `/proc/self/task`) from the netns client's separate-process descendant.
fn split_strace_tid_prefix(line: &str) -> (Option<i32>, &str) {
    let trimmed = line.trim_start();
    let digits_end = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());
    if digits_end == 0 {
        return (None, trimmed);
    }
    let tid = trimmed[..digits_end].parse::<i32>().ok();
    let rest = trimmed[digits_end..].trim_start();
    (tid, rest)
}

/// The first-argument fd of a `syscall(FD, ...)` line (e.g. `write(26, ...)` →
/// `Some(26)`). `body` has already had its PID prefix stripped. `None` if the args do
/// not begin with an integer (e.g. a `<... resumed>` fragment that omits the fd).
fn syscall_fd(body: &str) -> Option<i32> {
    let open = body.find('(')?;
    let after = &body[open + 1..];
    let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
    after[..end].parse::<i32>().ok()
}

/// The source fd of a `splice(SRC, NULL, DST, NULL, len, flags)` line — the FIRST
/// positional argument. `body` has its PID prefix stripped. `None` on a `<...
/// resumed>` fragment or a malformed line.
fn splice_source_fd(body: &str) -> Option<i32> {
    let open = body.find("splice(")? + "splice(".len();
    let args = &body[open..];
    // splice args are comma-separated: SRC, off_in, DST, off_out, len, flags
    let src = args.split(',').next()?.trim();
    let end = src.find(|c: char| !c.is_ascii_digit()).unwrap_or(src.len());
    src.get(..end)?.parse::<i32>().ok()
}

/// Render `bytes` as the `\xHH\xHH...` hex form strace `-xx` emits, so a marker can
/// be substring-matched against a traced buffer line.
fn to_strace_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 4);
    for b in bytes {
        let _ = write!(s, "\\x{b:02x}");
    }
    s
}
