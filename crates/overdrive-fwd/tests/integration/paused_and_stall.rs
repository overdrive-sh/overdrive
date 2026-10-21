//! Paused readers (R5-49; R2-DRAIN, ADR-0183) and the bounded stall
//! (R5-62; EVMOD-ADOPT, § *Memory and backpressure bounds*).
//!
//! A slow reader is backpressure: the target is never shut before its last
//! accepted byte, a paused flow holds only its own buffers with no
//! deadline, and other flows proceed.

use std::io::{ErrorKind, Write};
use std::net::Shutdown;
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardOutcome, PairSide,
};

use super::support::{
    END_WAIT, FarEnd, StreamFlow, assert_no_end, pattern, read_until_end, tcp_mem_max,
    unreclaimable_slab_kib, wait_end_of,
};

/// `FWD_CHUNK`: at most one chunk of scratch per running work item
/// (§ *Forwarding*, "Run").
const FWD_CHUNK: usize = 64 * 1024;

/// Absence hold window for a paused vsock reader: how long the half-close is
/// observed NOT to arrive before the reader resumes.
const PAUSED_ABSENCE_HOLD: Duration = Duration::from_secs(2);

/// Absence hold window for a stalled or paused flow: how long it is observed
/// to stay open, un-ended and un-aborted.
const STALL_ABSENCE_HOLD: Duration = Duration::from_secs(3);

/// Polling step while waiting for a positive control.
const CONTROL_POLL: Duration = Duration::from_millis(50);

/// Writes without blocking until the writer would block; returns the bytes
/// accepted.
fn fill_until_blocked(mut w: impl Write, data: &[u8]) -> usize {
    let mut sent = 0usize;
    let deadline = Instant::now() + END_WAIT;
    loop {
        match w.write(&data[sent..]) {
            Ok(n) => sent += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // The forwarder may still be moving the last chunk: confirm
                // the block holds after a settle.
                thread::sleep(Duration::from_millis(200));
                match w.write(&data[sent..]) {
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return sent,
                    Ok(n) => sent += n,
                    Err(e) => panic!("writer: {e}"),
                }
            }
            Err(e) => panic!("writer: {e}"),
        }
        assert!(sent < data.len(), "the whole payload was accepted with the reader stopped");
        assert!(Instant::now() < deadline, "the writer never blocked against a stopped reader");
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_paused_tcp_reader_never_sees_the_half_close_before_its_last_byte() {
    // vsock source → TCP target.
    let flow = StreamFlow::new();
    let data = pattern(30, 32 << 20);
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &flow.v_far).write_all(&data).expect("write");
            flow.v_far.shutdown(libc::SHUT_WR);
        });
        let paused_until = Instant::now() + PAUSED_ABSENCE_HOLD;
        while Instant::now() < paused_until {
            let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("live");
            assert!(!state.a_to_b.target_shut, "the target was shut before its last accepted byte");
            thread::sleep(Duration::from_millis(50));
        }
        let (got, end) = read_until_end(&flow.t_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(got == data, "every byte, then the half-close: {} of {}", got.len(), data.len());
    });
    let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("B→A still open");
    assert!(state.a_to_b.target_shut);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_paused_vsock_reader_never_sees_the_half_close_before_its_last_byte() {
    // TCP source → vsock target; the guest side's watch is the oracle for
    // when the half-close arrived.
    let flow = StreamFlow::new();
    let data = pattern(31, 32 << 20);
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &flow.t_far).write_all(&data).expect("write");
            flow.t_far.shutdown(Shutdown::Write).expect("FIN");
        });
        // Positive control: the TCP → vsock direction is live and moving
        // bytes, so the absent half-close below is the pause, not an idle
        // pair.
        let control_deadline = Instant::now() + END_WAIT;
        while flow.fwd.pair_state(flow.id).expect("pair_state").expect("live").b_to_a.moved == 0 {
            assert!(
                Instant::now() < control_deadline,
                "the pair never moved a byte toward the vsock"
            );
            thread::sleep(CONTROL_POLL);
        }
        thread::sleep(PAUSED_ABSENCE_HOLD);
        assert_eq!(
            flow.peer.take_end().expect("take_end"),
            None,
            "no half-close reached the paused side"
        );
        let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("live");
        assert!(!state.b_to_a.target_shut);
        let (got, end) = read_until_end(&flow.v_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(got == data, "every byte, then the half-close: {} of {}", got.len(), data.len());
    });
    assert_eq!(flow.far_end(), ForwardOutcome::GuestHalfClose);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_target_that_stops_receiving_aborts_the_pair() {
    let flow = StreamFlow::new();
    flow.v_far.shutdown(libc::SHUT_RD);
    let StreamFlow { fwd, id, v, v_far, t, t_far, .. } = flow;
    (&mut &t_far).write_all(b"toward a closed receiver").expect("write");
    let end = wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    let (_, far) = read_until_end(super::support::bounded(&t_far));
    assert_eq!(far, FarEnd::Error(ErrorKind::ConnectionReset));
    drop((v, v_far, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_paused_flow_holds_open_with_no_deadline_while_other_flows_proceed() {
    let paused = StreamFlow::new();
    paused.v_far.set_nonblocking(true);
    let held = fill_until_blocked(&paused.v_far, &pattern(32, 64 << 20));
    let other = StreamFlow::new();
    let data = pattern(33, 16 << 20);
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &other.v_far).write_all(&data).expect("write");
            other.v_far.shutdown(libc::SHUT_WR);
        });
        let (got, end) = read_until_end(&other.t_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(got == data, "the other flow proceeds byte-exact");
    });
    thread::sleep(STALL_ABSENCE_HOLD);
    assert_eq!(paused.fwd.take_end().expect("take_end"), None, "a paused flow is never aborted");
    assert!(paused.fwd.pair_state(paused.id).expect("pair_state").is_some());
    assert!(held > 0);
}

/// R5-62: the bound of one stalled direction is the source's receive
/// queue, plus the target's send queue, plus the far reader's receive
/// buffer, plus one chunk; it does not grow with the stall's length.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_ten_second_stall_holds_at_most_the_per_flow_bound_and_kernel_memory_stays_flat() {
    let flow = StreamFlow::new();
    let data = pattern(34, 256 << 20);
    flow.v_far.set_nonblocking(true);
    let accepted = fill_until_blocked(&flow.v_far, &data);
    let bound = usize::try_from(flow.v.buffer_size()).expect("buffer size")
        + tcp_mem_max("tcp_wmem")
        + tcp_mem_max("tcp_rmem")
        + FWD_CHUNK;
    assert!(
        accepted <= bound,
        "stalled flow accepted {accepted} bytes, above the per-flow bound {bound}"
    );

    thread::sleep(Duration::from_secs(1));
    let slab_start = unreclaimable_slab_kib();
    thread::sleep(Duration::from_secs(9));
    let slab_end = unreclaimable_slab_kib();
    assert!(
        slab_end <= slab_start + 1024,
        "unreclaimable slab rose {} KiB over the stall (measured ≤ 0.6 MiB, P-67)",
        slab_end.saturating_sub(slab_start)
    );
    match (&mut &flow.v_far).write(&data[accepted..]) {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        other => panic!("the sender must still be blocked after 10 s, got {other:?}"),
    }
    assert_eq!(flow.fwd.take_end().expect("take_end"), None);

    flow.v_far.set_nonblocking(false);
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &flow.v_far).write_all(&data[accepted..]).expect("resume");
            flow.v_far.shutdown(libc::SHUT_WR);
        });
        let (got, end) = read_until_end(&flow.t_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(got == data, "the flow resumes in order: {} of {}", got.len(), data.len());
    });
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_stalled_tcp_source_toward_a_paused_vsock_reader_is_bounded_too() {
    let flow = StreamFlow::new();
    let data = pattern(35, 256 << 20);
    flow.t_far.set_nonblocking(true).expect("nonblocking");
    let accepted = fill_until_blocked(&flow.t_far, &data);
    let bound = tcp_mem_max("tcp_wmem")
        + tcp_mem_max("tcp_rmem")
        + usize::try_from(flow.v_far.buffer_size()).expect("buffer size")
        + FWD_CHUNK;
    assert!(accepted <= bound, "accepted {accepted} bytes, above the per-flow bound {bound}");
    assert_eq!(flow.fwd.take_end().expect("take_end"), None);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_read_shutdown_only_peer_holds_the_flow_open_until_it_closes() {
    // `shutdown(SHUT_RD)` sends nothing (net/ipv4/tcp.c:3113-3114 @ v7.0):
    // the flow looks exactly like a stalled reader and holds with no
    // deadline and no abort (owner policy: none).
    let flow = StreamFlow::new();
    flow.t_far.shutdown(Shutdown::Read).expect("SHUT_RD");
    flow.v_far.set_nonblocking(true);
    let _held = fill_until_blocked(&flow.v_far, &pattern(36, 64 << 20));
    thread::sleep(STALL_ABSENCE_HOLD);
    assert_no_end(&flow.fwd);
    assert!(flow.fwd.pair_state(flow.id).expect("pair_state").is_some());
    let StreamFlow { fwd, id, v, v_far, t, t_far, .. } = flow;
    drop(t_far);
    let end = wait_end_of(&fwd, id);
    assert!(
        matches!(
            end.outcome,
            ForwardOutcome::Aborted { cause: AbortCause::SocketError { side: PairSide::B, .. } }
        ),
        "closing with unread bytes resets the TCP half, got {end:?}"
    );
    drop((v, v_far, t));
}

/// V-EV-SAFE, full-target resume, vsock target (ADR-0204 rule 5; the
/// round-16 wait-queue trigger): a TCP source toward a vsock target whose
/// peer's credit is exhausted — the guest-side reader stops, the source
/// fills until its writer blocks, and the pair holds with no end. When the
/// reader resumes, the stalled direction resumes through the target's
/// write-space wait-queue entry alone (the vsock's credit update wakes the
/// socket's wait queue; `sk_write_space` is never overridden): every byte
/// arrives once and in order, then the half-close. The earlier override's
/// resumption (P-67) is not evidence for the entry; this case is.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_target_with_its_peers_credit_exhausted_resumes_byte_exact_to_eof() {
    let flow = StreamFlow::with(|_, v_far, _, _| v_far.set_buffer_size(64 * 1024));
    let data = pattern(37, 64 << 20);
    flow.t_far.set_nonblocking(true).expect("nonblocking");
    let accepted = fill_until_blocked(&flow.t_far, &data);
    assert!(accepted < data.len(), "the source blocked against the exhausted credit");
    thread::sleep(STALL_ABSENCE_HOLD);
    assert_eq!(flow.fwd.take_end().expect("take_end"), None, "a stalled pair is never ended");
    let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("live");
    assert!(!state.b_to_a.target_shut, "the stalled target is not shut");

    flow.t_far.set_nonblocking(false).expect("blocking");
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &flow.t_far).write_all(&data[accepted..]).expect("resume the source");
            flow.t_far.shutdown(Shutdown::Write).expect("FIN");
        });
        let (got, end) = read_until_end(&flow.v_far);
        assert_eq!(end, FarEnd::Eof, "the half-close follows the last byte");
        assert!(
            got == data,
            "the vsock target resumes byte-exact: {} of {}",
            got.len(),
            data.len()
        );
    });
    assert_eq!(flow.far_end(), ForwardOutcome::GuestHalfClose);
}

/// V-EV-SAFE, full-target resume, TCP target under the wait-queue trigger,
/// repeated: the guest-side source refills the TCP target to its blocked
/// point in three rounds; between rounds the far reader drains, and each
/// round's resumption is a wake of the target's write-space wait-queue
/// entry. The flow ends byte-exact with a clean half-close.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_target_resumes_through_its_wait_queue_entry_on_every_refill() {
    let flow = StreamFlow::new();
    let data = pattern(38, 48 << 20);
    flow.v_far.set_nonblocking(true);
    let mut sent = 0usize;
    let mut got: Vec<u8> = Vec::with_capacity(data.len());
    let mut buf = vec![0u8; 1 << 20];
    flow.t_far.set_read_timeout(Some(Duration::from_millis(200))).expect("timeout");
    for _round in 0..3 {
        sent += fill_until_blocked(&flow.v_far, &data[sent..]);
        assert_eq!(flow.fwd.take_end().expect("take_end"), None, "stalled, not ended");
        // Drain what the target delivered until it goes quiet.
        loop {
            match std::io::Read::read(&mut &flow.t_far, &mut buf) {
                Ok(0) => panic!("the target half-closed before the source did"),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
                Err(e) => panic!("read: {e}"),
            }
        }
    }
    flow.v_far.set_nonblocking(false);
    flow.t_far.set_read_timeout(Some(END_WAIT)).expect("timeout");
    thread::scope(|s| {
        s.spawn(|| {
            (&mut &flow.v_far).write_all(&data[sent..]).expect("the rest");
            flow.v_far.shutdown(libc::SHUT_WR);
        });
        let (rest, end) = read_until_end(&flow.t_far);
        got.extend_from_slice(&rest);
        assert_eq!(end, FarEnd::Eof);
    });
    assert!(got == data, "the TCP target resumes byte-exact: {} of {}", got.len(), data.len());
}
