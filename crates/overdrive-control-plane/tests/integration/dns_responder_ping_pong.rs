//! S-DBN-PINGPONG — the dial-by-name-responder BIDIRECTIONAL PING-PONG demo
//! (ADR-0072 REV-2; GH #243; roadmap 03-02 / US-DBN-3 · K-DBN-3).
//!
//! This Tier-3 `#[tokio::test]` is the in-process proof of the genuinely-new
//! composition this slice exists to cover: **two workloads each
//! simultaneously a mesh SERVICE (inbound leg-C + frontend `F`) AND an egress
//! DIALER (leg-B)** — `a` and `b`, ×2 on ONE `serve`. It drives ONLY the
//! production entry points — `run_server_with_obs_and_driver` (boot) + `POST
//! /v1/jobs` (deploy, the in-process `overdrive deploy` driving port) +
//! `ip netns exec <ns> getent ahostsv4` (resolve, NOT `dig` — K2) from inside
//! each deployed workload's PRODUCTION-provisioned netns + a PLAINTEXT
//! `TcpStream` dial from the SAME netns (capture / translate / mTLS).
//!
//! ## Why this is the NEW ground (not 02-02 twice)
//!
//! The 02-02 walking skeleton (`dns_responder_walking_skeleton.rs`) proves the
//! SINGLE-direction loop: a `client` that only dials, a `server` that only
//! serves. NO test in the feature exercised a workload that is BOTH a reachable
//! mesh service AND an egress dialer. That is exactly what `a` and `b` are here:
//! each is a Service (declares a `[[listener]]` → `start_alloc` installs the
//! inbound leg-C rules when `workload_addr` is Some) AND a dial source (its
//! per-alloc netns carries the production resolv.conf + the `start_alloc`
//! outbound leg-B nft-TPROXY rule), ×2 coexisting on one boot sharing the fixed
//! `ovd-veth-cli` / `ovd-veth-bk` host routing point. 03-01 already showed that
//! sibling responder fixtures collide on the shared `:53` / `FrontendAddrAllocator`
//! / identical-name state, so "two services coexisting and dialing each other"
//! is a live composition risk this test now guards.
//!
//! Because each alloc is BOTH a leg-B TLS CLIENT and a leg-C TLS SERVER, and the
//! production `HostMtlsEnforcement` reads ONE `svid_for(&alloc)` for BOTH legs
//! (`enforce_outbound` AND `enforce_inbound` call `svid_or_fail(&conn.alloc)`),
//! each of `a`/`b` is supplied a SINGLE per-alloc SVID carrying BOTH
//! `ClientAuth` AND `ServerAuth` EKUs, plus the production dataplane's hardcoded
//! intra-mesh leg-B sentinel SNI SAN (`peer.overdrive.local`) the leg-B client
//! handshake verifies the dialed server against. (02-02's `HeldServerIdentity`
//! handed the `server` alloc a ServerAuth-only leaf and every other alloc a
//! ClientAuth-only leaf — which would fail BOTH legs here, since each alloc
//! plays both roles.)
//!
//! ## The vertical-slice litmus (CLAUDE.md "Build vertical slices")
//!
//! NO test binds `:53`, installs a `resolv.conf`, allocates `F`, programs a
//! map, or hand-installs the egress/inbound capture — production does ALL of
//! those itself:
//!
//! - the `:53` responder is bound by `DnsResponder::probe` (spawned by
//!   `run_server`, DDN-6);
//! - the per-netns `/etc/netns/<ns>/resolv.conf` is written by the production
//!   `veth_provisioner::provision_workload_netns` (D-TME-9), so a `getent` from
//!   inside a deployed workload's netns reaches the responder through the
//!   production resolv.conf;
//! - the STABLE frontend `F ∈ 10.98.0.0/16` is bound by the production
//!   `FrontendAddrAllocator` (01-04/01-05);
//! - the egress nft-TPROXY capture AND the inbound leg-C rules are installed
//!   per-workload by `start_alloc` (`install_outbound_tproxy` keyed on `iifname
//!   <host_veth>`, and `install_inbound_tproxy` per declared listener port when
//!   `workload_addr` is Some) — so a connect to a peer's `F` from inside a
//!   deployed workload's netns is captured + translated + mTLS'd, and the peer's
//!   own listener is reachable through ITS leg-C, both with NO test rule.
//!
//! ## CORRECTED EGRESS MODEL — the workload speaks PLAINTEXT (RCA, 2026-06-27)
//!
//! Per the ADR-0072 workload-identity model — "workloads hold NOTHING; the
//! kernel/agent does mTLS" — the egress capture lands the dialer's connect on
//! the agent's PLAINTEXT leg-F. A full-rustls test client gets no TLS peer and
//! its ClientHello tunnels as plaintext → stall/RST (RCA
//! `root-cause-analysis-dial-by-name-agent-originated-mtls-stall.md`, the
//! TEST-MODEL MISMATCH). The corrected model: **the test dialer speaks
//! PLAINTEXT** (sends a byte-distinct REQUEST, reads the byte-distinct
//! `PONG count=N` RESPONSE over a bare `TcpStream`), modelling a real
//! identity-unaware workload. The mTLS proof MOVES OFF the dialer (it terminates
//! no TLS) and ONTO the inter-agent **leg-B ↔ leg-C** hop — the only segment the
//! agent encrypts. On single-node that hop is host-local (`lo`); the
//! `WireCapture` 0x17 oracle proves it carries TLS-1.3 application_data records
//! (both directions) with zero cleartext, applied to BOTH hops (a→b and b→a).
//! Do NOT copy the INBOUND keystone's
//! (`canonical_address_inbound_walking_skeleton.rs`) "client presents TLS" dial
//! shape onto these egress hops — that is the exact 02-02 model error.
//!
//! ## Counters advance — the bidirectional-loop-is-live proof
//!
//! Each workload's Python server bumps an inbound counter per accepted
//! connection and replies `PONG count=<n> ...` (byte-distinct per direction so
//! a→b's RESPONSE never matches b→a's). The test dials each peer TWICE and
//! asserts the returned count STRICTLY INCREASES — proving the real peer→caller
//! reply pipe carried the peer-authored count (not an echo), in BOTH directions.
//!
//! ## E05 relationship — this is the in-process "what, forever"; E05 is the
//! black-box "why" (still pending #227/#75)
//!
//! This in-process test (test-PKI seam via `mtls_identity_override`) is the
//! regression witness for the bidirectional composition. The BLACK-BOX
//! operator-observable capture — the PRODUCTION workload-identity CA → SVID →
//! leg-C/leg-B mTLS path driven through the BUILT `overdrive` binary with the
//! deployed `ping_pong.py` workloads dialing autonomously, NO test seam — is
//! `verification/expectations/E05-dial-by-name-ping-pong-mtls`, which genuinely
//! needs the full-system EDD harness #227 (the disposable full-system Lima VM)
//! on #75 (the Image Factory OS image) and stays `pending` on exactly that.
//! #227/#75 are the wrong gate for THIS in-process test (it needs neither),
//! which is why the prior `#[should_panic]` scaffold that gated GREEN on them
//! was a category error (review-03-02.md).
//!
//! Requires root + `CAP_NET_ADMIN`/`CAP_SYS_ADMIN`. A non-root run SKIPs
//! cleanly (the K1 root gate — a real early return, NOT a panic). `uname -r` is
//! recorded. Run via `cargo xtask lima run -- cargo nextest run -p
//! overdrive-control-plane --features integration-tests`. NEVER `--no-run`.
//!
//! ## Per-host singleton — runs SEQUENTIALLY with the sibling DNS responder tests
//!
//! This fixture boots a full production composition root that binds the
//! process-wide `:53` DNS responder and attaches XDP to the FIXED
//! `ovd-veth-cli` / `ovd-veth-bk` ifaces. nextest runs each `#[test]` in a
//! SEPARATE process by default, so two such fixtures collide on the `:53` bind /
//! `IfaceXdpSlotBusy`. This module is in the `.config/nextest.toml`
//! `host-kernel-shared` single-writer group alongside
//! `dns_responder_walking_skeleton` / `dns_responder_nxdomain` (matched
//! by-module, rename-proof).
//!
//! MERGE-BLOCKING on the pinned-6.18 appliance-kernel Tier-3 matrix (ADR-0068);
//! dev-Lima is necessary-but-not-sufficient and MUST be re-confirmed on 6.18
//! (DEVOPS/Tier-3 obligation, criterion 5 — an in-test `assert uname == 6.18`
//! would fail on dev-Lima, so `record_kernel()` records `uname -r` and the 6.18
//! assertion lives in the merge-gate matrix, not here).
//!
//! Helpers (PKI, netns entry, getent, the plaintext dial, the 0x17 wire oracle)
//! are kept LOCAL to this module — sibling `tests/integration/<scenario>.rs`
//! files are distinct module roots and cannot import each other's items (sharing
//! would require promoting them into a shared module AND touching the 02-02 /
//! 03-01 files, out of this step's boundary; same note as `dns_responder_nxdomain.rs`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unnecessary_cast,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    reason = "Tier-3 bidirectional ping-pong body; failures must panic with informative messages; \
              F/a/b are the ADR-0072 REV-2 stable-frontend / mesh-name vocabulary; the composed \
              flow is one long scenario; the AF_PACKET WireCapture 0x17 oracle is mirrored \
              verbatim from dns_responder_walking_skeleton.rs (the same cast lints that file \
              allows at file scope)"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
// constants — the pinned single bidirectional example (criterion 1: no PBT)
// ============================================================================

/// The TWO `[service].id`s of the demo — the pinned bidirectional example. `a`
/// is reachable at `a.svc.overdrive.local`, `b` at `b.svc.overdrive.local`. A
/// dials b, B dials a — a closed two-hop ping-pong loop.
const SERVICE_A_ID: &str = "a";
const SERVICE_B_ID: &str = "b";

/// The mesh names each half resolves (the on-wire `getaddrinfo` queries — K2).
/// `format!("{id}.{}", MeshServiceName::SUFFIX)`; pinned as literals so the
/// on-wire name a real stub resolver would query is visible at the call site.
const PEER_NAME_FROM_A: &str = "b.svc.overdrive.local";
const PEER_NAME_FROM_B: &str = "a.svc.overdrive.local";

/// The declared TCP listener ports — chosen to AVOID 5353 (systemd-resolved /
/// mDNS owns it in the dev Lima VM) and 53 (the in-agent DNS responder). A and
/// B differ so the two specs never collide on a shared port observation, and so
/// the inter-agent wire-capture of each direction filters cleanly by dport.
const SERVICE_A_PORT: u16 = 18971;
const SERVICE_B_PORT: u16 = 18972;

/// The PLAINTEXT request the dialer sends (byte-distinct per direction so the
/// wire-scan's cleartext check can never confuse a→b with b→a). The server
/// IGNORES the body and replies its own counted PONG — proving the real
/// peer→caller reply pipe, not an echo.
const REQUEST_A_TO_B: &[u8] =
    b"OVERDRIVE_PINGPONG_REQUEST_a_dials_b_svc_by_name_plaintext_legf_0302";
const REQUEST_B_TO_A: &[u8] =
    b"OVERDRIVE_PINGPONG_REQUEST_b_dials_a_svc_by_name_plaintext_legf_0302";

/// The fixed sentinel SNI the PRODUCTION dataplane uses for the agent's
/// intra-mesh **leg-B** peer dial (`overdrive-dataplane::mtls::outbound`,
/// hardcoded `"peer.overdrive.local"`). EACH alloc's SVID must carry this SAN —
/// every alloc here is a leg-B dialer AND the dialed leg-C server, so the leg-B
/// client handshake verifies the dialed peer's presented SVID against
/// `peer.overdrive.local`.
const MESH_PEER_SNI: &str = "peer.overdrive.local";

/// The production per-host stable-frontend block (`10.98.0.0/16`,
/// `WORKLOAD_FRONTEND_BASE`). `F` answered for a `<job>` is a member; a
/// per-instance backend addr lives in `10.99.0.0/16` and is NEVER the answer.
const FRONTEND_FIRST_OCTET: u8 = 10;
const FRONTEND_SECOND_OCTET: u8 = 98;
/// The per-instance workload (backend) block second octet (`10.99.0.0/16`,
/// `WORKLOAD_SUBNET_BASE`) — `getent` MUST NEVER answer an addr here.
const WORKLOAD_SECOND_OCTET: u8 = 99;

/// `lo` — where the agent's INTER-AGENT leg-B ↔ leg-C TLS records physically
/// carry their bytes on single-node. The agent's host-originated leg-B re-dial
/// to the resolved backend is diverted on the kernel OUTPUT hook (REV-5) and
/// routed via `local table 100` (loopback re-entry) into the leg-C
/// `127.0.0.1:<agent_port>` IP_TRANSPARENT listener — so the leg-B
/// application_data records traverse `lo` carrying `dport = <peer SERVICE_PORT>`
/// (TPROXY preserves the orig daddr/dport). The 0x17 oracle captures here; the
/// plaintext leg-F (dialer → F) and leg-S (agent → backend) ride the
/// per-workload VETHs (DIFFERENT ifaces) and never pollute this capture.
const LOOPBACK_IFACE: &str = "lo";

/// The K2 resolution budget — `getent` must answer a stable `F` within this
/// window of the workload reaching running-AND-healthy (the bridge writing the
/// `service_backends` row that the responder's `name_index` reads).
const RESOLVE_BUDGET: Duration = Duration::from_secs(8);

// ============================================================================
// root gate + kernel record (the K1 root gate / ADR-0068 merge-gate record)
// ============================================================================

/// True iff this process is uid 0 (root). The real `EbpfDataplane` XDP attach,
/// per-workload netns provision, nft, `ip rule`, `IP_TRANSPARENT`, and the `:53`
/// responder bind all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run
/// cannot stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Record the running kernel — the Tier-3 verdict is pinned to a kernel (dev-Lima
/// and the pinned-6.18 appliance kernel differ — ADR-0068; the merge gate is the
/// 6.18 matrix, criterion 5).
fn record_kernel() -> String {
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[03-02] uname -r = {kr} (MERGE GATE = pinned-6.18 Tier-3 matrix, ADR-0068)");
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
/// egress rule already installed) is located so a `getent` + dial can run there
/// — NOT a test-created netns.
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
// `/etc/resolv.conf` from the MOUNT namespace, which is unchanged — so it would
// query the HOST's systemd-resolved. `ip netns exec` enters BOTH the net
// namespace AND bind-mounts the per-netns resolv.conf over `/etc/resolv.conf`,
// so `getent` (a real `getaddrinfo` call) resolves through the production
// responder. `getent` is a stub resolver: it DISCARDS a reply whose source addr
// is not the queried server addr, so it only succeeds when the production
// responder source-pinned its reply (`ipi_spec_dst`) — the K2 litmus.

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

/// `Some(F)` ⇔ `getent ahostsv4 <mesh_name>` run inside `netns` (via `ip netns
/// exec`, so the production resolv.conf + responder are used) resolves to a V4
/// addr in the stable-frontend block `10.98.0.0/16` AND NOT a per-instance
/// backend in `10.99.0.0/16` (the SQ1 guard). Returns the resolved `F`.
fn resolve_frontend_in_netns(netns: &str, mesh_name: &str) -> Option<Ipv4Addr> {
    let out = Command::new("ip")
        .args(["netns", "exec", netns, "getent", "ahostsv4", mesh_name])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let addrs = parse_getent_v4(&stdout);
    eprintln!(
        "[03-02] getent ahostsv4 {mesh_name} in {netns} -> {addrs:?} (code {:?})",
        out.status.code()
    );
    addrs.into_iter().find(|a| {
        let o = a.octets();
        o[0] == FRONTEND_FIRST_OCTET && o[1] == FRONTEND_SECOND_OCTET
    })
}

/// Poll `resolve_frontend_in_netns` until it answers a stable `F` within
/// `budget` — re-querying because the responder's `name_index` exposes the
/// `<job>` only after the backend reaches running-AND-healthy (the bridge writes
/// the `service_backends` row).
fn poll_resolve_frontend(netns: &str, mesh_name: &str, budget: Duration) -> Option<Ipv4Addr> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(f) = resolve_frontend_in_netns(netns, mesh_name) {
            return Some(f);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// ============================================================================
// netns entry (enter a PRODUCTION netns for the plaintext dial)
// ============================================================================

/// `setns(open("/var/run/netns/<ns>"), CLONE_NEWNET)` — move THIS thread into
/// the named network namespace. Returns false on any failure. Used to enter a
/// DEPLOYED workload's PRODUCTION netns (so the egress rule is the production
/// one), never a test-created netns.
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
// the in-netns PLAINTEXT dial + counter read
// ============================================================================

/// One captured-and-replied PONG: the strictly-increasing inbound counter the
/// dialed peer authored, recovered from its `PONG count=<n> ...` reply.
struct PongResult {
    /// `Some(n)` ⇔ a byte-complete `PONG count=<n>` reply was read from the
    /// peer over the real peer→caller pipe; `None` on RST / timeout / no reply.
    count: Option<u64>,
    observed_rst: bool,
}

/// Parse the `count=<n>` field out of a `PONG count=<n> date=<...>` reply line.
fn parse_pong_count(reply: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(reply).ok()?;
    for tok in text.split_whitespace() {
        if let Some(rest) = tok.strip_prefix("count=") {
            return rest.parse::<u64>().ok();
        }
    }
    None
}

/// A real workload's PLAINTEXT dial: from inside `netns`, connect to
/// `(peer_frontend, peer_port)`, send `request`, read the peer's `PONG count=N`
/// reply. The agent captures this on its plaintext leg-F and originates mTLS on
/// leg-B → leg-C (proven separately by the inter-agent wire capture). Runs on a
/// dedicated thread so the `setns` does not leak into the test runtime thread.
fn dial_peer_in_netns(
    netns: &str,
    peer_frontend: Ipv4Addr,
    peer_port: u16,
    request: &'static [u8],
) -> PongResult {
    let ns = netns.to_owned();
    std::thread::spawn(move || {
        if !enter_netns(&ns) {
            eprintln!("[03-02] setns into {ns} failed (dial)");
            return PongResult { count: None, observed_rst: true };
        }
        let server_addr = SocketAddrV4::new(peer_frontend, peer_port);
        // FAIL-FAST: bound the connect so a SYN with no SYN-ACK (a routing /
        // capture failure) returns a clear timeout in 10s instead of blocking
        // past nextest's reap. A real captured dial completes in <1ms.
        let mut tcp = match TcpStream::connect_timeout(
            &std::net::SocketAddr::V4(server_addr),
            Duration::from_secs(10),
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[03-02] dial connect {server_addr} failed: kind={:?} err={e}", e.kind());
                return PongResult { count: None, observed_rst: true };
            }
        };
        tcp.set_nodelay(true).ok();
        tcp.set_read_timeout(Some(Duration::from_secs(8))).ok();

        let mut observed_rst = tcp.write_all(request).and_then(|()| tcp.flush()).is_err();
        let mut got = Vec::new();
        if !observed_rst {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut buf = vec![0u8; 4096];
            // Read until a newline-terminated PONG line is in hand (the server
            // replies `PONG count=<n> date=<...>\n`) or the deadline / EOF.
            while !got.contains(&b'\n') && Instant::now() < deadline {
                match tcp.read(&mut buf) {
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
        PongResult { count: parse_pong_count(&got), observed_rst }
    })
    .join()
    .expect("netns dial thread")
}

// ============================================================================
// Fresh focused PKI (root → intermediate → leaf, rcgen + rustls) — the 02-02
// shape, extended: each mesh peer's leaf carries BOTH ClientAuth + ServerAuth.
// ============================================================================

struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: CertificateDer<'static>,
    spiffe: overdrive_core::SpiffeId,
    serial: CertSerial,
}

/// The TWO per-workload SVIDs (a + b). Each leaf carries BOTH `ClientAuth` AND
/// `ServerAuth` EKUs (the workload is BOTH a leg-B dialer and a leg-C server)
/// plus the `peer.overdrive.local` mesh-peer SNI SAN (leg-B verifies the dialed
/// peer against it).
struct TestPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    a_leaf: Leaf,
    b_leaf: Leaf,
}

impl TestPki {
    fn mint() -> Self {
        let root = MintedCa::mint_root("overdrive-dial-by-name-0302-ROOT-CA");
        let intermediate = root.mint_intermediate("overdrive-dial-by-name-0302-INTERMEDIATE-CA");

        // Each mesh peer is BOTH a leg-B client (dials its peer) AND a leg-C
        // server (is dialed by its peer). The production HostMtlsEnforcement
        // reads ONE svid_for(&alloc) for BOTH legs, so each leaf MUST carry
        // BOTH EKUs and the mesh-peer SNI SAN.
        let a_spiffe = "spiffe://overdrive.local/ns/default/sa/a";
        let b_spiffe = "spiffe://overdrive.local/ns/default/sa/b";
        let a_leaf = intermediate.mint_dual_role_leaf(a_spiffe, &[MESH_PEER_SNI]);
        let b_leaf = intermediate.mint_dual_role_leaf(b_spiffe, &[MESH_PEER_SNI]);

        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem,
            a_leaf,
            b_leaf,
        }
    }

    fn trust_bundle(&self) -> TrustBundle {
        TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        )
    }

    fn a_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.a_leaf)
    }

    fn b_svid_material(&self) -> SvidMaterial {
        svid_from_leaf(&self.b_leaf)
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

    /// Mint a leaf carrying BOTH `ClientAuth` AND `ServerAuth` EKUs — the
    /// dual-role mesh-peer leaf each of `a`/`b` needs (each is a leg-B client
    /// AND a leg-C server, reading ONE `svid_for(&alloc)` for both legs).
    fn mint_dual_role_leaf(&self, spiffe: &str, dns_sans: &[&str]) -> Leaf {
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
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
        ];
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

/// The agent's held-identity `IdentityRead` double — the ONLY holder of SVID
/// material (workloads hold nothing). ALLOC-AWARE by mesh id: the `a` alloc's
/// agent presents the dual-role `a` leaf on BOTH its leg-B (dialing b.svc) and
/// its leg-C (serving a.svc); the `b` alloc's agent presents the dual-role `b`
/// leaf likewise. (Unlike 02-02's `server`/`client` split, NEITHER alloc is
/// single-role here — each plays both, so each needs its own dual-EKU leaf.)
struct HeldMeshIdentities {
    a_svid: SvidMaterial,
    b_svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for HeldMeshIdentities {
    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        // The alloc id contains its `<job>` id ("a" / "b"). Match on the `/a/`
        // / `/b/` job segment to route the right dual-role leaf. (The bare
        // single-char match would over-match; the alloc id embeds `/job/<id>/`.)
        let id = alloc.as_str();
        if id.contains("/a/") || id.contains("-a-") {
            Some(self.a_svid.clone())
        } else if id.contains("/b/") || id.contains("-b-") {
            Some(self.b_svid.clone())
        } else {
            // Unknown alloc — fail-closed via None (AbsentSvid). Should not
            // happen: only `a` and `b` are deployed.
            None
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

        let identity: Arc<dyn IdentityRead> = Arc::new(HeldMeshIdentities {
            a_svid: pki.a_svid_material(),
            b_svid: pki.b_svid_material(),
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
            // future so a stalled task join during a live workload does not hang
            // to nextest's slow-test reap. The AllocCleanup guard reaps the
            // workloads after this returns. Test-only.
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
/// (`POST /v1/jobs` over the production HTTPS driving port). Returns `true` on a
/// 2xx accept.
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
        eprintln!("[03-02] deploy non-success: {status} {}", String::from_utf8_lossy(&body));
    }
    status.is_success()
}

/// Stop a deployed workload through the real in-process stop driving port
/// (`POST /v1/jobs/{id}/stop`). Drives `StopAllocation` → `worker.stop_alloc`
/// (which stops the per-alloc accept loops), the SAME path `overdrive job stop`
/// drives. Returns `true` on a 2xx accept.
async fn run_server_stop(skeleton: &Skeleton, workload_id: &str) -> bool {
    let url = format!("https://localhost:{}/v1/jobs/{workload_id}/stop", skeleton.bound.port());
    let resp = skeleton.client.post(&url).send().await.expect("stop: POST /v1/jobs/{id}/stop");
    let status = resp.status();
    let body = resp.bytes().await.expect("read stop response body");
    if !status.is_success() {
        eprintln!("[03-02] stop non-success: {status} {}", String::from_utf8_lossy(&body));
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
// the mesh-peer workload spec (a Service that is BOTH a listener AND, by living
// in a production netns with the egress rule, an egress dial source)
// ============================================================================

/// Build a Service spec whose exec driver launches a Python TCP server bound on
/// `0.0.0.0:self_port` inside its netns. The server BUMPS an inbound counter per
/// accepted connection and replies `PONG count=<n> date=<iso>` (NOT an echo) —
/// so a dialer's `count` strictly increasing across two dials proves THIS
/// server authored and sent the counted reply over the real S→C pipe.
///
/// The workload declares a `[[listener]]`, so `start_alloc` installs the inbound
/// leg-C rules for it once its `workload_addr` materialises — making it a
/// reachable mesh service. It is ALSO a dial source: the test drives a plaintext
/// connect from inside ITS netns (where `start_alloc` installed the egress
/// rule), so the same workload exercises BOTH the service (leg-C) and dialer
/// (leg-B) roles. (The autonomous `examples/dial-by-name-responder/ping_pong.py`
/// dial is what the BLACK-BOX E05 capture exercises; the in-process test drives
/// the dials deterministically, the 02-02 model, for CI reliability.)
fn mesh_peer_service_spec(workload_id: &str, self_port: u16) -> ServiceSpecInput {
    let server_script = format!(
        r"
import socket, datetime
inbound = 0
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('0.0.0.0', {self_port}))
s.listen(8)
while True:
    c, _ = s.accept()
    try:
        _ = c.recv(256)
        inbound += 1
        date = datetime.datetime.now().isoformat(timespec='seconds')
        c.sendall(b'PONG count=%d date=%s\n' % (inbound, date.encode()))
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
        listeners: vec![ListenerInput { port: self_port, protocol: "tcp".to_owned() }],
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    }
}

// ============================================================================
// inter-agent leg-B ↔ leg-C 0x17 confidentiality oracle (mirrors the proven
// dns_responder_walking_skeleton.rs technique: AF_PACKET capture on `lo`, walk
// TLS record framing, count 0x17 app-data records per direction, scan for
// cleartext markers). This is the EGRESS-path mTLS proof: the dialer speaks
// plaintext (terminates no TLS), so the encryption proof can only come from the
// segment the agent encrypts — the inter-agent leg-B ↔ leg-C hop.
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

/// The result of scanning the captured inter-agent wire on `wire_port`: how many
/// genuine `0x17` application_data records crossed in each direction, and how
/// many times the cleartext request/response marker appeared (MUST be 0 on the
/// encrypted leg).
#[derive(Debug, Clone, Copy, Default)]
struct WireScan {
    records_to_wire_port: u64,
    records_from_wire_port: u64,
    plaintext_marker_hits: u64,
}

impl WireScan {
    fn has_app_data(&self) -> bool {
        self.records_to_wire_port > 0 || self.records_from_wire_port > 0
    }
}

/// A live AF_PACKET/SOCK_RAW capture on `iface` that records every frame into a
/// buffer on a background thread until `stop_and_scan`. Filtered (at scan time)
/// to TCP frames touching `wire_port` (as src OR dst). Needs root + CAP_NET_RAW
/// (the Tier-3 root gate provides both).
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
        // The cleartext-marker count is a SECONDARY corroborating signal; the
        // LOAD-BEARING encryption proof is the directional 0x17 counts. The
        // marker counter only adds a "no request/response plaintext leaked onto
        // the encrypted stream" check, scoped to a TLS-bearing stream
        // (`records > 0`).
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

/// Dial `(peer_frontend, peer_port)` from inside `dialer_netns` (plaintext
/// workload dialer) AND capture the inter-agent leg-B ↔ leg-C hop on `lo` so the
/// caller can assert BOTH the byte-complete `PONG count` round-trip (PongResult)
/// AND that the inter-agent hop carried TLS-1.3 records with no cleartext
/// (WireScan). The capture is filtered on the PEER's service port — the orig
/// daddr/dport TPROXY preserves on the leg-B ↔ leg-C `lo` segment.
fn dial_peer_with_mtls_proof(
    dialer_netns: &str,
    peer_frontend: Ipv4Addr,
    peer_port: u16,
    request: &'static [u8],
) -> (PongResult, WireScan) {
    // Capture the inter-agent leg-B ↔ leg-C hop on the host's `lo` BEFORE the
    // dial, so the very first leg-B record is on the captured wire.
    let wire = WireCapture::start(LOOPBACK_IFACE, peer_port);
    let dial = dial_peer_in_netns(dialer_netns, peer_frontend, peer_port, request);
    // Brief settle so the last leg-B/leg-C app-data record is drained before the
    // capture stops (the round-trip already completed in `dial`).
    std::thread::sleep(Duration::from_millis(200));
    let scan = wire.stop_and_scan(request, b"PONG count=");
    (dial, scan)
}

/// Assert the inter-agent leg-B ↔ leg-C hop carried TLS-1.3 application_data
/// records in BOTH directions and NO cleartext request/PONG marker — the
/// EGRESS-path mTLS proof. Separate from (and asserted AFTER) the resolve +
/// counter assertions, per the K2 two-culprits honesty.
fn assert_inter_agent_hop_is_mtls(scan: &WireScan, scenario: &str, wire_port: u16) {
    assert!(
        scan.has_app_data(),
        "{scenario}: the inter-agent leg-B ↔ leg-C hop (captured on lo:{wire_port}) must carry \
         TLS-1.3 0x17 application_data records — proving the agent originated mTLS on leg-B and \
         terminated it on leg-C. A cleartext passthrough would show ZERO records. got {scan:?}"
    );
    assert!(
        scan.records_to_wire_port > 0,
        "{scenario}: the request direction (toward the backend) of the inter-agent hop must carry \
         0x17 records (the agent's leg-B encrypted the dialer's request). got {scan:?}"
    );
    assert!(
        scan.records_from_wire_port > 0,
        "{scenario}: the response direction (from the backend) of the inter-agent hop must carry \
         0x17 records (the peer's PONG rode back over leg-C kTLS). got {scan:?}"
    );
    assert_eq!(
        scan.plaintext_marker_hits, 0,
        "{scenario}: NO cleartext request/PONG marker may appear on the encrypted inter-agent \
         leg-B ↔ leg-C wire — a non-zero count means the agent passed the dialer's bytes through \
         in cleartext instead of encrypting them. got {scan:?}"
    );
}

// ============================================================================
// back-door observation reads (no production path exercised by these helpers)
// ============================================================================

/// `Some(addr)` ⇔ the `<job>`'s `service_backends` row currently advertises a
/// HEALTHY backend whose addr is a per-instance mesh workload_addr ∈
/// `10.99.0.0/16` (NOT the `host_ipv4` fallback). This is the precondition for
/// the dial-by-name loop: the re-keyed `MtlsResolve` translates `F` → this
/// backend, so it must be the routable per-instance addr.
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

/// `Some(())` ⇔ the workload has ≥1 Terminated row and NO Running row — the stop
/// converged (the accept loops stopped). Polled before `shutdown()` so the
/// accept-loop threads are actually stopped before the runtime drops.
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

/// Deploy a mesh-peer Service and wait until the bridge advertises a STABLE
/// per-instance backend addr ∈ `10.99.0.0/16` in the `service_backends` row —
/// i.e. the alloc is a settled Path-A mesh alloc whose canonical `workload_addr`
/// the bridge reads. Returns that backend addr (used to derive the netns).
async fn deploy_and_wait_stable_backend(
    skeleton: &Skeleton,
    workload_id: &str,
    self_port: u16,
) -> Ipv4Addr {
    let submitted =
        run_server_deploy(skeleton, mesh_peer_service_spec(workload_id, self_port)).await;
    assert!(submitted, "the {workload_id} Service spec must be accepted by the deploy handler");
    let addr = poll_until(Duration::from_secs(30), Duration::from_millis(250), || {
        let obs = skeleton.obs();
        let id = workload_id.to_owned();
        async move { stable_mesh_backend_addr(&obs, &id).await }
    })
    .await;
    addr.unwrap_or_else(|| {
        panic!(
            "S-DBN-PINGPONG: the {workload_id} workload must reach a settled Path-A mesh alloc \
             whose service_backends row advertises a per-instance workload_addr ∈ 10.99.0.0/16 \
             within 30s (the re-keyed MtlsResolve translates F → this addr; a host_ipv4 fallback \
             would not reach the in-netns listener)"
        )
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
// S-DBN-PINGPONG — two services dial each other by name; counters advance; each
// hop mTLS'd. The in-process bidirectional proof (review-03-02.md resolution a).
// ============================================================================

/// S-DBN-PINGPONG (US-DBN-3 · K-DBN-3) — two services dial each other by name;
/// counters advance; each hop is mTLS'd.
///
/// ONE `run_server_with_obs_and_driver` boot; deploys `a` and `b` (each a
/// Service whose Python server bumps an inbound counter + replies `PONG
/// count=<n>`) via the production deploy port (`POST /v1/jobs`, two deploys);
/// each reaches Running-AND-HEALTHY → the bridge writes a healthy
/// `service_backends` row → the responder's `name_index` exposes the `<job>`
/// bound a stable `F ∈ 10.98.0.0/16`. Then, deterministically (the 02-02 model):
///
/// - from `a`'s PRODUCTION netns: `getent b.svc.overdrive.local` → `F_b ∈
///   10.98/16` (NOT a `10.99/16` backend addr — asserted SEPARATELY); a
///   PLAINTEXT dial to `(F_b, B_PORT)` twice, asserting `b`'s returned `count`
///   STRICTLY INCREASES (its inbound counter advanced over the real reply pipe);
///   the inter-agent leg-B ↔ leg-C hop captured on `lo:B_PORT` carries 0x17
///   records both directions with zero cleartext.
/// - symmetrically from `b`'s netns for `a`.
///
/// The genuinely-new composition (review-03-02.md): each of `a`/`b` is
/// SIMULTANEOUSLY a mesh service (inbound leg-C + frontend `F`) AND an egress
/// dialer (leg-B) — ×2 on one boot sharing the fixed `ovd-veth-cli` /
/// `ovd-veth-bk` host routing point. Each alloc's SINGLE SVID carries BOTH
/// ClientAuth + ServerAuth (the production enforcement reads one
/// `svid_for(&alloc)` for both legs). NO test binds `:53`, allocates `F`,
/// installs resolv.conf, programs a map, or hand-installs the egress/inbound
/// capture — production does all of it.
///
/// CORRECTED TEST MODEL (02-02 RCA): the dialer speaks PLAINTEXT (the EGRESS
/// capture lands on the agent's plaintext leg-F); the mTLS is proven on the
/// inter-agent leg-B ↔ leg-C hop via the `lo:<peer port>` 0x17 oracle (see the
/// file-level docstring + `assert_inter_agent_hop_is_mtls`).
///
/// MERGE-BLOCKING on the pinned-6.18 Tier-3 matrix (ADR-0068); `record_kernel()`
/// records `uname -r` (the 6.18 assertion is the merge-gate matrix's job, not an
/// in-test assert — dev-Lima is 7.0, criterion 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn two_services_dial_each_other_by_name_counters_advance_each_hop_mtls() {
    if !is_root() {
        eprintln!(
            "SKIP two_services_dial_each_other_by_name_counters_advance_each_hop_mtls: not root"
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

    // Deploy BOTH mesh peers. Each is a Service (declares a [[listener]] → the
    // inbound leg-C rules) AND a dial source (its netns carries the egress
    // rule). Each reaches Running with a per-instance backend addr ∈ 10.99/16;
    // the bridge writes a healthy service_backends row; the responder's
    // name_index exposes each <job> bound a stable F.
    let a_backend = deploy_and_wait_stable_backend(&skeleton, SERVICE_A_ID, SERVICE_A_PORT).await;
    let b_backend = deploy_and_wait_stable_backend(&skeleton, SERVICE_B_ID, SERVICE_B_PORT).await;
    let a_netns = netns_name_for_workload_addr(a_backend);
    let b_netns = netns_name_for_workload_addr(b_backend);
    eprintln!(
        "[03-02] a backend = {a_backend} (netns {a_netns}); b backend = {b_backend} (netns {b_netns})"
    );

    // Settle: a Running row precedes the per-alloc mTLS intercept install (each
    // peer's egress nft-TPROXY capture + leg-F listener, and its leg-C accept
    // loop) by a short window. Dialing before both legs are live races a fast
    // handshake failure. The sibling S-DBN tests settle 500ms; match them.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ---- DIRECTION 1: a dials b.svc.overdrive.local ------------------------

    // (1a) RESOLVE — getent from inside a's PRODUCTION netns must answer b's
    //      STABLE frontend F_b ∈ 10.98/16 within the K2 budget. Asserted FIRST
    //      and SEPARATELY from the mTLS assertion (K2 two-culprits honesty).
    let ns = a_netns.clone();
    let f_b = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, PEER_NAME_FROM_A, RESOLVE_BUDGET)
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| {
        panic!(
            "S-DBN-PINGPONG (a→b): getent({PEER_NAME_FROM_A}) from inside a's netns must resolve \
             to b's STABLE frontend F ∈ 10.98.0.0/16 within {RESOLVE_BUDGET:?} (the production \
             responder bound :53, source-pinned the reply, and the name_index exposed b after \
             running-and-healthy). A timeout means EITHER the source-pin is missing OR the \
             healthy-gate regressed (K2 two culprits)."
        )
    });
    assert_frontend_subnet(f_b, "S-DBN-PINGPONG (a→b)", b_backend);
    eprintln!("[03-02] a resolved b's STABLE frontend F_b = {f_b}");

    // (1b) DIAL ×2 — plaintext connect from a's netns to (F_b, B_PORT); b's
    //      inbound counter must STRICTLY INCREASE across the two dials (proving
    //      the real b→a reply pipe carried b's counted PONG, not an echo). The
    //      inter-agent leg-B↔leg-C hop is captured for the mTLS proof.
    let (a_to_b_1, scan_a_to_b) = {
        let ns = a_netns.clone();
        tokio::task::spawn_blocking(move || {
            dial_peer_with_mtls_proof(&ns, f_b, SERVICE_B_PORT, REQUEST_A_TO_B)
        })
        .await
        .expect("a→b dial-1 task join")
    };
    let a_to_b_2 = {
        let ns = a_netns.clone();
        tokio::task::spawn_blocking(move || {
            dial_peer_in_netns(&ns, f_b, SERVICE_B_PORT, REQUEST_A_TO_B)
        })
        .await
        .expect("a→b dial-2 task join")
    };
    assert_counter_advances(&a_to_b_1, &a_to_b_2, "S-DBN-PINGPONG (a→b)");
    eprintln!("[03-02] a→b inter-agent leg-B↔leg-C wire scan = {scan_a_to_b:?}");
    assert_inter_agent_hop_is_mtls(&scan_a_to_b, "S-DBN-PINGPONG (a→b)", SERVICE_B_PORT);

    // ---- DIRECTION 2: b dials a.svc.overdrive.local ------------------------

    let ns = b_netns.clone();
    let f_a = tokio::task::spawn_blocking(move || {
        poll_resolve_frontend(&ns, PEER_NAME_FROM_B, RESOLVE_BUDGET)
    })
    .await
    .expect("resolve task join")
    .unwrap_or_else(|| {
        panic!(
            "S-DBN-PINGPONG (b→a): getent({PEER_NAME_FROM_B}) from inside b's netns must resolve \
             to a's STABLE frontend F ∈ 10.98.0.0/16 within {RESOLVE_BUDGET:?}"
        )
    });
    assert_frontend_subnet(f_a, "S-DBN-PINGPONG (b→a)", a_backend);
    eprintln!("[03-02] b resolved a's STABLE frontend F_a = {f_a}");

    let (b_to_a_1, scan_b_to_a) = {
        let ns = b_netns.clone();
        tokio::task::spawn_blocking(move || {
            dial_peer_with_mtls_proof(&ns, f_a, SERVICE_A_PORT, REQUEST_B_TO_A)
        })
        .await
        .expect("b→a dial-1 task join")
    };
    let b_to_a_2 = {
        let ns = b_netns.clone();
        tokio::task::spawn_blocking(move || {
            dial_peer_in_netns(&ns, f_a, SERVICE_A_PORT, REQUEST_B_TO_A)
        })
        .await
        .expect("b→a dial-2 task join")
    };
    assert_counter_advances(&b_to_a_1, &b_to_a_2, "S-DBN-PINGPONG (b→a)");
    eprintln!("[03-02] b→a inter-agent leg-B↔leg-C wire scan = {scan_b_to_a:?}");
    assert_inter_agent_hop_is_mtls(&scan_b_to_a, "S-DBN-PINGPONG (b→a)", SERVICE_A_PORT);

    eprintln!(
        "[03-02] VERDICT: WORKS — a and b each resolved the OTHER by name to its stable frontend \
         (F_b={f_b}, F_a={f_a}), both inbound counters advanced, and BOTH hops are mTLS'd on the \
         inter-agent leg, driven through in-process run_server + two deploys on the REAL \
         EbpfDataplane (each workload BOTH a mesh service AND an egress dialer), on kernel {kr}. \
         (MERGE GATE: pinned-6.18 Tier-3 matrix, ADR-0068.)"
    );

    // Stop BOTH workloads through the production stop path BEFORE shutdown so the
    // accept-loop threads (each peer's leg-C + leg-F) are actually stopped, not
    // timed-out-around: a live alloc's accept loop survives the in-process
    // `Runtime::drop` and hangs teardown to nextest's ~120s reap.
    stop_and_converge(&skeleton, SERVICE_A_ID).await;
    stop_and_converge(&skeleton, SERVICE_B_ID).await;
    skeleton.shutdown().await;
}

/// Assert the resolved `frontend` is the STABLE frontend F ∈ 10.98.0.0/16 and
/// is byte-DISTINCT from the peer's per-instance backend addr ∈ 10.99.0.0/16
/// (the stable-frontend split — DNS answers F, NOT the backend addr; the SQ1
/// guard).
fn assert_frontend_subnet(frontend: Ipv4Addr, scenario: &str, peer_backend: Ipv4Addr) {
    let o = frontend.octets();
    assert_eq!(
        (o[0], o[1]),
        (FRONTEND_FIRST_OCTET, FRONTEND_SECOND_OCTET),
        "{scenario}: getent must resolve to the STABLE frontend F ∈ 10.98.0.0/16 (got {frontend}), \
         NEVER a per-instance backend addr ∈ 10.99.0.0/16",
    );
    assert_ne!(
        o[1], WORKLOAD_SECOND_OCTET,
        "{scenario}: getent must NOT answer a per-instance backend addr ∈ 10.99.0.0/16 (got {frontend})",
    );
    assert_ne!(
        frontend, peer_backend,
        "{scenario}: the answered F (the frontend) must be byte-DISTINCT from the peer's \
         per-instance backend addr {peer_backend} — the stable-frontend split means DNS answers F",
    );
}

/// Assert two successive dials to the same peer landed it (no RST, a count read)
/// and the peer's inbound `count` STRICTLY INCREASED — proving the real
/// peer→caller reply pipe carried the peer-authored, advancing counter (not an
/// echo, not a stale cache). This IS the "counters advance" / bidirectional-loop
/// -is-live proof for this direction.
fn assert_counter_advances(first: &PongResult, second: &PongResult, scenario: &str) {
    assert!(
        !first.observed_rst && !second.observed_rst,
        "{scenario}: neither dial to the resolved F may observe a transport RST — the agent leg \
         must terminate cleanly and the round-trip complete (first.rst={}, second.rst={})",
        first.observed_rst,
        second.observed_rst,
    );
    let c1 = first.count.unwrap_or_else(|| {
        panic!(
            "{scenario}: the first PLAINTEXT dial must read the peer's `PONG count=<n>` reply \
             byte-complete — proving the connect to the resolved F was captured by the PRODUCTION \
             egress nft-TPROXY rule on the agent's plaintext leg-F, the re-keyed MtlsResolve \
             translated F → the peer's live backend, the agent originated mTLS on leg-B → leg-C, \
             and the peer's counted PONG rode back. Removing the production responder spawn \
             (getent times out earlier) or the by_frontend translation arm (connect to F \
             fail-closes) takes this RED."
        )
    });
    let c2 = second.count.unwrap_or_else(|| {
        panic!("{scenario}: the second PLAINTEXT dial must also read a `PONG count=<n>` reply")
    });
    assert!(
        c2 > c1,
        "{scenario}: the peer's inbound counter must STRICTLY INCREASE across two dials (proving \
         the live bidirectional loop reaches the peer and its counter advances over the real reply \
         pipe, not an echo) — got count {c1} then {c2}",
    );
    eprintln!("[03-02] {scenario}: peer inbound counter advanced {c1} -> {c2}");
}
