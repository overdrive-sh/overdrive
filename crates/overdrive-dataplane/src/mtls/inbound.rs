//! INBOUND enforcement mechanism (`Direction::Inbound`).
//!
//! The agent owns leg C (client-facing kTLS, `accept()`ed off the `nft`-TPROXY +
//! `IP_TRANSPARENT` intercept) and dials leg S (the server workload's real
//! plaintext socket) inside `enforce`. Productionises increment-i (composed
//! inbound flow proven end-to-end): orig-dst already recovered by the worker →
//! rustls SERVER handshake on leg C (present held server SVID, REQUIRE+VERIFY the
//! client SVID via `WebPkiClientVerifier`) → arm kTLS-RX → dial leg S → start the
//! agent-light `splice(legC → pipe → legS)` deliver pump. NO sockmap on leg C (a
//! psock fights kTLS-RX, D-MTLS-5).
//!
//! Runs on a blocking task (synchronous rustls + raw setsockopt).

// `leg_c`/`leg_s` are the ADR-0069 contract leg names — deliberately parallel.
#![allow(clippy::similar_names, reason = "leg C/S names are the ADR-0069 contract vocabulary")]

use std::net::{SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use overdrive_core::AllocationId;
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcementError, MtlsLimits, Result};
use rustls::{ExtractedSecrets, ServerConnection};

use super::splice::PumpHandle;
use super::{ConnState, ktls, tls_config};

/// Establish INBOUND steady-state. Consumes `leg_c` (owned); dials leg S; returns
/// a [`ConnState`] holding both legs + the deliver splice pump.
pub(super) fn establish(
    leg_c: OwnedFd,
    orig_dst: SocketAddrV4,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    limits: MtlsLimits,
) -> Result<ConnState> {
    let leg_c_fd = leg_c.as_raw_fd();

    // 1. rustls SERVER handshake on leg C: present the held server SVID,
    //    REQUIRE+VERIFY the client SVID chains to the bundle. peer_certificates()
    //    is read inside the handshake driver BEFORE extract_secrets consumes the
    //    connection (fail-closed guard, Mechanics #6). On nocert/wrongca the
    //    handshake aborts → PeerVerificationFailed; no leg S is dialed, nothing
    //    spliced (fail-closed). `early_deliver` is any 0.5-RTT request plaintext the
    //    client sent and rustls already decrypted during the handshake window
    //    (drained from `conn.reader()`); it must reach leg S ahead of the deliver pump.
    let (secrets, early_deliver) =
        server_handshake(leg_c_fd, svid, bundle, alloc, limits.handshake_deadline)?;

    // 2. Arm kTLS-RX (+TX) on leg C from the extracted secrets. The RX `rec_seq`
    //    already accounts for the early records rustls decrypted in step 1, so the
    //    deliver pump resumes at the NEXT on-wire record.
    ktls::arm_ktls_tx_rx(leg_c_fd, secrets)?;

    // 3. Dial leg S (the server workload's real plaintext socket). The orig-dst
    //    selected the server SVID; the worker provides the server's real listener
    //    addr indirectly (single-node: the server workload listens on a known
    //    loopback addr the worker passes as the dial target via the test harness;
    //    in production the control plane owns the orig-dst → server-listener map).
    //    For the walking-skeleton gate the server's real addr equals the orig-dst's
    //    server identity; the harness binds S on the orig-dst loopback addr. The
    //    leg-S dial is SO_MARK-stamped (F5 inbound intercept-recursion exemption) so
    //    the nft-TPROXY rule does NOT re-intercept the agent's own dial to S.
    let leg_s = super::dial_leg_s(server_dial_addr(orig_dst), limits.handshake_deadline)?;
    let leg_s_fd = leg_s.as_raw_fd();

    // 4. Start the deliver decrypt pump splice(legC → pipe → legS) on the plain
    //    (no-psock) kTLS-RX leg C — the C→S request path. The kernel
    //    tls_sw_splice_read decrypts each record. The early-data plaintext from
    //    step 1 (`early_deliver`) is written to leg S FIRST, ahead of the splice loop,
    //    so the server sees the client's 0.5-RTT request in order with no loss.
    //    `liveness` reports Running.
    let deliver =
        PumpHandle::spawn_decrypt(leg_c_fd, leg_s_fd, early_deliver, super::now_unix_nanos());

    // 5. Start the response encrypt pump read(legS) → write_all(legC) — the S→C
    //    response leg (GAP 2 inbound half). leg S is plaintext; the blocking write
    //    into leg C drives leg C's kTLS-TX, encrypting S's reply back to the client
    //    (the proven kTLS-TX primitive). Auxiliary pump (torn down with the
    //    connection; not the `liveness`-observed pump).
    let response =
        PumpHandle::spawn_encrypt(leg_s_fd, leg_c_fd, Vec::new(), super::now_unix_nanos());

    // Detach leg ownership into the ConnState (both legs closed on teardown).
    let leg_s_owned = unsafe { OwnedFd::from_raw_fd(leg_s_fd) };
    std::mem::forget(leg_s);
    Ok(super::new_conn_state_bidi(vec![leg_c, leg_s_owned], deliver, vec![response]))
}

/// The server workload's real plaintext listener addr the agent dials leg S to.
/// For the single-node walking skeleton the harness binds S on the recovered
/// original destination's loopback addr; the control-plane orig-dst → server-
/// listener map is the production source (#178-adjacent). Here the orig-dst's
/// address+port IS the server's listener (the harness arranges this).
const fn server_dial_addr(orig_dst: SocketAddrV4) -> SocketAddrV4 {
    orig_dst
}

/// Drive the rustls SERVER handshake on leg C (a raw fd, borrowed not owned),
/// reading `peer_certificates()` for the fail-closed guard BEFORE extracting the
/// secrets. Returns `(secrets, early_deliver)` — `early_deliver` is the client's
/// 0.5-RTT request plaintext that rustls decrypted during the handshake and that
/// must reach leg S ahead of the deliver pump (empty when the client sent none
/// before its `Finished` was processed). The borrowed stream's Drop is suppressed
/// so leg C stays open.
fn server_handshake(
    leg_c_fd: RawFd,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    deadline: std::time::Duration,
) -> Result<(ExtractedSecrets, Vec<u8>)> {
    let cfg = tls_config::server_config(svid, bundle)?;
    let tcp = unsafe { TcpStream::from_raw_fd(leg_c_fd) };
    tcp.set_read_timeout(Some(deadline)).ok();
    let mut tcp = tcp;
    let mut conn = ServerConnection::new(cfg).map_err(|e| {
        MtlsEnforcementError::HandshakeFailed { reason: format!("ServerConnection: {e}") }
    })?;
    let result = (|| {
        drive_handshake_server(&mut conn, &mut tcp, alloc, deadline)?;
        // fail-closed guard: the client MUST have presented a verified cert.
        match conn.peer_certificates() {
            Some(certs) if !certs.is_empty() => {}
            _ => {
                return Err(MtlsEnforcementError::PeerVerificationFailed {
                    reason: "client presented no certificate (fail-closed)".into(),
                });
            }
        }
        // Drain any early application_data rustls decrypted while finishing the
        // handshake BEFORE `dangerous_extract_secrets` consumes the connection
        // (kTLS 0.5-RTT early-data correctness — see `mtls::drain_early_plaintext`).
        let early_deliver = super::drain_early_plaintext(&mut conn.reader());
        let secrets = conn.dangerous_extract_secrets().map_err(|e| {
            MtlsEnforcementError::HandshakeFailed { reason: format!("extract secrets: {e}") }
        })?;
        Ok((secrets, early_deliver))
    })();
    std::mem::forget(tcp); // keep leg C open for the kTLS arm + splice
    result
}

/// Drive the rustls SERVER handshake to completion, bounded by an OVERALL
/// wall-clock `deadline`. The per-read `SO_RCVTIMEO` bounds each individual
/// `read_tls`, but a stalled/silent client that dribbles a byte just before each
/// timeout would otherwise re-enter `read_tls` forever — the LOOP itself has no
/// cap. The `Instant::now() >= cap` check is the wall-clock bound that makes a
/// silent client fail-closed (`HandshakeTimeout`, F4): the contract requires the
/// handshake-and-arm to complete within `limits.handshake_deadline` or refuse, so
/// the stalled client cannot pin agent resources. `server_handshake`'s `forget`
/// keeps leg C open; the caller's `enforce` error path closes it (and never dials
/// leg S), so no plaintext is spliced to the server workload.
fn drive_handshake_server(
    conn: &mut ServerConnection,
    tcp: &mut TcpStream,
    alloc: &AllocationId,
    deadline: std::time::Duration,
) -> Result<()> {
    use std::io::ErrorKind;
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
            // flush any final handshake bytes
            while conn.wants_write() {
                conn.write_tls(tcp).map_err(|e| MtlsEnforcementError::HandshakeFailed {
                    reason: format!("final write_tls: {e}"),
                })?;
            }
            return Ok(());
        }
        match conn.read_tls(tcp) {
            Ok(0) => {
                return Err(MtlsEnforcementError::HandshakeFailed {
                    reason: "EOF during handshake (client closed)".into(),
                });
            }
            Ok(_) => {
                // client-auth failures surface HERE as a rustls error → fail-closed
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
