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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// A connection-level self-teardown trigger (B) D-MTLS-16 / ADR-0070. Installed by
/// the adapter after the connection is registered; invoked by EITHER pump thread on
/// its own terminal exit (EOF / error / `ETIMEDOUT`) so the connection tears itself
/// down fail-closed — close both legs, stop the sibling pump, reclaim kTLS — without
/// a central worker query. Idempotent: the first pump to exit triggers it; the
/// sibling's later exit is a no-op (the trigger fires once, off a detached reaper, so
/// the calling pump thread never joins itself). The closure carries the connection's
/// `EnforcedConnectionId` / alloc for the re-homed telemetry.
pub(super) type SelfTeardown = Arc<dyn Fn() + Send + Sync>;

/// Shared observation surface for one connection's pump. `liveness` reads
/// `bytes_spliced` + `running`; `teardown` sets `stop`.
#[derive(Default)]
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
    /// (B) D-MTLS-16: the connection-level self-teardown trigger. The adapter installs
    /// it post-registration; the pump invokes it ON A TERMINAL EXIT THAT WAS NOT a
    /// deliberate `teardown` (i.e. EOF / error / `ETIMEDOUT`, NOT `stop == true`). The
    /// `OnceLock` makes installation race-free and the read lock-free; the trigger's
    /// own idempotency guards a double-fire from both pumps.
    self_teardown: OnceLock<SelfTeardown>,
}

impl PumpState {
    /// Install the (B) self-teardown trigger for this connection. Called by the
    /// adapter exactly once, after the connection is registered. A second install is
    /// a no-op (the first winner stands) — the trigger is connection-level, shared by
    /// both the primary and the sibling pump's state.
    pub(super) fn install_self_teardown(&self, trigger: SelfTeardown) {
        // `set` returns Err if already installed; the first install wins and a
        // duplicate is harmless (both carry the same connection's teardown).
        let _ = self.self_teardown.set(trigger);
    }

    /// `true` iff the (B) self-teardown trigger has been installed on this pump's
    /// state. Test-only observable for the `register`-wires-every-pump unit test
    /// (`mod.rs`): a pump whose trigger is absent is a permanent self-teardown no-op
    /// (`fire_self_teardown_if_unexpected` finds `None`), which is exactly the
    /// per-connection-leak this proves against.
    #[cfg(test)]
    pub(super) fn has_self_teardown(&self) -> bool {
        self.self_teardown.get().is_some()
    }

    /// Fire the (B) self-teardown trigger IF this exit was terminal-unexpected (the
    /// pump broke on EOF / error / `ETIMEDOUT`, not on a deliberate `teardown` that
    /// set `stop`). A deliberate teardown is already reclaiming the connection, so
    /// re-triggering would be redundant. The trigger itself is idempotent, so a race
    /// between the two pumps' exits collapses to one teardown.
    fn fire_self_teardown_if_unexpected(&self) {
        if self.stop.load(Ordering::SeqCst) {
            return; // a deliberate teardown is already in progress
        }
        if let Some(trigger) = self.self_teardown.get() {
            trigger();
        }
    }
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

/// Why a pump thread reached its terminal exit — distinguishes a graceful close (a
/// clean EOF, or a deliberate `teardown` that set `stop`) from a transport-death (an
/// error / the (C) kernel-reaped `ETIMEDOUT` on a leg). A [`PumpExit::TransportDeath`]
/// on ANY pump that carries the (B) trigger fires the per-connection self-teardown;
/// the trigger is installed into EVERY pump (primary + each aux) by
/// [`super::HostMtlsEnforcement::register`], so whichever pump FIRST observes a
/// transport death self-tears the connection down — notably the OUTBOUND aux return
/// pump (`splice(legB → legF)`), the sole observer of leg-B death while the workload
/// is idle. A [`PumpExit::Graceful`] close (a clean EOF, or a finished response
/// direction) on ANY pump does NOT reclaim — the `PumpExit::Graceful` gate (NOT a
/// primary-only install) is what preserves the D-MTLS-16 intent that a clean
/// half-close MUST NOT nuke a connection whose request path is still live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PumpExit {
    /// Clean EOF on a leg, or a deliberate `teardown` set `stop` — NOT a death.
    Graceful,
    /// A leg error / the (C) kernel-reaped `ETIMEDOUT` — the connection is dead.
    TransportDeath,
}

/// A1 (ADR-0070 amendment 2026-07-01, Root Cause A): forward a source directional
/// clean-close to the OPPOSING leg as a half-close. On a [`PumpExit::Graceful`] that
/// is a genuine *source* EOF (a peer FIN observed on the pump's source leg —
/// `POLLHUP`-with-no-`POLLIN`, `n_in == 0`, or the `splice_pipe_to_dst` dst-EOF —
/// NOT a deliberate `teardown`), `shutdown(dst_fd, SHUT_WR)` mirrors the peer's FIN
/// onto the opposing leg's write side so the peer on that leg surfaces read-EOF. This
/// is the S-DBN-CHURN fix: a backend that closes cleanly on the return-decrypt pump's
/// source (leg-B) forwards the FIN to leg-F, so a client holding an in-flight read
/// fails fast instead of hanging until `TCP_USER_TIMEOUT`.
///
/// Half-close correctness is PRESERVED (D-MTLS-16): `SHUT_WR` closes ONLY the write
/// side of the opposing leg — the sibling direction stays live if the peer is still
/// sending, and this does NOT reclaim the connection (`mark_exited` still does not
/// fire the (B) self-teardown for `Graceful`). The connection reclaims naturally once
/// BOTH directions have forwarded their close and both legs reach full EOF.
///
/// Gated on `exit == Graceful && !state.stop`: a DELIBERATE `teardown` set `stop`
/// (and also breaks `Graceful` at the top of the loop), and the reclaim path owns the
/// close on that shape — a redundant `SHUT_WR` there is wrong. This reuses the SAME
/// `stop`-guard `fire_self_teardown_if_unexpected` uses to distinguish a deliberate
/// teardown from a source EOF. `SHUT_WR` on a kTLS-TX `dst` (the encrypt pumps' leg-B
/// / leg-C) sends a bare TCP FIN without a TLS `close_notify` — accepted for v1 (the
/// peer's kTLS-RX observes the transport FIN as a clean read-EOF, which is all the
/// opposing pump needs; emitting `close_notify` is out of scope for the amendment).
fn forward_half_close_if_source_eof(dst_fd: RawFd, exit: PumpExit, state: &PumpState) {
    // Forward ONLY on a genuine source clean-close: a `Graceful` exit that was NOT a
    // deliberate `teardown`. `stop == true` breaks `Graceful` at the top of the loop
    // and means the reclaim path already owns the close — reuse the SAME stop-guard
    // `fire_self_teardown_if_unexpected` uses. A `TransportDeath` exit reclaims via
    // (B) self-teardown, not a half-close forward.
    if exit != PumpExit::Graceful || state.stop.load(Ordering::SeqCst) {
        return;
    }
    // SAFETY: `shutdown(SHUT_WR)` closes only the WRITE side of the opposing leg,
    // mirroring the source peer's FIN onto `dst`. It does not close the fd (the
    // adapter's per-connection table owns it) and does not touch the sibling
    // direction's read side — half-close correctness (D-MTLS-16) is preserved. Works
    // uniformly on AF_INET (production leg fds) and AF_UNIX (test socketpair).
    unsafe {
        libc::shutdown(dst_fd, libc::SHUT_WR);
    }
}

/// (B) D-MTLS-16 / ADR-0070: a pump thread's single terminal exit point — mark the
/// pump exited (`running = false`, the `liveness` `Gone` observable) and, on a
/// [`PumpExit::TransportDeath`] (NOT a graceful close), fire the connection's
/// self-teardown. Every pump (primary + each aux) carries the trigger (installed by
/// [`super::HostMtlsEnforcement::register`]), so whichever pump first observes a
/// transport death drives the reclaim; the trigger's own idempotency collapses a race
/// between the two pumps to one teardown. The whole connection then reclaims
/// fail-closed off the pump's own thread (no central worker query). The order is
/// load-bearing: clear `running` BEFORE firing so the reaper's `liveness` re-query
/// observes `Gone`. The self-teardown trigger runs the reclaim on a detached reaper
/// (the pump thread never joins itself).
fn mark_exited(state: &PumpState, exit: PumpExit) {
    state.running.store(false, Ordering::SeqCst);
    if exit == PumpExit::TransportDeath {
        state.fire_self_teardown_if_unexpected();
    }
}

/// Drain `n_in` bytes from the pipe (read half `pipe_r`) into the plaintext
/// destination socket `dst_fd`, advancing the progress counter per spliced chunk.
/// Returns `Some(exit)` if the drain hit a terminal condition (a `dst` leg error ⇒
/// `TransportDeath`, a clean `dst` EOF ⇒ `Graceful`) and `None` once the whole
/// `n_in` is delivered. On a full `dst` send buffer (`EAGAIN`) it parks on
/// `poll(dst, POLLOUT)` (bounded 40 ms) and retries, re-checking the stop flag each
/// iteration. Extracted from `run_decrypt_pump` so the outer splice-in loop stays
/// under the line ceiling.
fn splice_pipe_to_dst(
    pipe_r: RawFd,
    dst_fd: RawFd,
    n_in: isize,
    flags: libc::c_uint,
    state: &PumpState,
) -> Option<PumpExit> {
    let mut remaining = n_in;
    while remaining > 0 && !state.stop.load(Ordering::SeqCst) {
        // `remaining` is the byte count returned by the prior `splice` (always
        // > 0 here), so the sign-loss cast to usize cannot lose sign.
        #[allow(clippy::cast_sign_loss)]
        let want = remaining as usize;
        // SAFETY: splice from the pipe into the plaintext destination socket.
        let n_out = unsafe {
            libc::splice(pipe_r, std::ptr::null_mut(), dst_fd, std::ptr::null_mut(), want, flags)
        };
        if n_out < 0 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::EAGAIN {
                // Destination socket send buffer is full. Wait on the REAL condition —
                // `poll(dst, POLLOUT)` blocks until the kernel reports the socket is
                // writable again (or the bounded timeout elapses). The next `splice`
                // retries once `dst` drains; the stop flag is re-checked each iteration.
                let mut pfd = libc::pollfd { fd: dst_fd, events: libc::POLLOUT, revents: 0 };
                // SAFETY: single pollfd, bounded 40 ms timeout; advisory.
                unsafe {
                    libc::poll(std::ptr::from_mut(&mut pfd), 1, 40);
                }
                continue;
            }
            return Some(PumpExit::TransportDeath); // dst leg error
        }
        if n_out == 0 {
            return Some(PumpExit::Graceful); // dst clean EOF
        }
        // `n_out` is a positive byte count from `splice`; the cast to u64 is exact
        // and non-negative.
        #[allow(clippy::cast_sign_loss)]
        record_progress(state, n_out as u64);
        remaining -= n_out;
    }
    None
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
            mark_exited(state, PumpExit::TransportDeath);
            return;
        }
        record_progress(state, early.len() as u64);
        state.record_pending.store(false, Ordering::SeqCst);
    }

    let mut fds = [0 as RawFd; 2];
    // SAFETY: `pipe2` writes two fds into the 2-element array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK) } != 0 {
        mark_exited(state, PumpExit::TransportDeath);
        return;
    }
    let pipe_r = fds[0];
    let pipe_w = fds[1];
    let flags = (libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK) as libc::c_uint;
    let chunk: usize = 65536;

    let exit = loop {
        if state.stop.load(Ordering::SeqCst) {
            break PumpExit::Graceful; // a deliberate teardown stopped us
        }
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
            if pfd.revents & libc::POLLERR != 0 {
                break PumpExit::TransportDeath; // source error (e.g. ETIMEDOUT/RST)
            }
            if pfd.revents & libc::POLLHUP != 0 {
                break PumpExit::Graceful; // source hung up with nothing left — clean EOF
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
            break PumpExit::TransportDeath; // splice error on the source leg
        }
        if n_in == 0 {
            break PumpExit::Graceful; // clean EOF on the source
        }
        if let Some(inner) = splice_pipe_to_dst(pipe_r, dst_fd, n_in, flags, state) {
            break inner;
        }
    };

    // SAFETY: closing the pipe ends; the leg fds are owned by the adapter's
    // per-connection table, closed on teardown.
    unsafe {
        libc::close(pipe_r);
        libc::close(pipe_w);
    }
    state.record_pending.store(false, Ordering::SeqCst);
    // A1 (ADR-0070 amendment): a source clean-close forwards a half-close to the
    // opposing leg (`dst_fd`) BEFORE `mark_exited`, so the peer on that leg surfaces
    // read-EOF. For the return-decrypt pump (`legB → legF`), this is the S-DBN-CHURN
    // fix: a backend FIN on leg-B mirrors to leg-F. A deliberate teardown does not
    // forward (the reclaim path owns the close).
    forward_half_close_if_source_eof(dst_fd, exit, state);
    mark_exited(state, exit);
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
            mark_exited(state, PumpExit::TransportDeath);
            return;
        }
        record_progress(state, prelude.len() as u64);
        state.record_pending.store(false, Ordering::SeqCst);
    }

    let mut buf = vec![0u8; 65536];
    let exit = loop {
        if state.stop.load(Ordering::SeqCst) {
            break PumpExit::Graceful; // a deliberate teardown stopped us
        }
        match (&src).read(&mut buf) {
            Ok(0) => break PumpExit::Graceful, // clean EOF — source closed gracefully
            Ok(n) => {
                state.record_pending.store(true, Ordering::SeqCst);
                // Blocking write_all into the kTLS-TX leg: the kernel waits for send
                // buffer space and frames exactly one record per write (the proven
                // kTLS-TX primitive, NOT a nonblocking splice).
                if (&dst).write_all(&buf[..n]).and_then(|()| (&dst).flush()).is_err() {
                    break PumpExit::TransportDeath; // dst leg write error
                }
                record_progress(state, n as u64);
                state.record_pending.store(false, Ordering::SeqCst);
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                // No data within the 40 ms `SO_RCVTIMEO` window (`WouldBlock`/`TimedOut`),
                // OR the blocking read was interrupted by a signal (`Interrupted` ==
                // `EINTR`) — re-check stop and loop. A purely idle connection sits here,
                // `Running` (no pending record). `EINTR` is a BENIGN interruption (a
                // signal delivered to this thread mid-`read` — e.g. a debugger PTRACE
                // attach, a timer, a `SIGCHLD`), NOT a transport death: retrying the
                // read is the only correct response (the POSIX contract). Misclassifying
                // it as `TransportDeath` would spuriously self-tear-down a healthy
                // connection now that every pump carries the (B) trigger. NOTE: a genuine
                // `TCP_USER_TIMEOUT` `ETIMEDOUT` also maps to `TimedOut`; distinguishing
                // the (C) kernel-reap from the 40 ms poll re-check (so the peer-vanishes
                // case fires (B) promptly rather than at the next read error) is part of
                // step 06-03 commit 2's e2e proof — the connection still self-tears-down
                // here on the subsequent leg error/EOF, just not on this poll tick.
                state.record_pending.store(false, Ordering::SeqCst);
            }
            Err(_) => break PumpExit::TransportDeath, // src leg read error
        }
    };

    std::mem::forget(src);
    std::mem::forget(dst);
    state.record_pending.store(false, Ordering::SeqCst);
    // A1 (ADR-0070 amendment): a source clean-close forwards a half-close to the
    // opposing leg (`dst_fd`) BEFORE `mark_exited`. For the encrypt pumps the `dst`
    // is a kTLS-TX leg (leg-B / leg-C), so `SHUT_WR` sends a bare TCP FIN without a
    // TLS `close_notify` — accepted for v1 (the peer's kTLS-RX sees a clean read-EOF,
    // which is all the opposing pump needs). A deliberate teardown does not forward.
    forward_half_close_if_source_eof(dst_fd, exit, state);
    mark_exited(state, exit);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "standard test idiom: expect-with-message is the right failure for a test precondition (matches the mod.rs test-module allow)"
)]
mod tests {
    //! Boundary unit tests for the (B) D-MTLS-16 self-teardown gate (`mark_exited` →
    //! `fire_self_teardown_if_unexpected`) — its own driving port (a decision over the
    //! `PumpState` atomics + the [`PumpExit`] reason, Mandate 2). Pins the load-bearing
    //! distinctions: a `TransportDeath` exit fires the trigger; a `Graceful` exit (a
    //! clean EOF, OR a deliberate `teardown`) does NOT — a clean half-close must not
    //! nuke a connection whose sibling direction is still live; `running` is cleared to
    //! `Gone` on EVERY exit; and the trigger fires at most once across both pumps.

    use super::*;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::AtomicU32;

    fn fire_counter() -> (Arc<AtomicU32>, SelfTeardown) {
        let fired = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&fired);
        let trigger: SelfTeardown = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        (fired, trigger)
    }

    /// A `TransportDeath` exit (a leg error / the (C) kernel-reaped `ETIMEDOUT`) fires
    /// the (B) self-teardown trigger exactly once and clears `running` to the `Gone`
    /// observable. Kills a `delete state.fire_self_teardown_if_unexpected()` mutation
    /// (the trigger would never fire) and a `running` not-cleared mutation.
    #[test]
    fn transport_death_exit_fires_self_teardown_once_and_marks_gone() {
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        let (fired, trigger) = fire_counter();
        state.install_self_teardown(trigger);

        mark_exited(&state, PumpExit::TransportDeath);

        assert_eq!(fired.load(Ordering::SeqCst), 1, "a transport-death exit fires (B)");
        assert!(!state.running.load(Ordering::SeqCst), "running cleared ⇒ liveness Gone");
    }

    /// A `Graceful` exit (a clean EOF on one direction) does NOT fire (B) — the
    /// connection's sibling direction may still be live, and a clean half-close is not
    /// a connection death. This is the orthogonal-safety gate that lets the (B) trigger
    /// be installed into EVERY pump (primary + each aux, per
    /// [`super::HostMtlsEnforcement::register`]) WITHOUT a clean half-close on any of
    /// them nuking a live connection: `mark_exited` keys the fire on
    /// `PumpExit::TransportDeath` ONLY, not on which pump (primary vs aux) the state
    /// belongs to — `PumpState` carries no primary/aux distinction, so this case
    /// covers every pump uniformly. Kills a `PumpExit::TransportDeath`-arm-deletion /
    /// a `==` → `!=` mutation in `mark_exited` (either would self-tear-down a
    /// gracefully half-closing connection — the exact half-close hazard the
    /// `PumpExit::Graceful` gate exists to prevent). `running` is STILL cleared to
    /// `Gone`.
    #[test]
    fn graceful_eof_exit_does_not_fire_self_teardown() {
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        let (fired, trigger) = fire_counter();
        state.install_self_teardown(trigger);

        mark_exited(&state, PumpExit::Graceful);

        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "a clean EOF half-close must NOT self-tear-down (sibling direction may be live)"
        );
        assert!(!state.running.load(Ordering::SeqCst), "running still cleared ⇒ liveness Gone");
    }

    /// A deliberate `teardown` (stop == true) does NOT re-fire (B) even on a
    /// `TransportDeath` exit — the external teardown is already reclaiming the
    /// connection. Kills a `delete the stop-guard` mutation (which would double-reclaim
    /// when a leg errors during a deliberate teardown).
    #[test]
    fn deliberate_teardown_does_not_refire_even_on_transport_death() {
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        state.stop.store(true, Ordering::SeqCst); // a deliberate teardown set this
        let (fired, trigger) = fire_counter();
        state.install_self_teardown(trigger);

        mark_exited(&state, PumpExit::TransportDeath);

        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "a deliberate teardown (stop == true) must NOT re-fire (B) self-teardown"
        );
        assert!(!state.running.load(Ordering::SeqCst), "running still cleared ⇒ liveness Gone");
    }

    /// No trigger installed (a pump that exits before the adapter registered the
    /// connection) is a safe no-op — `mark_exited` still clears `running` but fires
    /// nothing. Pins the `OnceLock::get()` None branch.
    #[test]
    fn transport_death_without_installed_trigger_is_a_noop() {
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        // No `install_self_teardown` — the trigger is absent.
        mark_exited(&state, PumpExit::TransportDeath);
        assert!(!state.running.load(Ordering::SeqCst), "running cleared even with no trigger");
    }

    // ========================================================================
    // A1 (ADR-0070 amendment) — the directional clean-close half-close forward.
    // The pump's terminal-exit DECISION over (dst_fd, PumpExit, PumpState) is its
    // own driving port (Mandate 2). The observable is real: a socketpair peer on
    // the dst leg's READ side surfaces EOF (read returns 0) iff `SHUT_WR` was
    // forwarded onto the dst leg's write side.
    // ========================================================================

    /// A real connected `AF_UNIX` socketpair. `held` is the leg the pump would own
    /// (`dst_fd` — passed to the forward), `peer` is the OPPOSING end the test reads
    /// to observe whether the pump forwarded a `SHUT_WR` (a `read` on `peer` returns
    /// `Ok(0)` — EOF — iff the pump shut down `held`'s write side). Both raw fds are
    /// kept live by the returned `UnixStream`s (dropped at test end).
    fn dst_socketpair() -> (std::os::unix::net::UnixStream, std::os::unix::net::UnixStream) {
        std::os::unix::net::UnixStream::pair().expect("AF_UNIX socketpair")
    }

    /// `true` iff a blocking `read` on `peer` observes EOF (`Ok(0)`) within a short
    /// bound — i.e. the pump forwarded `shutdown(SHUT_WR)` onto the socketpair's other
    /// end. A short read timeout guards against the negative case hanging the test.
    fn peer_read_saw_eof(peer: &std::os::unix::net::UnixStream) -> bool {
        peer.set_read_timeout(Some(Duration::from_millis(500))).ok();
        let mut buf = [0u8; 8];
        // After a `SHUT_WR` on the held end, the peer's read returns Ok(0) (EOF). With
        // no shutdown, the socket stays open and the read blocks to the timeout
        // (WouldBlock/TimedOut), which is NOT EOF.
        matches!((&*peer).read(&mut buf), Ok(0))
    }

    /// S-CHURN-HALFCLOSE-FORWARD: a SOURCE clean-close (`Graceful` with `stop == false`)
    /// forwards `shutdown(SHUT_WR)` to the `dst` leg — the socketpair peer on the dst
    /// leg's READ side surfaces EOF after the forward — AND `mark_exited` still does
    /// NOT fire self-teardown on `Graceful` (half-close correctness preserved). This is
    /// the A1 mutation-killing proof: deleting the `shutdown(SHUT_WR)` forward makes the
    /// dst peer never see EOF.
    #[test]
    fn source_clean_close_forwards_half_close_to_dst_and_does_not_reclaim() {
        let (held, peer) = dst_socketpair();
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        let (fired, trigger) = fire_counter();
        state.install_self_teardown(trigger);

        // A genuine SOURCE clean EOF: Graceful with stop == false (the pump broke on
        // POLLHUP-with-no-POLLIN / n_in == 0 / dst-EOF, NOT a deliberate teardown).
        forward_half_close_if_source_eof(held.as_raw_fd(), PumpExit::Graceful, &state);
        mark_exited(&state, PumpExit::Graceful);

        assert!(
            peer_read_saw_eof(&peer),
            "a source clean-close (Graceful, stop==false) must forward shutdown(SHUT_WR) to the \
             dst leg — the opposing peer's read must surface EOF (the S-DBN-CHURN half-close forward)"
        );
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "a clean half-close forward must NOT fire (B) self-teardown — no reclaim on Graceful \
             (D-MTLS-16 half-close correctness: the sibling direction may still be live)"
        );
        assert!(!state.running.load(Ordering::SeqCst), "running still cleared ⇒ liveness Gone");
    }

    /// S-CHURN-TEARDOWN-NO-FORWARD: a DELIBERATE teardown (`stop == true`, which also
    /// breaks `Graceful` at the top of the loop) does NOT forward a half-close — the
    /// reclaim path owns the close and a redundant `SHUT_WR` here is wrong. The dst leg
    /// is NOT shut down by the pump. Pins the `!state.stop` guard on the forward (the
    /// SAME stop-guard `fire_self_teardown_if_unexpected` uses).
    #[test]
    fn deliberate_teardown_does_not_forward_half_close() {
        let (held, peer) = dst_socketpair();
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);
        state.stop.store(true, Ordering::SeqCst); // a deliberate teardown set this

        forward_half_close_if_source_eof(held.as_raw_fd(), PumpExit::Graceful, &state);

        assert!(
            !peer_read_saw_eof(&peer),
            "a deliberate teardown (stop == true) must NOT forward shutdown(SHUT_WR) — the reclaim \
             path owns the close; the dst peer must NOT surface EOF from a pump-side half-close"
        );
    }

    /// A `TransportDeath` exit does NOT forward a half-close — the forward is scoped to
    /// the CLEAN-close (`Graceful`) class; a transport death reclaims via (B)
    /// self-teardown, not a half-close forward. Pins the `exit == Graceful` guard on
    /// the forward (a mutation flipping the arm to fire on `TransportDeath` would shut
    /// the dst write side down twice — once here, once via the reclaim path).
    #[test]
    fn transport_death_does_not_forward_half_close() {
        let (held, peer) = dst_socketpair();
        let state = PumpState::default();
        state.running.store(true, Ordering::SeqCst);

        forward_half_close_if_source_eof(held.as_raw_fd(), PumpExit::TransportDeath, &state);

        assert!(
            !peer_read_saw_eof(&peer),
            "a TransportDeath exit must NOT forward a half-close — the forward is Graceful-only \
             (transport death reclaims via (B) self-teardown, not a half-close forward)"
        );
    }
}
