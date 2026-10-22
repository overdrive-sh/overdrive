//! Datagram mode (EVMOD-DGRAM, ADR-0201): R5-52 (frames over one vsock
//! STREAM) and R5-51 (empties, frame-shaped payloads, oversize, invalid
//! and truncated frames).
//!
//! The host's datagram association shape: half A the accepted vsock `V_h`
//! carrying ADR-0149 frames (magic `OVD1`, BE `u32` length, payload), half
//! B the connected UDP socket `H`.

use std::io::{Read, Write};
use std::mem;
use std::net::UdpSocket;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardId, ForwardOutcome, PairMode, PairSide,
};
use overdrive_core::guest_transport::wire::{
    DATAGRAM_FRAME_HEADER_LEN, DATAGRAM_FRAME_MAGIC, DatagramFrame, MAX_GUEST_DATAGRAM_PAYLOAD,
};
use overdrive_fwd::KernelFlowForwarder;

use super::support::{
    END_WAIT, Vsock, live, open_context, pattern, udp_pair, vsock_pair, wait_end_of,
};

/// One attached datagram association and its far ends.
struct Association {
    fwd: KernelFlowForwarder,
    id: ForwardId,
    v: Vsock,
    v_far: Vsock,
    h: UdpSocket,
    u_far: UdpSocket,
}

fn associate(vsock_buffer: Option<u64>) -> Association {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    if let Some(bytes) = vsock_buffer {
        v.set_buffer_size(bytes);
        v_far.set_buffer_size(bytes);
    }
    let (h, u_far) = udp_pair();
    // Deep receive queues at both fixture ends: `h` is the UDP source queue
    // the forwarder drains, and `u_far` is the test reader's queue. A loss in
    // a case that asserts delivery is therefore the forwarder's.
    force_rcvbuf(&h, 32 << 20);
    force_rcvbuf(&u_far, 32 << 20);
    u_far.set_read_timeout(Some(END_WAIT)).expect("timeout");
    v_far.set_read_timeout(END_WAIT);
    let id = fwd
        .attach(&v as &dyn AsFd, &h as &dyn AsFd, PairMode::Datagram { udp: PairSide::B })
        .expect("attach a datagram association");
    Association { fwd, id, v, v_far, h, u_far }
}

fn force_rcvbuf(sock: &impl AsRawFd, bytes: libc::c_int) {
    // SAFETY: valid option value of the passed length.
    let ret = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUFFORCE,
            (&raw const bytes).cast(),
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert!(ret == 0, "SO_RCVBUFFORCE as root: {}", std::io::Error::last_os_error());
}

/// Payload `i` of a run: its length from the run's size range, its bytes
/// position-dependent and tagged with `i` in the first 8 bytes.
fn payload(i: usize, min: usize, max: usize) -> Vec<u8> {
    let span = max - min + 1;
    let len = min + (i.wrapping_mul(7919) % span);
    let mut p = pattern(i as u64, len);
    p[..8].copy_from_slice(&(i as u64).to_be_bytes());
    p
}

fn frame(p: &[u8]) -> Vec<u8> {
    DatagramFrame::new(p).expect("payload within the bound").encode()
}

/// Reads exactly one frame from the vsock stream; `None` at end-of-stream.
fn read_frame(v: &Vsock) -> Option<Vec<u8>> {
    let mut header = [0u8; DATAGRAM_FRAME_HEADER_LEN];
    let mut r = v;
    match r.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return None,
        Err(e) => panic!("frame header: {e}"),
    }
    assert_eq!(header[..4], DATAGRAM_FRAME_MAGIC, "every frame toward the guest carries the magic");
    let len = u32::from_be_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
    assert!(len <= MAX_GUEST_DATAGRAM_PAYLOAD as usize, "declared length within the bound");
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).expect("a whole frame body");
    Some(body)
}

/// `drops` of the UDP socket bound to `port` (`/proc/net/udp`).
fn udp_drops(port: u16) -> u64 {
    let table = std::fs::read_to_string("/proc/net/udp").expect("read /proc/net/udp");
    let local = format!("0100007F:{port:04X}");
    table
        .lines()
        .skip(1)
        .find(|l| l.split_whitespace().nth(1) == Some(local.as_str()))
        .and_then(|l| l.split_whitespace().last().and_then(|d| d.parse().ok()))
        .unwrap_or_else(|| panic!("UDP socket {local} in /proc/net/udp"))
}

/// R5-52: each run sends `count` datagrams each way, the sender never more
/// than `WINDOW` ahead of its receiver, and asserts every datagram
/// delivered once, whole and in order.
fn mixed_run(vsock_buffer: u64, min: usize, max: usize, count: usize) {
    const WINDOW: usize = 32;
    let a = associate(Some(vsock_buffer));
    let to_udp = AtomicUsize::new(0);
    let to_vsock = AtomicUsize::new(0);
    let started = Instant::now();
    thread::scope(|s| {
        // vsock far → UDP far.
        s.spawn(|| {
            let mut w = &a.v_far;
            for i in 0..count {
                while i >= to_udp.load(Ordering::Acquire) + WINDOW {
                    thread::yield_now();
                }
                w.write_all(&frame(&payload(i, min, max))).expect("write a frame");
            }
        });
        s.spawn(|| {
            let mut buf = vec![0u8; 65_536];
            for i in 0..count {
                let n = a.u_far.recv(&mut buf).expect("a datagram reaches the UDP far end");
                assert!(
                    buf[..n] == payload(i, min, max),
                    "datagram {i} toward UDP: whole, once, in order"
                );
                to_udp.store(i + 1, Ordering::Release);
            }
        });
        // UDP far → vsock far.
        s.spawn(|| {
            for i in 0..count {
                while i >= to_vsock.load(Ordering::Acquire) + WINDOW {
                    thread::yield_now();
                }
                a.u_far.send(&payload(i, min, max)).expect("send a datagram");
            }
        });
        for i in 0..count {
            let got = read_frame(&a.v_far).expect("a frame reaches the vsock far end");
            assert!(
                got == payload(i, min, max),
                "datagram {i} toward vsock: whole, once, in order"
            );
            to_vsock.store(i + 1, Ordering::Release);
        }
    });
    assert!(started.elapsed() < Duration::from_secs(120), "no stall");
    assert!(a.fwd.pair_state(a.id).expect("pair_state").is_some(), "the association is still live");
}

#[test]
fn mix_64_to_59000_bytes_at_256_kib_is_delivered_once_whole_in_order() {
    mixed_run(256 * 1024, 64, 59_000, 2_000);
}

#[test]
fn mix_64_to_59000_bytes_at_64_kib_is_delivered_once_whole_in_order() {
    mixed_run(64 * 1024, 64, 59_000, 2_000);
}

#[test]
fn mixs_64_to_2047_bytes_at_256_kib_is_delivered_once_whole_in_order() {
    mixed_run(256 * 1024, 64, 2_047, 20_000);
}

#[test]
fn mixs_64_to_2047_bytes_at_64_kib_is_delivered_once_whole_in_order() {
    mixed_run(64 * 1024, 64, 2_047, 20_000);
}

#[test]
fn a_stalled_vsock_reader_fills_the_udp_source_queue_and_datagrams_drop_only_there() {
    // R5-52: datagrams wait in the source UDP socket's receive queue; once
    // it is full the kernel drops new ones (net/ipv4/udp.c:1721-1731 @
    // v7.0). Frames the vsock accepted only in part resume at the next byte:
    // the guest side reads only whole, canonical, strictly increasing frames.
    let a = associate(None);
    let port = a.h.local_addr().expect("addr").port();
    let sent = 4_000usize;
    for i in 0..sent {
        a.u_far.send(&payload(i, 40_000, 59_000)).expect("send");
    }
    // Loopback delivery is synchronous in `send`, so the counter is final
    // once the sends return; poll (bounded) rather than assume a settle.
    let deadline = Instant::now() + END_WAIT;
    let mut drops = udp_drops(port);
    while drops == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
        drops = udp_drops(port);
    }
    assert!(drops > 0, "with the vsock reader stopped the UDP source queue must overflow");
    let mut last: Option<u64> = None;
    let mut received = 0u64;
    a.v_far.set_read_timeout(Duration::from_secs(2));
    while let Some(body) = read_frame_or_quiet(&a.v_far) {
        let index = u64::from_be_bytes(body[..8].try_into().expect("tag"));
        assert!(
            last.is_none_or(|l| index > l),
            "frame {index} after {last:?}: duplicated or reordered"
        );
        assert!(
            body == payload(index as usize, 40_000, 59_000),
            "frame {index} whole and unchanged"
        );
        last = Some(index);
        received += 1;
    }
    assert_eq!(
        received + drops,
        sent as u64,
        "every datagram either arrives once or drops at the source"
    );
}

/// Like [`read_frame`], but `None` once the stream is quiet for the read
/// timeout (the run sent everything it will).
fn read_frame_or_quiet(v: &Vsock) -> Option<Vec<u8>> {
    let mut first = [0u8; 1];
    match (&mut &*v).read(&mut first) {
        Ok(0) => None,
        Ok(_) => {
            let mut rest = [0u8; DATAGRAM_FRAME_HEADER_LEN - 1];
            (&mut &*v).read_exact(&mut rest).expect("the rest of the header");
            let mut header = [0u8; DATAGRAM_FRAME_HEADER_LEN];
            header[0] = first[0];
            header[1..].copy_from_slice(&rest);
            assert_eq!(header[..4], DATAGRAM_FRAME_MAGIC);
            let len = u32::from_be_bytes(header[4..8].try_into().expect("4 bytes")) as usize;
            let mut body = vec![0u8; len];
            (&mut &*v).read_exact(&mut body).expect("a whole frame body");
            Some(body)
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            None
        }
        Err(e) => panic!("frame read: {e}"),
    }
}

#[test]
fn empty_datagrams_cross_once_and_in_order_both_ways() {
    // R5-51: an empty datagram is a zero-length frame toward the guest and a
    // zero-length datagram toward the peer, in order with its neighbours.
    let a = associate(None);
    for p in [&b"before"[..], b"", b"after"] {
        (&mut &a.v_far).write_all(&frame(p)).expect("write");
    }
    let mut buf = [0u8; 64];
    for want in [&b"before"[..], b"", b"after"] {
        let n = a.u_far.recv(&mut buf).expect("recv");
        assert_eq!(&buf[..n], want);
    }
    for p in [&b"before"[..], b"", b"after"] {
        a.u_far.send(p).expect("send");
    }
    for want in [&b"before"[..], b"", b"after"] {
        assert_eq!(read_frame(&a.v_far).expect("frame"), want);
    }
    // Exactly once: nothing further arrives either way.
    a.u_far.set_read_timeout(Some(Duration::from_millis(300))).expect("timeout");
    assert!(a.u_far.recv(&mut buf).is_err(), "no duplicate toward UDP");
    assert_eq!(super::support::queued_bytes(a.v_far.as_fd()), 0, "no duplicate toward vsock");
}

#[test]
fn a_frame_shaped_payload_is_stripped_of_exactly_one_outer_frame() {
    let a = associate(None);
    let inner = frame(b"inner");
    (&mut &a.v_far).write_all(&frame(&inner)).expect("write");
    let mut buf = [0u8; 64];
    let n = a.u_far.recv(&mut buf).expect("recv");
    assert_eq!(&buf[..n], &inner[..], "the payload that parses as a frame arrives unchanged");
    a.u_far.send(&inner).expect("send");
    assert_eq!(read_frame(&a.v_far).expect("frame"), inner);
}

#[test]
fn an_oversize_datagram_is_dropped_at_the_source_and_counted() {
    let a = associate(None);
    a.u_far.send(b"first").expect("send");
    a.u_far.send(&vec![0xa5u8; MAX_GUEST_DATAGRAM_PAYLOAD as usize + 1]).expect("send oversize");
    a.u_far.send(b"second").expect("send");
    assert_eq!(read_frame(&a.v_far).expect("frame"), b"first");
    assert_eq!(
        read_frame(&a.v_far).expect("frame"),
        b"second",
        "the oversize datagram is not sent"
    );
    assert_eq!(a.fwd.inventory().expect("inventory").dgram_oversize, 1);
    assert!(
        a.fwd.pair_state(a.id).expect("pair_state").is_some(),
        "an oversize drop does not end the association"
    );
}

fn assert_frame_invalid(bytes: &[u8], then_half_close: bool) {
    let a = associate(None);
    (&mut &a.v_far).write_all(bytes).expect("write");
    if then_half_close {
        a.v_far.shutdown(libc::SHUT_WR);
    }
    let end = wait_end_of(&a.fwd, a.id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::FrameInvalid { side: PairSide::A } }
    );
    assert_eq!(live(&a.fwd), (0, 0, 0));
}

#[test]
fn a_bad_magic_aborts_frame_invalid() {
    let mut bytes = frame(b"payload");
    bytes[0] = b'X';
    assert_frame_invalid(&bytes, false);
}

#[test]
fn a_declared_length_above_the_bound_aborts_frame_invalid() {
    let mut header = DATAGRAM_FRAME_MAGIC.to_vec();
    header.extend_from_slice(&(MAX_GUEST_DATAGRAM_PAYLOAD + 1).to_be_bytes());
    assert_frame_invalid(&header, false);
}

#[test]
fn a_truncated_frame_at_a_clean_peer_end_aborts_frame_invalid() {
    // EV-DGRAM D1 / U-EV-9: a clean-class end (0x2 only) that leaves a frame
    // incomplete aborts FrameInvalid, after every whole frame before it is
    // sent; a waiting pair or a silently dropped frame is the defect the
    // rule removes (`trunc_fix=0`, P-72 § 8).
    let a = associate(None);
    let mut bytes = frame(b"whole");
    let partial = frame(&pattern(40, 100));
    bytes.extend_from_slice(&partial[..DATAGRAM_FRAME_HEADER_LEN + 10]);
    (&mut &a.v_far).write_all(&bytes).expect("write");
    a.v_far.shutdown(libc::SHUT_WR);
    let mut buf = [0u8; 64];
    let n = a.u_far.recv(&mut buf).expect("the whole frame before the truncation is sent");
    assert_eq!(&buf[..n], b"whole");
    let end = wait_end_of(&a.fwd, a.id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::FrameInvalid { side: PairSide::A } }
    );
    assert_eq!(end.moved_ab, 5, "payload bytes in datagram mode");
}

#[test]
fn a_clean_peer_end_after_whole_frames_completes() {
    let a = associate(None);
    (&mut &a.v_far).write_all(&frame(b"last")).expect("write");
    a.v_far.shutdown(libc::SHUT_WR);
    let mut buf = [0u8; 64];
    let n = a.u_far.recv(&mut buf).expect("recv");
    assert_eq!(&buf[..n], b"last");
    assert_eq!(wait_end_of(&a.fwd, a.id).outcome, ForwardOutcome::Completed);
}

#[test]
fn an_abort_class_peer_end_with_a_partial_frame_queued_is_a_peer_abort() {
    // Every production end of a datagram vsock is abort-class: the cause is
    // PeerAbort whatever frames are queued, never FrameInvalid.
    let a = associate(None);
    let partial = frame(&pattern(41, 100));
    (&mut &a.v_far).write_all(&partial[..DATAGRAM_FRAME_HEADER_LEN + 10]).expect("write");
    let Association { fwd, id, v, v_far, h, u_far } = a;
    drop(v_far);
    assert_eq!(
        wait_end_of(&fwd, id).outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    drop((v, h, u_far));
}

#[test]
fn a_refused_udp_destination_aborts_with_a_socket_error() {
    // A connected UDP socket's ICMP-reported ECONNREFUSED ends the
    // association `SocketError` (the host owner's DestinationUnreachable).
    let a = associate(None);
    let Association { fwd, id, v, v_far, h, u_far } = a;
    drop(u_far);
    (&mut &v_far).write_all(&frame(b"to nobody")).expect("write");
    let end = wait_end_of(&fwd, id);
    assert!(
        matches!(end.outcome, ForwardOutcome::Aborted { cause: AbortCause::SocketError { side: PairSide::B, errno } }
            if errno.get() == libc::ECONNREFUSED as u16),
        "got {end:?}"
    );
    drop((v, v_far, h));
}

fn force_sndbuf(sock: &impl AsRawFd, bytes: libc::c_int) {
    // SAFETY: valid option value of the passed length.
    let ret = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUFFORCE,
            (&raw const bytes).cast(),
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert!(ret == 0, "SO_SNDBUFFORCE as root: {}", std::io::Error::last_os_error());
}

/// V-EV-SAFE, full-target resume, UDP target (ADR-0204 rule 5; the round-16
/// wait-queue trigger; the `sock_wfree` fact, `net/core/sock.c:2675-2693 @
/// v7.0`): the association's UDP half `H` has its send buffer forced below
/// one datagram, so after each send the target is full until the previous
/// datagram's skb is freed, and the forwarder's next send waits on the
/// target's write-space wait-queue entry (UDP never takes
/// `SOCK_USE_WRITE_QUEUE`, so the wake comes from `sock_wfree` on whatever
/// context frees the skb). Every one of `COUNT` frames toward the UDP side
/// arrives once, whole and in order — no frame is dropped at a full target
/// and the direction never stalls — and the association stays live.
///
/// What this proves over loopback: resumption. `loopback_xmit` orphans the
/// sender's skb (`drivers/net/loopback.c:80 @ v7.0`) inside
/// `dev_queue_xmit`'s bottom-half-disabled section, so the target's
/// write-space wake comes from the bottom-half class, not from the
/// preemptible `sock_wfree` class a virtio transmit-free path takes. That
/// class is discharged by the `fwd_safe` model and the DELIVER source review
/// of `sk_write_space` (`test-scenarios.md`, "UDP resume over loopback").
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_udp_target_whose_send_buffer_is_full_resumes_and_delivers_every_frame() {
    const COUNT: usize = 2_000;
    let a = associate(None);
    force_sndbuf(&a.h, 4_096);
    thread::scope(|s| {
        s.spawn(|| {
            let mut w = &a.v_far;
            for i in 0..COUNT {
                w.write_all(&frame(&payload(i, 16_000, 32_000))).expect("write a frame");
            }
        });
        let mut buf = vec![0u8; 65_536];
        for i in 0..COUNT {
            let n = a.u_far.recv(&mut buf).expect("a datagram reaches the UDP far end");
            assert!(
                buf[..n] == payload(i, 16_000, 32_000),
                "datagram {i} toward a full UDP target: whole, once, in order"
            );
        }
    });
    assert!(a.fwd.pair_state(a.id).expect("pair_state").is_some(), "the association is live");
    assert_eq!(a.fwd.take_end().expect("take_end"), None, "a full target never ends the pair");
}
