//! Density and fail-closed cells of the vsock attachment (GH #295, P6-S15;
//! Roadmap impact 08-04, the E18 replacement by V-9 through `serve` +
//! `deploy`; R5-25, R5-55 (a), (d), (e)), on native metal.
//!
//! These are rerunnable correctness contracts at density, not the V-9
//! measurement: V-9's per-attachment costs, latencies and bounds at 1 / 64 /
//! 1,024 / 4,096 / 16,384 attachments are a benchmark with its own
//! methodology and baseline (`.claude/rules/testing.md` § "Classify external
//! execution before writing it"); its numbers are not asserted here. What is
//! asserted at [`ATTACHMENTS`] VMs on one node:
//!
//! - every VM's guest application completes its own TCP and UDP round trip
//!   through its own attachment, exactly once, from the node's address, and
//!   every VM holds its own workload address (R5-55 (a): unique leases);
//! - with every VM running, `serve` dies: while it is down no connection to
//!   any VM's address reaches a host socket (the constant guest-prefix rules
//!   and the absent routes stay; R5-25), and the next boot ends every VMM it
//!   does not supervise before it serves, then places every persisted Job
//!   again on the reclaimed CIDs (NO-SURVIVE, R5-55 (d), (e)).
//!
//! **The fail-closed oracle.** A host socket that *could* be reached is
//! bound first: a wildcard listener on [`PROBE_PORT`]. Its control is a
//! connect to the node's own loopback address, which it accepts, so the
//! listener is live and an empty accept queue later is evidence. A connect
//! to a VM's workload address on that port must then fail with the error the
//! constant rules give a host-local connect — `ECONNREFUSED` from the output
//! reject, or `ENETUNREACH` where no route covers the prefix (V-19) — never
//! succeed, never time out (a SYN that left the host), and the listener must
//! accept nothing. The same dial is made with `serve` up (the production
//! rules) and down (the rules `serve` left behind), so a down-only artefact
//! cannot pass it. A connect that *succeeds* to a VM address is not a
//! control this cell can use: there is no host-local delivery to the guest
//! prefix with `serve` up either (INB-1, ADR-0173) — a host client reaches a
//! VM port only through the owner's in-process `GuestStreamDial`, and a
//! remote one only through leg C's mTLS (R5-54).
//!
//! Bounds: [`ATTACHMENTS`] VMs of 128 MiB guest RAM each; every case
//! stops its Jobs and shuts `serve` down. Lane: `integration-tests,kvm-tests`,
//! `cargo xtask metal run --`; module in `host-kernel-shared`. Pending
//! DELIVER step 14-05.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::doc_markdown, reason = "kernel and protocol names in prose")]

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Duration;

use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::id::AllocationId;
use overdrive_testing::guest_app::{RemotePeer, subcommand};
use serial_test::serial;

use super::vm_walking_skeleton::TeardownBound;
use super::vsock_support::{Lane, alloc_of, arg, hypervisor_of, process_alive, vsock_cid};

/// VMs per node in these cells (V-9's smallest density point above one).
const ATTACHMENTS: usize = 64;

/// The port of the fail-closed cell's dials and of its host wildcard
/// listener (the host socket no dial may reach).
const PROBE_PORT: u16 = 7_777;

/// How long one fail-closed dial may take. The constant rules refuse a
/// host-local connect at once; a dial that runs this long sent a SYN that
/// nothing answered, which is a failure of the cell.
const DIAL_BOUND: Duration = Duration::from_secs(2);

fn tag(n: usize) -> String {
    format!("density-{n:02}")
}

/// Every Job's first allocation, once all of them satisfy `done`.
async fn wait_all(
    lane: &Lane,
    ids: &[String],
    bound: Duration,
    what: &str,
    done: impl Fn(AllocStateWire) -> bool + Copy + Send + Sync,
) -> BTreeMap<String, (AllocationId, Option<std::net::Ipv4Addr>)> {
    let mut out = BTreeMap::new();
    for id in ids {
        let rows = lane
            .wait_rows(id, bound, what, |rows| rows.first().is_some_and(|r| done(r.state)))
            .await;
        let row = rows.first().expect("one allocation");
        out.insert(id.clone(), (alloc_of(row), row.workload_addr));
    }
    out
}

/// The host wildcard listener on [`PROBE_PORT`], proven live: a connect to
/// the node's loopback address on that port is accepted (the control).
fn live_host_listener() -> TcpListener {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PROBE_PORT))
        .expect("precondition: the host wildcard probe listener binds");
    let control = TcpStream::connect_timeout(
        &SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, PROBE_PORT)),
        DIAL_BOUND,
    )
    .expect("control: a host-local connect to a node address reaches the listener");
    let (accepted, _) = listener.accept().expect("control: the listener accepts the control dial");
    drop((control, accepted));
    listener.set_nonblocking(true).expect("precondition: a non-blocking accept queue read");
    listener
}

/// Asserts that a dial of `id`'s workload address on [`PROBE_PORT`] is
/// refused as the constant rules refuse it and reaches no host socket.
fn assert_dial_reaches_no_host_socket(
    listener: &TcpListener,
    id: &str,
    addr: Ipv4Addr,
    when: &str,
) {
    let target = SocketAddr::V4(SocketAddrV4::new(addr, PROBE_PORT));
    match TcpStream::connect_timeout(&target, DIAL_BOUND) {
        Ok(stream) => panic!(
            "{id} ({when}): a connect to {target} succeeded (local {:?}); a guest-prefix \
             address other than the gateway is not host-local",
            stream.local_addr()
        ),
        Err(error) => assert!(
            matches!(error.raw_os_error(), Some(libc::ECONNREFUSED | libc::ENETUNREACH)),
            "{id} ({when}): a connect to {target} must be refused by the constant rules \
             (ECONNREFUSED) or find no route (ENETUNREACH), got {error:?} (kind {:?})",
            error.kind()
        ),
    }
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        Ok((_, peer)) => {
            panic!("{id} ({when}): the dial of {target} reached the host listener from {peer}")
        }
        Err(error) => panic!("{id} ({when}): reading the probe listener's queue: {error}"),
    }
}

/// R5-55 (a) at density: 64 VMs each complete their own TCP and UDP round
/// trip through their own attachment; the remote peer serves each tag once,
/// from the node's address; every VM holds its own workload address.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-05"]
async fn every_vm_at_density_round_trips_through_its_own_attachment() {
    let _bound = TeardownBound::arm();
    let peer = RemotePeer::start("vsock-density", 6).expect("precondition: the remote peer starts");
    let lane = Lane::boot("vsock-density").await;
    let (tcp, udp) = (arg(peer.tcp_addr()), arg(peer.udp_addr()));
    let mut ids = Vec::with_capacity(ATTACHMENTS);
    for n in 0..ATTACHMENTS {
        let t = tag(n);
        ids.push(lane.deploy_job(&t, &[subcommand::ROUND_TRIP, &tcp, &udp, &t]).await);
    }

    let ended = wait_all(&lane, &ids, Duration::from_secs(600), "a terminal allocation", |s| {
        matches!(s, AllocStateWire::Terminated | AllocStateWire::Failed)
    })
    .await;
    for id in &ids {
        let rows = lane.rows(id).await;
        assert_eq!(
            rows.first().map(|r| r.state),
            Some(AllocStateWire::Terminated),
            "{id}: its guest application completed both round trips; row {:?}",
            rows.first()
        );
    }
    let addresses: BTreeSet<_> = ended.values().filter_map(|(_, addr)| *addr).collect();
    assert_eq!(addresses.len(), ATTACHMENTS, "every VM holds its own workload address");

    let log = peer.snapshot();
    let tcp_tags: BTreeSet<_> = log.requests.iter().map(|r| r.tag.clone()).collect();
    let udp_tags: BTreeSet<_> = log.datagrams.iter().map(|d| d.tag.clone()).collect();
    let expected: BTreeSet<_> = (0..ATTACHMENTS).map(tag).collect();
    assert_eq!(log.requests.len(), ATTACHMENTS, "each TCP request served once");
    assert_eq!(tcp_tags, expected, "every VM's request, none crossed");
    assert_eq!(log.datagrams.len(), ATTACHMENTS, "each datagram served once");
    assert_eq!(udp_tags, expected, "every VM's datagram, none crossed");
    assert!(
        log.requests.iter().all(|r| *r.from.ip() == peer.node_side_addr())
            && log.datagrams.iter().all(|d| *d.from.ip() == peer.node_side_addr()),
        "the node opened every flow"
    );

    lane.shutdown().await;
}

/// R5-25 and R5-55 (d), (e) at density: with 64 VMs running, `serve` dies.
/// With `serve` up and while it is down, a host-local connection to any VM's
/// address is refused by the constant rules and reaches no host socket — a
/// live wildcard listener on the dialled port accepts nothing.
/// The next boot ends every VMM it does not supervise before serving, then
/// places every persisted Job again, on the reclaimed CIDs.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 14-05"]
async fn serve_down_at_density_is_fail_closed_and_the_next_boot_reclaims_every_vm() {
    let _bound = TeardownBound::arm();
    let mut lane = Lane::boot("vsock-density-down").await;
    let mut ids = Vec::with_capacity(ATTACHMENTS);
    for n in 0..ATTACHMENTS {
        ids.push(lane.deploy_job(&tag(n), &[subcommand::IDLE]).await);
    }
    let running = wait_all(&lane, &ids, Duration::from_secs(600), "Running", |s| {
        s == AllocStateWire::Running
    })
    .await;
    let mut vmms = BTreeMap::new();
    for (id, (alloc, _)) in &running {
        let (pid, argv) = hypervisor_of(alloc).unwrap_or_else(|| panic!("{id}: its VMM runs"));
        vmms.insert(id.clone(), (pid, vsock_cid(&argv)));
    }
    let cids: BTreeSet<u32> = vmms.values().map(|(_, cid)| *cid).collect();
    assert_eq!(cids.len(), ATTACHMENTS, "every VM holds its own CID");

    let listener = live_host_listener();
    let addresses: BTreeMap<_, _> = running
        .iter()
        .map(|(id, (_, addr))| {
            (id.clone(), addr.unwrap_or_else(|| panic!("{id}: a workload address")))
        })
        .collect();
    for (id, addr) in &addresses {
        assert_dial_reaches_no_host_socket(&listener, id, *addr, "serve up");
    }

    lane.kill_serve().await;

    for (id, addr) in &addresses {
        assert_dial_reaches_no_host_socket(&listener, id, *addr, "serve down");
    }
    drop(listener);

    lane.reboot_serve().await;

    for (id, (pid, _)) in &vmms {
        assert!(!process_alive(*pid), "{id}: the boot ended the VMM it does not supervise");
    }
    let replaced = {
        let mut out = BTreeMap::new();
        for (id, (old, _)) in &running {
            let rows = lane
                .wait_rows(id, Duration::from_secs(600), "a new Running allocation", |rows| {
                    rows.iter().any(|r| r.state == AllocStateWire::Running && alloc_of(r) != *old)
                })
                .await;
            let next = rows
                .iter()
                .find(|r| r.state == AllocStateWire::Running && alloc_of(r) != *old)
                .map(alloc_of)
                .expect("the re-placed allocation");
            out.insert(id.clone(), next);
        }
        out
    };
    let new_cids: BTreeSet<u32> = replaced
        .values()
        .map(|alloc| vsock_cid(&hypervisor_of(alloc).expect("the re-placed VMM runs").1))
        .collect();
    assert_eq!(new_cids, cids, "the next boot places every Job on the reclaimed CIDs");

    for id in &ids {
        lane.stop(id).await;
    }
    lane.shutdown().await;
}
