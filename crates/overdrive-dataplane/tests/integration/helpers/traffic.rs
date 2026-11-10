//! Sustained-traffic generation + capture helpers for Tier 3
//! zero-drop swap tests (S-2.2-09 / Slice 03).
//!
//! Companion to `super::packets` and `super::veth`. The single-frame
//! `synthesise_tcp_syn_with_src_port` helper from `packets.rs`
//! produces one Ethernet+IPv4+TCP-SYN frame; this module loops it
//! into a paced send for a duration, and provides a deadline-bounded
//! capture loop. Send count and receive count are the load-bearing
//! observables — the zero-drop gate is `sent == received`.
//!
//! Per `.claude/rules/development.md` § "aya-rs XDP / TC kernel-side
//! patterns" — this is purely userspace test infrastructure; the
//! kernel side just sees a stream of `AF_PACKET` `sendto(2)`
//! syscalls.
//!
//! # Why coarse-grained pacing
//!
//! The CI gate is `sent == received`, not absolute pps. A precise
//! token-bucket rate limiter would buy nothing — what matters is
//! that the sender (a) emits a known count of frames and (b) does
//! not overrun the kernel's socket queue (which would cause
//! `sendto` to drop or block before the kernel even sees the
//! frame). A "send batch, sleep, repeat" loop is sufficient: each
//! batch's elapsed time plus the sleep determines effective pps,
//! and overruns surface as `sendto` errors that we propagate up
//! rather than silently absorbing.

#![allow(
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::ptr_as_ptr,
    clippy::ref_as_ptr,
    clippy::borrow_as_ptr,
    clippy::unnecessary_cast,
    clippy::unnested_or_patterns,
    clippy::unchecked_time_subtraction,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::expect_used
)]

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::packets::{ETH_HDR_LEN, IPV4_HDR_LEN, synthesise_tcp_syn_with_src_port};

const ETH_P_ALL: std::os::raw::c_int = 0x0003;

/// Send TCP SYN frames at a target rate for the configured duration.
/// Returns the count of frames `sendto` returned ≥ 0 for (the actual
/// send count — zero-drop semantics demand we know the truth).
///
/// Pacing strategy: the run is divided into 100 ms slices. Each
/// slice emits `target_pps / 10` frames, then sleeps for whatever
/// remains of its 100 ms budget. Runs go for `duration`; if the
/// total elapsed wall-clock crosses `duration`, the final slice
/// truncates rather than overshooting.
///
/// `base_src_port` seeds the TCP source port; each successive frame
/// uses `base_src_port + i` so the placeholder slot hash spreads
/// across the inner-array slots.
pub fn send_at_rate(
    iface: &str,
    vip_octets: [u8; 4],
    vip_port: u16,
    target_pps: u32,
    duration: Duration,
    base_src_port: u16,
) -> Result<usize, std::io::Error> {
    let ifindex = if_nametoindex(iface)?;

    // SAFETY: AF_PACKET / SOCK_RAW socket — standard syscall surface.
    let fd: RawFd =
        unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sll.sll_family = libc::AF_PACKET as u16;
    sll.sll_protocol = (ETH_P_ALL as u16).to_be();
    sll.sll_ifindex = ifindex as i32;
    sll.sll_halen = 6;

    let started = Instant::now();
    let deadline = started + duration;
    let slice = Duration::from_millis(100);
    let frames_per_slice = (target_pps / 10).max(1);

    let mut sent: usize = 0;
    let mut src_port_off: u16 = 0;

    while Instant::now() < deadline {
        let slice_start = Instant::now();
        let mut slice_sent: u32 = 0;
        while slice_sent < frames_per_slice && Instant::now() < deadline {
            let src_port = base_src_port.wrapping_add(src_port_off);
            src_port_off = src_port_off.wrapping_add(1);
            let frame = synthesise_tcp_syn_with_src_port(vip_octets, vip_port, src_port);
            // SAFETY: sendto with sockaddr_ll for the bound iface.
            let rc = unsafe {
                libc::sendto(
                    fd,
                    frame.as_ptr() as *const _,
                    frame.len(),
                    0,
                    (&sll as *const _) as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
                )
            };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                // ENOBUFS / EAGAIN under heavy load: back off briefly
                // and retry. These are transient socket-queue
                // pressure signals, not actual drops — the frame
                // never left userspace, so it was not "sent". Other
                // errnos are propagated.
                let kind = err.raw_os_error();
                // EAGAIN == EWOULDBLOCK on Linux — single match arm.
                if matches!(kind, Some(libc::ENOBUFS) | Some(libc::EAGAIN)) {
                    std::thread::sleep(Duration::from_micros(100));
                    continue;
                }
                // SAFETY: fd from socket() above.
                unsafe { libc::close(fd) };
                return Err(err);
            }
            sent = sent.saturating_add(1);
            slice_sent = slice_sent.saturating_add(1);
        }
        // Sleep the rest of the 100 ms slice (if any). If sending
        // took the full 100 ms (rate-limited by kernel), no sleep.
        let elapsed = slice_start.elapsed();
        if elapsed < slice {
            std::thread::sleep(slice - elapsed);
        }
    }

    // SAFETY: fd was returned by socket() above.
    unsafe { libc::close(fd) };
    Ok(sent)
}

/// Capture rewritten frames (XDP_TX'd back from the host's XDP
/// program) on `socket_fd` until `deadline` OR until `expected_max`
/// frames have been collected, whichever comes first.
///
/// Filters on dest IP `10.1.0.0/24` — the test backends' subnet —
/// so we drop the original outbound SYN (dst `10.0.0.1`, the VIP)
/// and only retain rewritten round-trip frames. Returns the
/// captured frames in arrival order.
///
/// Drain shape: spin tightly while data is available (every recv
/// success returns immediately and we re-enter the loop without
/// sleeping). Only yield (10 µs) when the kernel reports `EAGAIN`
/// — i.e. the socket queue is empty. This is necessary at sustained
/// rates: each `nanosleep` between frames is fertile ground for
/// `SO_RCVBUF` overflow when frames arrive faster than the loop
/// drains.
pub fn capture_until_deadline(
    socket_fd: RawFd,
    deadline: Instant,
    expected_max: usize,
) -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut buf = vec![0u8; 2048];
    while Instant::now() < deadline && frames.len() < expected_max {
        // SAFETY: recv into our owned buf.
        let n = unsafe { libc::recv(socket_fd, buf.as_mut_ptr() as *mut _, buf.len(), 0) };
        if n > 0 {
            let n = n as usize;
            if n >= ETH_HDR_LEN + IPV4_HDR_LEN {
                let dst_oct1 = buf[ETH_HDR_LEN + 16];
                let dst_oct2 = buf[ETH_HDR_LEN + 17];
                if dst_oct1 == 10 && dst_oct2 == 1 {
                    frames.push(buf[..n].to_vec());
                }
            }
            // continue without sleeping — drain as fast as we can.
        } else {
            // Empty queue (EAGAIN on a nonblocking socket) — short
            // yield. 10 µs is small enough to keep up with bursts;
            // a longer sleep here is the canonical drop trigger.
            std::thread::sleep(Duration::from_micros(10));
        }
    }
    frames
}

/// Enlarge `socket_fd`'s `SO_RCVBUF` to `bytes`. Used to absorb
/// burst traffic on Tier 3 sustained-rate tests — the kernel's
/// default 256 KB receive buffer overflows in tens of milliseconds
/// at multi-kpps rates, dropping frames before our recv loop can
/// process them. Returns Ok on success, propagates errno otherwise.
///
/// Note that the kernel doubles the requested value internally
/// (it stores half overhead). The actual buffer size after this
/// call may be 2× the request. The call clamps silently to
/// `net.core.rmem_max` — operators tuning for sustained traffic
/// should verify this sysctl is generous enough.
pub fn set_socket_rcvbuf(socket_fd: RawFd, bytes: i32) -> Result<(), std::io::Error> {
    // SAFETY: setsockopt with a stack-local int per the SOL_SOCKET
    // / SO_RCVBUF contract.
    let rc = unsafe {
        libc::setsockopt(
            socket_fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            (&bytes as *const i32) as *const libc::c_void,
            std::mem::size_of::<i32>() as libc::socklen_t,
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn if_nametoindex(iface: &str) -> Result<u32, std::io::Error> {
    let cstr = std::ffi::CString::new(iface).expect("iface name has no NUL");
    // SAFETY: thin syscall wrapper; pointer not retained past call.
    let idx = unsafe { libc::if_nametoindex(cstr.as_ptr()) };
    if idx == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(idx)
}
