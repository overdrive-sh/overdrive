//! The watch of a dialled `V_h` (ADR-0216, EVMOD-STREAM-END) and
//! WATCH-ABORT (ADR-0211): the watch classifies the guest's direction and
//! queues its record before the holder is woken; an owner abort records
//! and unhooks without touching the socket; only a context release shuts a
//! watched socket.
//!
//! The device release's orphan sweep path (record queued before the holder
//! is woken, then `ECONNRESET`, `drivers/vhost/vsock.c:725-750 @ v7.0`)
//! needs a vhost peer and is not in this host-role suite.

use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsFd;
use std::thread;
use std::time::Duration;

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, AbortResult, FlowForwarder, ForwardOutcome,
};

use super::support::{
    END_WAIT, FarEnd, live, open_context, read_exact_n, read_until_end, vsock_pair, wait_end_of,
};

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_guest_half_close_is_recorded_and_the_hosts_direction_stays_with_the_holder() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let id = fwd.watch(&v as &dyn AsFd).expect("watch");
    (&mut &v_far).write_all(b"reply").expect("write");
    v_far.shutdown(libc::SHUT_WR);
    v.set_read_timeout(END_WAIT);
    let (got, end) = read_until_end(&v);
    assert_eq!((got.as_slice(), end), (&b"reply"[..], FarEnd::Eof));
    let record = wait_end_of(&fwd, id);
    assert_eq!(record.outcome, ForwardOutcome::GuestHalfClose);
    assert_eq!((record.moved_ab, record.moved_ba), (0, 0), "a watch moves no byte");
    // The holder still holds an open direction: the watch shut nothing.
    (&mut &v).write_all(b"more").expect("the host's direction is the holder's");
    assert_eq!(read_exact_n(&v_far, 4), b"more");
    assert_eq!(live(&fwd), (0, 0, 0));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_guest_close_with_a_direction_open_is_recorded_aborted() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let id = fwd.watch(&v as &dyn AsFd).expect("watch");
    drop(v_far);
    assert!(matches!(
        wait_end_of(&fwd, id).outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { .. } }
    ));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_watch_on_a_vsock_whose_guest_direction_already_ended_records_at_once() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    v_far.shutdown(libc::SHUT_WR);
    thread::sleep(super::support::QUIET);
    let id = fwd.watch(&v as &dyn AsFd).expect("an ended guest direction is still watchable");
    assert_eq!(wait_end_of(&fwd, id).outcome, ForwardOutcome::GuestHalfClose);
    drop((v, v_far));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn the_record_is_queued_before_the_holder_observes_the_end() {
    // Record first, then the original callback (hazard 8 is the opposite
    // order): a holder woken by its read's end always finds the record.
    for round in 0..400u32 {
        let fwd = open_context();
        let (v, v_far) = vsock_pair();
        let id = fwd.watch(&v as &dyn AsFd).expect("watch");
        v.set_read_timeout(END_WAIT);
        let reader = thread::spawn(move || {
            let (_, end) = read_until_end(&v);
            (v, end)
        });
        if round % 2 == 0 {
            v_far.shutdown(libc::SHUT_WR);
        } else {
            drop(v_far);
        }
        let (v, _end) = reader.join().expect("holder");
        let record = fwd.take_end().expect("take_end");
        assert!(
            matches!(record, Some(end) if end.id == id),
            "round {round}: the holder's read ended before its watch's record was queued"
        );
        drop(v);
    }
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn watch_abort_records_unhooks_and_never_touches_the_socket() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let id = fwd.watch(&v as &dyn AsFd).expect("watch");
    assert_eq!(fwd.abort(id).expect("abort"), AbortResult::Aborted);
    // The record is queued before abort returns (EVMOD-REC).
    let record = fwd.take_end().expect("take_end").expect("queued before abort returned");
    assert_eq!(
        (record.id, record.outcome),
        (id, ForwardOutcome::Aborted { cause: AbortCause::Detached })
    );
    assert_eq!(live(&fwd), (0, 0, 0));
    // Nothing was sent on the vsock: the guest side sees no end.
    v_far.set_read_timeout(Duration::from_millis(300));
    let mut buf = [0u8; 1];
    match (&mut &v_far).read(&mut buf) {
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
        other => panic!("a watch abort must send nothing to the guest, got {other:?}"),
    }
    // The holder's socket is as it was, both ways.
    (&mut &v).write_all(b"ping").expect("holder writes");
    assert_eq!(read_exact_n(&v_far, 4), b"ping");
    (&mut &v_far).write_all(b"pong").expect("guest writes");
    assert_eq!(read_exact_n(&v, 4), b"pong");
    // The socket is unhooked: it can be watched again.
    let again = fwd.watch(&v as &dyn AsFd).expect("re-watch after abort");
    assert!(again.get() > id.get());
    assert_eq!(fwd.abort(id).expect("abort a stale id"), AbortResult::AlreadyEnded);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn abort_all_records_every_watch_and_pair_and_returns_their_count() {
    let fwd = open_context();
    let (v1, _f1) = vsock_pair();
    let (v2, _f2) = vsock_pair();
    let (v3, _f3) = vsock_pair();
    let (t3, _t3_far) = super::support::tcp_pair();
    let w1 = fwd.watch(&v1 as &dyn AsFd).expect("watch");
    let w2 = fwd.watch(&v2 as &dyn AsFd).expect("watch");
    let p3 = super::support::attach_stream(&fwd, &v3, &t3);
    assert_eq!(fwd.abort_all().expect("abort_all"), 3);
    let mut ids = Vec::new();
    while let Some(end) = fwd.take_end().expect("take_end") {
        assert_eq!(end.outcome, ForwardOutcome::Aborted { cause: AbortCause::Detached });
        ids.push(end.id);
    }
    ids.sort();
    assert_eq!(ids, vec![w1, w2, p3], "every record queued before abort_all returned, once each");
    assert_eq!(live(&fwd), (0, 0, 0));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn a_context_release_shuts_every_watched_socket_and_queues_no_record() {
    let guest_view = open_context();
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let guest_watch = guest_view.watch(&v_far as &dyn AsFd).expect("the guest side's classifier");
    let _id = fwd.watch(&v as &dyn AsFd).expect("watch");
    drop(fwd);
    // The release shut V_h both ways: the guest side classifies an abort.
    assert!(matches!(
        wait_end_of(&guest_view, guest_watch).outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { .. } }
    ));
    // The holder's read returns 0 or fails ENOTCONN (af_vsock.c:1220,
    // :2522-2531 @ v7.0; F11), and its write fails EPIPE.
    v.set_read_timeout(END_WAIT);
    let mut buf = [0u8; 8];
    match (&mut &v).read(&mut buf) {
        Ok(0) => {}
        Err(e) if e.kind() == ErrorKind::NotConnected => {}
        other => panic!("after a release the holder reads 0 or ENOTCONN, got {other:?}"),
    }
    match (&mut &v).write(b"x") {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {}
        other => panic!("after a release the holder's write fails EPIPE, got {other:?}"),
    }
}
