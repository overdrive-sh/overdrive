//! Tier-3 COMPOSED BIDIRECTIONAL WALKING SKELETON (step 05-01) — THE walking
//! skeleton for ADR-0071 § Enforcement Tier-3 obligations (b)+(c)+(d):
//! getsockname → resolve → enforce mTLS in BOTH directions, on ONE Path-A
//! mechanism (per-workload netns+veth + nft-TPROXY + getsockname), with a real
//! 0x17 TLS-1.3 wire capture and no RST.
//!
//! This DRIVES the PRODUCTION composition root that landed in 02/03/04 — it
//! authors NO new production code (CLAUDE.md § "Implement to the design"):
//!
//!   - OUTBOUND (driven through PRODUCTION `MtlsInterceptWorker::start_alloc` →
//!     the spawned `accept_loop` — the SHIPPING code, NOT a hand-rolled replica):
//!     `start_alloc(spec{host_veth=VETH_H})` binds the production leg-F (the
//!     IP_TRANSPARENT bind under test), installs the `iifname <host_veth>` egress
//!     nft-TPROXY rule, and spawns the production outbound accept loop. The netns
//!     workload's `connect(target)` ingresses vethH → PREROUTING → TPROXY →
//!     PRODUCTION leg-F → PRODUCTION `accept_loop`: `getsockname` orig-dst
//!     recovery → the injected `MtlsResolve` double → the 3-arm branch
//!     (`Mesh`→`enforce` real mTLS / `NonMesh`→production cleartext pass-through /
//!     `MeshUnreachable`→production fail-closed drop). ALL THREE Q3 arms are
//!     driven END-TO-END through the production accept loop. The ONLY injected
//!     double is the resolve port (a `ScriptedResolve` — legitimate, it is an
//!     injected port; the production resolve index 01-03 is its own DST's job).
//!     The enforce substrate is the REAL `HostMtlsEnforcement` (ADR-0069,
//!     UNCHANGED 4-method port).
//!   - INBOUND (driven through the SPAWNED PRODUCTION inbound `accept_loop`,
//!     symmetric with the outbound half): `start_alloc(&server_spec)` binds the
//!     PRODUCTION leg-C (IP_TRANSPARENT) at an ephemeral port and spawns the
//!     production inbound accept loop over it (`AcceptLeg::Inbound`:
//!     `accept_inbound_leg → spawn_enforce` → the real `HostMtlsEnforcement::
//!     enforce`). The test discovers the production-bound leg-C ephemeral addr via
//!     the architect-pinned accessor `MtlsInterceptWorker::leg_c_addr` (D-TME-13)
//!     and installs its OWN `install_inbound_tproxy(virt, leg_c_port)` redirect to
//!     that PRODUCTION leg-C — the production inbound rule is #178-deferred
//!     (`start_alloc` installs none), and #178's production install is *expected*
//!     to reuse this leg-C bound-addr read pending its install site/timing design
//!     (it may read an inline `start_alloc` local rather than this accessor — see
//!     D-TME-13). A client's `connect(virt)` → PREROUTING → TPROXY →
//!     PRODUCTION leg-C → the spawned production inbound `accept_loop`: getsockname
//!     orig-dst recovery → SERVER handshake on leg-C + the SO_MARK-exempt leg-S
//!     dial — 0x17 on the leg-C wire. NO test code accepts or enforces on the
//!     inbound leg; production owns it, exactly as the outbound half does. BOTH
//!     halves (the C1 blocking core + all three Q3 arms outbound; the inbound
//!     round-trip) are driven through the spawned production `accept_loop`.
//!
//! The mTLS substrate (`HostMtlsEnforcement`) is REUSED from `overdrive-dataplane`
//! (a production `[dependencies]` edge of `overdrive-worker`); the egress topology
//! + `KernelStateLock` + scrub-hygiene are REUSED from the sibling
//! `egress_tproxy_capture.rs`; the focused PKI / `HeldIdentities` / 0x17 wire-scan
//! are RE-AUTHORED FRESH here (the dataplane `tests/helpers/` are a reference to
//! replicate, NOT a cross-crate import — a crate's `tests/` tree is not exported;
//! see step 05-03's identical fresh-authoring note in the roadmap).
//!
//! ## Oracles (all observable kernel/wire side effects, ADR-0071 Tier-3 (b/c/d))
//!
//!   O1 (orig-dst recovery, AC2): `getsockname`-recovered orig_dst == the dialed
//!      dst on BOTH legs (outbound leg-F → mesh backend; inbound leg-C → virt).
//!   O2 (encryption on the wire, AC3): an AF_PACKET capture on `lo` shows TLS-1.3
//!      `application_data` records (content-type byte `0x17`) in BOTH directions —
//!      the bytes are ENCRYPTED end-to-end, NOT cleartext (the request/response
//!      markers never appear on the encrypted wire).
//!   O3 (round-trip completes, AC1): both directions complete a byte-exact
//!      application round-trip (the OUTBOUND workload's request reaches the mesh
//!      server and its reply returns; the INBOUND client's request reaches S and
//!      its reply returns).
//!   O4 (no RST post-arm, AC4): NORMAL and TRACED (slow) timing both complete
//!      without a transport RST / corruption (ADR-0069 Slice 00 obligation).
//!   O5 (Q3 enrollment correctness, AC5): all three resolve arms exercised
//!      end-to-end — `Mesh`→enforce mTLS, `NonMesh`→cleartext pass-through,
//!      `MeshUnreachable`→fail-closed (NO silent cleartext to a should-be-mesh
//!      peer).
//!   O6 (F5, AC6): the agent's own leg-B/leg-S dials are NOT re-captured (the
//!      handshakes complete — a re-captured agent dial would recurse and the
//!      handshake would never finish); a WORKLOAD cannot self-exempt (its
//!      SO_MARK-stamped dial is STILL captured — the mark is skb-local and does
//!      not cross the veth/netns boundary).
//!
//! ## Authn-only boundary (AC8, Q4 / #178)
//!
//! `expected_peer`/`expected_svid` stay `None` for every connection. This test
//! asserts (a) 0x17 on the wire, (b) the handshake authenticates the backend's
//! chain to the bundle, (c) the correct backend was dialed — it MUST NOT assert
//! intended-peer "protection". The wrong-but-valid-peer case is NOT called
//! "protected" (it is the #178 upgrade).
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN. A non-root run SKIPs. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run` (a compile-only gate is green
//! even when every fixture refuses at boot). `uname -r` is recorded.
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design
//! (node-global converge-on-boot), so the test scrubs ALL `overdrive-mtls` nft
//! state + the fwmark rule/route + the test netns/veth/lo-addrs at START
//! (tolerate pre-existing) AND END. A cross-PROCESS `flock(2)` lock
//! (`KernelStateLock`, on the SAME path the sibling kernel-touching suites use)
//! serialises the kernel-touching tests — nextest runs each `#[test]` in a
//! separate process, so an in-process lock cannot serialise node-global state.

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
    reason = "Tier-3 composed walking-skeleton test bodies; the O1..O6 oracle bullets in the module docstring are a numbered narrative list; skip messages + evidence go to stderr; failures must panic with informative messages; the libc FFI casts are width conversions on compile-time constants (ETH_P_ALL.to_be() as i32 mirrors traffic.rs); leg F/B/C/S are the ADR-0069 contract vocabulary; the composed bidirectional flow is a single long scenario; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the per-byte \\xNN python-literal fold reads clearer than a write! accumulator in a test fixture; const-fn-ability on test constructors is not load-bearing"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::driver::{AllocationSpec, Resources};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_worker::mtls_intercept::install_inbound_tproxy;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

// ============================================================================
// topology constants (mirror the increment-b egress spike + the inbound recipe)
// ============================================================================

const NS_W: &str = "nsW-bidi0501";
const VETH_W: &str = "vethW-bidi05";
const VETH_H: &str = "vethH-bidi05";
const HOST_GW: &str = "10.99.0.1";
const WL_ADDR: &str = "10.99.0.2";
const SUBNET_LEN: &str = "24";

/// The mesh backend the OUTBOUND workload dials — a host-side lo-bound address it
/// routes to via the gateway, so its egress genuinely INGRESSES vethH and hits
/// PREROUTING (not loopback-to-self inside the netns). This is the dialed
/// `orig_dst` the resolve consumer classifies `Mesh`, and the address the real
/// mesh mTLS server (leg-B's peer) binds.
const MESH_BACKEND_IP: &str = "10.200.0.1";
const MESH_BACKEND_PORT: u16 = 18801;

/// A genuinely NON-mesh destination the workload dials for the `NonMesh`
/// pass-through arm (AC5): a host-lo address that resolves to no mesh backend, so
/// the agent relays it in cleartext, by design.
const NONMESH_IP: &str = "10.200.0.2";
const NONMESH_PORT: u16 = 18802;

/// A should-be-mesh destination the workload dials for the `MeshUnreachable`
/// fail-closed arm (AC5): the resolve consumer classifies it `MeshUnreachable`,
/// so the agent REFUSES — drops leg-F, NO cleartext, NO dial.
const UNREACHABLE_IP: &str = "10.200.0.3";
const UNREACHABLE_PORT: u16 = 18803;

/// The INBOUND server workload's virtual (logical) address — the loopback addr a
/// client dials and the inbound nft-TPROXY rule matches. The agent's leg-S dial
/// (SO_MARK-exempt) reaches the real server S bound here verbatim
/// (`server_dial_addr(orig_dst) == orig_dst`, mtls/inbound.rs).
const INBOUND_VIRT_IP: &str = "127.0.0.91";
const INBOUND_VIRT_PORT: u16 = 18811;

/// `lo` — where every leg's TLS records (leg-B to the lo-bound mesh backend, leg-C
/// to the lo-bound virt, leg-S to the lo-bound server) physically carry their
/// bytes, so the AF_PACKET 0x17 confidentiality oracle captures there.
const LOOPBACK_IFACE: &str = "lo";

/// The OUTBOUND application request the workload sends through leg-F → (mTLS
/// leg-B) → the mesh server. The mesh server must receive it byte-exact as
/// plaintext, and it must NEVER appear on the encrypted leg-B wire.
const OUTBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_OUTBOUND_REQUEST_workload_to_mesh_must_arrive_plaintext_at_backend_S_0501";
/// The OUTBOUND application response the mesh server replies; it rides back over
/// leg-B's kTLS to leg-F and the workload reads it byte-exact.
const OUTBOUND_RESPONSE: &[u8] =
    b"OVERDRIVE_OUTBOUND_RESPONSE_mesh_reply_rides_back_over_legB_ktls_to_workload_0501";
/// The INBOUND application request a client sends toward the virt; the agent
/// decrypts it on leg-C and splices it to server S.
const INBOUND_REQUEST: &[u8] =
    b"OVERDRIVE_INBOUND_REQUEST_client_to_server_must_arrive_plaintext_at_S_0501";
/// The INBOUND application response server S replies; it rides back over leg-C's
/// kTLS-TX to the client byte-exact.
const INBOUND_RESPONSE: &[u8] =
    b"OVERDRIVE_INBOUND_RESPONSE_server_reply_rides_back_over_legC_ktls_to_client_0501";

// ============================================================================
// Cross-process kernel-state exclusion (shared with the sibling suites)
// ============================================================================

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL: every test touching them touches the SAME kernel state.
/// nextest runs each `#[test]` in a SEPARATE PROCESS, so an in-process lock
/// cannot serialise them — an `flock(2)` on the fixed path (shared with
/// `egress_tproxy_capture.rs` / `mtls_intercept_install.rs`) spans processes.
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
// command shims (mirror egress_tproxy_capture.rs)
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

/// Tear down the per-test netns + veth pair + the lo-bound addresses (mesh
/// backend, non-mesh, unreachable, inbound virt). The shared `overdrive-mtls`
/// infra is handled by `clean_shared_infra`.
fn teardown_topology() {
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
    ip_quiet(&["addr", "del", &format!("{NONMESH_IP}/32"), "dev", "lo"]);
    ip_quiet(&["addr", "del", &format!("{UNREACHABLE_IP}/32"), "dev", "lo"]);
    ip_quiet(&["addr", "del", &format!("{INBOUND_VIRT_IP}/32"), "dev", "lo"]);
}

/// Stand up the netns + veth pair + addresses + host routing hygiene EXACTLY as
/// the increment-b egress spike does, plus the lo-bound addresses the OUTBOUND
/// dial targets (mesh / non-mesh / unreachable) and the INBOUND virt live on.
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

    // The OUTBOUND dial targets live on host lo (the host binds+listens on them;
    // the workload routes to them via the gateway). The INBOUND virt + the
    // server S both live on the same lo addr.
    ip(&["addr", "add", &format!("{MESH_BACKEND_IP}/32"), "dev", "lo"]);
    ip(&["addr", "add", &format!("{NONMESH_IP}/32"), "dev", "lo"]);
    ip(&["addr", "add", &format!("{UNREACHABLE_IP}/32"), "dev", "lo"]);
    ip(&["addr", "add", &format!("{INBOUND_VIRT_IP}/32"), "dev", "lo"]);

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
// Fresh focused PKI (re-authored — replicates the dataplane `mtls_pki.rs`
// reference: a real root → intermediate → leaf chain on `rcgen` + `rustls`)
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
    /// The INBOUND server SVID (server-workload; the agent presents on leg-C).
    server_leaf: Leaf,
    /// The OUTBOUND real mesh peer leaf: a SERVER cert with a DNS SAN matching the
    /// fixed leg-B SNI (`peer.overdrive.local`, per mtls/outbound.rs) so the
    /// agent's leg-B client handshake verifies the mesh server's cert.
    peer_leaf: Leaf,
    client_alloc: AllocationId,
    server_alloc: AllocationId,
}

impl TestPki {
    /// The DNS SAN the OUTBOUND mesh peer presents (matches the FIXED SNI the
    /// adapter's leg-B client handshake uses in `mtls::outbound::client_handshake`
    /// — `peer.overdrive.local`).
    const PEER_SNI: &'static str = "peer.overdrive.local";
    /// The DNS SAN the INBOUND server SVID carries (matches the SNI the inbound
    /// client presents toward the virt).
    const SERVER_SNI: &'static str = "server.overdrive.local";

    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-mtls-05-01-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-mtls-05-01-INTERMEDIATE-CA");

        let client_spiffe = "spiffe://overdrive.local/ns/default/sa/client";
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        let client_leaf = intermediate.mint_leaf(client_spiffe, None, true);
        let server_leaf = intermediate.mint_leaf(server_spiffe, Some(Self::SERVER_SNI), false);
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
            server_leaf,
            peer_leaf,
            client_alloc: AllocationId::new("alloc-bidi-client").expect("valid alloc"),
            server_alloc: AllocationId::new("alloc-bidi-server").expect("valid alloc"),
        }
    }

    fn ca_cert_pem(&self) -> &str {
        &self.ca_cert_pem
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

    fn client_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.client_leaf)
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

/// The agent's held-identity store — the ONLY holder of SVID material (workloads
/// hold nothing; the agent reads through THIS `IdentityRead` port and NEVER mints,
/// #26 is a reader). `None` is explicit absence. Re-authored fresh per the
/// `mtls_agent_handshake.rs` reference.
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
    svids.insert(pki.server_alloc.clone(), pki.server_svid_material());
    HeldIdentities { svids, bundle: pki.trust_bundle() }
}

/// The `AllocationSpec` the OUTBOUND production `start_alloc` consumes: keyed on
/// the CLIENT alloc id (so production's `enforce` selects the held client SVID for
/// the leg-B handshake) with `host_veth = Some(VETH_H)` (the channel the
/// action-shim C3 provision seam sets in production, JOIN-6 — drives the egress
/// nft-TPROXY install matching `iifname VETH_H`).
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

/// The `AllocationSpec` the INBOUND production `start_alloc` consumes: keyed on the
/// SERVER alloc id (so production's `enforce` selects the held server SVID for the
/// leg-C SERVER handshake). `host_veth = None`: the inbound nft-TPROXY rule is the
/// test-installed redirect to the production-bound leg-C (the production inbound
/// rule is #178-deferred — `start_alloc` installs none), NOT an egress rule, so no
/// veth match is needed.
fn build_server_spec(pki: &TestPki) -> AllocationSpec {
    AllocationSpec {
        alloc: pki.server_alloc.clone(),
        identity: pki.server_leaf.spiffe.clone(),
        command: "/bin/true".to_owned(),
        args: vec![],
        resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
        probe_descriptors: Vec::new(),
        netns: None,
        host_veth: None,
    }
}

// ============================================================================
// 0x17 wire scan (re-authored — replicates the dataplane `traffic.rs` technique:
// AF_PACKET capture on `lo`, walk TLS record framing, count 0x17 app-data
// records per direction, scan for cleartext markers)
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

/// The result of scanning a captured wire on `wire_port`: how many genuine
/// `0x17` application_data records crossed in each direction, and how many times
/// EITHER cleartext marker appeared (MUST be 0 on the encrypted leg).
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
/// buffer on a background thread until `stop_and_scan`. Filtered (at scan time)
/// to TCP frames touching `wire_port` (as src OR dst).
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
        // `plaintext_marker_hits` is a SECONDARY corroborating signal, NOT the
        // primary confidentiality oracle. The LOAD-BEARING encryption proof is the
        // directional `0x17` counts — `records_to_wire_port > 0` AND
        // `records_from_wire_port > 0` (asserted by the caller): a cleartext leg
        // fails that combination (it has zero TLS records in at least one
        // direction). The marker counter only adds a belt-and-braces "and no
        // request/response plaintext leaked onto the encrypted stream" check.
        //
        // Scoping: count markers ONLY on a TLS-bearing stream (`records > 0`). In
        // the single-`lo` topology the agent↔S leg-S (inbound) is a CLEARTEXT
        // stream that ALSO touches `wire_port` (the agent dials the virt verbatim,
        // `server_dial_addr(orig_dst) == orig_dst`), so its payload legitimately
        // CONTAINS the markers — S is an identity-unaware plaintext workload, by
        // design. The `records > 0` gate exempts that leg-S plaintext stream (the
        // markers ARE its raw payload) and scopes the marker check to the encrypted
        // client-facing leg, where a marker WOULD be a breach. This isolates the
        // SECONDARY check to the same wire the dataplane harness isolates by
        // putting S in a separate netns; the PRIMARY oracle remains the directional
        // 0x17 counts above.
        if records > 0 {
            plaintext_marker_hits += count_subslices(stream, request_marker);
            plaintext_marker_hits += count_subslices(stream, response_marker);
        }
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
// Scriptable resolve double (replicates SimMtlsResolve's role — maps a fixed
// orig_dst → MtlsResolution arm so the OUTBOUND accept loop exercises all three)
// ============================================================================

use async_trait::async_trait;
use overdrive_core::traits::mtls_resolve::{
    MtlsResolution, MtlsResolve, MtlsResolveError, ResolvedBackend,
};

/// A scripted [`MtlsResolve`]: each `orig_dst` maps to a pre-programmed
/// [`MtlsResolution`] arm. `Mesh` carries the RESOLVED backend addr (the agent's
/// leg-B dial target — the real mesh mTLS server). `expected_svid` is `None`
/// (v1 authn-only). An unscripted addr resolves `NonMesh` (the conservative
/// pass-through default).
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
// real mTLS peers — the agent's dial targets (re-authored fresh, the outbound
// counterpart of mtls_roles.rs::InboundServer)
// ============================================================================

/// Spawn the OUTBOUND mesh peer: a real rustls TLS-1.3 SERVER on
/// `MESH_BACKEND_IP:MESH_BACKEND_PORT` (host lo) presenting the PEER SVID and
/// REQUIRE+VERIFYing the client SVID chains to the bundle. This is the real
/// backend the agent's leg-B client handshake reaches. Reads `OUTBOUND_REQUEST`
/// byte-exact (decrypted), replies `OUTBOUND_RESPONSE`. Returns a join handle
/// whose `bool` reports the byte-exact request receipt.
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
            eprintln!("[05-01] mesh peer client verifier: {e}");
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
            eprintln!("[05-01] mesh peer server config: {e}");
            return false;
        }
    };
    // Suppress the TLS 1.3 NewSessionTicket: the agent's leg-B is kTLS-RX-armed
    // immediately after the handshake, and a raw kTLS-RX hits EIO on a
    // post-handshake ticket record (mtls/outbound.rs sentinel_peer_recv sets the
    // same `send_tls13_tickets = 0` for exactly this reason). Without this the
    // return splice pump errors on the ticket and the workload sees an EOF with no
    // response.
    cfg.send_tls13_tickets = 0;
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[05-01] mesh peer bind {bind}: {e}");
            return false;
        }
    };
    let (tcp, _peer) = match accept_with_timeout(&listener, Duration::from_secs(12)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[05-01] mesh peer accept: {e}");
            return false;
        }
    };
    tcp.set_nodelay(true).ok();
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut conn = match rustls::ServerConnection::new(Arc::new(cfg)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-01] mesh peer ServerConnection: {e}");
            return false;
        }
    };
    if !drive_server_handshake(&mut conn, &mut tcp) {
        eprintln!("[05-01] mesh peer handshake failed");
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

/// Spawn the INBOUND server workload S: a PLAINTEXT server on the virt
/// (`INBOUND_VIRT_IP:INBOUND_VIRT_PORT`, host lo) — identity-unaware, holds
/// nothing. The agent's SO_MARK-exempt leg-S dial reaches it (the dialed orig-dst
/// IS the virt; `server_dial_addr(orig_dst) == orig_dst`). Reads the decrypted
/// `INBOUND_REQUEST` byte-exact and replies `INBOUND_RESPONSE`.
fn spawn_inbound_server() -> std::thread::JoinHandle<bool> {
    let bind = SocketAddrV4::new(INBOUND_VIRT_IP.parse().expect("virt ip"), INBOUND_VIRT_PORT);
    std::thread::spawn(move || inbound_server_run(bind))
}

fn inbound_server_run(bind: SocketAddrV4) -> bool {
    let listener = match TcpListener::bind(bind) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[05-01] inbound server bind {bind}: {e}");
            return false;
        }
    };
    let (mut tcp, _peer) = match accept_with_timeout(&listener, Duration::from_secs(12)) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[05-01] inbound server accept (leg-S dial must reach S): {e}");
            return false;
        }
    };
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut buf = vec![0u8; 4096];
    while got.len() < INBOUND_REQUEST.len() && Instant::now() < deadline {
        match tcp.read(&mut buf) {
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
    let request_ok = got == INBOUND_REQUEST;
    let _ = tcp.write_all(INBOUND_RESPONSE).and_then(|()| tcp.flush());
    std::thread::sleep(Duration::from_millis(300));
    request_ok
}

/// What the INBOUND client observed: did it read the server's response byte-exact
/// back over leg-C's kTLS, and the server SPIFFE-id it extracted from the verified
/// leg-C SERVER leaf (proving the agent presented the held server SVID).
struct InboundClientResult {
    received_response_byte_exact: bool,
    observed_rst: bool,
    presented_server_spiffe: Option<overdrive_core::SpiffeId>,
}

/// Spawn the INBOUND client: a real rustls TLS-1.3 client presenting the CLIENT
/// SVID, aimed at the virt (TPROXY-intercepted to the agent's leg-C). Verifies the
/// agent's server cert chains to the CA. Sends `INBOUND_REQUEST` after a delay (so
/// it lands AFTER the agent arms kTLS-RX), reads `INBOUND_RESPONSE` byte-exact.
fn spawn_inbound_client(
    pki: &TestPki,
    send_delay: Duration,
) -> std::thread::JoinHandle<InboundClientResult> {
    let virt = SocketAddrV4::new(INBOUND_VIRT_IP.parse().expect("virt ip"), INBOUND_VIRT_PORT);
    let client_cert = pki.client_leaf.cert_der.clone();
    let intermediate = pki.intermediate_cert_der();
    let client_key = pki.client_leaf.key_der.clone_key();
    let ca_pem = pki.ca_cert_pem().to_string();
    std::thread::spawn(move || {
        inbound_client_run(virt, client_cert, intermediate, client_key, &ca_pem, send_delay)
    })
}

fn inbound_client_run(
    virt: SocketAddrV4,
    cert: CertificateDer<'static>,
    intermediate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    ca_pem: &str,
    send_delay: Duration,
) -> InboundClientResult {
    use rustls::pki_types::ServerName;
    use rustls::{ClientConfig, ClientConnection};

    let fail = || InboundClientResult {
        received_response_byte_exact: false,
        observed_rst: true,
        presented_server_spiffe: None,
    };
    let roots = ca_root_store(ca_pem);
    let cfg = match ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(vec![cert, intermediate], key)
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[05-01] inbound client config: {e}");
            return fail();
        }
    };
    let Ok(tcp) = TcpStream::connect(virt) else {
        eprintln!("[05-01] inbound client connect {virt} failed");
        return fail();
    };
    tcp.set_nodelay(true).ok();
    let sni = ServerName::try_from(TestPki::SERVER_SNI.to_string()).expect("server SNI");
    let mut conn = ClientConnection::new(Arc::new(cfg), sni).expect("inbound ClientConnection");
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
    if !drive_client_handshake(&mut conn, &mut tcp) {
        eprintln!("[05-01] inbound client handshake failed");
        return fail();
    }
    // The handshake verified the agent's server cert chains to the bundle root;
    // extract the SERVER SPIFFE-id from the presented leaf (proves the agent
    // presented the held server SVID — AC3 inbound identity).
    let presented_server_spiffe = peer_presented_leaf_spiffe(conn.peer_certificates());
    std::thread::sleep(send_delay);

    let mut observed_rst = false;
    {
        let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
        if tls.write_all(INBOUND_REQUEST).and_then(|()| tls.flush()).is_err() {
            observed_rst = true;
        }
    }
    let mut got = Vec::new();
    if !observed_rst {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut buf = vec![0u8; 4096];
        while got.len() < INBOUND_RESPONSE.len() && Instant::now() < deadline {
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
    let received_response_byte_exact = got == INBOUND_RESPONSE;
    std::thread::sleep(Duration::from_millis(300));
    InboundClientResult { received_response_byte_exact, observed_rst, presented_server_spiffe }
}

// ---- shared TLS + socket helpers (re-authored from mtls_roles.rs) ----

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

fn drive_server_handshake(conn: &mut rustls::ServerConnection, tcp: &mut TcpStream) -> bool {
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

/// Extract the SPIFFE-id (sole URI SAN) from chain position 0 (the leaf) of a
/// presented cert chain (the verified SERVER leaf the inbound client received).
fn peer_presented_leaf_spiffe(
    certs: Option<&[CertificateDer<'_>]>,
) -> Option<overdrive_core::SpiffeId> {
    use x509_parser::prelude::FromDer as _;
    let leaf = certs?.first()?;
    let (_, parsed) = x509_parser::certificate::X509Certificate::from_der(leaf.as_ref()).ok()?;
    let san = parsed.subject_alternative_name().ok()??;
    let uri = san.value.general_names.iter().find_map(|gn| match gn {
        x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
        _ => None,
    })?;
    uri.parse::<overdrive_core::SpiffeId>().ok()
}

/// Accept one connection within `timeout` by polling a non-blocking accept.
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
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

/// Run a `/dev/tcp` client INSIDE the workload netns: connect to `dst`, send
/// `marker`, read back `want` bytes of echo. Returns the bytes the client read.
/// Optionally stamps `SO_MARK` on the client socket via Python (the self-exempt
/// probe): a workload's SO_MARK is skb-local and does NOT cross the veth/netns
/// boundary, so the host-side rule still captures it.
fn run_netns_client(
    dst: SocketAddrV4,
    request: &[u8],
    want: usize,
    so_mark: Option<u32>,
) -> std::process::Output {
    let req_literal: String = request.iter().map(|b| format!("\\x{b:02x}")).collect();
    let mark_line =
        so_mark.map_or_else(String::new, |m| format!("s.setsockopt(socket.SOL_SOCKET,36,{m})\n"));
    let script = format!(
        "\
import socket,sys
s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)
{mark_line}s.settimeout(10)
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
// the agent — drives the PRODUCTION worker (start_alloc → accept_loop) +
// HostMtlsEnforcement::enforce
// ============================================================================

// ============================================================================
// THE deliverable scenario
// ============================================================================

/// THE composed bidirectional walking skeleton (ADR-0071 Tier-3 (b)+(c)+(d)).
///
/// Drives, through the production worker seams + the real `HostMtlsEnforcement`
/// substrate, BOTH directions end-to-end on the ONE Path-A mechanism, twice (a
/// NORMAL and a TRACED/slow timing pass), and proves O1–O6.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn composed_bidirectional_mtls_completes_no_rst_with_tls13_wire_capture() {
    if !is_root() {
        eprintln!(
            "SKIP composed_bidirectional_mtls_completes_no_rst_with_tls13_wire_capture: not root"
        );
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[05-01] uname -r = {kr}");

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

    // Run BOTH timing regimes (AC4): NORMAL (no inserted delay) and TRACED (a slow
    // inter-write / handshake-window delay) — both must complete without RST.
    for (regime, handshake_delay) in
        [("NORMAL", Duration::ZERO), ("TRACED", Duration::from_millis(250))]
    {
        eprintln!("[05-01] ===== regime: {regime} (handshake_delay={handshake_delay:?}) =====");
        run_one_regime(&adapter, &pki, &kr, handshake_delay);
    }

    eprintln!(
        "[05-01] VERDICT: WORKS — composed bidirectional transparent mTLS (getsockname→resolve→\
         enforce, BOTH directions, 0x17 wire capture, no RST, all three Q3 arms, F5) validated on \
         kernel {kr}. Authn-only boundary honoured (expected_peer/expected_svid None)."
    );

    teardown_topology();
    clean_shared_infra();
}

/// One full bidirectional pass under the given timing regime. Proves O1–O6 for
/// this regime; the caller runs it twice (NORMAL + TRACED) for AC4.
///
/// **Driven through the PRODUCTION composition root.** Both legs run through
/// `MtlsInterceptWorker::start_alloc` → the spawned `accept_loop` (the shipping
/// code), NOT a hand-rolled replica. `start_alloc` binds the PRODUCTION leg-F
/// (the IP_TRANSPARENT bind under test), installs the egress nft-TPROXY rule, and
/// spawns the outbound accept→`getsockname`→resolve→3-arm-branch→enforce loop.
/// The ONLY injected double is the `resolve` port (a `ScriptedResolve` — the
/// production resolve index 01-03 is its own DST's job; the C1 contract is
/// driving production `accept_loop`'s branch + wiring, not the resolve index).
/// The enforce substrate is the REAL `HostMtlsEnforcement` (ADR-0069). All
/// oracles are observed at the wire/byte boundary; production exposes no
/// per-connection return value, so O1 (orig-dst recovery) is proven through the
/// byte-exact round-trip to the resolved backend (a wrong orig-dst would resolve
/// to the wrong arm and the handshake to the right peer would never complete) +
/// the sibling `start_alloc_legf_must_be_ip_transparent_for_real_tproxy_traffic`
/// guard's direct `Routed::Outbound { peer == dialed }` spy assertion.
fn run_one_regime(
    adapter: &Arc<HostMtlsEnforcement>,
    pki: &TestPki,
    kr: &str,
    handshake_delay: Duration,
) {
    let _ = kr;
    let _ = handshake_delay; // production accept_loop owns its own timing; the
    // TRACED regime's slow signal is the inbound client's send_delay (below) +
    // the netns clients' own write cadence — both still complete without RST.
    // ----------------------------------------------------------------
    // OUTBOUND leg (workload = client). The PRODUCTION accept_loop drives
    // capture → getsockname → resolve(3-arm) → enforce.
    // ----------------------------------------------------------------
    let mesh_backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().unwrap(), MESH_BACKEND_PORT);
    let nonmesh = SocketAddrV4::new(NONMESH_IP.parse().unwrap(), NONMESH_PORT);
    let unreachable = SocketAddrV4::new(UNREACHABLE_IP.parse().unwrap(), UNREACHABLE_PORT);

    // The scripted resolve table the PRODUCTION accept_loop consumes:
    // mesh_backend → Mesh(backend.addr = mesh_backend), unreachable →
    // MeshUnreachable. nonmesh is unscripted → NonMesh (the conservative default).
    let mut table = BTreeMap::new();
    table.insert(
        mesh_backend,
        MtlsResolution::Mesh(ResolvedBackend { addr: mesh_backend, expected_svid: None }),
    );
    table.insert(unreachable, MtlsResolution::MeshUnreachable);
    let resolve: Arc<dyn MtlsResolve> = Arc::new(ScriptedResolve::new(table));

    // Build the PRODUCTION worker over the REAL enforce substrate + the injected
    // resolve double, then drive `start_alloc` — this binds the PRODUCTION leg-F,
    // installs the egress rule on VETH_H, and spawns the PRODUCTION outbound
    // accept_loop. `spec.alloc = client_alloc` so production's `enforce` selects
    // the held CLIENT SVID for the leg-B handshake.
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(adapter) as Arc<dyn MtlsEnforcement>;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::new(SimClock::new()),
    ));
    let spec = build_client_spec(pki, Some(VETH_H.to_owned()));
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

    // --- OUTBOUND arm 1: Mesh → enforce mTLS (the primary deliverable) ---
    // Start the wire capture on the mesh-backend port BEFORE the workload dials so
    // the first leg-B record is on the captured wire (O2). The leg-B records carry
    // dst = mesh_backend_port (the agent's leg-B dial target is the mesh backend).
    let outbound_wire = WireCapture::start(LOOPBACK_IFACE, MESH_BACKEND_PORT);
    let mesh_peer = spawn_mesh_peer(pki);

    // The workload (inside the netns) dials the mesh backend, sends the request,
    // reads the response. Its egress ingresses vethH → PREROUTING → TPROXY →
    // PRODUCTION leg-F → PRODUCTION accept_loop → getsockname → resolve(Mesh) →
    // enforce. NO test code touches the accept path — production owns it.
    let req = OUTBOUND_REQUEST.to_vec();
    let want_resp = OUTBOUND_RESPONSE.len();
    let mesh_client =
        std::thread::spawn(move || run_netns_client(mesh_backend, &req, want_resp, None));

    // O3 (round-trip): the workload reads the mesh server's response byte-exact,
    // and the mesh server received the workload's request byte-exact — driven
    // entirely through the PRODUCTION accept_loop's Mesh arm.
    let client_out = mesh_client.join().expect("outbound mesh client thread");
    let client_read = client_out.stdout.clone();
    let mesh_request_ok = mesh_peer.join().expect("mesh peer thread");
    eprintln!(
        "[05-01][outbound Mesh] netns client exit={:?} stdout_len={} stderr={} | mesh_request_ok={}",
        client_out.status.code(),
        client_read.len(),
        String::from_utf8_lossy(&client_out.stderr).trim(),
        mesh_request_ok,
    );
    assert!(
        client_out.status.success(),
        "O3 outbound: the netns workload client must exit cleanly (got {:?}, stderr={})",
        client_out.status.code(),
        String::from_utf8_lossy(&client_out.stderr).trim()
    );
    assert_eq!(
        client_read,
        OUTBOUND_RESPONSE,
        "O3 outbound: the workload must read the mesh server's response byte-exact over leg-B's \
         kTLS — through the PRODUCTION accept_loop Mesh arm (got {} bytes)",
        client_read.len()
    );
    assert!(
        mesh_request_ok,
        "O3 outbound: the mesh server must receive the workload's request byte-exact (decrypted)"
    );
    // O1 (orig-dst recovery, observed through the round-trip): production's
    // `getsockname` recovery inside `accept_loop` is NOT a return value here. The
    // byte-exact round-trip to the mesh backend IS the O1 oracle: had production
    // recovered the wrong orig-dst, resolve would have classified the wrong arm
    // (NonMesh/MeshUnreachable, not Mesh→mesh_backend) and the mTLS handshake to
    // the mesh peer would never have completed. The sibling guard
    // `start_alloc_legf_must_be_ip_transparent_for_real_tproxy_traffic` asserts the
    // recovered orig-dst directly (`Routed::Outbound { peer == dialed }`) via a
    // spy; here it is proven end-to-end through the completed encrypted round-trip.

    // O2 (0x17 on the wire): the leg-B wire shows TLS-1.3 application_data records
    // in BOTH directions and NO cleartext marker. The DIRECTIONAL 0x17 counts are
    // the load-bearing confidentiality oracle (a cleartext leg-B would have zero
    // records in at least one direction).
    let scan = outbound_wire.stop_and_scan(OUTBOUND_REQUEST, OUTBOUND_RESPONSE);
    eprintln!("[05-01][outbound Mesh] leg-B wire scan = {scan:?}");
    assert!(
        scan.has_app_data(),
        "O2 outbound: the leg-B wire must carry TLS-1.3 0x17 application_data records, got {scan:?}"
    );
    assert!(
        scan.records_to_wire_port > 0,
        "O2 outbound: the request direction (toward the mesh backend) must carry 0x17 records"
    );
    assert!(
        scan.records_from_wire_port > 0,
        "O2 outbound: the response direction (from the mesh backend) must carry 0x17 records"
    );
    assert_eq!(
        scan.plaintext_marker_hits, 0,
        "O2 outbound: NO cleartext request/response marker may appear on the encrypted leg-B wire"
    );
    // O4 (no RST, outbound): a mid-stream RST would TRUNCATE/CORRUPT the byte-exact
    // round-trip above — the byte-exact `client_read == OUTBOUND_RESPONSE` (and the
    // mesh server's byte-exact receipt) IS the no-RST oracle for the outbound Mesh
    // arm, both genuinely observable. (The production accept_loop holds the enforced
    // handle internally; the test does not, so no pump-internal liveness read is
    // available — and the byte-exact round-trip is the stronger, observable proof.)

    // --- OUTBOUND arm 2: MeshUnreachable → fail-closed (NO cleartext) ---
    // A real listener on `unreachable` so that IF production wrongly fell back to
    // cleartext, the workload's bytes would land here. The PRODUCTION accept_loop's
    // MeshUnreachable arm must drop leg-F fail-closed — this sentinel must NOT
    // accept. (Driven END-TO-END through production for the first time.)
    let fc_listener = TcpListener::bind(unreachable).expect("bind fail-closed sentinel listener");
    fc_listener.set_nonblocking(true).expect("set_nonblocking on fail-closed sentinel");
    let fc_req = OUTBOUND_REQUEST.to_vec();
    let fc_client = std::thread::spawn(move || {
        // want=0: the workload sends but expects no response (fail-closed drops it).
        run_netns_client(unreachable, &fc_req, 0, None)
    });
    // POSITIVE interception signal (debugging.md §8/§11 — confirm the producer ran,
    // do not discard the workload's own connect outcome). Capture, do NOT `let _`,
    // the workload's connect/read result. Under the PRODUCTION path the TPROXY
    // divert delivers the workload's SYN to the transparent leg-F, the spawned
    // `accept_loop` ACCEPTS it (3-way handshake completes → the workload's
    // `connect()` SUCCEEDS), production then resolves MeshUnreachable and DROPS the
    // accepted leg-F (no enforce, no upstream dial). A *never-intercepted*
    // connection would instead see ConnectionRefused / ENETUNREACH on `connect()`
    // (no SYN-ACK, no transparent socket) → `run_netns_client` emits
    // `CLIENT-FAIL:...refused`/`unreachable`. The discriminator between
    // intercepted-then-dropped and never-intercepted is therefore the *connect*
    // outcome: any post-connect drop (clean EOF for want=0, or a Broken-pipe /
    // Connection-reset once production drops the accepted leg-F) is the
    // intercepted-then-fail-closed shape; only a CONNECT-time refusal/unreachability
    // is never-intercepted. Asserting "connect was not refused" proves interception
    // HAPPENED, and the silent-sentinel poll below proves fail-closed FIRED — a real
    // differential against the NonMesh-relays arm.
    let fc_out = fc_client.join().expect("fail-closed netns client thread");
    let fc_stderr = String::from_utf8_lossy(&fc_out.stderr);
    eprintln!(
        "[05-01][outbound MeshUnreachable] netns client exit={:?} stdout_len={} stderr={}",
        fc_out.status.code(),
        fc_out.stdout.len(),
        fc_stderr.trim(),
    );
    // Intercepted-then-dropped, NOT refused: the workload's `connect()` must have
    // SUCCEEDED (leg-F accepted it). A connect-time refusal/unreachability is the
    // never-intercepted shape; a post-connect EOF/RST is intercepted-then-dropped.
    let connect_refused = fc_stderr.contains("refused")
        || fc_stderr.contains("unreachable")
        || fc_stderr.contains("Errno 111") // ECONNREFUSED
        || fc_stderr.contains("Errno 101"); // ENETUNREACH
    assert!(
        !connect_refused,
        "O5 fail-closed POSITIVE signal: the workload's connect() must SUCCEED (production leg-F \
         accepted it → intercepted) before production drops it fail-closed — a connect-time \
         refusal/unreachability means the connection was never intercepted, not \
         intercepted-then-dropped. stderr={}",
        fc_stderr.trim()
    );
    // …and fail-closed dropped it with no echo: stdout carries no relayed/echoed
    // bytes (a silent-cleartext fallback would have relayed the request to the
    // sentinel below and could have echoed, not returned empty).
    assert!(
        fc_out.stdout.is_empty(),
        "O5 fail-closed POSITIVE signal: the dropped leg-F must yield no echo (empty stdout), \
         got {} bytes",
        fc_out.stdout.len()
    );
    // NEGATIVE signal (retained, load-bearing as a set): NO cleartext reached the
    // should-be-mesh sentinel. Bounded poll-loop over a deadline (debugging.md §11 —
    // not a single poll after one fixed sleep): `accept()` must stay err for the
    // whole window, proving no connection ever lands on the sentinel.
    let fc_deadline = std::time::Instant::now() + Duration::from_millis(800);
    loop {
        match fc_listener.accept() {
            Ok((_s, peer)) => panic!(
                "O5 fail-closed: production accept_loop's MeshUnreachable arm must drop leg-F \
                 fail-closed — NO connection may reach the should-be-mesh sentinel (no silent \
                 cleartext), but one arrived from {peer}"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= fc_deadline {
                    break; // window elapsed with no accept — fail-closed confirmed
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("fail-closed sentinel accept errored unexpectedly: {e}"),
        }
    }
    drop(fc_listener);

    // --- OUTBOUND arm 3: NonMesh → cleartext pass-through (by design) ---
    // A real cleartext echo server on `nonmesh`. The PRODUCTION accept_loop's
    // NonMesh arm relays leg-F to it in cleartext (NO mTLS) — the by-design
    // classification arm. (Driven END-TO-END through production for the first time.)
    let nm_echo = spawn_cleartext_echo(nonmesh);
    let nm_req = OUTBOUND_REQUEST.to_vec();
    let nm_want = OUTBOUND_REQUEST.len(); // the echo returns the request bytes
    let nm_client = std::thread::spawn(move || run_netns_client(nonmesh, &nm_req, nm_want, None));
    let nm_out = nm_client.join().expect("nonmesh client thread");
    let nm_echo_ok = nm_echo.join().expect("nonmesh echo thread");
    // O5 pass-through: the non-mesh upstream received the workload's bytes (cleartext
    // relay by design, through the production NonMesh arm) and echoed them back.
    assert!(
        nm_echo_ok,
        "O5 pass-through: production accept_loop's NonMesh arm must relay the workload's bytes \
         cleartext to the non-mesh upstream (it must receive them)"
    );
    assert_eq!(
        nm_out.stdout, OUTBOUND_REQUEST,
        "O5 pass-through: the workload must read the non-mesh echo back (cleartext round-trip \
         through the production NonMesh arm)"
    );

    // Tear the production outbound intercept down before the inbound leg (removes
    // the egress rule by guard handle; the shared chain survives).
    worker.stop_alloc(&pki.client_alloc);

    // ----------------------------------------------------------------
    // INBOUND leg (workload = server). Capture → enforce (server handshake).
    // ----------------------------------------------------------------
    drive_inbound_leg(adapter, pki, handshake_delay);

    // O6 (F5 — workload cannot self-exempt): an EXPLICIT self-exempt-impossible
    // probe, driven through the PRODUCTION accept_loop.
    prove_workload_cannot_self_exempt(adapter, pki, handshake_delay);
}

/// INBOUND leg, driven through the SPAWNED PRODUCTION inbound `accept_loop` —
/// symmetric with the outbound half. `start_alloc(&server_spec)` binds the
/// PRODUCTION leg-C (the IP_TRANSPARENT inbound listener) at a worker-chosen
/// ephemeral port and spawns the production inbound accept loop over it
/// (`accept_loop`'s `AcceptLeg::Inbound` arm: `accept_inbound_leg → spawn_enforce`
/// → the real `HostMtlsEnforcement::enforce`). NO test code touches the inbound
/// accept/enforce path — production owns it, exactly as the outbound half does.
///
/// **How the client reaches production leg-C (D-TME-13 + #178).** The inbound
/// nft-TPROXY redirect that routes a client's virt to leg-C is #178-deferred:
/// production `start_alloc` installs NO inbound rule (its match key is the
/// server's virt, an east-west fact v1 has no production source for —
/// `start_alloc` records `tproxy_guard = None`). So the test discovers leg-C's
/// ephemeral bound addr via the architect-pinned accessor
/// [`MtlsInterceptWorker::leg_c_addr`] (D-TME-13 — a production-legitimate
/// diagnostic observability surface, mirroring the leg-F **capture pattern** the
/// egress rule encodes; leg-F has no public accessor — its port is an inline
/// `start_alloc` local). #178's production inbound-redirect install is *expected*
/// to reuse this leg-C bound-addr read pending its install site/timing design;
/// if #178 mirrors leg-F and installs in `start_alloc` it may read an inline
/// local rather than this accessor, leaving the accessor test-observability-only.
/// The test installs its OWN `install_inbound_tproxy(virt, leg_c_port)` redirect
/// to that PRODUCTION-bound leg-C. A client's `connect(virt)` → PREROUTING →
/// TPROXY → production leg-C → the spawned production inbound `accept_loop`. When
/// #178 lands the production inbound rule it replaces this test redirect; the
/// accessor and the spawned accept loop are unchanged.
///
/// All oracles stay observable kernel/wire side effects (O1 orig-dst via the
/// byte-exact round-trip, O2 `0x17` both directions, O3 byte-exact, O4 no-RST) —
/// none read program-internal state. authn-only v1 (AC8 / #178): the client
/// asserts chain-to-bundle + held-server-SVID, never an intended-peer
/// "protection" claim; `expected_peer`/`expected_svid` stay `None`.
fn drive_inbound_leg(adapter: &Arc<HostMtlsEnforcement>, pki: &TestPki, handshake_delay: Duration) {
    let virt = SocketAddrV4::new(INBOUND_VIRT_IP.parse().unwrap(), INBOUND_VIRT_PORT);

    // Build the PRODUCTION worker over the REAL enforce substrate and drive
    // `start_alloc` for the SERVER alloc — this binds the PRODUCTION leg-C
    // IP_TRANSPARENT listener at an ephemeral port and spawns the PRODUCTION
    // inbound `accept_loop` over it. `spec.alloc = server_alloc` so production's
    // `enforce` selects the held SERVER SVID for the leg-C handshake. The resolve
    // port is unused by the inbound arm (it goes straight to `spawn_enforce`), but
    // is a mandatory `new()` param — an empty `ScriptedResolve` suffices. No
    // `host_veth`: the inbound rule is the test-installed redirect below
    // (#178-deferred), not the egress rule.
    let resolve: Arc<dyn MtlsResolve> = Arc::new(ScriptedResolve::new(BTreeMap::new()));
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(adapter) as Arc<dyn MtlsEnforcement>;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::new(SimClock::new()),
    ));
    let spec = build_server_spec(pki);
    worker.start_alloc(&spec).expect(
        "PRODUCTION start_alloc must bind leg-C + spawn the inbound accept_loop for the server alloc",
    );

    // Discover the PRODUCTION-bound leg-C ephemeral addr via the D-TME-13 accessor
    // (the listener is owned by the spawned accept loop; this is the only way to
    // learn WHERE it accepts). `is_some()` ⇔ a live intercept is recorded.
    let leg_c_addr = worker
        .leg_c_addr(&spec.alloc)
        .expect("leg_c_addr must expose the production-bound leg-C for a live intercept");
    let leg_c_port = leg_c_addr.port();

    // Install the INBOUND nft-TPROXY rule (#178-deferred in production): a client
    // dialing the virt is redirected to the PRODUCTION leg-C. The F5 exemption
    // (chain head) lets the agent's SO_MARK-stamped leg-S dial reach the real
    // server S verbatim. This redirect is the ONLY test-supplied plumbing; the
    // accept/enforce that runs on the other end is the spawned production loop.
    let inbound_guard = install_inbound_tproxy(virt, leg_c_port)
        .expect("install_inbound_tproxy must append the per-virt TPROXY rule");
    let dump = nft_dump_table();
    assert!(
        dump.contains(&format!("ip daddr {INBOUND_VIRT_IP}")) && dump.contains("tproxy to"),
        "the inbound per-virt tproxy rule must be installed, got:\n{dump}"
    );

    // The real server workload S binds the virt; the agent's SO_MARK-exempt leg-S
    // dial reaches it (server_dial_addr(orig_dst) == orig_dst == virt).
    let server = spawn_inbound_server();
    // Give S a moment to bind before the client connects / the agent dials.
    std::thread::sleep(Duration::from_millis(200));

    // Start the leg-C wire capture (filtered to the virt port) BEFORE the client
    // connects so the first record is on the captured wire.
    let inbound_wire = WireCapture::start(LOOPBACK_IFACE, INBOUND_VIRT_PORT);

    // The inbound client (presents the CLIENT SVID, dials the virt → TPROXY →
    // PRODUCTION leg-C → spawned production inbound accept_loop), delayed so its
    // first app write lands after the agent arms kTLS-RX. The TRACED regime's slow
    // signal is folded into this send_delay.
    let inbound_client = spawn_inbound_client(pki, Duration::from_millis(400).max(handshake_delay));

    // NO test code accepts or enforces — the spawned PRODUCTION inbound accept_loop
    // accepts leg-C, recovers orig-dst via getsockname, and enforces (server
    // handshake + SO_MARK-exempt leg-S dial to S). The test only joins the client +
    // server and reads the wire.

    // O3 (inbound round-trip): the client reads S's response byte-exact over leg-C's
    // kTLS, S received the request byte-exact, and the agent presented the held
    // server SVID (AC3 inbound identity proof — read from the verified leg-C leaf).
    let server_request_ok = server.join().expect("inbound server thread");
    let client_result = inbound_client.join().expect("inbound client thread");
    assert!(
        server_request_ok,
        "O3 inbound: server S must receive the client's request byte-exact (decrypted on the \
         production leg-C, spliced to leg-S) — through the spawned production inbound accept_loop"
    );
    assert!(
        client_result.received_response_byte_exact,
        "O3 inbound: the client must read S's response byte-exact back over the production leg-C's \
         kTLS"
    );
    // O1 (inbound orig-dst recovery, observed through the round-trip): production's
    // `getsockname` recovery inside the inbound accept_loop is NOT a return value
    // here. The byte-exact round-trip to the virt IS the O1 oracle — had production
    // recovered the wrong orig-dst, its SO_MARK-exempt leg-S dial would have aimed
    // at the wrong addr and S would never have received the request byte-exact.
    // O4 (no RST, inbound): the client's `observed_rst == false` is a genuinely
    // observable no-RST oracle; the byte-exact round-trip above is the corroborating
    // proof (a mid-stream RST would truncate it).
    assert!(!client_result.observed_rst, "O4 inbound: the client must NOT observe a transport RST");
    // AC3 inbound identity: the agent presented the HELD server SVID (the client
    // verified its chain-to-bundle and extracted its SPIFFE-SAN). This is the
    // chain-to-bundle authn proof — NOT an intended-peer "protection" claim (AC8).
    assert_eq!(
        client_result.presented_server_spiffe.as_ref(),
        Some(&pki.server_leaf.spiffe),
        "inbound authn: the agent's leg-C SERVER handshake must present the HELD server SVID; the \
         client must verify it chains to the bundle and the leaf URI-SAN is the server SPIFFE"
    );

    // O2 (0x17 on the leg-C wire): TLS-1.3 application_data records in BOTH
    // directions and NO cleartext marker (the request reaches S decrypted; the
    // response rides back encrypted). The DIRECTIONAL 0x17 counts are load-bearing.
    let inbound_scan = inbound_wire.stop_and_scan(INBOUND_REQUEST, INBOUND_RESPONSE);
    eprintln!("[05-01][inbound] leg-C wire scan = {inbound_scan:?}");
    assert!(
        inbound_scan.has_app_data(),
        "O2 inbound: the leg-C wire must carry TLS-1.3 0x17 application_data records, got \
         {inbound_scan:?}"
    );
    assert!(
        inbound_scan.records_to_wire_port > 0,
        "O2 inbound: the request direction (toward the virt) must carry 0x17 records"
    );
    assert!(
        inbound_scan.records_from_wire_port > 0,
        "O2 inbound: the response direction (from the virt) must carry 0x17 records"
    );
    assert_eq!(
        inbound_scan.plaintext_marker_hits, 0,
        "O2 inbound: NO cleartext request/response marker may appear on the encrypted leg-C wire"
    );

    // Tear the production inbound intercept down (aborts the accept loop, drains
    // the enforced handle through `enforcement.teardown`); then drop the test
    // redirect guard (removes the per-virt rule).
    worker.stop_alloc(&spec.alloc);
    drop(inbound_guard);
}

/// O6 explicit self-exempt-impossible (F5 / AC6): drive a WORKLOAD netns dial that
/// stamps `SO_MARK = MTLS_LEG_S_DIAL_MARK` inside its own netns THROUGH the
/// PRODUCTION accept_loop. The mark is skb-local and does NOT cross the
/// veth/netns boundary, so the host-side `iifname VETH_H` egress rule STILL
/// captures it — production's accept_loop recovers the dialed mesh backend via
/// getsockname and enforces mTLS, and the round-trip completes through the agent,
/// NOT direct. A workload cannot forge the agent's exemption.
fn prove_workload_cannot_self_exempt(
    adapter: &Arc<HostMtlsEnforcement>,
    pki: &TestPki,
    handshake_delay: Duration,
) {
    let _ = handshake_delay;
    let mesh_backend = SocketAddrV4::new(MESH_BACKEND_IP.parse().unwrap(), MESH_BACKEND_PORT);
    let mut table = BTreeMap::new();
    table.insert(
        mesh_backend,
        MtlsResolution::Mesh(ResolvedBackend { addr: mesh_backend, expected_svid: None }),
    );
    let resolve: Arc<dyn MtlsResolve> = Arc::new(ScriptedResolve::new(table));

    // PRODUCTION worker over the real enforce substrate; start_alloc binds the
    // PRODUCTION leg-F + installs the egress rule + spawns the accept_loop.
    let enforcement: Arc<dyn MtlsEnforcement> = Arc::clone(adapter) as Arc<dyn MtlsEnforcement>;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::new(SimClock::new()),
    ));
    let spec = build_client_spec(pki, Some(VETH_H.to_owned()));
    worker.start_alloc(&spec).expect("start_alloc (self-exempt probe)");

    // A real backend so IF the marked workload dial self-exempted, it would land
    // here instead of leg-F. It must NOT.
    let mesh_peer = spawn_mesh_peer(pki);
    let mesh_wire = WireCapture::start(LOOPBACK_IFACE, MESH_BACKEND_PORT);

    // The workload stamps SO_MARK = MTLS_LEG_S_DIAL_MARK inside its OWN netns.
    let req = OUTBOUND_REQUEST.to_vec();
    let want = OUTBOUND_RESPONSE.len();
    let marked_client = std::thread::spawn(move || {
        run_netns_client(mesh_backend, &req, want, Some(MTLS_LEG_S_DIAL_MARK))
    });

    // Production accept_loop must STILL capture it on leg-F (the mark did not cross
    // the veth) and enforce mTLS — the round-trip completes through the agent. A
    // self-exempted dial would instead reach the mesh backend DIRECT in cleartext
    // (no 0x17), which the assertions below forbid.
    let client_out = marked_client.join().expect("self-exempt client thread");
    assert_eq!(
        client_out.stdout, OUTBOUND_RESPONSE,
        "F5 self-exempt-impossible: the marked workload dial still rode the agent's mTLS path \
         through the PRODUCTION accept_loop (read the mesh response through the agent)"
    );
    let mesh_ok = mesh_peer.join().expect("self-exempt mesh peer thread");
    assert!(
        mesh_ok,
        "F5 self-exempt-impossible: the mesh server received the request via the agent"
    );
    // The bytes rode an mTLS leg-B — 0x17 on the wire, no cleartext. Had the marked
    // dial self-exempted (reached the backend direct), the wire would carry the
    // cleartext markers and ZERO 0x17 records.
    let scan = mesh_wire.stop_and_scan(OUTBOUND_REQUEST, OUTBOUND_RESPONSE);
    eprintln!("[05-01][F5 self-exempt-impossible] leg-B wire scan = {scan:?}");
    assert!(
        scan.has_app_data() && scan.plaintext_marker_hits == 0,
        "F5 self-exempt-impossible: the captured marked dial rode an encrypted leg-B (0x17, no \
         cleartext) through the production accept_loop, got {scan:?}"
    );
    worker.stop_alloc(&pki.client_alloc);
}

/// Spawn a cleartext ECHO server on `addr` (the `NonMesh` upstream). Reads the
/// request bytes and echoes them back. Returns a join handle whose `bool` reports
/// it received the request byte-exact.
fn spawn_cleartext_echo(addr: SocketAddrV4) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        let Ok(listener) = TcpListener::bind(addr) else {
            eprintln!("[05-01] cleartext echo bind {addr} failed");
            return false;
        };
        let Ok((mut tcp, _)) = accept_with_timeout(&listener, Duration::from_secs(12)) else {
            eprintln!("[05-01] cleartext echo accept timed out");
            return false;
        };
        tcp.set_nodelay(true).ok();
        tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut buf = vec![0u8; 4096];
        while got.len() < OUTBOUND_REQUEST.len() && Instant::now() < deadline {
            match tcp.read(&mut buf) {
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
        let ok = got == OUTBOUND_REQUEST;
        let _ = tcp.write_all(&got).and_then(|()| tcp.flush());
        std::thread::sleep(Duration::from_millis(200));
        ok
    })
}
