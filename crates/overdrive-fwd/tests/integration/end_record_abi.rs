//! The end-record ABI against the built module (EVMOD-REC, ADR-0202;
//! § *Control interface*, "End record" and the `read()` / `poll()` rows):
//! the Tier-3 round trip of the `#[repr(C)]` mirror's layout. The context
//! descriptor is read directly through the adapter's pinned
//! `readiness_fd()`, so the bytes checked are the module's own, not the
//! adapter's decode of them.
//!
//! Pinned facts: 32-byte records in native byte order, oldest first, whole
//! records only; a buffer shorter than one record is `EINVAL`; `EAGAIN`
//! when none is queued (the context is `O_NONBLOCK`); `EPOLLIN` while a
//! record is queued.

use std::net::TcpStream;
use std::os::fd::AsRawFd;

use nix::libc;
use overdrive_core::guest_transport::forwarder::{FlowForwarder, ForwardId};
use overdrive_fwd::KernelFlowForwarder;

use super::support::{Vsock, attach_stream, live, open_context, tcp_pair, vsock_pair};

const RECORD: usize = 32;

/// `read(2)` on the context descriptor.
fn read_context(fwd: &KernelFlowForwarder, buf: &mut [u8]) -> Result<usize, i32> {
    // SAFETY: `buf` is a valid writable buffer of `buf.len()` bytes.
    let n =
        unsafe { libc::read(fwd.readiness_fd().as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(std::io::Error::last_os_error().raw_os_error().expect("errno"))
    } else {
        Ok(n as usize)
    }
}

/// Whether the context descriptor polls readable now.
fn readable(fwd: &KernelFlowForwarder) -> bool {
    let mut pfd =
        libc::pollfd { fd: fwd.readiness_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, zero timeout.
    let ret = unsafe { libc::poll(&raw mut pfd, 1, 0) };
    assert!(ret >= 0, "poll: {}", std::io::Error::last_os_error());
    pfd.revents & libc::POLLIN != 0
}

/// The sockets of one live pair, held so the pair stays alive.
type HeldPair = (Vsock, Vsock, TcpStream, TcpStream);

/// Three live pairs, aborted together: three `Detached` records queued.
fn three_detached_records(fwd: &KernelFlowForwarder) -> (Vec<ForwardId>, Vec<HeldPair>) {
    let mut ids = Vec::new();
    let mut held = Vec::new();
    for _ in 0..3 {
        let (v, v_far) = vsock_pair();
        let (t, t_far) = tcp_pair();
        ids.push(attach_stream(fwd, &v, &t));
        held.push((v, v_far, t, t_far));
    }
    assert_eq!(
        fwd.abort_all().expect("abort_all"),
        3,
        "abort_all returns once every record is queued"
    );
    (ids, held)
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn the_module_writes_whole_32_byte_records_at_the_pinned_offsets() {
    let fwd = open_context();
    assert!(!readable(&fwd), "a fresh context has nothing queued");
    let (ids, _held) = three_detached_records(&fwd);
    assert!(readable(&fwd), "EPOLLIN while a record is queued");

    // A buffer shorter than one record is refused, and consumes nothing.
    let mut short = [0u8; RECORD - 1];
    assert_eq!(read_context(&fwd, &mut short), Err(libc::EINVAL));
    assert_eq!(live(&fwd).2, 3, "a refused read consumes no record");

    // 100 bytes hold three whole records: 96 are returned.
    let mut buf = [0u8; 100];
    assert_eq!(read_context(&fwd, &mut buf), Ok(3 * RECORD));
    let mut seen = Vec::new();
    for rec in buf[..3 * RECORD].chunks_exact(RECORD) {
        let id = u64::from_ne_bytes(rec[0..8].try_into().expect("8 bytes"));
        assert_eq!(rec[8], 2, "kind: pair aborted");
        assert_eq!(rec[9], 4, "cause: detached");
        assert_eq!(rec[10], 0, "side: 0 for cause 4");
        assert_eq!(rec[11], 0, "reserved");
        assert_eq!(u16::from_ne_bytes([rec[12], rec[13]]), 0, "errno: 0 outside cause 2");
        assert_eq!(u16::from_ne_bytes([rec[14], rec[15]]), 0, "reserved");
        assert_eq!(u64::from_ne_bytes(rec[16..24].try_into().expect("8 bytes")), 0, "moved_ab");
        assert_eq!(u64::from_ne_bytes(rec[24..32].try_into().expect("8 bytes")), 0, "moved_ba");
        seen.push(id);
    }
    seen.sort_unstable();
    let mut want: Vec<u64> = ids.iter().copied().map(ForwardId::get).collect();
    want.sort_unstable();
    assert_eq!(seen, want, "one record per id, none lost");

    // Nothing left: EAGAIN on the non-blocking context, no EPOLLIN.
    assert_eq!(read_context(&fwd, &mut buf), Err(libc::EAGAIN));
    assert!(!readable(&fwd));
    assert_eq!(fwd.take_end().expect("take_end"), None);
    assert_eq!(live(&fwd), (0, 0, 0));
}

#[test]
#[ignore = "pending DELIVER step 4"]
fn the_moved_counts_of_a_record_are_the_bytes_each_target_accepted() {
    use std::io::Write;

    use overdrive_core::guest_transport::forwarder::ForwardOutcome;

    let fwd = open_context();
    let (v, v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    let id = attach_stream(&fwd, &v, &t);
    (&v_far).write_all(&[0xab; 1000]).expect("A→B");
    v_far.shutdown(libc::SHUT_WR);
    (&t_far).write_all(&[0xcd; 300]).expect("B→A");
    t_far.shutdown(std::net::Shutdown::Write).expect("FIN");
    let end = super::support::wait_end_of(&fwd, id);
    assert_eq!(end.outcome, ForwardOutcome::Completed);
    assert_eq!((end.moved_ab, end.moved_ba), (1000, 300), "moved_ab is half A → half B");
    drop((v, v_far, t, t_far));
}

/// How long a pair's end may take to queue its record.
const QUEUE_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

/// Polls the inventory (never `take_end`) until `n` records are queued.
fn wait_queued(fwd: &KernelFlowForwarder, n: u32) {
    let start = std::time::Instant::now();
    while live(fwd).2 != n {
        assert!(
            start.elapsed() < QUEUE_BOUND,
            "{n} queued end records within {QUEUE_BOUND:?}; inventory {:?}",
            fwd.inventory()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(readable(fwd), "EPOLLIN while {n} records are queued");
}

/// How a cell ends one pair.
#[derive(Debug, Clone, Copy)]
enum EndBy {
    /// `abort(id)`: its `Detached` record is queued when the call returns.
    Abort,
    /// Both far ends `SHUT_WR`: the pair completes on the module's
    /// completion path (EVMOD-WQ), off the caller's thread.
    CleanClose,
}

fn end_pair(fwd: &KernelFlowForwarder, id: ForwardId, held: &HeldPair, by: EndBy) {
    use overdrive_core::guest_transport::forwarder::AbortResult;
    match by {
        EndBy::Abort => {
            assert_eq!(fwd.abort(id).expect("abort"), AbortResult::Aborted, "{id:?} was live");
        }
        EndBy::CleanClose => {
            let (_, v_far, _, t_far) = held;
            v_far.shutdown(libc::SHUT_WR);
            t_far.shutdown(std::net::Shutdown::Write).expect("FIN from the TCP far end");
        }
    }
}

/// D-12 (EVMOD-REC, the `FlowForwarder` contract: `take_end` returns the
/// oldest queued record) — the Tier-3 pair of the `flow_quota` driver's
/// `endq` projection. Pair A is ended and its record observed queued (the
/// inventory's `queued_ends` = 1 and `EPOLLIN`) before pair B is ended, so
/// the queue order is fixed by an explicit sequence, not by the per-CPU
/// completion order of concurrent ends; then two `take_end` calls return A's
/// record, then B's — compared in order, never sorted (the cell above sorts
/// and is order-blind). Both end paths are exercised: `abort` and a clean
/// close completed by the module.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the context's queued records,
/// in read order; expected: `[A, B]`, then none.
#[test]
#[ignore = "pending DELIVER step 4"]
fn take_end_returns_records_in_end_order() {
    for by in [EndBy::Abort, EndBy::CleanClose] {
        let fwd = open_context();
        let mut pairs = Vec::new();
        for _ in 0..2 {
            let (v, v_far) = vsock_pair();
            let (t, t_far) = tcp_pair();
            let id = attach_stream(&fwd, &v, &t);
            pairs.push((id, (v, v_far, t, t_far)));
        }
        let (a, held_a) = &pairs[0];
        let (b, held_b) = &pairs[1];
        assert_eq!(live(&fwd), (2, 0, 0), "{by:?}: two live pairs, nothing queued");

        end_pair(&fwd, *a, held_a, by);
        wait_queued(&fwd, 1);
        assert_eq!(live(&fwd).0, 1, "{by:?}: B is still live while A's record is queued");
        end_pair(&fwd, *b, held_b, by);
        wait_queued(&fwd, 2);

        let first = fwd.take_end().expect("take_end").expect("A's record");
        let second = fwd.take_end().expect("take_end").expect("B's record");
        assert_eq!(
            [first.id, second.id],
            [*a, *b],
            "{by:?}: take_end returns the records in the order the pairs ended"
        );
        assert_eq!(fwd.take_end().expect("take_end"), None, "{by:?}: nothing else queued");
        assert_eq!(live(&fwd), (0, 0, 0), "{by:?}: the context is empty");
    }
}
