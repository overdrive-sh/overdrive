//! Context release (R5-61; EVMOD-CTL, ADR-0199; EVMOD-REC, ADR-0202;
//! EVMOD-LIFE, ADR-0198): a released context aborts every pair and watch it
//! created and queues no record; one record per id under a close storm; a
//! child spawned by the owner holds no context (`O_CLOEXEC`). The module's
//! unload safety (MODULE-REFCOUNT) is `unload.rs` (V-EV-UNLOAD), on the
//! metal host only.
//!
//! The `Forwarder` component recovery through the production supervisor
//! (abort, close the old context, then open the new) is the composed
//! scenario of R5-61's seeded-sim and serve-level cells, not this suite.

use std::collections::BTreeSet;
use std::io::Write;
use std::os::fd::AsFd;
use std::process::{Command, Stdio};

use nix::libc;
use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardOutcome, ForwarderInventory, PairSide,
};

use super::support::{
    Vsock, assert_far_tcp_reset, attach_stream, holds_forwarder_context, live, open_context,
    pattern, read_exact_n, reset_close, tcp_pair, vsock_pair, wait_end,
};

/// Raises the soft descriptor limit to the hard limit (a storm holds four
/// sockets per pair).
fn raise_nofile(at_least: u64) {
    // SAFETY: valid rlimit out-parameter.
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: valid rlimit out-parameter.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut lim) }, 0, "getrlimit");
    assert!(lim.rlim_max >= at_least, "precondition: RLIMIT_NOFILE hard limit ≥ {at_least}");
    lim.rlim_cur = lim.rlim_max;
    // SAFETY: valid rlimit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) }, 0, "setrlimit");
}

#[test]
fn a_released_context_aborts_every_pair_and_watch_and_resets_every_far_end() {
    raise_nofile(4096);
    let guest_view = open_context();
    let owner = open_context();
    let mut tcp_fars = Vec::new();
    let mut guest_watches = BTreeSet::new();
    let mut held = Vec::new();
    for i in 0..48u64 {
        let (v, v_far) = vsock_pair();
        let (t, t_far) = tcp_pair();
        guest_watches.insert(guest_view.watch(&v_far as &dyn AsFd).expect("guest-side watch"));
        attach_stream(&owner, &v, &t);
        if i % 2 == 0 {
            // Half the pairs are stalled with bytes queued toward a TCP
            // reader that never reads (the 8 MiB shape of P-70).
            v_far.set_nonblocking(true);
            let data = pattern(i, 8 << 20);
            let mut sent = 0;
            while let Ok(n) = (&mut &v_far).write(&data[sent..]) {
                sent += n;
            }
        }
        tcp_fars.push(t_far);
        held.push((v, v_far, t));
    }
    for _ in 0..16 {
        let (w, w_far) = vsock_pair();
        guest_watches.insert(guest_view.watch(&w_far as &dyn AsFd).expect("guest-side watch"));
        owner.watch(&w as &dyn AsFd).expect("owner watch");
        held.push((w, w_far, tcp_pair().0));
    }
    assert_eq!(live(&owner).0, 48);
    assert_eq!(live(&owner).1, 16);

    drop(owner);

    // The release completed before close() returned (fs/file_table.c:528-530
    // @ v7.0): every TCP far end is reset now.
    for t_far in &tcp_fars {
        assert_far_tcp_reset(t_far);
    }
    // Every vsock far end — pairs' and watches' — classifies an abort.
    let mut seen = BTreeSet::new();
    for _ in 0..guest_watches.len() {
        let end = wait_end(&guest_view);
        assert!(
            matches!(end.outcome, ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { .. } }),
            "a released context's vsock half reaches its peer as an abort, got {end:?}"
        );
        assert!(seen.insert(end.id), "one record per guest-side watch");
    }
    assert_eq!(seen, guest_watches);
    // A new context starts with nothing of the old one's.
    let next = open_context();
    assert_eq!(next.inventory().expect("inventory"), ForwarderInventory::default());
    drop(held);
}

#[test]
fn one_context_release_leaves_another_contexts_pairs_forwarding() {
    let kept = open_context();
    let released = open_context();
    let (v1, v1_far) = vsock_pair();
    let (t1, t1_far) = tcp_pair();
    let kept_id = attach_stream(&kept, &v1, &t1);
    let (v2, _v2_far) = vsock_pair();
    let (t2, t2_far) = tcp_pair();
    attach_stream(&released, &v2, &t2);
    drop(released);
    assert_far_tcp_reset(&t2_far);
    (&mut &v1_far).write_all(b"still-forwarding").expect("write");
    assert_eq!(read_exact_n(&t1_far, 16), b"still-forwarding");
    assert!(kept.pair_state(kept_id).expect("pair_state").is_some());
    assert_eq!(kept.take_end().expect("take_end"), None);
}

#[test]
fn a_thousand_pair_close_storm_yields_exactly_one_record_per_id() {
    raise_nofile(8192);
    let fwd = open_context();
    let mut ids = BTreeSet::new();
    let mut fars = Vec::new();
    let mut held: Vec<(Vsock, Vsock, std::net::TcpStream)> = Vec::new();
    for _ in 0..1_000 {
        let (v, v_far) = vsock_pair();
        let (t, t_far) = tcp_pair();
        assert!(ids.insert(attach_stream(&fwd, &v, &t)), "ids are never reused");
        fars.push(t_far);
        held.push((v, v_far, t));
    }
    // Every destination resets at once.
    for t_far in fars {
        reset_close(t_far);
    }
    let mut seen = BTreeSet::new();
    for _ in 0..ids.len() {
        let end = wait_end(&fwd);
        assert!(
            matches!(
                end.outcome,
                ForwardOutcome::Aborted {
                    cause: AbortCause::SocketError { side: PairSide::B, .. }
                }
            ),
            "got {end:?}"
        );
        assert!(seen.insert(end.id), "duplicate record for {:?}", end.id);
    }
    assert_eq!(seen, ids, "no record lost");
    assert_eq!(fwd.take_end().expect("take_end"), None, "no record beyond one per id");
    let inv = fwd.inventory().expect("inventory");
    assert_eq!((inv.live_pairs, inv.queued_ends, inv.attached, inv.aborted), (0, 0, 1_000, 1_000));
    drop(held);
}

#[test]
fn a_child_spawned_while_the_context_is_open_keeps_nothing_alive() {
    // O_CLOEXEC (EV-CTL pin 4.8): `serve` spawns the VMMs; no child may hold
    // the context past the owner.
    let fwd = open_context();
    let (v, _v_far) = vsock_pair();
    let (t, t_far) = tcp_pair();
    attach_stream(&fwd, &v, &t);
    let mut child = Command::new("sleep")
        .arg("600")
        .stdin(Stdio::null())
        .spawn()
        .expect("spawn a stand-in VMM");
    assert!(!holds_forwarder_context(child.id()), "the child inherited the forwarder context");
    drop(fwd);
    assert_far_tcp_reset(&t_far);
    assert!(child.try_wait().expect("try_wait").is_none(), "the child is still running");
    child.kill().expect("kill");
    child.wait().expect("reap");
    drop((v, t));
}
