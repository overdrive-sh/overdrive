//! End classification of a stream pair (R5-45 a–d; EVMOD-END, ADR-0180;
//! CLEAN-FULL-END, END-IDEMPOTENT): a clean end is forwarded after the last
//! accepted byte, every abort reaches every far end as a reset, never EOF.
//!
//! Ends present at attach (R5-45 d) are in `attach_watch.rs`.
//!
//! The guest side's close of its vsock end is [`close_watched`]: its watch
//! (the guest-side classifier of every [`StreamFlow`]) is aborted first, as
//! the far end's own forwarder would be, so the close reaches `V_h`.
//!
//! The vsock reset delivered by the device release's orphan sweep
//! (`drivers/vhost/vsock.c:746-750 @ v7.0`) needs a vhost peer; the
//! loopback transport has no device release, so this host-role suite does
//! not cover it.

use std::io::{ErrorKind, Write};
use std::net::Shutdown;
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardOutcome, PairSide, SourceEnd,
};
use overdrive_core::guest_transport::wire::KernelErrno;

use super::support::{
    END_WAIT, FarEnd, StreamFlow, TCP_CLOSE_WAIT, assert_far_tcp_clean, assert_no_end,
    close_watched, pattern, read_exact_n, read_until_end, reset_close, wait_tcp_state,
};

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_peer_half_close_is_forwarded_as_fin_after_the_last_byte() {
    // R5-45 (a): flags 0x2 (virtio_transport_common.c:1084-1096 @ v7.0).
    let flow = StreamFlow::new();
    let data = pattern(20, 3 << 20);
    (&mut &flow.v_far).write_all(&data).expect("write");
    flow.v_far.shutdown(libc::SHUT_WR);
    assert_far_tcp_clean(&flow.t_far, &data);
    let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("still live: B→A open");
    assert_eq!(state.a_to_b.source_end, Some(SourceEnd::Clean));
    assert!(state.a_to_b.target_shut);
    assert_eq!(state.a_to_b.moved, data.len() as u64);
    // The other direction still forwards.
    (&mut &flow.t_far).write_all(b"still-open").expect("write B→A");
    assert_eq!(read_exact_n(&flow.v_far, 10), b"still-open");
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_fin_is_forwarded_as_a_guest_half_close_after_the_last_byte() {
    let flow = StreamFlow::new();
    let data = pattern(21, 1 << 20);
    (&mut &flow.t_far).write_all(&data).expect("write");
    flow.t_far.shutdown(Shutdown::Write).expect("FIN");
    let (got, end) = read_until_end(&flow.v_far);
    assert_eq!(end, FarEnd::Eof);
    assert!(got == data, "byte-exact before the half-close");
    assert_eq!(flow.far_end(), ForwardOutcome::GuestHalfClose);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_peer_close_with_a_direction_open_aborts_and_resets_the_tcp_far_end() {
    // R5-45 (b): 0x3 with the peer's direction open.
    let flow = StreamFlow::new();
    (&mut &flow.v_far).write_all(b"partial").expect("write");
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    close_watched(&peer, far_watch, v_far);
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    let (_, far) = read_until_end(super::support::bounded(&t_far));
    assert_eq!(far, FarEnd::Error(ErrorKind::ConnectionReset), "never EOF after an abort");
    drop((v, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_half_close_then_a_close_while_this_side_still_sends_is_an_abort() {
    // RCV_SHUTDOWN newly set while this side still sends: the peer stopped
    // receiving (a send would fail EPIPE, af_vsock.c:2176-2177 @ v7.0).
    let flow = StreamFlow::new();
    flow.v_far.shutdown(libc::SHUT_WR);
    assert_far_tcp_clean(&flow.t_far, &[]);
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    close_watched(&peer, far_watch, v_far);
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    // The TCP far end already read EOF for A→B; its own direction is reset.
    let deadline = Instant::now() + END_WAIT;
    loop {
        match (&mut &t_far).write(b"x") {
            Err(e) if matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::BrokenPipe) => {
                break;
            }
            Ok(_) | Err(_) => {}
        }
        assert!(Instant::now() < deadline, "the TCP far end's direction must be reset");
        thread::sleep(Duration::from_millis(5));
    }
    drop((v, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_reset_after_a_fin_aborts_with_epipe_and_resets_the_guest_side() {
    // R5-45 (c) and hazard 16: after its FIN the forwarder stops reading the
    // TCP half, so the later reset is seen only by sk_error_report
    // (net/ipv4/tcp_input.c:4890-4902 @ v7.0); in CLOSE_WAIT its errno is
    // EPIPE (:4905-4930).
    let flow = StreamFlow::new();
    flow.t_far.shutdown(Shutdown::Write).expect("FIN");
    wait_tcp_state(&flow.t, TCP_CLOSE_WAIT);
    let (_, eof) = read_until_end(&flow.v_far);
    assert_eq!(eof, FarEnd::Eof, "the FIN reached the guest side as a half-close");
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    reset_close(t_far);
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted {
            cause: AbortCause::SocketError {
                side: PairSide::B,
                errno: KernelErrno::new(libc::EPIPE as u16).expect("EPIPE"),
            },
        }
    );
    // The watch recorded the first class of its direction — the clean
    // half-close — and the pair's abort then shut V_h both ways; the guest
    // side holder's own direction is now reset (a write fails).
    assert_eq!(
        super::support::wait_end_of(&peer, far_watch).outcome,
        ForwardOutcome::GuestHalfClose
    );
    let deadline = Instant::now() + END_WAIT;
    while (&mut &v_far).write(b"x").is_ok() {
        assert!(Instant::now() < deadline, "the guest side's direction must end after the abort");
        thread::sleep(Duration::from_millis(5));
    }
    drop((v, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_reset_with_both_directions_open_aborts_with_econnreset() {
    let flow = StreamFlow::new();
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    reset_close(t_far);
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: PairSide::B, errno: KernelErrno::ECONNRESET }
        }
    );
    assert!(matches!(
        super::support::wait_end_of(&peer, far_watch).outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { .. } }
    ));
    drop((v, v_far, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn clean_full_end_a_peer_close_after_both_half_closes_completes_every_time() {
    // CLEAN-FULL-END and the h_lateshut falsifier: the guest side closes the
    // instant it reads EOF, so its 0x3 races the forwarder's own SHUT_WR on
    // V_h. The forwarder reads its own `sk_shutdown & SEND_SHUTDOWN` under
    // the socket lock (af_vsock.c:1203, :1216, :1221 @ v7.0), so the 0x3 is
    // never classified before it. A forwarder flag published after the
    // SHUTDOWN left aborted 270 of 300 (P-71).
    for round in 0..300u64 {
        let flow = StreamFlow::new();
        let data = pattern(round, 4096);
        flow.v_far.shutdown(libc::SHUT_WR);
        (&mut &flow.t_far).write_all(&data).expect("write B→A");
        flow.t_far.shutdown(Shutdown::Write).expect("FIN");
        let (got, end) = read_until_end(&flow.v_far);
        assert_eq!((got.len(), end), (data.len(), FarEnd::Eof));
        let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
        close_watched(&peer, far_watch, v_far);
        let record = super::support::wait_end_of(&fwd, id);
        assert_eq!(
            record.outcome,
            ForwardOutcome::Completed,
            "round {round}: a clean full end aborted"
        );
        assert_far_tcp_clean(&t_far, &[]);
        drop((v, t));
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn end_idempotent_a_trailing_reset_after_completion_adds_nothing() {
    let flow = StreamFlow::new();
    flow.v_far.shutdown(libc::SHUT_WR);
    flow.t_far.shutdown(Shutdown::Write).expect("FIN");
    assert_eq!(flow.end().outcome, ForwardOutcome::Completed);
    // The owner has not closed its descriptors yet; the far ends now close
    // and reset after the pair's end.
    let StreamFlow { fwd, peer, far_watch, v, v_far, t, t_far, .. } = flow;
    reset_close(t_far);
    close_watched(&peer, far_watch, v_far);
    assert_no_end(&fwd);
    let inv = fwd.inventory().expect("inventory");
    assert_eq!((inv.completed, inv.aborted, inv.live_pairs, inv.queued_ends), (1, 0, 0, 0));
    drop((v, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_peer_close_with_a_stalled_pipeline_aborts_after_no_more_bytes() {
    // An abort never forwards a byte past it: with 6 MiB queued toward a
    // stopped TCP reader, the guest side closing aborts the pair and the TCP
    // far end reads a reset at or before the end of what was accepted.
    let flow = StreamFlow::new();
    flow.v_far.set_nonblocking(true);
    let data = pattern(22, 6 << 20);
    let mut sent = 0usize;
    while let Ok(n) = (&mut &flow.v_far).write(&data[sent..]) {
        sent += n;
        if sent == data.len() {
            break;
        }
    }
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    close_watched(&peer, far_watch, v_far);
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    let (got, far) = read_until_end(super::support::bounded(&t_far));
    assert_eq!(far, FarEnd::Error(ErrorKind::ConnectionReset));
    assert!(got.len() as u64 <= end.moved_ab, "no byte past the abort");
    assert!(got == data[..got.len()], "what arrived is a byte-exact prefix");
    drop((v, t));
}
