//! The stream-mode battery (§ *Source, build and test*, Tier-3: stall,
//! long, slow, cycles, burst, bidir, eof, tiny, cut, rst, kill, tight
//! buffers), run against `ovd_fwd` through `KernelFlowForwarder`.
//!
//! Every pair is the host's TcpConnect shape: half A an accepted vsock
//! (`V_h`), half B a TCP socket (`D`). The vsock far end is watched in a
//! second context, so a reset that reaches it is observed as the guest-side
//! classification (`Aborted`), never as a plain socket's end-of-stream.
//! Payloads are position-dependent (`pattern`), so a lost, duplicated or
//! reordered chunk fails the byte-exact comparison.

use std::io::{ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardOutcome, PairSide,
};
use overdrive_core::guest_transport::wire::KernelErrno;

use super::support::{
    END_WAIT, FarEnd, StreamFlow, assert_far_tcp_reset, attach_stream, live, open_context, pattern,
    read_until_end, send_tcp, send_vsock, set_tcp_buffers, tcp_pair, vsock_pair, wait_end_of,
};

/// Both directions: `ab` vsock far → TCP far, `ba` TCP far → vsock far,
/// each half-closed after its last byte; asserts byte-exact delivery, a
/// clean end at both far ends, and `Completed` with the moved counts.
fn clean_both_ways(flow: StreamFlow, ab: &[u8], ba: &[u8]) {
    thread::scope(|s| {
        s.spawn(|| send_vsock(&flow.v_far, ab));
        s.spawn(|| send_tcp(&flow.t_far, ba));
        let at_tcp = s.spawn(|| read_until_end(&flow.t_far));
        let at_vsock = s.spawn(|| read_until_end(&flow.v_far));
        let (got_ab, end_ab) = at_tcp.join().expect("reader");
        let (got_ba, end_ba) = at_vsock.join().expect("reader");
        assert_eq!(end_ab, FarEnd::Eof, "the TCP far end reads a clean end");
        assert_eq!(end_ba, FarEnd::Eof, "the vsock far end reads a clean end");
        assert!(got_ab == ab, "A→B byte-exact: {} of {} bytes", got_ab.len(), ab.len());
        assert!(got_ba == ba, "B→A byte-exact: {} of {} bytes", got_ba.len(), ba.len());
    });
    let end = flow.end();
    assert_eq!(end.outcome, ForwardOutcome::Completed);
    assert_eq!((end.moved_ab, end.moved_ba), (ab.len() as u64, ba.len() as u64));
    assert_eq!(
        flow.far_end(),
        ForwardOutcome::GuestHalfClose,
        "the guest side saw a clean half-close"
    );
    flow.release();
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn eof_a_flow_that_carries_no_byte_completes_cleanly() {
    clean_both_ways(StreamFlow::new(), &[], &[]);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn tiny_one_byte_each_way_completes_cleanly() {
    clean_both_ways(StreamFlow::new(), b"a", b"b");
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn bidir_64_mib_each_way_is_byte_exact() {
    clean_both_ways(StreamFlow::new(), &pattern(1, 64 << 20), &pattern(2, 64 << 20));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn long_512_mib_in_one_direction_is_byte_exact() {
    clean_both_ways(StreamFlow::new(), &pattern(3, 512 << 20), &[]);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn tight_buffers_of_64_kib_are_byte_exact_both_ways() {
    let flow = StreamFlow::with(|v, v_far, t, t_far| {
        v.set_buffer_size(64 * 1024);
        v_far.set_buffer_size(64 * 1024);
        set_tcp_buffers(t, 64 * 1024);
        set_tcp_buffers(t_far, 64 * 1024);
    });
    clean_both_ways(flow, &pattern(4, 32 << 20), &pattern(5, 32 << 20));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn burst_ten_thousand_small_writes_arrive_in_order() {
    let flow = StreamFlow::new();
    let data = pattern(6, 10_000 * 33);
    thread::scope(|s| {
        s.spawn(|| {
            let mut far = &flow.v_far;
            let mut at = 0usize;
            let mut size = 1usize;
            while at < data.len() {
                let take = size.min(data.len() - at);
                far.write_all(&data[at..at + take]).expect("small write");
                at += take;
                size = size % 64 + 1;
            }
            flow.v_far.shutdown(libc::SHUT_WR);
        });
        let (got, end) = read_until_end(&flow.t_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(got == data, "burst byte-exact: {} of {}", got.len(), data.len());
    });
    flow.t_far.shutdown(Shutdown::Write).expect("FIN");
    assert_eq!(flow.end().outcome, ForwardOutcome::Completed);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn slow_a_reader_taking_4_kib_per_millisecond_receives_every_byte() {
    let flow = StreamFlow::new();
    let data = pattern(7, 8 << 20);
    thread::scope(|s| {
        s.spawn(|| send_vsock(&flow.v_far, &data));
        let mut got = Vec::with_capacity(data.len());
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut &flow.t_far, &mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) => panic!("slow reader: {e}"),
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(got == data, "slow reader byte-exact: {} of {}", got.len(), data.len());
    });
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn stall_a_reader_that_stops_blocks_the_sender_and_resumes_byte_exact() {
    let flow = StreamFlow::new();
    let data = pattern(8, 64 << 20);
    // The sender writes without blocking until the forwarder stops taking
    // bytes: the source queue holds the rest, nothing grows past it.
    flow.v_far.set_nonblocking(true);
    let mut sent = 0usize;
    let blocked_by = Instant::now() + END_WAIT;
    loop {
        match std::io::Write::write(&mut &flow.v_far, &data[sent..]) {
            Ok(n) => sent += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => panic!("sender: {e}"),
        }
        assert!(Instant::now() < blocked_by, "the sender never blocked against a stopped reader");
        assert!(sent < data.len(), "64 MiB were accepted with no reader: the stall is unbounded");
    }
    thread::sleep(Duration::from_secs(2));
    let state = flow.fwd.pair_state(flow.id).expect("pair_state").expect("live");
    assert_eq!(state.a_to_b.source_end, None);
    assert!(!state.a_to_b.target_shut);
    // Resume: the reader drains, the sender finishes.
    flow.v_far.set_nonblocking(false);
    thread::scope(|s| {
        s.spawn(|| send_vsock(&flow.v_far, &data[sent..]));
        let (got, end) = read_until_end(&flow.t_far);
        assert_eq!(end, FarEnd::Eof);
        assert!(
            got == data,
            "stalled flow byte-exact after resume: {} of {}",
            got.len(),
            data.len()
        );
    });
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn cycles_a_thousand_flows_each_end_with_one_record_and_leave_nothing_live() {
    let fwd = open_context();
    let mut last = 0u64;
    for round in 0..1_000u64 {
        let (v, v_far) = vsock_pair();
        let (t, t_far) = tcp_pair();
        let id = attach_stream(&fwd, &v, &t);
        assert!(id.get() > last, "ids strictly increase");
        last = id.get();
        let data = pattern(round, 1024);
        send_vsock(&v_far, &data);
        send_tcp(&t_far, &data);
        v_far.set_read_timeout(END_WAIT);
        t_far.set_read_timeout(Some(END_WAIT)).expect("timeout");
        assert_eq!(read_until_end(&t_far), (data.clone(), FarEnd::Eof));
        assert_eq!(read_until_end(&v_far), (data, FarEnd::Eof));
        assert_eq!(wait_end_of(&fwd, id).outcome, ForwardOutcome::Completed);
        assert_eq!(live(&fwd), (0, 0, 0), "round {round}: nothing live after the record");
    }
    assert_eq!(fwd.inventory().expect("inventory").completed, 1_000);
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn cut_a_tcp_far_end_closed_with_unread_bytes_aborts_and_resets_the_guest_side() {
    let flow = StreamFlow::new();
    (&mut &flow.v_far).write_all(&pattern(9, 1 << 20)).expect("bytes the far reader never reads");
    thread::sleep(Duration::from_millis(200));
    // close() with unread data sends a reset (net/ipv4/tcp.c, tcp_close).
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    drop(t_far);
    let end = wait_end_of(&fwd, id);
    assert!(
        matches!(
            end.outcome,
            ForwardOutcome::Aborted { cause: AbortCause::SocketError { side: PairSide::B, .. } }
        ),
        "a TCP reset aborts the pair SocketError on half B, got {end:?}"
    );
    assert!(
        matches!(wait_end_of(&peer, far_watch).outcome, ForwardOutcome::Aborted { .. }),
        "the guest side classifies an abort, never a clean end"
    );
    drop((v, v_far, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn rst_a_tcp_far_end_reset_while_idle_aborts_with_econnreset() {
    let flow = StreamFlow::new();
    let StreamFlow { fwd, peer, id, far_watch, v, v_far, t, t_far } = flow;
    super::support::reset_close(t_far);
    let end = wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: PairSide::B, errno: KernelErrno::ECONNRESET }
        }
    );
    assert!(matches!(wait_end_of(&peer, far_watch).outcome, ForwardOutcome::Aborted { .. }));
    drop((v, v_far, t));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn kill_the_process_holding_the_vsock_far_end_aborts_and_resets_the_tcp_far_end() {
    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    let id = attach_stream(&fwd, &v, &t);
    (&mut &t_far).write_all(&pattern(10, 256 << 10)).expect("bytes toward the guest");
    // The child holds the only other reference to the far end as fd 3.
    let raw = v_far.raw();
    // SAFETY: dup2 in the forked child before exec, async-signal-safe.
    let mut child = unsafe {
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .pre_exec(move || {
                if libc::dup2(raw, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            })
            .spawn()
            .expect("spawn the far-end holder")
    };
    drop(v_far);
    child.kill().expect("kill -9 the holder");
    child.wait().expect("reap");
    let end = wait_end_of(&fwd, id);
    assert_eq!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert_far_tcp_reset(&t_far);
    drop((v, t));
}
