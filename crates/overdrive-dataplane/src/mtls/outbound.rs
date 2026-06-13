//! OUTBOUND enforcement mechanism (`Direction::Outbound`) + the probe sentinel.
//!
//! The agent owns leg F (workload-facing plaintext, `accept()`ed off the
//! `cgroup_connect4`-rewrite intercept) and dials leg B (peer-facing kTLS). The
//! forward path is an AGENT-LIGHT `read → write_all` COPY pump (`legF → legB`) —
//! ASYMMETRIC to the splice (kTLS-RX) directions, the inbound deliver pump
//! (`splice(legC → legS)`) and the return pump (`splice(legB → legF)`). leg B is
//! kTLS-TX-armed, so the kernel `tls_sw_sendmsg` encrypts each written record
//! SYNCHRONOUSLY on `write`; the agent does ZERO crypto, but it DOES copy each
//! record's plaintext through a userspace buffer (a `read`+`write` per record). A
//! `splice` INTO a kTLS-TX socket is NOT used — it loses records the same way the
//! abandoned sockmap egress redirect did.
//!
//! This replaced the sockmap egress redirect (`sk_skb/stream_verdict` +
//! `bpf_sk_redirect_map(flags=0)` into leg B's kTLS-TX), which enqueues on leg B's
//! psock and defers delivery to the `MSG_DONTWAIT` `sk_psock_backlog` workqueue —
//! `-EAGAIN`-stalling ~10–15% of records (the byte queued-but-undelivered), a
//! structural loss no userspace lever fixes. See
//! `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`.
//!
//! `establish` sequence (no sockmap, no enroll, no ARMED gate, no engagement poll):
//!   1. lossless pre-arm capture off leg F (bounded by `max_prearm_bytes`): read the
//!      plaintext the workload wrote during the handshake window;
//!   2. dial leg B (cgroup-scoped exempt from the workload `cgroup_connect4`
//!      intercept, F5) → rustls CLIENT handshake presenting the held SVID → arm
//!      kTLS-TX/RX on leg B;
//!   3. flush the captured pre-arm plaintext through leg B (kTLS-TX encrypts it as
//!      the first application_data);
//!   4. drain leg F's recv queue once more + flush (catches any byte the workload
//!      wrote in the window between the last capture read and the arm);
//!   5. spawn the FORWARD encrypt pump (`read → write_all` COPY `legF → legB`;
//!      kTLS-TX encrypts each `write`) AND the RETURN pump `splice(legB → legF)`
//!      (zero-copy out of kTLS-RX; decrypts on splice-out).
//!
//! Runs on a blocking task (synchronous rustls + raw setsockopt).

// `leg_f`/`leg_b` (and the fd-suffixed locals) are the canonical leg names from
// the spike findings and ADR-0069 — deliberately parallel; renaming them to
// satisfy `similar_names` would lose the contract vocabulary.
#![allow(clippy::similar_names, reason = "leg F/B names are the ADR-0069 contract vocabulary")]

use std::io::ErrorKind;
use std::net::{SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use overdrive_core::AllocationId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{
    MtlsEnforcementError, MtlsLimits, ProbeSentinel, Result,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, ExtractedSecrets};

use super::splice::PumpHandle;
use super::{ConnState, ktls, tls_config};

/// Establish OUTBOUND steady-state. Consumes `leg_f` (owned); dials leg B; returns
/// a [`ConnState`] holding both legs + the forward (primary) and return pumps.
pub(super) fn establish(
    leg_f: OwnedFd,
    peer: SocketAddrV4,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    limits: MtlsLimits,
) -> Result<ConnState> {
    let leg_f_fd = leg_f.as_raw_fd();

    // 1. Lossless pre-arm capture off leg F (bounded by max_prearm_bytes): the
    //    plaintext the workload wrote during the handshake window, read off leg F's
    //    own recv queue (leg F is an ordinary accepted socket — no sockmap, no
    //    verdict; the bytes simply sit on its recv queue).
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

    // 3. rustls CLIENT handshake on leg B presenting the held SVID; verify the peer
    //    against the trust bundle. `early_return` is any 0.5-RTT plaintext the peer
    //    sent and rustls already decrypted during the handshake window (drained from
    //    `conn.reader()` before the secrets were extracted — see
    //    `mtls::drain_early_plaintext`); it must reach leg F ahead of the return pump.
    let (secrets, early_return) =
        client_handshake(leg_b, svid, bundle, alloc, limits.handshake_deadline)?;

    // 4. Arm kTLS-TX/RX on leg B from the extracted secrets. TX so the forward
    //    write_all encrypts; RX so the return splice decrypts. The RX `rec_seq`
    //    already accounts for the early records rustls decrypted in step 3, so the
    //    return pump resumes at the NEXT on-wire record.
    ktls::arm_ktls_tx_rx(leg_b_fd, secrets)?;

    // 5. Drain leg F's recv queue once more — catches any byte the workload wrote in
    //    the window between the last capture read (step 1) and the arm (step 4).
    //    Combined with the step-1 pre-arm capture, this is the COMPLETE set of
    //    forward bytes that landed before the forward pump takes over.
    let mut prelude = held;
    prelude.extend(super::drain_recv_queue(leg_f_fd, limits.max_prearm_bytes, alloc)?);

    // 6. Spawn the FORWARD encrypt pump: it writes the captured pre-arm `prelude`
    //    FIRST (on its OWN thread — so leg B's kTLS-TX has a SINGLE writer for every
    //    forward byte, no cross-thread establish→pump handoff that desyncs the kTLS
    //    record sequence), then read(legF) → write_all(legB) for the steady state.
    //    leg B is kTLS-TX-armed, so the kernel tls_sw_sendmsg encrypts each blocking
    //    write (the proven kTLS-TX primitive — a nonblocking splice into kTLS-TX
    //    loses records). Agent does no crypto. This is the primary pump `liveness`
    //    observes (the request-carrying direction).
    let forward = PumpHandle::spawn_encrypt(leg_f_fd, leg_b_fd, prelude, super::now_unix_nanos());

    // 7. Spawn the RETURN decrypt pump splice(legB → pipe → legF): leg B is
    //    kTLS-RX-armed, so the kernel tls_sw_splice_read decrypts each record on
    //    splice-out. The early-data plaintext from step 3 (`early_return`) is written
    //    to leg F FIRST, ahead of the splice loop, so the workload sees the peer's
    //    0.5-RTT reply in order with no loss. Auxiliary (torn down with the
    //    connection; not the `liveness`-observed pump).
    let ret = PumpHandle::spawn_decrypt(leg_b_fd, leg_f_fd, early_return, super::now_unix_nanos());

    // Detach leg-B ownership into the ConnState. `client_handshake` already
    // suppressed the leg-B `TcpStream`'s Drop (`forget`), so reconstructing an
    // `OwnedFd` from `leg_b_fd` here is the SINGLE owner — no double-close.
    // SAFETY: `leg_b_fd` has exactly one live owner (the forgotten stream's fd);
    // wrapping it in an `OwnedFd` transfers that sole ownership to the ConnState.
    let leg_b_owned = unsafe { OwnedFd::from_raw_fd(leg_b_fd) };
    Ok(super::new_conn_state_bidi(vec![leg_f, leg_b_owned], forward, vec![ret]))
}

/// Drive the rustls CLIENT handshake on `leg_b` (an owned `TcpStream`), drain any
/// 0.5-RTT early plaintext rustls decrypted during the handshake, then extract the
/// secrets for the kTLS arm. Returns `(secrets, early_return)` — `early_return` is
/// the peer's early application_data plaintext that must reach leg F ahead of the
/// return pump (empty when the peer sent none before its `Finished` was processed).
/// The stream's Drop is suppressed (`forget`) so the leg-B fd stays open for the
/// arm + splice.
fn client_handshake(
    leg_b: TcpStream,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    deadline: std::time::Duration,
) -> Result<(ExtractedSecrets, Vec<u8>)> {
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
    drive_handshake_client(&mut conn, &mut tcp, alloc, deadline)?;
    // Drain any early application_data rustls decrypted while finishing the handshake
    // BEFORE `dangerous_extract_secrets` consumes the connection (kTLS 0.5-RTT
    // early-data correctness — see `mtls::drain_early_plaintext`).
    let early_return = super::drain_early_plaintext(&mut conn.reader());
    let secrets = conn.dangerous_extract_secrets().map_err(|e| {
        MtlsEnforcementError::HandshakeFailed { reason: format!("extract secrets: {e}") }
    })?;
    std::mem::forget(tcp); // keep the leg-B fd open for the kTLS arm + splice
    Ok((secrets, early_return))
}

/// Drive the rustls CLIENT handshake to completion, bounded by an OVERALL
/// wall-clock `deadline`. The per-read `SO_RCVTIMEO` (set by the caller) bounds
/// each individual `read_tls`, but a stalled/silent peer that dribbles a byte just
/// before each timeout would otherwise re-enter `read_tls` forever — there is no
/// per-read cap on the LOOP. The `Instant::now() >= cap` check is the wall-clock
/// bound that makes a silent peer fail-closed (`HandshakeTimeout`, F4): the
/// contract requires the handshake-and-arm to complete within
/// `limits.handshake_deadline` or refuse, so the stalled peer cannot pin agent
/// resources. The caller's `enforce` error path closes the owned legs; no kTLS is
/// armed and no cleartext egresses.
fn drive_handshake_client(
    conn: &mut ClientConnection,
    tcp: &mut TcpStream,
    alloc: &AllocationId,
    deadline: std::time::Duration,
) -> Result<()> {
    let cap = std::time::Instant::now() + deadline;
    loop {
        if std::time::Instant::now() >= cap {
            return Err(MtlsEnforcementError::HandshakeTimeout { alloc: alloc.clone(), deadline });
        }
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

/// Earned-Trust probe (`MtlsEnforcement::probe`; D-MTLS-11; contract postcondition).
/// Exercises the substrate the proxy relies on — kTLS arm + an agent-light forward
/// `splice` of one record through the kTLS-TX leg — on a loopback sentinel BEFORE
/// any connection is enforced, and tears the sentinel state down before return. Per
/// SD-5 / D-MTLS-12 (user-approved 2026-06-12) the sentinel handshake uses an
/// EPHEMERAL THROWAWAY self-signed cert minted in-process via `rcgen` —
/// substrate-self-test crypto, signed by neither CA, never in the trust bundle,
/// never on a real wire (loopback agent-to-itself only); #26 stays a READER, NOT an
/// issuer.
pub(super) fn run_probe_sentinels() -> Result<()> {
    // The process-default rustls `CryptoProvider` is installed once by the
    // COMPOSITION ROOT (`overdrive-control-plane`'s `serve` boot, mirroring the
    // operator/workload-CA TLS bootstrap), NOT by this adapter — a library mutating
    // process-global crypto state is the wrong layer. The sentinel handshakes below
    // consume that installed default via the `ServerConfig::builder()` /
    // `ClientConfig::builder()` surfaces, which is the caller's responsibility.
    probe_ktls_arm_and_forward_encrypt_round_trip()
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

/// The kTLS-arm + agent-light forward-encrypt round-trip on a loopback sentinel,
/// mirroring the real OUTBOUND lifecycle: dial a loopback sentinel peer (leg B),
/// rustls CLIENT handshake, arm kTLS-TX/RX, then `read → write_all` COPY a sentinel
/// record from a plaintext source leg into leg B's kTLS-TX — the sentinel peer
/// (kTLS-RX) MUST reconstruct the exact sentinel plaintext (proving the byte rode
/// the agent-light forward encrypt pump → leg B's kTLS TX → the peer's kTLS-RX,
/// ENCRYPTED on the wire). Any failure ⇒ `Probe`.
fn probe_ktls_arm_and_forward_encrypt_round_trip() -> Result<()> {
    const SENTINEL: &[u8] =
        b"OVERDRIVE_MTLS_PROBE_SENTINEL_ktls_arm_forward_encrypt_roundtrip_0001";

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

        // 2. Leg F sentinel source: a self-connected loopback pair the agent owns.
        //    f_writer's bytes appear on f_source's recv queue; the forward encrypt
        //    pump reads from f_source.
        let f_listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("legF bind: {e}"))?;
        let f_addr = f_listener.local_addr().map_err(|e| format!("legF addr: {e}"))?;
        let f_writer =
            std::net::TcpStream::connect(f_addr).map_err(|e| format!("legF connect: {e}"))?;
        f_writer.set_nodelay(true).ok();
        let (f_source, _) = f_listener.accept().map_err(|e| format!("legF accept: {e}"))?;
        f_source.set_nodelay(true).ok();

        // 3. Leg B: dial the sentinel peer, rustls CLIENT handshake, arm kTLS-TX/RX.
        let leg_b =
            std::net::TcpStream::connect(peer_addr).map_err(|e| format!("legB connect: {e}"))?;
        leg_b.set_nodelay(true).ok();
        let leg_b_fd = leg_b.as_raw_fd();
        let secrets = sentinel_client_handshake(leg_b)?;
        ktls::arm_ktls_tx_rx(leg_b_fd, secrets).map_err(|e| format!("kTLS arm: {e}"))?;

        // 4. Spawn the forward encrypt pump (f_source → leg B's kTLS-TX) with the
        //    SENTINEL as its `prelude` — exercising the EXACT production pre-arm path:
        //    the pump's own thread write_all's the prelude into leg B's kTLS-TX, the
        //    kernel tls_sw_sendmsg encrypts it. f_writer is unused (the prelude is the
        //    payload); the pump then reads f_source (empty) until teardown.
        let forward = PumpHandle::spawn_encrypt(
            f_source.as_raw_fd(),
            leg_b_fd,
            SENTINEL.to_vec(),
            super::now_unix_nanos(),
        );

        // 5. The sentinel peer (kTLS-RX) must reconstruct the exact sentinel.
        let got = peer.join().map_err(|_| "sentinel peer thread panicked".to_string())??;

        // Sentinel teardown (contract: "leaks no sentinel state"). Stop the forward
        // pump, drop the owned legs (closes f_writer / f_source), and close leg B by
        // raw fd (the handshake `forget`'d its stream to keep the fd open for the arm).
        let mut forward = forward;
        forward.stop_and_join();
        drop(f_writer);
        drop(f_source);
        // SAFETY: `leg_b_fd` is the sole live owner (the handshake forgot the
        // stream); closing it here reclaims the kTLS leg.
        unsafe { libc::close(leg_b_fd) };

        if got == SENTINEL {
            Ok(())
        } else {
            Err(format!(
                "forward-encrypt round-trip mismatch: peer reconstructed {} of {} sentinel bytes (forward encrypt pump produced cleartext / no bytes, or kTLS RX failed)",
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
/// `want` bytes of decrypted plaintext (proving the spliced bytes arrived
/// ENCRYPTED and decrypt to the sentinel). Uses an `AcceptAny` client-verifier
/// (loopback self-test; no real trust bundle). Returns the decrypted bytes.
fn sentinel_peer_recv(
    listener: &std::net::TcpListener,
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    want: usize,
) -> std::result::Result<Vec<u8>, String> {
    use std::io::Read as _;
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
    // Drain any 0.5-RTT early plaintext rustls decrypted while finishing the
    // handshake BEFORE extract consumes the connection — those bytes seed `got` so
    // the sentinel never loses an early-arriving record (kTLS early-data
    // correctness — see `mtls::drain_early_plaintext`).
    let mut got = super::drain_early_plaintext(&mut conn.reader());
    let secrets =
        conn.dangerous_extract_secrets().map_err(|e| format!("sentinel extract secrets: {e}"))?;
    ktls::arm_ktls_tx_rx(fd, secrets).map_err(|e| format!("sentinel kTLS-RX arm: {e}"))?;
    std::mem::forget(tcp); // keep the fd open for the kTLS read
    // Read decrypted plaintext off the kTLS-RX leg. Reconstruct an owning
    // `TcpStream` from the fd — dropping it at the end of this fn closes the leg.
    let stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
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
    // Drain any early plaintext rustls decrypted before extract consumes the
    // connection (kTLS early-data correctness — see `mtls::drain_early_plaintext`).
    // The probe's leg B is forward-only (the sentinel peer reads, never replies
    // before the pump), so this is empty in practice; draining keeps the extract's
    // `rec_seq` correct and the shape uniform with the production reader legs.
    let early = super::drain_early_plaintext(&mut conn.reader());
    debug_assert!(early.is_empty(), "probe leg B sentinel received unexpected early data");
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
