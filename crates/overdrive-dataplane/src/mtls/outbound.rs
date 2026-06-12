//! OUTBOUND enforcement mechanism (`Direction::Outbound`) + the probe sentinels.
//!
//! The agent owns leg F (workload-facing plaintext, `accept()`ed off the
//! `cgroup_connect4`-rewrite intercept) and dials leg B (peer-facing kTLS). This
//! productionises increment-e + increment-f wired together for the first time:
//! lossless leg-F capture → rustls CLIENT handshake on leg B presenting the held
//! SVID → SOCKMAP-insert-before-TCP_ULP → arm kTLS → flush pre-arm plaintext →
//! install the forward sockmap EGRESS-redirect (agent-idle) → start the return
//! splice pump.
//!
//! Runs on a blocking task (synchronous rustls + raw setsockopt + aya load).

// `leg_f`/`leg_b` (and the fd-suffixed locals) are the canonical leg names from
// the spike findings and ADR-0069 — deliberately parallel; renaming them to
// satisfy `similar_names` would lose the contract vocabulary.
#![allow(
    clippy::similar_names,
    clippy::cast_possible_truncation,
    reason = "leg F/B names are the ADR-0069 contract vocabulary; the FFI size casts are compile-time-constant struct widths (raw syscall glue)"
)]

use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use aya::Ebpf;
use aya::maps::{Array, SockMap};
use aya::programs::SkSkb;
use overdrive_core::AllocationId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    MtlsEnforcementError, MtlsLimits, ProbeSentinel, Result,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, ExtractedSecrets};

use super::splice::PumpHandle;
use super::{ConnState, ktls, tls_config};

/// The forward sockmap slot indices (mirror the kernel-side `MTLS_SOCKMAP`).
const F_IDX: u32 = 0;
const B_IDX: u32 = 1;

/// Establish OUTBOUND steady-state. Consumes `leg_f` (owned); dials leg B; returns
/// a [`ConnState`] holding both legs + the return splice pump.
pub(super) fn establish(
    leg_f: OwnedFd,
    peer: SocketAddrV4,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    limits: MtlsLimits,
    bpf_obj: &'static [u8],
) -> Result<ConnState> {
    let leg_f_fd = leg_f.as_raw_fd();

    // 1. Lossless pre-arm capture off leg F (bounded by max_prearm_bytes).
    let held = super::drain_prearm(
        leg_f_fd,
        limits.max_prearm_bytes,
        alloc,
        std::time::Duration::from_millis(250),
    )?;

    // 2. Dial leg B (the agent's own dial — cgroup-scoped exempt from the workload
    //    cgroup_connect4 intercept, F5).
    let leg_b = super::dial_leg(peer, limits.handshake_deadline)?;
    let leg_b_fd = leg_b.as_raw_fd();

    // 3. Load the BPF + insert leg B into MTLS_SOCKMAP BEFORE TCP_ULP "tls"
    //    (the arming invariant, D-MTLS-7). The sk_skb verdict is attached here.
    let mut forward = ForwardRedirect::load(bpf_obj)?;
    forward.insert_leg_b(leg_b_fd)?;

    // 4. rustls CLIENT handshake on leg B presenting the held SVID; verify the
    //    peer against the trust bundle.
    let secrets = client_handshake(leg_b, svid, bundle, limits.handshake_deadline)?;

    // 5. Arm kTLS on leg B from the extracted secrets (the sockmap insert in step
    //    3 preceded the ULP install this performs — the arming invariant, D-MTLS-7).
    ktls::arm_ktls_tx_rx(leg_b_fd, secrets)?;

    // 6. Flush the captured pre-arm plaintext through leg B (kTLS encrypts it as
    //    the first application_data).
    super::flush_through(leg_b_fd, &held)?;

    // 6b. Re-drain leg F's recv queue for any plaintext that arrived DURING the
    //     handshake-and-arm window (after the step-1 pre-arm drain ended but before
    //     the redirect is installed) and flush it through leg B. This closes the
    //     lossless-capture window: every byte present on leg F before the redirect
    //     engages is captured by userspace; only bytes arriving AFTER the redirect
    //     is live ride the agent-idle egress path. Without this, an arm-window byte
    //     would sit in leg F's recv queue (the strparser only redirects NEW skbs
    //     after it engages) and be lost — the source of the intermittent
    //     forward-delivery miss. Draining BEFORE the sockmap insert avoids racing
    //     the strparser.
    let arm_window = super::drain_leg_to_empty(leg_f_fd, limits.max_prearm_bytes, alloc)?;
    super::flush_through(leg_b_fd, &arm_window)?;

    // 7. Install the forward egress-redirect: insert leg F (slot 0), set FPORT to
    //    leg F's host-order local port, flip ARMED=1 → agent-idle forward.
    let leg_f_port = super::local_port(leg_f_fd)?;
    forward.install_forward(leg_f_fd, leg_f_port)?;

    // 7b. Settle: give the kernel a beat to wire leg F's sk_psock/strparser ingress
    //     path after the sockmap insert, so the FIRST steady-state skb is parsed by
    //     the stream_verdict (not delivered to the recv queue before the parser is
    //     live — the "invocations=0" race; `findings-egress-ktls-splice.md`).
    std::thread::sleep(std::time::Duration::from_millis(80));

    // 8. Start the return splice pump (legB → pipe → legF) on the plain kTLS-RX
    //    leg B. `liveness` reports Running.
    let pump = PumpHandle::spawn(leg_b_fd, leg_f_fd, super::now_unix_nanos());

    // The forward-redirect BPF (sockmap membership + verdict) must outlive the
    // connection for its steady-state life; leak it (single-flow walking skeleton —
    // per-connection BPF lifecycle is a later-slice concern). Teardown closes the
    // legs; the BPF maps are reclaimed at process exit.
    forward.persist();

    // Detach leg-B ownership into the ConnState. `client_handshake` already
    // suppressed the leg-B `TcpStream`'s Drop (`forget`), so reconstructing an
    // `OwnedFd` from `leg_b_fd` here is the SINGLE owner — no double-close.
    // SAFETY: `leg_b_fd` has exactly one live owner (the forgotten stream's fd);
    // wrapping it in an `OwnedFd` transfers that sole ownership to the ConnState.
    let leg_b_owned = unsafe { OwnedFd::from_raw_fd(leg_b_fd) };
    Ok(super::new_conn_state(vec![leg_f, leg_b_owned], pump))
}

/// Drive the rustls CLIENT handshake on `leg_b` (an owned `TcpStream`), then
/// extract the secrets for the kTLS arm. The stream's Drop is suppressed
/// (`forget`) so the leg-B fd stays open for the arm + splice.
fn client_handshake(
    leg_b: TcpStream,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    deadline: std::time::Duration,
) -> Result<ExtractedSecrets> {
    let cfg = tls_config::client_config(svid, bundle)?;
    let mut tcp = leg_b;
    tcp.set_read_timeout(Some(deadline)).ok();
    // The peer's server cert is verified against the bundle; the SNI is the
    // workload's intended peer name. v1 is single-node + authn-only, so use a
    // fixed sentinel name that the test peer presents a SAN for.
    let sni = ServerName::try_from("peer.overdrive.local".to_string())
        .map_err(|e| MtlsEnforcementError::HandshakeFailed { reason: format!("SNI: {e}") })?;
    let mut conn = ClientConnection::new(cfg, sni).map_err(|e| {
        MtlsEnforcementError::HandshakeFailed { reason: format!("ClientConnection: {e}") }
    })?;
    drive_handshake_client(&mut conn, &mut tcp)?;
    let secrets = conn.dangerous_extract_secrets().map_err(|e| {
        MtlsEnforcementError::HandshakeFailed { reason: format!("extract secrets: {e}") }
    })?;
    std::mem::forget(tcp); // keep the leg-B fd open for the kTLS arm + splice
    Ok(secrets)
}

fn drive_handshake_client(conn: &mut ClientConnection, tcp: &mut TcpStream) -> Result<()> {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            conn.write_tls(tcp).map_err(|e| MtlsEnforcementError::HandshakeFailed {
                reason: format!("write_tls: {e}"),
            })?;
        }
        if !conn.is_handshaking() {
            return Ok(());
        }
        match conn.read_tls(tcp) {
            Ok(0) => {
                return Err(MtlsEnforcementError::HandshakeFailed {
                    reason: "EOF during handshake (peer closed)".into(),
                });
            }
            Ok(_) => {
                conn.process_new_packets().map_err(|e| {
                    MtlsEnforcementError::PeerVerificationFailed { reason: format!("{e}") }
                })?;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => {
                return Err(MtlsEnforcementError::HandshakeFailed {
                    reason: format!("read_tls: {e}"),
                });
            }
        }
    }
}

/// The loaded forward-redirect BPF: the `MTLS_SOCKMAP` + `MTLS_FPORT` /
/// `MTLS_ARMED` maps and the attached `sk_skb_stream_verdict_mtls` verdict.
struct ForwardRedirect {
    bpf: Option<Ebpf>,
    sockmap: Option<SockMap<aya::maps::MapData>>,
}

impl ForwardRedirect {
    /// Load the BPF object (via the shared `pinning = ByName` `SERVICE_MAP`
    /// loader — aya 0.13.x cannot create the phase-2 HoM that shares this ELF from
    /// the ELF alone), take `MTLS_SOCKMAP`, and attach the verdict to it.
    fn load(bpf_obj: &'static [u8]) -> Result<Self> {
        let mut bpf = super::bpf_load::load_shared_bpf(bpf_obj).map_err(|e| {
            MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("load bpf: {e}")),
            }
        })?;

        let sockmap: SockMap<_> =
            SockMap::try_from(bpf.take_map("MTLS_SOCKMAP").ok_or_else(|| {
                MtlsEnforcementError::ForwardRedirectFailed {
                    source: std::io::Error::other("MTLS_SOCKMAP not found in BPF object"),
                }
            })?)
            .map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("MTLS_SOCKMAP: {e}")),
            })?;
        let sockmap_fd =
            sockmap.fd().try_clone().map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("sockmap fd clone: {e}")),
            })?;
        let prog: &mut SkSkb = bpf
            .program_mut("sk_skb_stream_verdict_mtls")
            .ok_or_else(|| MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other("sk_skb_stream_verdict_mtls program not found"),
            })?
            .try_into()
            .map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("program try_into: {e}")),
            })?;
        prog.load().map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
            source: std::io::Error::other(format!("verdict load: {e}")),
        })?;
        prog.attach(&sockmap_fd).map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
            source: std::io::Error::other(format!("verdict attach: {e}")),
        })?;
        Ok(Self { bpf: Some(bpf), sockmap: Some(sockmap) })
    }

    /// Insert leg B into the sockmap (slot 1) BEFORE the kTLS arm — the ordering
    /// invariant (D-MTLS-7). This insert is pre-ULP by construction (the arm runs
    /// after), so a failure here is a forward-redirect install failure, not the
    /// arming-order violation (that is the ULP-after-sockmap EINVAL the probe and
    /// `ktls::arm_ktls_tx_rx` detect).
    fn insert_leg_b(&mut self, leg_b_fd: RawFd) -> Result<()> {
        let sockmap = self
            .sockmap
            .as_mut()
            .unwrap_or_else(|| unreachable!("sockmap is Some between load() and persist()"));
        sockmap.set(B_IDX, &leg_b_fd, 0).map_err(|e| {
            MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("sockmap insert leg B: {e}")),
            }
        })?;
        Ok(())
    }

    /// Set FPORT to leg F's host-order local port + flip ARMED=1, THEN insert leg F
    /// (slot 0) into the sockmap LAST → the verdict EGRESS-redirects leg F's RX into
    /// leg B's kTLS TX. The order is load-bearing: the sk_skb verdict's strparser
    /// engages the instant leg F joins the sockmap and fires on the first skb, so
    /// FPORT + ARMED MUST already be set — otherwise the first redirectable skb sees
    /// `fport == 0` (→ SK_PASS, stranded in the recv queue) or `armed == 0` (→
    /// SK_DROP), losing a steady-state byte. Setting the maps before the membership
    /// closes that window (the intermittent forward-delivery miss).
    fn install_forward(&mut self, leg_f_fd: RawFd, leg_f_port: u16) -> Result<()> {
        let bpf = self
            .bpf
            .as_mut()
            .unwrap_or_else(|| unreachable!("bpf is Some between load() and persist()"));
        let mut fport: Array<_, u32> = Array::try_from(
            bpf.map_mut("MTLS_FPORT")
                .unwrap_or_else(|| unreachable!("MTLS_FPORT is in the loaded BPF object")),
        )
        .map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
            source: std::io::Error::other(format!("MTLS_FPORT: {e}")),
        })?;
        fport.set(0, u32::from(leg_f_port), 0).map_err(|e| {
            MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("set FPORT: {e}")),
            }
        })?;
        let mut armed: Array<_, u32> = Array::try_from(
            bpf.map_mut("MTLS_ARMED")
                .unwrap_or_else(|| unreachable!("MTLS_ARMED is in the loaded BPF object")),
        )
        .map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
            source: std::io::Error::other(format!("MTLS_ARMED: {e}")),
        })?;
        armed.set(0, 1u32, 0).map_err(|e| MtlsEnforcementError::ForwardRedirectFailed {
            source: std::io::Error::other(format!("set ARMED: {e}")),
        })?;
        // Membership LAST — now the verdict sees correct FPORT + ARMED on the first
        // skb the strparser delivers.
        let sockmap = self
            .sockmap
            .as_mut()
            .unwrap_or_else(|| unreachable!("sockmap is Some between load() and persist()"));
        sockmap.set(F_IDX, &leg_f_fd, 0).map_err(|e| {
            MtlsEnforcementError::ForwardRedirectFailed {
                source: std::io::Error::other(format!("sockmap insert leg F: {e}")),
            }
        })?;
        Ok(())
    }

    /// Leak the loaded BPF so the sockmap membership + verdict outlive the
    /// connection for its life (the steady-state forward path). Teardown closes
    /// the legs; the BPF maps are reclaimed at process exit (single-flow walking
    /// skeleton — per-connection BPF lifecycle is a later-slice concern).
    fn persist(&mut self) {
        if let Some(bpf) = self.bpf.take() {
            std::mem::forget(bpf);
        }
        if let Some(sockmap) = self.sockmap.take() {
            std::mem::forget(sockmap);
        }
    }
}

/// Earned-Trust probe (`MtlsEnforcement::probe`; D-MTLS-11; contract postcondition
/// (1)/(2)/(3)). Exercises the three catalogued substrate lies on a loopback
/// sentinel BEFORE any connection is enforced and tears the sentinel state down
/// before return. Per SD-5 / D-MTLS-12 (user-approved 2026-06-12) the sentinel
/// handshake uses an EPHEMERAL THROWAWAY self-signed cert minted in-process via
/// `rcgen` — substrate-self-test crypto, signed by neither CA, never in the trust
/// bundle, never on a real wire (loopback agent-to-itself only); #26 stays a
/// READER, NOT an issuer.
///
/// The three sentinels (1:1 with [`ProbeSentinel`]):
/// (1) **kTLS arm + forward egress-redirect round-trip** (composed — exercises BOTH
///     postcondition (1) and (2) in one loopback flow, exactly as the real OUTBOUND
///     `establish` does): a sentinel rustls TLS 1.3 handshake on a loopback leg B
///     arms kTLS; a sentinel byte written to a loopback leg F emerges ENCRYPTED on
///     leg B's wire via the sockmap EGRESS redirect (`flags=0`), the sentinel peer
///     decrypts it via kTLS-RX and a single `tls_sw_splice_read` returns the exact
///     sentinel plaintext (`findings.md` A / `findings-egress-ktls-splice.md`).
/// (3) **the arming invariant** — SOCKMAP insert MUST precede `TCP_ULP "tls"`; the
///     reverse ordering is observed to return `EINVAL` (`findings.md` D / D-MTLS-7).
pub(super) fn run_probe_sentinels(bpf_obj: &'static [u8]) -> Result<()> {
    // Install the ring `CryptoProvider` as the process default before any rustls
    // handshake (the substrate the proxy's TLS legs run on). `probe` is the
    // wire→probe→use entry point called once at node startup; `.ok()` is idempotent
    // (a second install returns Err, harmless). This arms the substrate the
    // contract probes — it is not "shaping production by simulation".
    let _ = rustls::crypto::ring::default_provider().install_default();
    probe_arm_and_forward_round_trip(bpf_obj)?;
    probe_arming_order_einval_raw()?;
    Ok(())
}

/// A throwaway self-signed sentinel cert+key for the loopback self-test handshake
/// (SD-5 / D-MTLS-12). Minted in-process, used ONLY agent-to-itself over loopback,
/// dropped before `probe` returns. Identifies no workload; never in the bundle.
fn sentinel_cert() -> std::result::Result<
    (rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>),
    String,
> {
    let params = rcgen::CertificateParams::new(vec!["sentinel.overdrive.invalid".to_string()])
        .map_err(|e| format!("sentinel cert params: {e}"))?;
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("sentinel key: {e}"))?;
    let cert = params.self_signed(&key).map_err(|e| format!("sentinel self_signed: {e}"))?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der())
        .map_err(|e| format!("sentinel key der: {e}"))?;
    Ok((cert_der, key_der))
}

/// Sentinel (1)+(2): the full composed kTLS-arm + forward egress-redirect
/// round-trip on a loopback sentinel. Mirrors the real OUTBOUND `establish`:
/// dial a loopback sentinel peer (leg B), SOCKMAP-insert leg B, rustls CLIENT
/// handshake, arm kTLS, install the forward F→B egress-redirect, write a sentinel
/// byte into leg F — and the sentinel peer (kTLS-RX) MUST receive the exact
/// sentinel plaintext (proving the byte rode the redirect → leg B's kTLS TX → the
/// peer's kTLS-RX, ENCRYPTED on the wire). Any failure ⇒ `Probe`.
fn probe_arm_and_forward_round_trip(bpf_obj: &'static [u8]) -> Result<()> {
    const SENTINEL: &[u8] =
        b"OVERDRIVE_MTLS_PROBE_SENTINEL_forward_egress_redirect_ktls_roundtrip_0001";

    let outcome = (|| -> std::result::Result<(), String> {
        // 1. Sentinel peer: a loopback TLS 1.3 server that arms kTLS-RX and reads
        //    the sentinel plaintext. Runs on its own thread.
        let (cert, key) = sentinel_cert()?;
        let peer_listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("peer bind: {e}"))?;
        let peer_addr = peer_listener.local_addr().map_err(|e| format!("peer addr: {e}"))?;
        let peer = std::thread::spawn(move || -> std::result::Result<Vec<u8>, String> {
            sentinel_peer_recv(&peer_listener, cert, key, SENTINEL.len())
        });

        // 2. Leg F: a self-connected loopback pair the agent owns. The ACCEPT end
        //    (f_target) goes into the sockmap (slot 0); the CONNECT end (f_writer)
        //    is where the sentinel byte is pushed.
        let f_listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("legF bind: {e}"))?;
        let f_addr = f_listener.local_addr().map_err(|e| format!("legF addr: {e}"))?;
        let f_writer =
            std::net::TcpStream::connect(f_addr).map_err(|e| format!("legF connect: {e}"))?;
        f_writer.set_nodelay(true).ok();
        let (f_target, _) = f_listener.accept().map_err(|e| format!("legF accept: {e}"))?;
        f_target.set_nodelay(true).ok();
        let f_target_fd = f_target.as_raw_fd();
        let f_target_port = f_target.local_addr().map_err(|e| format!("legF local: {e}"))?.port();

        // 3. Leg B: dial the sentinel peer; SOCKMAP-insert BEFORE TCP_ULP (the
        //    arming invariant), rustls CLIENT handshake, arm kTLS.
        let leg_b =
            std::net::TcpStream::connect(peer_addr).map_err(|e| format!("legB connect: {e}"))?;
        leg_b.set_nodelay(true).ok();
        let leg_b_fd = leg_b.as_raw_fd();

        let mut forward = ForwardRedirect::load(bpf_obj).map_err(|e| format!("load bpf: {e}"))?;
        forward.insert_leg_b(leg_b_fd).map_err(|e| format!("sockmap insert leg B: {e}"))?;

        let secrets = sentinel_client_handshake(leg_b)?;
        ktls::arm_ktls_tx_rx(leg_b_fd, secrets).map_err(|e| format!("kTLS arm: {e}"))?;

        // 4. Install the forward egress-redirect (slot 0 = leg F, FPORT, ARMED=1).
        forward
            .install_forward(f_target_fd, f_target_port)
            .map_err(|e| format!("install forward: {e}"))?;

        // Settle: give the kernel a beat to wire leg F's sk_psock/strparser ingress
        // path after the sockmap insert, so the FIRST sentinel skb is parsed by the
        // stream_verdict (not delivered to the recv queue before the parser is live
        // — the "invocations=0" race; `findings-egress-ktls-splice.md`).
        std::thread::sleep(std::time::Duration::from_millis(80));

        // 5. Push the sentinel byte into leg F's CONNECT end; it arrives on
        //    f_target's RX → the verdict EGRESS-redirects it into leg B's kTLS TX.
        let mut w = &f_writer;
        w.write_all(SENTINEL).map_err(|e| format!("legF write: {e}"))?;
        w.flush().map_err(|e| format!("legF flush: {e}"))?;

        // 6. The sentinel peer (kTLS-RX) must reconstruct the exact sentinel.
        let got = peer.join().map_err(|_| "sentinel peer thread panicked".to_string())??;
        // Sentinel teardown (contract: "leaks no sentinel state"). `leg_b`'s
        // `TcpStream` was `forget`'d inside `sentinel_client_handshake` to keep the
        // fd open for the kTLS arm — so close `leg_b_fd` by raw fd here. `f_target`
        // / `f_writer` are still owned, dropping them closes their fds; dropping
        // `forward` (the BPF object + sockmap) reclaims the maps. The round-trip is
        // complete, so there is no in-flight redirect to disturb.
        drop(f_writer);
        drop(f_target);
        // SAFETY: `leg_b_fd` is the sole live owner (the handshake forgot the
        // stream); closing it here reclaims the kTLS leg.
        unsafe { libc::close(leg_b_fd) };
        drop(forward);
        if got == SENTINEL {
            Ok(())
        } else {
            Err(format!(
                "forward-redirect round-trip mismatch: peer reconstructed {} of {} sentinel bytes (redirect produced cleartext / no bytes, or kTLS RX failed)",
                got.len(),
                SENTINEL.len()
            ))
        }
    })();
    outcome.map_err(|message| MtlsEnforcementError::Probe {
        which: ProbeSentinel::KtlsArmRoundTrip,
        message,
    })
}

/// The sentinel peer: a loopback TLS 1.3 server that arms kTLS-RX and reads
/// `want` bytes of decrypted plaintext (proving the redirected bytes arrived
/// ENCRYPTED and decrypt to the sentinel). Uses an `AcceptAny` client-verifier
/// (loopback self-test; no real trust bundle). Returns the decrypted bytes.
fn sentinel_peer_recv(
    listener: &std::net::TcpListener,
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    want: usize,
) -> std::result::Result<Vec<u8>, String> {
    let mut cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| format!("sentinel server config: {e}"))?;
    cfg.enable_secret_extraction = true;
    cfg.send_tls13_tickets = 0; // raw kTLS-RX hits EIO on a post-handshake ticket
    let (tcp, _) = listener.accept().map_err(|e| format!("sentinel accept: {e}"))?;
    tcp.set_nodelay(true).ok();
    let fd = tcp.as_raw_fd();
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    let mut conn = rustls::ServerConnection::new(std::sync::Arc::new(cfg))
        .map_err(|e| format!("sentinel ServerConnection: {e}"))?;
    drive_server_handshake(&mut conn, &mut tcp)?;
    let secrets =
        conn.dangerous_extract_secrets().map_err(|e| format!("sentinel extract secrets: {e}"))?;
    ktls::arm_ktls_tx_rx(fd, secrets).map_err(|e| format!("sentinel kTLS-RX arm: {e}"))?;
    std::mem::forget(tcp); // keep the fd open for the kTLS read
    // Read decrypted plaintext off the kTLS-RX leg. Reconstruct an owning
    // `TcpStream` from the fd — dropping it at the end of this fn closes the leg.
    let stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    let mut got = Vec::new();
    let mut buf = vec![0u8; 4096];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while got.len() < want && std::time::Instant::now() < deadline {
        match (&stream).read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(e) => return Err(format!("sentinel kTLS read: {e}")),
        }
    }
    // close the fd by letting `stream` drop (the peer side of the loopback).
    Ok(got)
}

/// Drive a sentinel rustls CLIENT handshake on `leg_b` against the loopback
/// sentinel peer (an `AcceptAny` server-verifier — loopback self-test). Returns
/// the extracted secrets for the kTLS arm; forgets the stream so the fd stays open.
fn sentinel_client_handshake(leg_b: TcpStream) -> std::result::Result<ExtractedSecrets, String> {
    let mut cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(SentinelAcceptAny))
        .with_no_client_auth();
    cfg.enable_secret_extraction = true;
    let sni = ServerName::try_from("sentinel.overdrive.invalid".to_string())
        .map_err(|e| format!("sentinel SNI: {e}"))?;
    let mut conn = ClientConnection::new(std::sync::Arc::new(cfg), sni)
        .map_err(|e| format!("sentinel ClientConnection: {e}"))?;
    let mut tcp = leg_b;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    drive_client_handshake_raw(&mut conn, &mut tcp)?;
    let secrets =
        conn.dangerous_extract_secrets().map_err(|e| format!("sentinel extract secrets: {e}"))?;
    std::mem::forget(tcp);
    Ok(secrets)
}

/// Loopback-only `ServerCertVerifier` for the sentinel self-test — accepts the
/// throwaway self-signed sentinel cert. NEVER on a real peer path (the production
/// `enforce` uses the real `WebPkiClientVerifier`/bundle; this is `probe`-only).
#[derive(Debug)]
struct SentinelAcceptAny;

impl rustls::client::danger::ServerCertVerifier for SentinelAcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

/// Raw client-handshake driver (no typed `MtlsEnforcementError` — `probe` collects
/// a `String` cause).
fn drive_client_handshake_raw(
    conn: &mut ClientConnection,
    tcp: &mut TcpStream,
) -> std::result::Result<(), String> {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            conn.write_tls(tcp).map_err(|e| format!("write_tls: {e}"))?;
        }
        if !conn.is_handshaking() {
            return Ok(());
        }
        match conn.read_tls(tcp) {
            Ok(0) => return Err("EOF during sentinel handshake".into()),
            Ok(_) => {
                conn.process_new_packets().map_err(|e| format!("process_new_packets: {e}"))?;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("read_tls: {e}")),
        }
    }
}

/// Raw server-handshake driver for the sentinel peer.
fn drive_server_handshake(
    conn: &mut rustls::ServerConnection,
    tcp: &mut TcpStream,
) -> std::result::Result<(), String> {
    use std::io::ErrorKind;
    loop {
        while conn.wants_write() {
            conn.write_tls(tcp).map_err(|e| format!("write_tls: {e}"))?;
        }
        if !conn.is_handshaking() {
            while conn.wants_write() {
                conn.write_tls(tcp).map_err(|e| format!("final write_tls: {e}"))?;
            }
            return Ok(());
        }
        match conn.read_tls(tcp) {
            Ok(0) => return Err("EOF during sentinel server handshake".into()),
            Ok(_) => {
                conn.process_new_packets().map_err(|e| format!("process_new_packets: {e}"))?;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("read_tls: {e}")),
        }
    }
}

/// Sentinel (3): the arming invariant — a SOCKMAP insert AFTER `TCP_ULP "tls"`
/// returns `EINVAL` (both replace `sk->sk_prot`; `findings.md` D / D-MTLS-7).
/// Create a bare BPF SOCKMAP via a raw `bpf(2)`
/// syscall, ULP-arm a loopback socket, then `bpf_map_update_elem` the ULP'd fd
/// into the sockmap and REQUIRE `EINVAL`. No aya internals; pure UAPI.
fn probe_arming_order_einval_raw() -> Result<()> {
    let outcome = (|| -> std::result::Result<(), String> {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
        let addr = listener.local_addr().map_err(|e| format!("addr: {e}"))?;
        let client = std::net::TcpStream::connect(addr).map_err(|e| format!("connect: {e}"))?;
        client.set_nodelay(true).ok();
        let (server, _) = listener.accept().map_err(|e| format!("accept: {e}"))?;
        server.set_nodelay(true).ok();
        let client_fd = client.as_raw_fd();

        // Create a BPF_MAP_TYPE_SOCKMAP (type 15) with 1 entry, u32 key, u32 value.
        let map_fd = bpf_create_sockmap().map_err(|e| format!("create sockmap: {e}"))?;

        // Install TCP_ULP "tls" FIRST (the wrong order).
        let ulp = b"tls\0";
        // SAFETY: 3-byte "tls" ULP option on a connected fd.
        let rc = unsafe {
            libc::setsockopt(client_fd, libc::SOL_TCP, libc::TCP_ULP, ulp.as_ptr().cast(), 3)
        };
        if rc != 0 {
            // SAFETY: closing the map fd we just created.
            unsafe { libc::close(map_fd) };
            return Err(format!(
                "TCP_ULP \"tls\" install failed ({}) — kernel TLS module absent",
                std::io::Error::last_os_error()
            ));
        }

        // Now attempt the sockmap insert AFTER the ULP — MUST return EINVAL.
        let update_rc = bpf_sockmap_update(map_fd, 0, client_fd);
        // SAFETY: closing the map fd.
        unsafe { libc::close(map_fd) };
        match update_rc {
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => Ok(()),
            Err(e) => Err(format!(
                "sockmap insert after TCP_ULP returned {e} — expected EINVAL (arming-order invariant not enforced)"
            )),
            Ok(()) => Err(
                "sockmap insert AFTER TCP_ULP succeeded — the arming-order invariant the proxy relies on (sockmap-before-ULP, D-MTLS-7) is NOT enforced by this kernel".into(),
            ),
        }
    })();
    outcome.map_err(|message| MtlsEnforcementError::Probe {
        which: ProbeSentinel::ArmingOrderEinval,
        message,
    })
}

/// Create a bare `BPF_MAP_TYPE_SOCKMAP` (1 entry, u32→u32) via a raw `bpf(2)`
/// syscall. Returns the map fd (caller closes it). Used only by the arming-order
/// sentinel — no persistent map, no pin.
fn bpf_create_sockmap() -> std::result::Result<RawFd, String> {
    const BPF_MAP_CREATE: libc::c_int = 0;
    const BPF_MAP_TYPE_SOCKMAP: u32 = 15;
    // The first fields of `bpf_attr`'s map-create anonymous union, in order.
    #[repr(C)]
    #[derive(Default)]
    struct MapCreateAttr {
        map_type: u32,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
        map_flags: u32,
        inner_map_fd: u32,
        numa_node: u32,
        map_name: [u8; 16],
        map_ifindex: u32,
        btf_fd: u32,
        btf_key_type_id: u32,
        btf_value_type_id: u32,
        btf_vmlinux_value_type_id: u32,
        map_extra: u64,
    }
    let attr = MapCreateAttr {
        map_type: BPF_MAP_TYPE_SOCKMAP,
        key_size: 4,
        value_size: 4,
        max_entries: 1,
        ..Default::default()
    };
    // SAFETY: `bpf(2)` with BPF_MAP_CREATE and a correctly-shaped attr prefix; the
    // kernel reads `size` bytes. Extra trailing zero bytes are tolerated.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_MAP_CREATE,
            std::ptr::from_ref(&attr),
            std::mem::size_of::<MapCreateAttr>(),
        )
    };
    if fd < 0 {
        return Err(format!("bpf(MAP_CREATE, SOCKMAP): {}", std::io::Error::last_os_error()));
    }
    Ok(fd as RawFd)
}

/// `bpf_map_update_elem(map_fd, &key, &value=fd, BPF_ANY)` via a raw `bpf(2)`
/// syscall. Returns the `io::Error` on failure (the arming-order sentinel
/// inspects its `EINVAL`).
fn bpf_sockmap_update(
    map_fd: RawFd,
    key: u32,
    sock_fd: RawFd,
) -> std::result::Result<(), std::io::Error> {
    const BPF_MAP_UPDATE_ELEM: libc::c_int = 2;
    const BPF_ANY: u64 = 0;
    #[repr(C)]
    struct MapElemAttr {
        map_fd: u32,
        key: u64,
        value: u64,
        flags: u64,
    }
    let value = sock_fd.cast_unsigned(); // sockmap value is the socket fd (u32)
    let attr = MapElemAttr {
        map_fd: map_fd.cast_unsigned(),
        key: std::ptr::from_ref(&key) as u64,
        value: std::ptr::from_ref(&value) as u64,
        flags: BPF_ANY,
    };
    // SAFETY: `bpf(2)` with BPF_MAP_UPDATE_ELEM and a correctly-shaped attr.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_bpf,
            BPF_MAP_UPDATE_ELEM,
            std::ptr::from_ref(&attr),
            std::mem::size_of::<MapElemAttr>(),
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
