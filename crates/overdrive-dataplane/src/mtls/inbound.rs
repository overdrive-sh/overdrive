//! INBOUND enforcement mechanism (`Direction::Inbound`).
//!
//! The agent owns leg C (client-facing kTLS, `accept()`ed off the `nft`-TPROXY +
//! `IP_TRANSPARENT` intercept). It performs the rustls SERVER handshake on leg C
//! (present held server SVID, REQUIRE+VERIFY the client SVID via
//! `WebPkiClientVerifier`), then selects the backend path: VM targets use the
//! guest-flow owner's `GuestStreamDial` and a temporary kTLS↔vsock userspace
//! relay (LEGS-DIAL); exec/process targets keep the plaintext TCP connect. NO
//! sockmap on leg C (a psock fights kTLS-RX, D-MTLS-5).
//!
//! Runs on a blocking task (synchronous rustls + raw setsockopt).

// `leg_c`/`leg_s` are the ADR-0069 contract leg names — deliberately parallel.
#![allow(clippy::similar_names, reason = "leg C/S names are the ADR-0069 contract vocabulary")]

use std::io::ErrorKind;
use std::net::{SocketAddrV4, TcpStream};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use overdrive_core::AllocationId;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestStream, GuestStreamDial, GuestStreamEnd,
};
use overdrive_core::traits::ca::{SvidMaterial, TrustBundle};
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcementError, MtlsLimits, Result};
use rustls::{ExtractedSecrets, ServerConnection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream as AsyncTcpStream;
use tokio::sync::oneshot;

use super::splice::{PumpExit, PumpHandle, PumpState, record_progress};
use super::{ConnState, ktls, tls_config};

/// Establish the temporary LEGS-DIAL path for a VM backend. The handshake and
/// kTLS arm remain blocking kernel/TLS work; the existing `GuestStreamDial`
/// owns the vsock connection, and its stream is relayed by the tracked pump.
#[allow(
    clippy::too_many_arguments,
    reason = "the private boundary passes the accepted handshake, dial, limits, and runtime dependencies without a new adapter type"
)]
pub(super) async fn establish_guest(
    leg_c: OwnedFd,
    orig_dst: SocketAddrV4,
    svid: &SvidMaterial,
    bundle: &TrustBundle,
    alloc: &AllocationId,
    limits: MtlsLimits,
    guest_dial: Arc<dyn GuestStreamDial>,
    runtime: tokio::runtime::Handle,
) -> Result<ConnState> {
    let svid = svid.clone();
    let bundle = bundle.clone();
    let alloc = alloc.clone();
    let (leg_c, early_deliver) = tokio::task::spawn_blocking(move || {
        let leg_c_fd = leg_c.as_raw_fd();
        let (secrets, early_deliver) =
            server_handshake(leg_c_fd, &svid, &bundle, &alloc, limits.handshake_deadline)?;
        super::arm_transport_death_timeouts(leg_c_fd, limits.pump_stall_deadline)?;
        ktls::arm_ktls_tx_rx(leg_c_fd, secrets)?;
        Ok::<_, MtlsEnforcementError>((leg_c, early_deliver))
    })
    .await
    .map_err(|error| {
        MtlsEnforcementError::Io(std::io::Error::other(format!("enforce task: {error}")))
    })??;

    let guest = match guest_dial.dial(orig_dst, DialPurpose::Relay).await {
        Ok(stream) => stream,
        Err(source) => {
            reset_tcp_leg(&leg_c);
            return Err(MtlsEnforcementError::GuestDial { source });
        }
    };
    let relay_leg = match leg_c.as_fd().try_clone_to_owned() {
        Ok(leg) => leg,
        Err(source) => {
            reset_tcp_leg(&leg_c);
            // Drop the paired GuestStream before returning so its existing
            // WATCH-ABORT contract releases the owner's flow slot.
            drop(guest);
            return Err(MtlsEnforcementError::Io(source));
        }
    };
    let pump = PumpHandle::spawn_async(runtime, super::now_unix_nanos(), move |state, stop| {
        relay_guest_stream(relay_leg, guest, early_deliver, state, stop)
    });
    Ok(super::new_conn_state_bidi(vec![leg_c], pump, Vec::new()))
}

enum RelayRead {
    Stopped,
    Client(std::io::Result<usize>),
    Guest(std::io::Result<usize>),
}

/// Bidirectional temporary relay between leg C's kTLS socket and the
/// move-only owner `GuestStream`. The async reads are cancellation-safe by the
/// GuestStream contract; the pump thread serializes each completed read into
/// the opposite write direction so one `GuestStream` remains its sole holder.
#[allow(
    clippy::too_many_lines,
    reason = "one pump state machine owns ordering, half-close, abort, and progress for both directions of the LEGS-DIAL relay"
)]
async fn relay_guest_stream(
    relay_leg: OwnedFd,
    mut guest: GuestStream,
    early_deliver: Vec<u8>,
    state: Arc<PumpState>,
    mut stop: oneshot::Receiver<()>,
) -> PumpExit {
    let tcp = unsafe { TcpStream::from_raw_fd(relay_leg.into_raw_fd()) };
    if let Err(error) = tcp.set_nonblocking(true) {
        tracing::warn!(error = %error, "could not make the kTLS relay leg nonblocking");
        reset_tcp_stream(&tcp);
        return PumpExit::TransportDeath;
    }
    let mut tls = match AsyncTcpStream::from_std(tcp) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(error = %error, "could not register the kTLS relay leg with Tokio");
            return PumpExit::TransportDeath;
        }
    };

    // Rustls may have decrypted 0.5-RTT application bytes before extracting
    // the kTLS secrets. They are the first bytes of the guest request.
    if !early_deliver.is_empty() {
        state.record_pending.store(true, Ordering::SeqCst);
        let mut written = 0;
        while written < early_deliver.len() {
            let result = tokio::select! {
                biased;
                _ = &mut stop => {
                    state.record_pending.store(false, Ordering::SeqCst);
                    return PumpExit::Graceful;
                }
                result = guest.write(&early_deliver[written..]) => result,
            };
            match result {
                Ok(0) => {
                    state.record_pending.store(false, Ordering::SeqCst);
                    reset_tls_after_guest_abort(&state, &tls);
                    return PumpExit::TransportDeath;
                }
                Ok(n) => {
                    written += n;
                    record_progress(&state, n as u64);
                }
                Err(error) => {
                    state.record_pending.store(false, Ordering::SeqCst);
                    tracing::debug!(error = %error, "writing early TLS plaintext to the guest stream failed");
                    reset_tls_after_guest_abort(&state, &tls);
                    return PumpExit::TransportDeath;
                }
            }
        }
        state.record_pending.store(false, Ordering::SeqCst);
    }

    let mut client_to_guest_open = true;
    let mut guest_to_client_open = true;
    let mut client_bytes = vec![0_u8; 16 * 1024];
    let mut guest_bytes = vec![0_u8; 16 * 1024];

    loop {
        if state.stop.load(Ordering::SeqCst) {
            return PumpExit::Graceful;
        }
        if !client_to_guest_open && !guest_to_client_open {
            return PumpExit::Graceful;
        }

        state.record_pending.store(false, Ordering::SeqCst);
        let event = tokio::select! {
            biased;
            _ = &mut stop => RelayRead::Stopped,
            result = tls.read(&mut client_bytes), if client_to_guest_open => RelayRead::Client(result),
            result = guest.read(&mut guest_bytes), if guest_to_client_open => RelayRead::Guest(result),
        };

        match event {
            RelayRead::Stopped => return PumpExit::Graceful,
            RelayRead::Client(Ok(0)) => {
                client_to_guest_open = false;
                state.record_pending.store(true, Ordering::SeqCst);
                let result = tokio::select! {
                    biased;
                    _ = &mut stop => {
                        state.record_pending.store(false, Ordering::SeqCst);
                        return PumpExit::Graceful;
                    }
                    result = guest.shutdown_write() => result,
                };
                state.record_pending.store(false, Ordering::SeqCst);
                if let Err(error) = result {
                    tracing::debug!(error = %error, "half-closing the guest stream after the TLS client FIN failed");
                    reset_tls_after_guest_abort(&state, &tls);
                    return PumpExit::TransportDeath;
                }
            }
            RelayRead::Client(Err(error)) if is_tls_fin_error(&error, tls.as_raw_fd()) => {
                // A TCP FIN without TLS close_notify is the temporary relay's
                // clean half-close. A kTLS RX socket also reports a TLS
                // close_notify as EIO when the plain read API has no ancillary
                // buffer for TLS_GET_RECORD_TYPE; distinguish that from a
                // socket error before propagating the clean end.
                client_to_guest_open = false;
                state.record_pending.store(true, Ordering::SeqCst);
                let result = tokio::select! {
                    biased;
                    _ = &mut stop => {
                        state.record_pending.store(false, Ordering::SeqCst);
                        return PumpExit::Graceful;
                    }
                    result = guest.shutdown_write() => result,
                };
                state.record_pending.store(false, Ordering::SeqCst);
                if let Err(error) = result {
                    tracing::debug!(error = %error, "half-closing the guest stream after the TLS client FIN failed");
                    reset_tls_after_guest_abort(&state, &tls);
                    return PumpExit::TransportDeath;
                }
            }
            RelayRead::Client(Ok(n)) => {
                state.record_pending.store(true, Ordering::SeqCst);
                let mut written = 0;
                while written < n {
                    let result = tokio::select! {
                        biased;
                        _ = &mut stop => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            return PumpExit::Graceful;
                        }
                        result = guest.write(&client_bytes[written..n]) => result,
                    };
                    match result {
                        Ok(0) => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            reset_tls_after_guest_abort(&state, &tls);
                            return PumpExit::TransportDeath;
                        }
                        Ok(wrote) => {
                            written += wrote;
                            record_progress(&state, wrote as u64);
                        }
                        Err(error) => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            tracing::debug!(error = %error, "writing TLS plaintext to the guest stream failed");
                            reset_tls_after_guest_abort(&state, &tls);
                            return PumpExit::TransportDeath;
                        }
                    }
                }
                state.record_pending.store(false, Ordering::SeqCst);
            }
            RelayRead::Client(Err(error)) => {
                tracing::debug!(error = %error, "reading the TLS client leg failed; dropping its guest stream");
                return PumpExit::TransportDeath;
            }
            RelayRead::Guest(Ok(0)) => match guest.end() {
                Some(GuestStreamEnd::HalfClose) => {
                    guest_to_client_open = false;
                    state.record_pending.store(true, Ordering::SeqCst);
                    let result = tokio::select! {
                        biased;
                        _ = &mut stop => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            return PumpExit::Graceful;
                        }
                        result = tls.shutdown() => result,
                    };
                    state.record_pending.store(false, Ordering::SeqCst);
                    if let Err(error) = result {
                        tracing::debug!(error = %error, "half-closing the TLS client leg after the guest FIN failed");
                        reset_async_tcp_stream(&tls);
                        return PumpExit::TransportDeath;
                    }
                }
                Some(GuestStreamEnd::Abort) | None => {
                    reset_tls_after_guest_abort(&state, &tls);
                    return PumpExit::TransportDeath;
                }
            },
            RelayRead::Guest(Ok(n)) => {
                state.record_pending.store(true, Ordering::SeqCst);
                let mut written = 0;
                while written < n {
                    let result = tokio::select! {
                        biased;
                        _ = &mut stop => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            return PumpExit::Graceful;
                        }
                        result = tls.write(&guest_bytes[written..n]) => result,
                    };
                    match result {
                        Ok(0) => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            reset_async_tcp_stream(&tls);
                            return PumpExit::TransportDeath;
                        }
                        Ok(wrote) => {
                            written += wrote;
                            record_progress(&state, wrote as u64);
                        }
                        Err(error) => {
                            state.record_pending.store(false, Ordering::SeqCst);
                            tracing::debug!(error = %error, "writing guest plaintext to the TLS client failed");
                            reset_async_tcp_stream(&tls);
                            return PumpExit::TransportDeath;
                        }
                    }
                }
                state.record_pending.store(false, Ordering::SeqCst);
            }
            RelayRead::Guest(Err(error)) => {
                tracing::debug!(
                    error = %error,
                    end = ?guest.end(),
                    "reading the guest stream failed; resetting its TLS client"
                );
                reset_tls_after_guest_abort(&state, &tls);
                return PumpExit::TransportDeath;
            }
        }
    }
}

fn reset_tcp_leg(leg: &OwnedFd) {
    reset_tcp_fd(leg.as_raw_fd());
}

fn reset_async_tcp_stream(tcp: &AsyncTcpStream) {
    reset_tcp_fd(tcp.as_raw_fd());
}

fn reset_tcp_stream(tcp: &TcpStream) {
    reset_tcp_fd(tcp.as_raw_fd());
}

fn reset_tls_after_guest_abort(state: &PumpState, tls: &AsyncTcpStream) {
    state.reset_peer_on_reclaim.store(true, Ordering::SeqCst);
    reset_async_tcp_stream(tls);
}

fn reset_tcp_fd(fd: RawFd) {
    let linger = libc::linger { l_onoff: 1, l_linger: 0 };
    // SAFETY: `fd` is a live TCP descriptor owned by the caller; SO_LINGER
    // with zero timeout requests the contract's reset of this leg.
    let _ = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            std::ptr::from_ref(&linger).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::linger>())
                .unwrap_or(libc::socklen_t::MAX),
        )
    };
}

fn is_tls_fin_error(error: &std::io::Error, fd: RawFd) -> bool {
    if error.kind() == ErrorKind::UnexpectedEof {
        return true;
    }
    if error.raw_os_error() != Some(libc::EIO) {
        return false;
    }
    socket_error(fd) == Some(0)
}

fn socket_error(fd: RawFd) -> Option<libc::c_int> {
    let mut error = 0;
    let mut length = libc::socklen_t::try_from(std::mem::size_of_val(&error)).ok()?;
    // SAFETY: the pointer/length describe the initialized c_int SO_ERROR result.
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            std::ptr::from_mut(&mut error).cast(),
            &raw mut length,
        )
    };
    (result == 0 && usize::try_from(length).ok()? == std::mem::size_of_val(&error)).then_some(error)
}

/// Establish INBOUND steady-state. Consumes `leg_c` (owned); dials leg S; returns
/// a [`ConnState`] holding both legs + the deliver splice pump.
pub(super) fn establish_tcp(
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

    // 1b. (C) D-MTLS-16 / ADR-0070: arm the kernel's transport-death reaping
    //     (`TCP_USER_TIMEOUT` = `pump_stall_deadline` + keepalive) on leg C BEFORE the
    //     kTLS ULP is installed on it (step 2). Setting these `IPPROTO_TCP` options on
    //     the RAW TCP socket first is load-bearing: once leg C carries the `tls` ULP,
    //     further TCP socket-option writes route through the ULP and disrupt the kTLS
    //     arm (the leg-C `ss -tie` `tcp-ulp-tls` observable vanishes).
    super::arm_transport_death_timeouts(leg_c_fd, limits.pump_stall_deadline)?;

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

    // 3b. (C) D-MTLS-16 / ADR-0070: arm the kernel's transport-death reaping on leg S
    //     (the plaintext server leg — no kTLS ULP, so the ordering constraint that
    //     governs leg C does not apply here) BEFORE the SD-2 pumps start. A server
    //     workload that vanishes (gone, half-open, unacked past the deadline) is
    //     reaped by the kernel with no userspace tick; the (B) pump task observes the
    //     resulting `ETIMEDOUT`/EOF and self-tears-down. With leg C's (C) set at step
    //     1b, both agent-owned legs now carry the transport-death timeouts. Replaces
    //     the retired central `MtlsSupervisor` point-query for the transport-death
    //     class.
    super::arm_transport_death_timeouts(leg_s_fd, limits.pump_stall_deadline)?;

    // 4. Start the deliver decrypt pump splice(legC → pipe → legS) on the plain
    //    (no-psock) kTLS-RX leg C — the C→S request path. The kernel
    //    tls_sw_splice_read decrypts each record. The early-data plaintext from
    //    step 1 (`early_deliver`) is written to leg S FIRST, ahead of the splice loop,
    //    so the server sees the client's 0.5-RTT request in order with no loss.
    //    `liveness` reports Running.
    let deliver =
        PumpHandle::spawn_decrypt(leg_c_fd, leg_s_fd, early_deliver, super::now_unix_nanos());

    // 5. Start the response encrypt pump — a blocking splice(legS → pipe → legC) —
    //    the S→C response leg (GAP 2 inbound half). leg S is plaintext; the blocking
    //    splice into leg C drives leg C's kTLS-TX, encrypting S's reply back to the
    //    client (the proven lossless kTLS-TX primitive,
    //    `findings-ktls-tx-blocking-splice.md`). Auxiliary pump (torn down with the
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
/// listener map is the production source (#241-adjacent). Here the orig-dst's
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
    let start = std::time::Instant::now();
    loop {
        if super::limits::handshake_expired(start.elapsed(), deadline) {
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
                if conn.peer_certificates().is_none() {
                    return Err(MtlsEnforcementError::PeerVerificationFailed {
                        reason: "client closed before presenting a verified certificate".into(),
                    });
                }
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
