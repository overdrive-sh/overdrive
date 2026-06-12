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
    _alloc: &AllocationId,
    limits: MtlsLimits,
) -> Result<ConnState> {
    let leg_c_fd = leg_c.as_raw_fd();

    // 1. rustls SERVER handshake on leg C: present the held server SVID,
    //    REQUIRE+VERIFY the client SVID chains to the bundle. peer_certificates()
    //    is read inside the handshake driver BEFORE extract_secrets consumes the
    //    connection (fail-closed guard, Mechanics #6). On nocert/wrongca the
    //    handshake aborts → PeerVerificationFailed; no leg S is dialed, nothing
    //    spliced (fail-closed).
    let secrets = server_handshake(leg_c_fd, svid, bundle, limits.handshake_deadline)?;

    // 2. Arm kTLS-RX (+TX) on leg C from the extracted secrets.
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

    // 4. Start the deliver splice pump (legC → pipe → legS) on the plain (no-psock)
    //    kTLS-RX leg C — the C→S request path. `liveness` reports Running.
    let deliver = PumpHandle::spawn(leg_c_fd, leg_s_fd, super::now_unix_nanos());

    // 5. Start the response splice pump (legS → pipe → legC) — the S→C response leg
    //    (GAP 2 inbound half). leg S is plaintext; splicing into leg C drives leg
    //    C's kTLS-TX, encrypting S's reply back to the client. Auxiliary pump (torn
    //    down with the connection; not the `liveness`-observed pump).
    let response = PumpHandle::spawn(leg_s_fd, leg_c_fd, super::now_unix_nanos());

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
/// secrets. The borrowed stream's Drop is suppressed so leg C stays open.
fn server_handshake(
    leg_c_fd: RawFd,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    deadline: std::time::Duration,
) -> Result<ExtractedSecrets> {
    let cfg = tls_config::server_config(svid, bundle)?;
    let tcp = unsafe { TcpStream::from_raw_fd(leg_c_fd) };
    tcp.set_read_timeout(Some(deadline)).ok();
    let mut tcp = tcp;
    let mut conn = ServerConnection::new(cfg).map_err(|e| {
        MtlsEnforcementError::HandshakeFailed { reason: format!("ServerConnection: {e}") }
    })?;
    let result = (|| {
        drive_handshake_server(&mut conn, &mut tcp)?;
        // fail-closed guard: the client MUST have presented a verified cert.
        match conn.peer_certificates() {
            Some(certs) if !certs.is_empty() => {}
            _ => {
                return Err(MtlsEnforcementError::PeerVerificationFailed {
                    reason: "client presented no certificate (fail-closed)".into(),
                });
            }
        }
        conn.dangerous_extract_secrets().map_err(|e| MtlsEnforcementError::HandshakeFailed {
            reason: format!("extract secrets: {e}"),
        })
    })();
    std::mem::forget(tcp); // keep leg C open for the kTLS arm + splice
    result
}

fn drive_handshake_server(conn: &mut ServerConnection, tcp: &mut TcpStream) -> Result<()> {
    use std::io::ErrorKind;
    loop {
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
