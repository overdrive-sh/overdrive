//! R5-59 (a), host flow seam: accept-path adapter equivalence of
//! `HostGuestFlowIo` and `SimGuestFlowIo` (D-UP-19; PD-5, approved
//! 2026-10-14) — the named Tier-3 pair of the sim's accept arbiters
//! (`queue_connection`, `release_accept`, `socket_observation`) that the
//! `owner_flows` and `udp_slots` conformance drivers read (QPROJ-7).
//!
//! **Oracle**, per connection and order-insensitive: no connection is
//! observable through the adapter before `accept` returns it — every
//! connection the guest opened is returned exactly once and no further
//! accept completes; each accepted connection carries its kernel identity
//! (listener port, the guest's real CID, its source port) and its buffered
//! first record byte-exact whatever its queue position; one with no buffered
//! record yields no record (end of stream once the guest ends it).
//!
//! **Stimulus.** A scripted guest vsock peer in a Cloud Hypervisor fork
//! microVM at a CID of the [`HOST_FLOW_ACCEPT_PATH`] band (the guest-kit
//! fixture, T-1; D-1, architect 2026-10-18) opens the connections of
//! [`CASES`] to the host's flow stream and control listeners — bound on
//! kernel-assigned ports (PORTS-CONFIG, ADR-0218: no test binds the 1240 /
//! 1243 defaults) and dialled at the ports the binds reported — from distinct
//! source ports. Its packets enter through vhost
//! (`drivers/vhost/vsock.c:555 @ v7.0`) and `virtio_transport_recv_listen`
//! (`net/vmw_vsock/virtio_transport_common.c:1557-1590,1685 @ v7.0`), so the
//! child creation and accept-queue insertion are the production path, and
//! the keys carry the guest's real CID (K-A1, V-10). The vsock loopback
//! transport cannot serve: its peers are CID 1, which is not a `GuestCid`,
//! and the adapter closes such a connection without returning it. The same
//! table is driven into `SimGuestFlowIo` with `queue_connection` /
//! `end_connection`.
//!
//! The records are opaque fixed-size byte patterns: the adapter reads
//! records, it does not parse them, and the oracle is byte equality.
//!
//! Lane: `integration-tests,kvm-tests`, native x86_64 metal (`cargo xtask
//! metal run --`); the listeners bind node-global ports, so the
//! control-plane library test binary is in `host-kernel-shared`. Pending the
//! attachment cut's step (7), which lands `HostGuestFlowIo`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "Tier-3 test code: a failed precondition panics"
)]

use std::collections::BTreeMap;
use std::net::Shutdown;
use std::time::Duration;

use overdrive_core::guest_transport::wire::{CONTROL_OPEN_LEN, FLOW_REQUEST_LEN};
use overdrive_core::guest_transport::{GuestCid, VsockListenPort};
use overdrive_core::vm::config::VsockPort;
use overdrive_testing::guest_kit::cid_bands::HOST_FLOW_ACCEPT_PATH;
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::VMADDR_CID_HOST;

use super::sim::{SimGuestFlowIo, SimPeerEnd};
use super::{FlowListener, GuestFlowIo, HostGuestFlowIo, OwnedSocket, RecordSocket};
use crate::guest_network::GuestConnKey;

/// One connection of the table: which listener, the guest's source port,
/// and whether a first record is buffered before the host accepts.
#[derive(Debug, Clone, Copy)]
struct Case {
    listener: FlowListener,
    source_port: u32,
    record: Option<u8>,
}

/// Interleaved across both listeners, records and none, so a record is
/// checked at every queue position.
const CASES: [Case; 6] = [
    Case { listener: FlowListener::Stream, source_port: 52_001, record: Some(0xa1) },
    Case { listener: FlowListener::Control, source_port: 52_002, record: None },
    Case { listener: FlowListener::Stream, source_port: 52_003, record: None },
    Case { listener: FlowListener::Control, source_port: 52_004, record: Some(0xc4) },
    Case { listener: FlowListener::Stream, source_port: 52_005, record: Some(0xa5) },
    Case { listener: FlowListener::Control, source_port: 52_006, record: Some(0xc6) },
];

/// The ports an adapter's two listeners bound.
#[derive(Debug, Clone, Copy)]
struct Bound {
    stream: VsockPort,
    control: VsockPort,
}

impl Bound {
    /// Binds both listeners of `io` on kernel-assigned ports, as boot step 9
    /// binds them under a `kernel_assigned()` configuration.
    async fn bind(io: &dyn GuestFlowIo) -> Self {
        let stream = io
            .bind_listener(FlowListener::Stream, VsockListenPort::KernelAssigned)
            .await
            .expect("bind the flow stream listener");
        let control = io
            .bind_listener(FlowListener::Control, VsockListenPort::KernelAssigned)
            .await
            .expect("bind the control listener");
        assert_ne!(stream, control, "two listeners, two ports");
        Self { stream, control }
    }

    const fn port(self, listener: FlowListener) -> VsockPort {
        match listener {
            FlowListener::Stream => self.stream,
            FlowListener::Control => self.control,
        }
    }
}

/// The record of a case: a request's or a control open's size, every byte
/// distinct from its neighbour's, seeded by the case's tag byte.
fn record_bytes(listener: FlowListener, tag: u8) -> Vec<u8> {
    let len = match listener {
        FlowListener::Stream => FLOW_REQUEST_LEN,
        FlowListener::Control => CONTROL_OPEN_LEN,
    };
    (0..len).map(|i| tag.wrapping_add(u8::try_from(i).unwrap())).collect()
}

/// What an adapter reported for one connection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Accepted {
    listener: FlowListener,
    peer_cid: GuestCid,
    record: Vec<u8>,
}

/// Reads `want` bytes or to the end of stream (a record read may return it
/// in pieces).
async fn read_all(io: &dyn GuestFlowIo, socket: &RecordSocket, want: usize) -> Vec<u8> {
    let mut got = Vec::new();
    loop {
        let max = want.saturating_sub(got.len()).max(1);
        let chunk = tokio::time::timeout(Duration::from_secs(10), io.read_record(socket, max))
            .await
            .expect("a record read completes")
            .expect("read_record");
        if chunk.is_empty() {
            return got;
        }
        got.extend(chunk);
        if got.len() >= want {
            return got;
        }
    }
}

/// Accepts every case's connection from `io` (`count` per listener), then
/// lets `end_unbuffered` end the connections with no record, then reads one
/// record from each. Asserts no further accept completes.
async fn drive(
    io: &dyn GuestFlowIo,
    bound: Bound,
    end_unbuffered: impl Fn(GuestConnKey),
) -> BTreeMap<u32, Accepted> {
    let mut accepted = Vec::new();
    for listener in [FlowListener::Stream, FlowListener::Control] {
        let count = CASES.iter().filter(|c| c.listener == listener).count();
        for _ in 0..count {
            let (socket, key) = tokio::time::timeout(Duration::from_secs(10), io.accept(listener))
                .await
                .expect("every opened connection is returned by accept")
                .expect("accept");
            assert_eq!(key.listener, bound.port(listener), "the accepting listener's bound port");
            accepted.push((listener, socket, key));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(500), io.accept(listener)).await.is_err(),
            "no connection beyond the opened ones is observable through {listener:?}"
        );
    }
    let mut out = BTreeMap::new();
    for (listener, socket, key) in &accepted {
        let case = CASES
            .iter()
            .find(|c| c.source_port == key.peer_port.as_u32())
            .unwrap_or_else(|| panic!("an unexpected source port {key:?}"));
        assert_eq!(case.listener, *listener);
        if case.record.is_none() {
            end_unbuffered(*key);
        }
        let want = case.record.map_or(0, |tag| record_bytes(*listener, tag).len());
        let record = read_all(io, socket, want).await;
        out.insert(
            key.peer_port.as_u32(),
            Accepted { listener: *listener, peer_cid: key.peer_cid, record },
        );
    }
    for (_, socket, _) in accepted {
        io.close(OwnedSocket::Record(socket)).await;
    }
    out
}

/// R5-59 (a), flow seam: the same connections, opened by a guest at a leased
/// CID to the real listeners and queued into the sim, are accepted with the
/// same identities and records.
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_flow_listeners_accept_path_matches_the_sim_per_connection() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let work = WorkDir::create(&staging, "accept-path-flow").expect("work dir");
    let image =
        kit.image(work.path(), "peer", &GuestImageSpec::scripted()).expect("the scripted image");
    let cid = HOST_FLOW_ACCEPT_PATH.raw(0);
    let mut guest =
        kit.boot_scripted(&image, cid, &work.path().join("run")).expect("the scripted guest");

    // The real adapter, its two listeners bound as boot binds them.
    let real = HostGuestFlowIo::new();
    let real_bound = Bound::bind(&real).await;
    for (n, case) in CASES.iter().enumerate() {
        let handle = format!("c{n}");
        let peer = guest.peer();
        peer.vsock_connect(
            &handle,
            VMADDR_CID_HOST,
            real_bound.port(case.listener).as_u32(),
            Some(case.source_port),
        )
        .expect("the guest connects from its chosen source port");
        if let Some(tag) = case.record {
            peer.write(&handle, &record_bytes(case.listener, tag))
                .expect("the guest buffers its record");
        }
    }
    // A connection with no record ends when the guest shuts it — after the
    // host accepted it, exactly where the sim's `end_connection` is issued.
    let guest = std::sync::Mutex::new(guest);
    let real_seen = drive(&real, real_bound, |key| {
        let n = CASES
            .iter()
            .position(|c| c.source_port == key.peer_port.as_u32())
            .expect("a case of the table");
        guest
            .lock()
            .unwrap()
            .peer()
            .shutdown(&format!("c{n}"), Shutdown::Both)
            .expect("the guest ends its connection");
    })
    .await;

    // The sim, driven by the same table.
    let sim = SimGuestFlowIo::new(0x5959);
    let sim_bound = Bound::bind(sim.as_ref()).await;
    let peer_cid = GuestCid::new(cid).expect("a band CID is a GuestCid");
    for case in CASES {
        let key = GuestConnKey {
            listener: sim_bound.port(case.listener),
            peer_cid,
            peer_port: VsockPort::new(case.source_port),
        };
        sim.queue_connection(
            case.listener,
            key,
            case.record.map(|tag| record_bytes(case.listener, tag)),
            None,
        );
    }
    let sim_seen =
        drive(sim.as_ref(), sim_bound, |key| sim.end_connection(key, SimPeerEnd::Eof)).await;

    assert_eq!(real_seen, sim_seen, "per connection: identity and record");
    for (port, accepted) in &real_seen {
        assert_eq!(accepted.peer_cid, peer_cid, "the key of {port} carries the guest's real CID");
    }
    drop(guest);
}

/// PORTS-CONFIG / round-11 repair (§ *Composition*, "Repair"): a second
/// `bind_listener` of a listener the seam already holds — the listener
/// repair after its accept task ended — consults no port, closes and binds
/// nothing, and returns the held socket's port; the held listener goes on
/// accepting the guest's connections at that same port. Here the second
/// call asks for a different fixed port: the answer is still the first.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repair_of_a_held_listener_keeps_its_socket_and_its_port() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let work = WorkDir::create(&staging, "held-listener-repair").expect("work dir");
    let image =
        kit.image(work.path(), "peer", &GuestImageSpec::scripted()).expect("the scripted image");
    let cid = HOST_FLOW_ACCEPT_PATH.raw(1);
    let mut guest =
        kit.boot_scripted(&image, cid, &work.path().join("run")).expect("the scripted guest");

    let real = HostGuestFlowIo::new();
    let first = real
        .bind_listener(FlowListener::Stream, VsockListenPort::KernelAssigned)
        .await
        .expect("the first bind");
    let other = VsockPort::new(if first.as_u32() == 60_001 { 60_002 } else { 60_001 });
    let again = real
        .bind_listener(FlowListener::Stream, VsockListenPort::Fixed(other))
        .await
        .expect("the repair of a held, still-listening socket succeeds");
    assert_eq!(
        again, first,
        "the repair keeps the held socket's port; its argument is not consulted"
    );
    match overdrive_testing::vsock::VsockListener::bind(
        overdrive_testing::vsock::VMADDR_CID_ANY,
        other.as_u32(),
    ) {
        Ok(_) => {}
        Err(e) => panic!("the repair bound nothing on {other}: {e}"),
    }

    guest
        .peer()
        .vsock_connect("after-repair", VMADDR_CID_HOST, first.as_u32(), Some(52_100))
        .expect("the guest connects to the held listener's port");
    let (socket, key) =
        tokio::time::timeout(Duration::from_secs(10), real.accept(FlowListener::Stream))
            .await
            .expect("the held listener accepts after the repair")
            .expect("accept");
    assert_eq!(key.listener, first, "accepted on the same port");
    assert_eq!(key.peer_port, VsockPort::new(52_100));
    real.close(OwnedSocket::Record(socket)).await;
    drop(guest);
}
