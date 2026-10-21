//! Attach and watch: the states accepted, the refusals and their
//! precedence, `bad_side`, id consumption, address families and the ULP
//! refusal (EVMOD-PAIR, EVMOD-ATTACH-END, EVMOD-KTLS; § *Control
//! interface*, "States attach and watch accept" and "Attach refusals").
//!
//! The adapter's mapping of the kernel's `EBADF` / `ENOTSOCK` and `EINVAL`
//! classes to a `ForwarderError` variant is not pinned by the design; those
//! cases assert only what is pinned — a typed refusal that is not any later
//! class's variant (precedence), and nothing hooked.

use std::io::Write;
use std::net::TcpStream;
use std::os::fd::AsFd;

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, FlowForwarder, ForwardOutcome, ForwarderError, PairMode, PairSide,
};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_fwd::KernelFlowForwarder;

use super::support::{
    TCP_CLOSE, TCP_CLOSE_WAIT, Vsock, VsockListener, assert_far_tcp_clean, attach_stream,
    install_tls_ulp, live, open_context, pipe, read_exact_n, reset_close, tcp_listener, tcp_pair,
    tcp6_pair, udp_pair, vsock_pair, wait_end_of, wait_tcp_state,
};

/// The variants of the refusal classes that follow `EBADF` / `ENOTSOCK`
/// and `EINVAL` in the precedence (ESOCKTNOSUPPORT onward).
fn is_later_class(e: &ForwarderError) -> bool {
    matches!(
        e,
        ForwarderError::UnsupportedSocket { .. }
            | ForwarderError::UpperLayerProtocol { .. }
            | ForwarderError::HalfEnded { .. }
            | ForwarderError::NotConnected { .. }
            | ForwarderError::AlreadyHooked { .. }
            | ForwarderError::BufferTooSmall { .. }
    )
}

/// After a refusal, both halves are unhooked: the same halves attach.
fn assert_nothing_hooked(fwd: &KernelFlowForwarder, v: &Vsock, t: &TcpStream) {
    let id = attach_stream(fwd, v, t);
    assert_eq!(fwd.abort(id).expect("abort"), AbortResult::Aborted);
    let end = wait_end_of(fwd, id);
    assert_eq!(end.outcome, ForwardOutcome::Aborted { cause: AbortCause::Detached });
}

// ---------------------------------------------------------------------------
// Accepted states
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 4"]
fn established_halves_are_accepted_and_forward_bytes_queued_before_attach() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    // Bytes that arrived before attach wait in the source's own queue; the
    // attach-time run forwards them (EVMOD-PAIR).
    (&v_far).write_all(b"early").expect("write before attach");
    let id = attach_stream(&fwd, &v, &t);
    assert_eq!(read_exact_n(&t_far, 5), b"early");
    assert_eq!(live(&fwd).0, 1);
    assert!(fwd.pair_state(id).expect("pair_state").is_some());
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_in_close_wait_is_accepted_and_its_fin_carried_out_at_attach() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    (&t_far).write_all(b"write-then-fin").expect("write");
    t_far.shutdown(std::net::Shutdown::Write).expect("FIN");
    wait_tcp_state(&t, TCP_CLOSE_WAIT);
    let id = attach_stream(&fwd, &v, &t);
    // The queued bytes and then the clean end reach the vsock far end.
    v_far.set_read_timeout(super::support::END_WAIT);
    let (got, end) = super::support::read_until_end(&v_far);
    assert_eq!(got, b"write-then-fin");
    assert_eq!(end, super::support::FarEnd::Eof);
    let state = fwd.pair_state(id).expect("pair_state").expect("live");
    assert!(state.b_to_a.target_shut, "the vsock target is shut after the TCP source's FIN");
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_half_whose_peer_half_closed_is_accepted_and_classified_clean_at_attach() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    (&v_far).write_all(b"tail").expect("write");
    v_far.shutdown(libc::SHUT_WR);
    std::thread::sleep(super::support::QUIET);
    let _id = attach_stream(&fwd, &v, &t);
    // `peer_shutdown = SEND_SHUTDOWN` only: a clean half-close of the peer's
    // direction, forwarded as EOF after the last byte.
    assert_far_tcp_clean(&t_far, b"tail");
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn ipv6_tcp_halves_are_accepted() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp6_pair();
    let _id = attach_stream(&fwd, &v, &t);
    (&v_far).write_all(b"over-ipv6").expect("write");
    assert_eq!(read_exact_n(&t_far, 9), b"over-ipv6");
}

// ---------------------------------------------------------------------------
// Ended halves: HalfEnded (ECONNABORTED with end_errno)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_reset_before_attach_is_refused_half_ended_with_econnreset() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    reset_close(t_far);
    wait_tcp_state(&t, TCP_CLOSE);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::HalfEnded { side, errno }) => {
            assert_eq!(side, PairSide::B);
            assert_eq!(errno, KernelErrno::ECONNRESET);
        }
        other => panic!("a reset TCP half must be HalfEnded, got {other:?}"),
    }
    // Refused, never hooked: the vsock half can still be attached.
    let (t2, _t2_far) = tcp_pair();
    let id = attach_stream(&fwd, &v, &t2);
    assert!(fwd.pair_state(id).expect("pair_state").is_some());
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_reset_in_close_wait_is_refused_half_ended_with_epipe() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    t_far.shutdown(std::net::Shutdown::Write).expect("FIN");
    wait_tcp_state(&t, TCP_CLOSE_WAIT);
    reset_close(t_far);
    wait_tcp_state(&t, TCP_CLOSE);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::HalfEnded { side, errno }) => {
            assert_eq!(side, PairSide::B);
            // `tcp_reset` reports EPIPE for a half in CLOSE_WAIT
            // (net/ipv4/tcp_input.c:4905-4930 @ v7.0).
            assert_eq!(errno, KernelErrno::new(libc::EPIPE as u16).expect("EPIPE"));
        }
        other => panic!("a reset CLOSE_WAIT half must be HalfEnded(EPIPE), got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_closing_vsock_half_is_refused_half_ended() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    // A peer close with nothing queued leaves the half in CLOSING
    // (virtio_transport_common.c:1249-1252 @ v7.0).
    drop(v_far);
    std::thread::sleep(super::support::QUIET);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::HalfEnded { side, errno }) => {
            assert_eq!(side, PairSide::A);
            assert_eq!(errno, KernelErrno::ECONNRESET, "no sk_err: end_errno is ECONNRESET");
        }
        other => panic!("a CLOSING vsock half must be HalfEnded, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_half_whose_peer_closed_with_bytes_queued_is_accepted_and_aborted_in_its_first_run() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    (&v_far).write_all(b"queued-then-close").expect("write");
    drop(v_far);
    std::thread::sleep(super::support::QUIET);
    let id = attach_stream(&fwd, &v, &t);
    let end = wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert_eq!(end.moved_ab, 0, "an abort found at attach moves no byte");
    super::support::assert_far_tcp_reset(&t_far);
}

// ---------------------------------------------------------------------------
// Owner-defect refusals
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 4"]
fn halves_that_never_connected_or_listen_are_refused_not_connected() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();

    let listening = tcp_listener();
    match fwd.attach(&v as &dyn AsFd, &listening as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::NotConnected { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a listening TCP half must be NotConnected, got {other:?}"),
    }
    let unconnected = Vsock::unconnected();
    match fwd.attach(&unconnected as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::NotConnected { side }) => assert_eq!(side, PairSide::A),
        other => panic!("an unconnected vsock half must be NotConnected, got {other:?}"),
    }
    let vlisten = VsockListener::bind();
    match fwd.watch(&vlisten as &dyn AsFd) {
        Err(ForwarderError::NotConnected { .. }) => {}
        other => panic!("a listening vsock must not be watched, got {other:?}"),
    }
    assert_nothing_hooked(&fwd, &v, &t);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_half_whose_own_send_this_side_shut_is_refused_not_connected() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    t.shutdown(std::net::Shutdown::Write).expect("own SHUT_WR");
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::NotConnected { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a TCP half in FIN_WAIT must be NotConnected, got {other:?}"),
    }
    let (v2, _v2_far) = vsock_pair();
    let (t2, _t2_far) = tcp_pair();
    v2.shutdown(libc::SHUT_WR);
    match fwd.attach(&v2 as &dyn AsFd, &t2 as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::NotConnected { side }) => assert_eq!(side, PairSide::A),
        other => panic!("a vsock half with its own send shut must be NotConnected, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn unsupported_kinds_are_refused_unsupported_socket_naming_the_side() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (v2, _v2_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    let (t2, _t2_far) = tcp_pair();
    let (u, _u_far) = udp_pair();
    // Not exactly one vsock STREAM half.
    match fwd.attach(&v as &dyn AsFd, &v2 as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { .. }) => {}
        other => panic!("two vsock halves must be UnsupportedSocket, got {other:?}"),
    }
    match fwd.attach(&t as &dyn AsFd, &t2 as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { .. }) => {}
        other => panic!("two TCP halves must be UnsupportedSocket, got {other:?}"),
    }
    // A UDP half in stream mode; a TCP half where datagram mode names UDP.
    match fwd.attach(&v as &dyn AsFd, &u as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a UDP half in stream mode must be UnsupportedSocket, got {other:?}"),
    }
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Datagram { udp: PairSide::B }) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a TCP half where UDP is named must be UnsupportedSocket, got {other:?}"),
    }
    // Watch takes one vsock STREAM half only.
    match fwd.watch(&t as &dyn AsFd) {
        Err(ForwarderError::UnsupportedSocket { .. }) => {}
        other => panic!("watching a TCP socket must be UnsupportedSocket, got {other:?}"),
    }
    assert_nothing_hooked(&fwd, &v, &t);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_with_an_upper_layer_protocol_is_refused() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    install_tls_ulp(&t);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UpperLayerProtocol { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a kTLS TCP half must be UpperLayerProtocol (EVMOD-KTLS), got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_half_already_attached_or_watched_is_refused_already_hooked() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    let (t2, _t2_far) = tcp_pair();
    let _id = attach_stream(&fwd, &v, &t);
    match fwd.attach(&v as &dyn AsFd, &t2 as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::AlreadyHooked { side }) => assert_eq!(side, PairSide::A),
        other => panic!("an attached half must be AlreadyHooked, got {other:?}"),
    }
    match fwd.watch(&v as &dyn AsFd) {
        Err(ForwarderError::AlreadyHooked { .. }) => {}
        other => panic!("watching an attached vsock must be AlreadyHooked, got {other:?}"),
    }
    // Hooked by another context: `sk_user_data` is per socket, not per context.
    let other_ctx = open_context();
    let (w, _w_far) = vsock_pair();
    let _watch = other_ctx.watch(&w as &dyn AsFd).expect("watch in another context");
    let (t3, _t3_far) = tcp_pair();
    match fwd.attach(&w as &dyn AsFd, &t3 as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::AlreadyHooked { side }) => assert_eq!(side, PairSide::A),
        other => panic!("a half watched elsewhere must be AlreadyHooked, got {other:?}"),
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_datagram_vsock_half_below_the_largest_frame_is_refused_buffer_too_small() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (u, _u_far) = udp_pair();
    v.set_buffer_size(32 * 1024);
    assert!(
        v.buffer_size()
            < u64::from(overdrive_core::guest_transport::wire::MAX_GUEST_DATAGRAM_FRAME)
    );
    match fwd.attach(&v as &dyn AsFd, &u as &dyn AsFd, PairMode::Datagram { udp: PairSide::B }) {
        Err(ForwarderError::BufferTooSmall { side }) => assert_eq!(side, PairSide::A),
        other => panic!("a 32 KiB vsock half must be BufferTooSmall (EV-DGRAM D3), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Precedence, bad_side, ids
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER step 4"]
fn refusals_follow_the_pinned_precedence() {
    let fwd = open_context();

    // ENOTSOCK precedes every later class: a pipe as half B with a
    // never-connected vsock as half A is not refused NotConnected.
    let (pipe_r, _pipe_w) = pipe();
    let unconnected = Vsock::unconnected();
    let e = fwd
        .attach(&unconnected as &dyn AsFd, &pipe_r as &dyn AsFd, PairMode::Stream)
        .expect_err("a pipe half is refused");
    assert!(!is_later_class(&e), "ENOTSOCK precedes ENOTCONN, got {e:?}");

    // EINVAL (one socket twice) precedes EPROTONOSUPPORT.
    let (k, _k_far) = tcp_pair();
    install_tls_ulp(&k);
    let e = fwd
        .attach(&k as &dyn AsFd, &k as &dyn AsFd, PairMode::Stream)
        .expect_err("same socket twice");
    assert!(!is_later_class(&e), "EINVAL precedes EPROTONOSUPPORT, got {e:?}");

    // ESOCKTNOSUPPORT precedes EPROTONOSUPPORT: two TCP halves, one with a ULP.
    let (k2, _k2_far) = tcp_pair();
    let e = fwd
        .attach(&k as &dyn AsFd, &k2 as &dyn AsFd, PairMode::Stream)
        .expect_err("two TCP halves");
    assert!(matches!(e, ForwarderError::UnsupportedSocket { .. }), "got {e:?}");

    // EPROTONOSUPPORT precedes the state check: a ULP half beside a
    // never-connected vsock half.
    let e =
        fwd.attach(&unconnected as &dyn AsFd, &k as &dyn AsFd, PairMode::Stream).expect_err("ULP");
    assert!(matches!(e, ForwarderError::UpperLayerProtocol { side: PairSide::B }), "got {e:?}");

    // The state check runs half A then half B: an unconnected A beside an
    // ended B names A.
    let (ended, ended_far) = tcp_pair();
    reset_close(ended_far);
    wait_tcp_state(&ended, TCP_CLOSE);
    let e = fwd
        .attach(&unconnected as &dyn AsFd, &ended as &dyn AsFd, PairMode::Stream)
        .expect_err("state");
    assert!(matches!(e, ForwarderError::NotConnected { side: PairSide::A }), "got {e:?}");

    // EBUSY precedes EMSGSIZE: an attached vsock half with a small buffer in
    // datagram mode.
    let (small, _small_far) = vsock_pair();
    small.set_buffer_size(32 * 1024);
    let (tt, _tt_far) = tcp_pair();
    let _small_id = attach_stream(&fwd, &small, &tt);
    let (u, _u_far) = udp_pair();
    let e =
        fwd.attach(&small as &dyn AsFd, &u as &dyn AsFd, PairMode::Datagram { udp: PairSide::B });
    assert!(matches!(e, Err(ForwarderError::AlreadyHooked { side: PairSide::A })), "got {e:?}");
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_refused_attach_consumes_an_id_and_ids_strictly_increase() {
    let fwd = open_context();
    let (v1, _v1_far) = vsock_pair();
    let (t1, _t1_far) = tcp_pair();
    let first = attach_stream(&fwd, &v1, &t1);
    assert_eq!(first.get(), 1, "ids start at 1 in a fresh context");

    let listening = tcp_listener();
    let (v2, _v2_far) = vsock_pair();
    fwd.attach(&v2 as &dyn AsFd, &listening as &dyn AsFd, PairMode::Stream).expect_err("refused");

    let (t2, _t2_far) = tcp_pair();
    let second = attach_stream(&fwd, &v2, &t2);
    assert!(
        second.get() >= first.get() + 2,
        "a refused attach consumes an id: {first:?} then {second:?}"
    );

    let (w, _w_far) = vsock_pair();
    let third = fwd.watch(&w as &dyn AsFd).expect("watch");
    assert!(third.get() > second.get(), "watches and pairs share one id sequence");
}

// ---------------------------------------------------------------------------
// Error-queue exclusion (ADR-0204 rule 5, "Excluded by precondition")
// ---------------------------------------------------------------------------

/// `SO_ZEROCOPY` (`include/uapi/asm-generic/socket.h` @ v7.0).
const SO_ZEROCOPY: libc::c_int = 60;
/// `SO_TIMESTAMPING_OLD` (`include/uapi/asm-generic/socket.h` @ v7.0).
const SO_TIMESTAMPING: libc::c_int = 37;
/// `SOF_TIMESTAMPING_TX_SOFTWARE`, a bit of `SOF_TIMESTAMPING_TX_RECORD_MASK`
/// (`include/uapi/linux/net_tstamp.h:59-63 @ v7.0`).
const SOF_TIMESTAMPING_TX_SOFTWARE: libc::c_int = 1 << 1;
/// `SOF_TIMESTAMPING_RX_SOFTWARE`: a receive-only bit, outside the mask.
const SOF_TIMESTAMPING_RX_SOFTWARE: libc::c_int = 1 << 3;

fn set_int_option(fd: std::os::fd::RawFd, name: libc::c_int, value: libc::c_int) {
    // SAFETY: valid option value of the passed length.
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            name,
            (&raw const value).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    assert!(ret == 0, "setsockopt({name}, {value}): {}", std::io::Error::last_os_error());
}

/// A TCP half with `SOCK_ZEROCOPY` set is refused `UnsupportedSocket`
/// naming it (the kernel's `ESOCKTNOSUPPORT`), nothing is hooked, and the
/// same socket attaches once the option is cleared (negative control: the
/// refusal is keyed on the option, not on the socket).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_with_zerocopy_set_is_refused_unsupported_socket() {
    use std::os::fd::AsRawFd;
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    set_int_option(t.as_raw_fd(), SO_ZEROCOPY, 1);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a SO_ZEROCOPY TCP half must be UnsupportedSocket {{ B }}, got {other:?}"),
    }
    assert_eq!(live(&fwd), (0, 0, 0), "a refused attach hooks nothing");
    set_int_option(t.as_raw_fd(), SO_ZEROCOPY, 0);
    assert_nothing_hooked(&fwd, &v, &t);
}

/// A TCP half with a transmit-timestamping bit in `sk_tsflags` is refused
/// `UnsupportedSocket` naming it; a receive-only timestamping bit (outside
/// `SOF_TIMESTAMPING_TX_RECORD_MASK`) is not refused (negative control: the
/// refusal is the transmit mask, not any timestamping).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_tcp_half_with_transmit_timestamping_is_refused_unsupported_socket() {
    use std::os::fd::AsRawFd;
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    set_int_option(t.as_raw_fd(), SO_TIMESTAMPING, SOF_TIMESTAMPING_TX_SOFTWARE);
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::B),
        other => {
            panic!("a TX-timestamping TCP half must be UnsupportedSocket {{ B }}, got {other:?}")
        }
    }
    assert_eq!(live(&fwd), (0, 0, 0), "a refused attach hooks nothing");
    set_int_option(t.as_raw_fd(), SO_TIMESTAMPING, SOF_TIMESTAMPING_RX_SOFTWARE);
    assert_nothing_hooked(&fwd, &v, &t);
}

/// A UDP half with `SOCK_ZEROCOPY` set is refused in datagram mode, naming
/// it; cleared, the same association attaches.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_udp_half_with_zerocopy_set_is_refused_unsupported_socket() {
    use std::os::fd::AsRawFd;
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (u, _u_far) = udp_pair();
    set_int_option(u.as_raw_fd(), SO_ZEROCOPY, 1);
    let mode = PairMode::Datagram { udp: PairSide::B };
    match fwd.attach(&v as &dyn AsFd, &u as &dyn AsFd, mode) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::B),
        other => panic!("a SO_ZEROCOPY UDP half must be UnsupportedSocket {{ B }}, got {other:?}"),
    }
    set_int_option(u.as_raw_fd(), SO_ZEROCOPY, 0);
    let id = fwd.attach(&v as &dyn AsFd, &u as &dyn AsFd, mode).expect("cleared, it attaches");
    assert_eq!(fwd.abort(id).expect("abort"), AbortResult::Aborted);
    let _ = wait_end_of(&fwd, id);
}

/// Watch refuses a vsock half with a transmit-timestamping bit, and the
/// vsock half of an attach with the same bit is refused naming side A; with
/// the bit cleared the same socket is watched (negative control).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_vsock_half_with_transmit_timestamping_is_refused_at_watch_and_attach() {
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    set_int_option(v.raw(), SO_TIMESTAMPING, SOF_TIMESTAMPING_TX_SOFTWARE);
    match fwd.watch(&v as &dyn AsFd) {
        Err(ForwarderError::UnsupportedSocket { .. }) => {}
        other => {
            panic!("watching a TX-timestamping vsock must be UnsupportedSocket, got {other:?}")
        }
    }
    match fwd.attach(&v as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream) {
        Err(ForwarderError::UnsupportedSocket { side }) => assert_eq!(side, PairSide::A),
        other => {
            panic!("a TX-timestamping vsock half must be UnsupportedSocket {{ A }}, got {other:?}")
        }
    }
    assert_eq!(live(&fwd), (0, 0, 0), "a refused watch or attach hooks nothing");
    set_int_option(v.raw(), SO_TIMESTAMPING, 0);
    let watch = fwd.watch(&v as &dyn AsFd).expect("cleared, the vsock half is watched");
    assert_eq!(fwd.abort(watch).expect("abort"), AbortResult::Aborted);
    let _ = wait_end_of(&fwd, watch);
}
