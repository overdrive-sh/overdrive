//! Agent-light `splice(2)` pump — the return (outbound `legB → pipe → legF`) and
//! deliver (inbound `legC → pipe → legS`) directions, on a plain (NO psock)
//! kTLS-RX leg.
//!
//! Proven in `findings-splice-return.md` (increment-h, RELAY_EXACT_CLEAN) and
//! `findings-inbound-intercept.md` §5: `splice(legX → pipe → legY,
//! SPLICE_F_MOVE|SPLICE_F_NONBLOCK)` decrypts each kTLS record
//! (`tls_sw_splice_read`) and moves the CLEAN decrypted plaintext (no TLS framing)
//! to the destination with the agent issuing only `splice`/`poll` — ZERO per-byte
//! userspace copy. ~1 splice per TLS record (≤ 16 KiB each).
//!
//! The pump runs on a blocking thread for the connection's life; it tracks a
//! shared bytes-spliced progress counter (`liveness` reads it via [`PumpHandle`])
//! and a stop flag (`teardown` sets it). SD-2: the port owns the pump.

use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Shared observation surface for one connection's splice pump. `liveness` reads
/// `bytes_spliced` + `running`; `teardown` sets `stop`.
#[derive(Debug, Default)]
pub(super) struct PumpState {
    /// Monotonic count of plaintext bytes the pump has spliced to the
    /// destination — the progress metric `liveness` watches for a stall.
    pub bytes_spliced: AtomicU64,
    /// Set by `teardown` to stop the pump; the pump thread exits its loop.
    pub stop: AtomicBool,
    /// Cleared when the pump thread has fully exited.
    pub running: AtomicBool,
    /// `true` while a record is pending on the source (kTLS-RX) leg — the
    /// stall-deadline only applies while a record is actually pending (a purely
    /// idle connection is `Running`, never `Stalled`).
    pub record_pending: AtomicBool,
    /// Wall-clock nanos (since the process monotonic origin) of the last
    /// progress advance, for the `Stalled { since }` computation.
    pub last_progress_unix_nanos: AtomicU64,
}

/// Handle the adapter holds per connection; gives `liveness`/`teardown` the
/// shared state and the worker-thread join handle.
pub(super) struct PumpHandle {
    pub state: Arc<PumpState>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl PumpHandle {
    /// Spawn the splice pump from `src_fd` (the plain kTLS-RX leg) into `dst_fd`
    /// (the plaintext destination — leg F outbound / leg S inbound). The pump
    /// drives until `state.stop` is set or both legs close.
    pub(super) fn spawn(src_fd: RawFd, dst_fd: RawFd, now_unix_nanos: u64) -> Self {
        let state = Arc::new(PumpState::default());
        state.running.store(true, Ordering::SeqCst);
        state.last_progress_unix_nanos.store(now_unix_nanos, Ordering::SeqCst);
        let pump_state = Arc::clone(&state);
        let join = std::thread::spawn(move || run_pump(src_fd, dst_fd, &pump_state));
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

/// The pump loop: `poll(src, POLLIN)` then `splice(src → pipe → dst)` per ready
/// record, advancing `bytes_spliced`. Bounded, non-blocking, agent-light.
fn run_pump(src_fd: RawFd, dst_fd: RawFd, state: &PumpState) {
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
        if pfd.revents & libc::POLLIN == 0 {
            state.record_pending.store(false, Ordering::SeqCst);
            continue;
        }
        if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            break; // peer closed — pump is done
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
                    std::thread::sleep(Duration::from_millis(1));
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
            state.bytes_spliced.fetch_add(n_out as u64, Ordering::SeqCst);
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
