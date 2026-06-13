//! Agent-light pumps moving plaintext across a kTLS boundary, in both directions.
//!
//! Two kinds, because the two directions across a kTLS socket are NOT symmetric at
//! the kernel:
//!
//! - **Decrypt pump (kTLS-RX source → plaintext destination)** — the outbound
//!   return (`legB → legF`) and the inbound deliver (`legC → legS`). Uses
//!   `splice(src → pipe → dst, SPLICE_F_MOVE|SPLICE_F_NONBLOCK)`: `tls_sw_splice_read`
//!   decrypts each kTLS record into the pipe and the plaintext destination accepts
//!   it — ZERO userspace copy, ~1 splice per TLS record. Proven in
//!   `findings-splice-return.md` (increment-h, RELAY_EXACT_CLEAN) and
//!   `findings-inbound-intercept.md` §5.
//!
//! - **Encrypt pump (plaintext source → kTLS-TX destination)** — the outbound
//!   forward (`legF → legB`) and the inbound response (`legS → legC`). Uses a
//!   BLOCKING `read(src)` → `write_all(dst)` copy. The destination's
//!   `sk->sk_prot->sendmsg` is `tls_sw_sendmsg`, so the kernel still does the
//!   AES-GCM in-kernel on the `write` — the agent does ZERO crypto, only the copy.
//!   `write_all` is the proven kTLS-TX primitive (the pre-arm `prelude` uses it and
//!   always arrives): a blocking userspace `write` to a kTLS socket waits for buffer
//!   space, framing exactly one record per `write`. **A `splice` into a
//!   kTLS-TX socket is NOT used** — `splice(pipe → ktls_tx, NONBLOCK)` consumes the
//!   bytes from the pipe and reports success (`n_out == len`) but the `tls_sw`
//!   splice/sendpage path does NOT reliably emit the record (the peer decrypts the
//!   PRIOR record only), the same untested-seam loss class the sockmap egress
//!   redirect suffered. Trace-confirmed: a forward splice reported `n_out=55
//!   errno=0` while the peer received 0 of those 55 bytes
//!   (`docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`,
//!   VERDICT — "a blocking userspace `write` to a kTLS socket waits for buffer
//!   space instead of `-EAGAIN`-stalling").
//!
//! Both kinds run on a blocking thread for the connection's life, track a shared
//! bytes-moved progress counter (`liveness` reads it via [`PumpHandle`]) and a stop
//! flag (`teardown` sets it). SD-2: the port owns the pump.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::{FromRawFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Shared observation surface for one connection's pump. `liveness` reads
/// `bytes_spliced` + `running`; `teardown` sets `stop`.
#[derive(Debug, Default)]
pub(super) struct PumpState {
    /// Monotonic count of plaintext bytes the pump has moved to the destination —
    /// the progress metric `liveness` watches for a stall.
    pub bytes_spliced: AtomicU64,
    /// Set by `teardown` to stop the pump; the pump thread exits its loop.
    pub stop: AtomicBool,
    /// Cleared when the pump thread has fully exited.
    pub running: AtomicBool,
    /// `true` while a record is pending on the source leg — the stall-deadline only
    /// applies while a record is actually pending (a purely idle connection is
    /// `Running`, never `Stalled`).
    pub record_pending: AtomicBool,
    /// Wall-clock nanos (since the process monotonic origin) of the last progress
    /// advance, for the `Stalled { since }` computation.
    pub last_progress_unix_nanos: AtomicU64,
}

/// Handle the adapter holds per connection; gives `liveness`/`teardown` the
/// shared state and the worker-thread join handle.
pub(super) struct PumpHandle {
    pub state: Arc<PumpState>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl PumpHandle {
    /// Spawn a DECRYPT pump: `splice(src_fd → pipe → dst_fd)` where `src_fd` is a
    /// kTLS-RX leg (the kernel decrypts each record) and `dst_fd` is a plaintext
    /// leg. Zero userspace copy. Used by the outbound return + the inbound deliver.
    ///
    /// `early` is the plaintext rustls already decrypted during the handshake
    /// (the peer's / client's 0.5-RTT early application_data, drained out of
    /// `conn.reader()` before `dangerous_extract_secrets` — see
    /// `mtls::drain_early_plaintext`). It is written to `dst_fd` FIRST, on the pump's
    /// OWN thread, BEFORE the splice loop opens — so the downstream leg sees every
    /// early byte ahead of the steady-state kTLS-RX records, in order, with a single
    /// writer (no cross-thread establish→pump handoff that could reorder). Pass an
    /// empty `Vec` when no early data was buffered.
    pub(super) fn spawn_decrypt(
        src_fd: RawFd,
        dst_fd: RawFd,
        early: Vec<u8>,
        now_unix_nanos: u64,
    ) -> Self {
        let state = Arc::new(PumpState::default());
        state.running.store(true, Ordering::SeqCst);
        state.last_progress_unix_nanos.store(now_unix_nanos, Ordering::SeqCst);
        let pump_state = Arc::clone(&state);
        let join =
            std::thread::spawn(move || run_decrypt_pump(src_fd, dst_fd, &early, &pump_state));
        Self { state, join: Some(join) }
    }

    /// Spawn an ENCRYPT pump: a blocking `read(src_fd) → write_all(dst_fd)` copy
    /// where `dst_fd` is a kTLS-TX leg (the kernel `tls_sw_sendmsg` encrypts each
    /// `write`) and `src_fd` is a plaintext leg. The agent does no crypto. Used by
    /// the outbound forward + the inbound response.
    ///
    /// `prelude` is written to `dst_fd` FIRST (as the pump's first record(s)),
    /// before any `read(src_fd)`. The outbound forward passes the captured pre-arm
    /// plaintext here so the SAME thread that drives the steady-state forward writes
    /// the pre-arm bytes too — leg B's kTLS-TX then has exactly ONE writer for every
    /// forward byte (no cross-thread establish→pump handoff that could desync the
    /// kTLS record sequence). Inbound response passes an empty `prelude`.
    pub(super) fn spawn_encrypt(
        src_fd: RawFd,
        dst_fd: RawFd,
        prelude: Vec<u8>,
        now_unix_nanos: u64,
    ) -> Self {
        let state = Arc::new(PumpState::default());
        state.running.store(true, Ordering::SeqCst);
        state.last_progress_unix_nanos.store(now_unix_nanos, Ordering::SeqCst);
        let pump_state = Arc::clone(&state);
        let join =
            std::thread::spawn(move || run_encrypt_pump(src_fd, dst_fd, &prelude, &pump_state));
        Self { state, join: Some(join) }
    }

    /// Signal the pump to stop and join its thread (idempotent).
    pub(super) fn stop_and_join(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

/// Record one progress advance of `n` bytes moved to the destination.
fn record_progress(state: &PumpState, n: u64) {
    state.bytes_spliced.fetch_add(n, Ordering::SeqCst);
    state.last_progress_unix_nanos.store(now_nanos(), Ordering::SeqCst);
}

/// Process-monotonic "now" in nanos (mirrors `mtls::now_unix_nanos`, duplicated
/// here to keep the pump module self-contained).
fn now_nanos() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_nanos() as u64
}

/// The DECRYPT pump loop: write any handshake-decrypted `early` plaintext to `dst`
/// first, then `poll(src, POLLIN)` + `splice(src → pipe → dst)` per ready record,
/// advancing the progress counter. Bounded, non-blocking, zero-copy on the steady
/// state. `src` is a kTLS-RX socket; `dst` is a plaintext socket.
///
/// `early` is the 0.5-RTT plaintext rustls already decrypted during the handshake
/// (drained from `conn.reader()` before the kTLS-RX arm — see
/// `mtls::drain_early_plaintext`). It is written to `dst` FIRST, on this thread,
/// with a plain blocking `write_all` (`dst` is plaintext — no kTLS on the
/// destination), so the downstream leg sees every early byte in order ahead of the
/// steady-state records. The kTLS-RX `rec_seq` already accounts for these records,
/// so the splice loop below resumes at the NEXT on-wire record — no byte is lost or
/// double-delivered.
fn run_decrypt_pump(src_fd: RawFd, dst_fd: RawFd, early: &[u8], state: &PumpState) {
    if !early.is_empty() {
        // SAFETY: borrow `dst_fd` as a `TcpStream` WITHOUT ownership; `forget` it so
        // the leg fd is not closed here (the adapter's per-connection table owns it).
        let dst = unsafe { TcpStream::from_raw_fd(dst_fd) };
        state.record_pending.store(true, Ordering::SeqCst);
        let wrote = (&dst).write_all(early).and_then(|()| (&dst).flush());
        std::mem::forget(dst);
        if wrote.is_err() {
            state.record_pending.store(false, Ordering::SeqCst);
            state.running.store(false, Ordering::SeqCst);
            return;
        }
        record_progress(state, early.len() as u64);
        state.record_pending.store(false, Ordering::SeqCst);
    }

    let mut fds = [0 as RawFd; 2];
    // SAFETY: `pipe2` writes two fds into the 2-element array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK) } != 0 {
        state.running.store(false, Ordering::SeqCst);
        return;
    }
    let pipe_r = fds[0];
    let pipe_w = fds[1];
    let flags = (libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK) as libc::c_uint;
    let chunk: usize = 65536;

    while !state.stop.load(Ordering::SeqCst) {
        let mut pfd = libc::pollfd { fd: src_fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: single pollfd, bounded 40 ms timeout.
        let pr = unsafe { libc::poll(std::ptr::from_mut(&mut pfd), 1, 40) };
        if pr <= 0 {
            state.record_pending.store(false, Ordering::SeqCst);
            continue;
        }
        // Drain readable data FIRST, then react to a hangup. POLLHUP arrives
        // coincident with the last readable bytes (`debugging.md` § 11 — confirm the
        // source is drained before treating a hangup as terminal). Only when there
        // is NO readable data is a hangup/error terminal.
        if pfd.revents & libc::POLLIN == 0 {
            if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                break; // source hung up with nothing left to read — pump is done
            }
            state.record_pending.store(false, Ordering::SeqCst);
            continue;
        }
        state.record_pending.store(true, Ordering::SeqCst);
        // SAFETY: splice from the kTLS-RX socket into the pipe (kernel decrypts).
        let n_in = unsafe {
            libc::splice(src_fd, std::ptr::null_mut(), pipe_w, std::ptr::null_mut(), chunk, flags)
        };
        if n_in < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::EAGAIN {
                state.record_pending.store(false, Ordering::SeqCst);
                continue;
            }
            break;
        }
        if n_in == 0 {
            break; // EOF on the source
        }
        let mut remaining = n_in;
        while remaining > 0 && !state.stop.load(Ordering::SeqCst) {
            // `remaining` is the byte count returned by the prior `splice` (always
            // > 0 here), so the sign-loss cast to usize cannot lose sign.
            #[allow(clippy::cast_sign_loss)]
            let want = remaining as usize;
            // SAFETY: splice from the pipe into the plaintext destination socket.
            let n_out = unsafe {
                libc::splice(
                    pipe_r,
                    std::ptr::null_mut(),
                    dst_fd,
                    std::ptr::null_mut(),
                    want,
                    flags,
                )
            };
            if n_out < 0 {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if errno == libc::EAGAIN {
                    // Destination socket send buffer is full. Wait on the REAL
                    // condition — `poll(dst, POLLOUT)` blocks until the kernel
                    // reports the socket is writable again (or the bounded timeout
                    // elapses). The next `splice` retries once `dst` drains; the stop
                    // flag is re-checked each iteration.
                    let mut pfd = libc::pollfd { fd: dst_fd, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: single pollfd, bounded 40 ms timeout; advisory.
                    unsafe {
                        libc::poll(std::ptr::from_mut(&mut pfd), 1, 40);
                    }
                    continue;
                }
                break;
            }
            if n_out == 0 {
                break;
            }
            // `n_out` is a positive byte count from `splice`; the cast to u64 is
            // exact and non-negative.
            #[allow(clippy::cast_sign_loss)]
            record_progress(state, n_out as u64);
            remaining -= n_out;
        }
    }

    // SAFETY: closing the pipe ends; the leg fds are owned by the adapter's
    // per-connection table, closed on teardown.
    unsafe {
        libc::close(pipe_r);
        libc::close(pipe_w);
    }
    state.record_pending.store(false, Ordering::SeqCst);
    state.running.store(false, Ordering::SeqCst);
}

/// The ENCRYPT pump loop: a blocking `read(src) → write_all(dst)` copy. `dst` is a
/// kTLS-TX socket, so each `write_all` frames exactly one TLS record the kernel
/// encrypts in `tls_sw_sendmsg` (the agent does no crypto). `src` is a plaintext
/// socket with a bounded read timeout so the stop flag is re-checked promptly.
fn run_encrypt_pump(src_fd: RawFd, dst_fd: RawFd, prelude: &[u8], state: &PumpState) {
    // Borrow both legs as `TcpStream`s WITHOUT taking ownership (forget at the end so
    // the leg fds are not closed — the adapter's per-connection table owns them).
    // SAFETY: the fds are live for the pump's lifetime (closed only on teardown,
    // after this thread is joined); we `forget` both at the end so Drop does not
    // double-close.
    let src = unsafe { TcpStream::from_raw_fd(src_fd) };
    let dst = unsafe { TcpStream::from_raw_fd(dst_fd) };
    // A short read timeout makes the blocking read return promptly so the stop flag
    // is re-checked; it does NOT drop data (a timeout just re-loops).
    src.set_read_timeout(Some(Duration::from_millis(40))).ok();

    // Write the prelude (captured pre-arm plaintext) FIRST, on THIS thread, so leg
    // B's kTLS-TX has a SINGLE writer for every forward byte. Writing the pre-arm
    // bytes from the `establish` thread and the steady-state bytes from this pump
    // thread desynced the kTLS-TX record sequence ~10-15% of the time (the peer
    // reconstructed only the pre-arm prefix) — routing both through this one thread
    // is the fix.
    if !prelude.is_empty() {
        state.record_pending.store(true, Ordering::SeqCst);
        if (&dst).write_all(prelude).and_then(|()| (&dst).flush()).is_err() {
            std::mem::forget(src);
            std::mem::forget(dst);
            state.record_pending.store(false, Ordering::SeqCst);
            state.running.store(false, Ordering::SeqCst);
            return;
        }
        record_progress(state, prelude.len() as u64);
        state.record_pending.store(false, Ordering::SeqCst);
    }

    let mut buf = vec![0u8; 65536];
    while !state.stop.load(Ordering::SeqCst) {
        match (&src).read(&mut buf) {
            Ok(0) => break, // EOF — source closed
            Ok(n) => {
                state.record_pending.store(true, Ordering::SeqCst);
                // Blocking write_all into the kTLS-TX leg: the kernel waits for send
                // buffer space and frames exactly one record per write (the proven
                // kTLS-TX primitive, NOT a nonblocking splice).
                if (&dst).write_all(&buf[..n]).and_then(|()| (&dst).flush()).is_err() {
                    break;
                }
                record_progress(state, n as u64);
                state.record_pending.store(false, Ordering::SeqCst);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // No data within the poll window — re-check stop and loop. A purely
                // idle connection sits here, `Running` (no pending record).
                state.record_pending.store(false, Ordering::SeqCst);
            }
            Err(_) => break,
        }
    }

    std::mem::forget(src);
    std::mem::forget(dst);
    state.record_pending.store(false, Ordering::SeqCst);
    state.running.store(false, Ordering::SeqCst);
}
