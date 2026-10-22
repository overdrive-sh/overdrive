//! The seam sim's own semantics (DG-3, QPROJ-7, QPROJ-8): holds, accept and
//! record delivery, K-A2′ (a VM's death), the dial's vsock connect, the
//! gateway route, the seam contract's edge cases, and the forwarder facts
//! the sim takes from its `SimFlowForwarder` arbiter (an end record from a
//! scripted cause, a context's release, a watch refused on a swept `V_h`).
//! These pin the fixture the owner bodies and the conformance drivers stand
//! on; a sim that regressed would let a wrong owner pass silently.
//!
//! The two gateway-route cells touch no socket and run now. Every other cell
//! builds sim sockets and the forwarder context (`overdrive-sim`
//! `SimSocket` / `SimFlowForwarder`, RED until DELIVER step 4, "Forwarder
//! module and adapter") and core vocabulary whose codecs are RED until
//! DELIVER step 2 (core types and wire codec), so each is ignored pending
//! step 4, the later of the two.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    reason = "sim self-tests assert their own fixture preconditions and print their seeds"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use overdrive_core::guest_transport::forwarder::{AbortCause, ForwardOutcome, PairMode, PairSide};
use overdrive_core::guest_transport::wire::{
    CONTROL_OPEN_LEN, ControlOpen, FlowId, FlowOrigin, FlowRequest, KernelErrno,
};
use overdrive_core::guest_transport::{GUEST_CID_BASE, GuestCid, LaunchToken, VsockListenPort};
use overdrive_core::guest_transport::{GUEST_FLOW_CONTROL_PORT, GUEST_FLOW_STREAM_PORT};
use overdrive_core::vm::config::VsockPort;

use super::{FarEndEvent, SimEffectKind, SimFailure, SimGuestFlowIo, SimPeerEnd};
use crate::guest_network::GuestConnKey;
use crate::guest_network::flow_io::{
    FlowIoError, FlowListener, ForwarderError, GuestFlowIo, OwnedSocket, PeerProtocol,
    PeerSocketSpec,
};

/// Every sim built here prints its seed (testing.md § "Seeding and
/// reproducibility").
fn sim(seed: u64) -> Arc<SimGuestFlowIo> {
    eprintln!("seam sim seed = {seed}");
    SimGuestFlowIo::new(seed)
}

fn cid() -> GuestCid {
    GuestCid::new(GUEST_CID_BASE + 7).expect("in the leased range")
}

fn token(byte: u8) -> LaunchToken {
    LaunchToken::from_bytes([byte; 16]).expect("non-zero token")
}

fn control_key(port: u32) -> GuestConnKey {
    GuestConnKey {
        listener: GUEST_FLOW_CONTROL_PORT,
        peer_cid: cid(),
        peer_port: VsockPort::new(port),
    }
}

fn stream_key(port: u32) -> GuestConnKey {
    GuestConnKey {
        listener: GUEST_FLOW_STREAM_PORT,
        peer_cid: cid(),
        peer_port: VsockPort::new(port),
    }
}

fn control_open(launch: LaunchToken) -> Vec<u8> {
    let bytes: [u8; CONTROL_OPEN_LEN] = ControlOpen { launch }.encode();
    bytes.to_vec()
}

fn flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Guest, seq).expect("seq below 2^31")
}

async fn bound(io: &Arc<SimGuestFlowIo>, listener: FlowListener) {
    let port = match listener {
        FlowListener::Stream => GUEST_FLOW_STREAM_PORT,
        FlowListener::Control => GUEST_FLOW_CONTROL_PORT,
    };
    io.bind_listener(listener, VsockListenPort::Fixed(port))
        .await
        .expect("the sim binds the listener");
}

/// A TcpConnect flow `seq` accepted, its request read, a non-mesh TCP peer
/// connected and the pair attached, through the seam calls the owner makes.
async fn attached_flow(
    io: &Arc<SimGuestFlowIo>,
    seq: u32,
) -> crate::guest_network::flow_io::AttachedPair {
    io.open_forwarder().await.expect("the sim forwarder opens");
    bound(io, FlowListener::Stream).await;
    let request = FlowRequest::TcpConnect {
        flow: flow(seq),
        destination: SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 443),
        launch: token(9),
    };
    io.queue_connection(
        FlowListener::Stream,
        stream_key(6000 + seq),
        Some(request.encode().to_vec()),
    );
    let (vsock, _) = io.accept(FlowListener::Stream).await.expect("accepted");
    let _request = io.read_record(&vsock, 32).await.expect("the request");
    let spec = PeerSocketSpec {
        protocol: PeerProtocol::Tcp,
        owner_egress_mark: true,
        abortive_linger: true,
        bind: None,
    };
    let peer = io.open_peer(spec).await.expect("a peer socket");
    io.connect_peer(&peer, SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 443))
        .await
        .expect("the peer connects");
    match io.attach(vsock, peer, PairMode::Stream) {
        Ok(pair) => pair,
        Err(refused) => panic!("an established pair attaches: {:?}", refused.error),
    }
}

/// A held effect waits for the driver; released, it applies; the issued log
/// records it in order.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_held_effect_applies_only_when_released() {
    let io = sim(1);
    io.hold(SimEffectKind::GatewayRoute);
    let seam = Arc::clone(&io);
    let task =
        tokio::spawn(
            async move { seam.converge_gateway_route(Ipv4Addr::new(100, 95, 0, 1)).await },
        );
    io.settled().await;
    assert!(!task.is_finished(), "a held route write does not complete");
    assert_eq!(io.gateway_route(), None, "a held route write changes nothing");
    let id = io.held_of(SimEffectKind::GatewayRoute).expect("the route write is held");
    io.release(id);
    io.settled().await;
    task.await.expect("the task does not panic").expect("the released write succeeds");
    assert_eq!(io.gateway_route(), Some(Ipv4Addr::new(100, 95, 0, 1)));
}

/// A held route write failed with `Netlink` reports
/// `GuestNetworkError::Netlink { LocalRouteConverge }` and leaves the route
/// as it was.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_failed_route_write_leaves_the_route_as_it_was() {
    let io = sim(2);
    io.script_failure(SimEffectKind::GatewayRoute, SimFailure::Netlink);
    let outcome = io.converge_gateway_route(Ipv4Addr::new(100, 95, 0, 1)).await;
    assert!(matches!(
        outcome,
        Err(crate::guest_network::GuestNetworkError::Netlink {
            operation: crate::guest_network::GuestNetworkOperation::LocalRouteConverge,
            ..
        })
    ));
    assert_eq!(io.gateway_route(), None);
}

/// Without auto-delivery a queued connection is accepted only once
/// released, and its buffered record is read only as far as released.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn accept_and_read_follow_the_releases() {
    let io = sim(3);
    io.set_auto_deliver(false);
    bound(&io, FlowListener::Control).await;
    io.queue_connection(FlowListener::Control, control_key(5000), Some(control_open(token(1))));
    assert_eq!(
        io.socket_observation().queued.get(&control_key(5000)),
        Some(&Some(token(1))),
        "the queue reports the buffered record's token"
    );
    let seam = Arc::clone(&io);
    let accept = tokio::spawn(async move { seam.accept(FlowListener::Control).await });
    io.settled().await;
    assert!(!accept.is_finished());
    io.release_accept(control_key(5000));
    io.settled().await;
    let (socket, key) = accept.await.expect("no panic").expect("accepted");
    assert_eq!(key, control_key(5000));
    assert!(io.socket_observation().queued.is_empty());
    io.release_read(key, Some(10));
    let first = io.read_record(&socket, CONTROL_OPEN_LEN).await.expect("ten bytes released");
    assert_eq!(first.len(), 10);
    io.release_read(key, None);
    let rest = io.read_record(&socket, CONTROL_OPEN_LEN).await.expect("the rest released");
    assert_eq!(first.len() + rest.len(), CONTROL_OPEN_LEN);
    io.close(OwnedSocket::Record(socket)).await;
    assert!(io.socket_observation().closed_by_owner.contains(&key));
}

/// K-A2′: an accepted connection whose VM died fails at its next read and
/// is reported `peer_gone` until the owner closes it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accepted_connection_fails_at_its_next_read_after_the_vm_dies() {
    let io = sim(4);
    bound(&io, FlowListener::Control).await;
    io.queue_connection(FlowListener::Control, control_key(5001), Some(control_open(token(2))));
    let (socket, key) = io.accept(FlowListener::Control).await.expect("accepted");
    io.vm_stop(cid());
    assert!(io.socket_observation().peer_gone.contains(&key));
    let read = io.read_record(&socket, CONTROL_OPEN_LEN).await;
    assert!(
        matches!(read, Err(FlowIoError::Io { ref source }) if source.kind() == io::ErrorKind::ConnectionReset)
    );
    io.abort(OwnedSocket::Record(socket)).await;
    let observed = io.socket_observation();
    assert!(!observed.peer_gone.contains(&key));
    assert!(observed.closed_by_owner.contains(&key));
}

/// K-A2′: a connection still queued when its VM died is accepted later and
/// delivers its buffered record, then the end.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_queued_connection_keeps_its_record_after_the_vm_dies() {
    let io = sim(5);
    bound(&io, FlowListener::Control).await;
    io.set_auto_deliver(false);
    io.queue_connection(FlowListener::Control, control_key(5002), Some(control_open(token(3))));
    io.vm_stop(cid());
    io.release_accept(control_key(5002));
    let (socket, key) = io.accept(FlowListener::Control).await.expect("accepted after the VM died");
    io.release_read(key, None);
    let record = io.read_record(&socket, CONTROL_OPEN_LEN).await.expect("the buffered record");
    assert_eq!(record, control_open(token(3)));
    let end = io.read_record(&socket, CONTROL_OPEN_LEN).await;
    assert!(
        matches!(end, Err(FlowIoError::Io { ref source }) if source.kind() == io::ErrorKind::ConnectionReset)
    );
    io.close(OwnedSocket::Record(socket)).await;
}

/// A dial's vsock connect to a CID whose VM is gone fails `ENODEV`
/// (`drivers/vhost/vsock.c:290 @ v7.0`); after a new launch it connects.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_vsock_connect_to_a_gone_vm_fails_enodev() {
    let io = sim(6);
    io.vm_stop(cid());
    let refused = io.connect_guest(cid()).await;
    assert!(
        matches!(refused, Err(FlowIoError::Io { ref source }) if source.raw_os_error() == Some(libc::ENODEV))
    );
    assert!(io.dials().is_empty(), "a failed connect opens no dial");
    io.vm_start(cid());
    let vh = io.connect_guest(cid()).await.expect("a running VM accepts the connect");
    assert_eq!(io.dials().len(), 1);
    io.close(OwnedSocket::Record(vh)).await;
}

/// `read_record` with `max = 0` is `InvalidInput`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_zero_length_read_is_invalid_input() {
    let io = sim(7);
    bound(&io, FlowListener::Stream).await;
    io.queue_connection(FlowListener::Stream, stream_key(6000), None);
    let (socket, key) = io.accept(FlowListener::Stream).await.expect("accepted");
    let read = io.read_record(&socket, 0).await;
    assert!(
        matches!(read, Err(FlowIoError::Io { ref source }) if source.kind() == io::ErrorKind::InvalidInput)
    );
    io.end_connection(key, SimPeerEnd::Eof);
    io.close(OwnedSocket::Record(socket)).await;
}

/// The sim queues no end record of its own: a destination reset scripted on
/// the attached TCP half is classified by the `SimFlowForwarder` arbiter,
/// whose one record (`SocketError` on the TCP half, side B) the seam's
/// `next_end` returns; the far end observed a reset.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_scripted_destination_reset_reaches_next_end_as_the_arbiters_record() {
    let io = sim(8);
    let pair = attached_flow(&io, 1).await;
    io.destination_resets(flow(1));
    let end = io.next_end().await.expect("the context is readable");
    assert_eq!(end.id, pair.id);
    assert!(matches!(
        end.outcome,
        ForwardOutcome::Aborted { cause: AbortCause::SocketError { side: PairSide::B, .. } }
    ));
    assert_eq!(io.socket_observation().far_end.get(&flow(1)), Some(&FarEndEvent::Reset));
    io.close(OwnedSocket::Pair(pair.vsock)).await;
    io.close(OwnedSocket::Pair(pair.peer)).await;
}

/// EVMOD-CTL: `close_forwarder` releases the context — its pairs are aborted
/// with no record, the TCP far end is reset, and the device's context table
/// shows it released holding nothing.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_released_context_aborts_its_pairs_with_no_record() {
    let io = sim(9);
    let pair = attached_flow(&io, 2).await;
    io.close_forwarder();
    assert_eq!(io.socket_observation().far_end.get(&flow(2)), Some(&FarEndEvent::Reset));
    let contexts = io.forwarder_contexts();
    assert_eq!(contexts.len(), 1);
    assert!(!contexts[0].open && contexts[0].live_pairs == 0 && contexts[0].live_watches == 0);
    io.abort(OwnedSocket::Pair(pair.vsock)).await;
    io.abort(OwnedSocket::Pair(pair.peer)).await;
}

/// EVMOD-ATTACH-END: a dial's `V_h` swept by the device release (the VMM
/// exited after the connect) is refused `HalfEnded` by the arbiter's
/// `watch`, with the sweep's errno.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_watch_of_a_swept_vh_is_refused_half_ended() {
    let io = sim(10);
    io.open_forwarder().await.expect("the sim forwarder opens");
    let vh = io.connect_guest(cid()).await.expect("connected");
    io.vm_stop(cid());
    let refused = io.watch(&vh);
    assert!(matches!(
        refused,
        Err(ForwarderError::HalfEnded { errno, .. }) if errno == KernelErrno::ECONNRESET
    ));
    io.close(OwnedSocket::Record(vh)).await;
}
