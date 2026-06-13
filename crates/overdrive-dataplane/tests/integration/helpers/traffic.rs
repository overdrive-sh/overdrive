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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::packets::{ETH_HDR_LEN, IPV4_HDR_LEN, synthesise_tcp_syn_with_src_port};

const ETH_P_ALL: std::os::raw::c_int = 0x0003;

// ---------------------------------------------------------------------------
// Peer-leg confidentiality oracle — a REAL AF_PACKET/SOCK_RAW capture on the
// interface carrying the peer-facing leg (transparent-mtls 01-01, F2 / GAP "the
// peer-facing leg shows TLS 1.3 records (tcpdump 0x17) ... the workload's plaintext
// appears ONLY on the host-internal leg"). The K1 North-Star observable: the oracle
// must be DERIVED FROM CAPTURED BYTES, never from handshake-success bookkeeping.
//
// `WireScan` is the post-capture analysis: parse Eth+IPv4+TCP frames on `lo`,
// reassemble the per-direction TCP byte stream, walk the TLS record framing to count
// genuine `application_data` (type 0x17) records, and scan the raw payload for the
// cleartext request marker (which MUST be absent on the peer leg).
// ---------------------------------------------------------------------------

/// The TLS `application_data` content type (the `0x17` the tcpdump oracle looks for).
const TLS_CONTENT_TYPE_APPLICATION_DATA: u8 = 0x17;
/// TLS 1.2/1.3 legacy record-layer version bytes (`0x0303`) — the two bytes that
/// follow the content type in every record header on the wire (TLS 1.3 keeps the
/// `0x0303` legacy record version for middlebox compatibility).
const TLS_LEGACY_RECORD_VERSION: [u8; 2] = [0x03, 0x03];
/// TLS record header length: `type(1) + version(2) + length(2)`.
const TLS_RECORD_HEADER_LEN: usize = 5;

/// The result of scanning a captured peer-leg wire: how many genuine TLS 1.3
/// `application_data` (`0x17`) records crossed the wire, and how many times the
/// cleartext request marker appeared in the raw payload (MUST be 0 — plaintext on
/// the peer leg is the confidentiality breach the whole feature exists to prevent).
#[derive(Debug, Clone, Copy, Default)]
pub struct WireScan {
    /// Count of TLS records whose content type is `0x17` (`application_data`),
    /// derived by walking the record framing of the reassembled per-direction
    /// streams — NOT a naive byte-substring count (which ciphertext could trip).
    pub app_data_records: u64,
    /// Count of appearances of the cleartext marker bytes in the captured TCP
    /// payload across both directions. Any non-zero value means plaintext leaked
    /// onto the peer-facing wire.
    pub plaintext_marker_hits: u64,
}

/// A live capture on `iface` (an `AF_PACKET`/`SOCK_RAW` socket, same socket-open
/// pattern as [`send_at_rate`]) that records every frame into a buffer on a
/// background thread until [`WireCapture::stop_and_scan`]. The capture is filtered
/// at scan time to TCP frames touching `wire_port` (as src OR dst port), so it
/// isolates the peer-facing leg's records from any other loopback traffic.
pub struct WireCapture {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Vec<Vec<u8>>>>,
    wire_port: u16,
}

impl WireCapture {
    /// Start capturing on `iface`. Bind an `AF_PACKET`/`SOCK_RAW` socket to the
    /// interface and drain frames into a buffer until stopped. `wire_port` is the
    /// peer-facing leg's port (the records are filtered to frames whose TCP src or
    /// dst port equals it at scan time).
    ///
    /// # Panics
    /// Panics if the `AF_PACKET` socket cannot be created or bound — these are
    /// precondition failures (need root/CAP_NET_RAW, which the Tier-3 gate has);
    /// a panic-with-message is the right Tier-3 fixture failure.
    #[must_use]
    pub fn start(iface: &str, wire_port: u16) -> Self {
        let ifindex = if_nametoindex(iface).expect("wire-capture: if_nametoindex");
        // SAFETY: AF_PACKET / SOCK_RAW socket — same surface as `send_at_rate`.
        let fd: RawFd =
            unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW, ETH_P_ALL.to_be() as i32) };
        assert!(fd >= 0, "wire-capture: socket: {}", std::io::Error::last_os_error());

        let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = ifindex as i32;
        // Bind to the iface so we only see its frames (loopback for the
        // peer/client legs, which live on 127.0.0.0/8).
        // SAFETY: bind an AF_PACKET socket to the resolved ifindex.
        let rc = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&sll).cast(),
                std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        assert!(rc == 0, "wire-capture: bind {iface}: {}", std::io::Error::last_os_error());
        // Non-blocking so the capture loop can poll the stop flag promptly.
        // SAFETY: fcntl on our own fd.
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
                    // Empty queue (EAGAIN) — short yield, then re-check stop.
                    std::thread::sleep(Duration::from_micros(200));
                }
            }
            // Final non-blocking drain so records written right before `stop` are not
            // lost (the connection's last app_data may still be in the socket queue).
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

    /// Stop the capture, join the thread, and scan the captured frames for the
    /// confidentiality oracle: the count of TLS 1.3 `application_data` records and
    /// the count of cleartext-marker appearances on the peer-facing wire.
    ///
    /// # Panics
    /// Panics if the capture thread panicked (a precondition failure).
    #[must_use]
    pub fn stop_and_scan(mut self, marker: &[u8]) -> WireScan {
        self.stop.store(true, Ordering::SeqCst);
        let frames = self.handle.take().expect("wire-capture handle").join().expect("capture join");
        scan_frames(&frames, self.wire_port, marker)
    }
}

/// Walk captured Ethernet+IPv4+TCP frames touching `wire_port`, reassemble the
/// per-direction TCP byte streams, count genuine TLS `application_data` (`0x17`)
/// records by walking the record framing, and count cleartext-marker appearances.
fn scan_frames(frames: &[Vec<u8>], wire_port: u16, marker: &[u8]) -> WireScan {
    // Per-direction reassembled payload, keyed by (src_port, dst_port). Loopback
    // preserves in-order delivery per direction, so concatenating payloads in
    // arrival order reconstructs each direction's TLS byte stream.
    let mut streams: std::collections::BTreeMap<(u16, u16), Vec<u8>> =
        std::collections::BTreeMap::new();

    for frame in frames {
        let Some((src_port, dst_port, payload)) = parse_tcp_payload(frame) else {
            continue;
        };
        if src_port != wire_port && dst_port != wire_port {
            continue; // not the peer-facing leg
        }
        if payload.is_empty() {
            continue;
        }
        streams.entry((src_port, dst_port)).or_default().extend_from_slice(payload);
    }

    let mut app_data_records: u64 = 0;
    let mut plaintext_marker_hits: u64 = 0;
    for stream in streams.values() {
        app_data_records += count_tls_app_data_records(stream);
        plaintext_marker_hits += count_subslices(stream, marker);
    }
    WireScan { app_data_records, plaintext_marker_hits }
}

/// Parse an Ethernet+IPv4+TCP frame, returning `(src_port, dst_port, tcp_payload)`
/// when it is a well-formed IPv4/TCP frame; `None` otherwise (non-IPv4, non-TCP,
/// truncated, or IP/TCP options making the offsets exceed the captured length).
fn parse_tcp_payload(frame: &[u8]) -> Option<(u16, u16, &[u8])> {
    if frame.len() < ETH_HDR_LEN + IPV4_HDR_LEN {
        return None;
    }
    // EtherType IPv4?
    if frame.get(12).copied()? != 0x08 || frame.get(13).copied()? != 0x00 {
        return None;
    }
    let ip = ETH_HDR_LEN;
    let vihl = frame.get(ip).copied()?;
    if vihl >> 4 != 4 {
        return None; // not IPv4
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

/// Count TLS records of content-type `application_data` (`0x17`) by walking the
/// record framing of a reassembled per-direction stream. Each record is
/// `type(1) version(2) length(2) payload(length)`; the walk skips
/// `TLS_RECORD_HEADER_LEN + length` per record, so a `0x17` byte INSIDE ciphertext
/// is never miscounted — only genuine record headers (with the `0x0303` legacy
/// version) advance the counter. A stream that desyncs (a header whose version is
/// not `0x0303`, or a length running past the captured bytes) stops the walk; the
/// records counted up to that point are the genuine ones.
fn count_tls_app_data_records(stream: &[u8]) -> u64 {
    let mut count: u64 = 0;
    let mut i = 0usize;
    while i + TLS_RECORD_HEADER_LEN <= stream.len() {
        let content_type = stream[i];
        let version = [stream[i + 1], stream[i + 2]];
        let length = u16::from_be_bytes([stream[i + 3], stream[i + 4]]) as usize;
        // A genuine TLS record header carries the `0x0303` legacy record version.
        // If it does not, we have desynced from the record framing (mid-ciphertext
        // or a non-TLS stream) — stop rather than risk a false count.
        if version != TLS_LEGACY_RECORD_VERSION {
            break;
        }
        if content_type == TLS_CONTENT_TYPE_APPLICATION_DATA {
            count += 1;
        }
        let next = i + TLS_RECORD_HEADER_LEN + length;
        if next <= i {
            break; // zero-length / overflow guard
        }
        i = next;
    }
    count
}

/// Count non-overlapping appearances of `needle` in `haystack`.
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
