//! P4-S5 — the constant guest-prefix rules of the production shared table
//! (ADR-0172): a guest-prefix packet not diverted to leg-C reaches no host
//! socket and does not leave the host; a leg-C-diverted connection reaches
//! leg-C in both directions. Tier-3, Lima + root, `host-kernel-shared`.
//!
//! netns-density-295 vsock attachment replacement. Contract:
//! `docs/feature/netns-density-295/feature-delta.md` § "[REF] leg-S over
//! vsock … and the mTLS forwarded-outbound port" ("Constant rules": the mTLS
//! worker writes the constant rule set in one nft transaction, among them the
//! guest-prefix output reject and prerouting drop of traffic not diverted to
//! leg-C; it holds no per-allocation element for the guest prefix), INB-1 (no
//! workload address is host-local, no route covers the prefix), and
//! obligations R5-43 / R5-3 / R5-25. Roadmap step 12 (firewall single cut:
//! rules 5 and 8 match the constant `GUEST_NODE_PREFIX`; `outbound_sources`
//! and `managed_guest_ips` deleted).
//!
//! # Production path
//!
//! The shared table is written by its owner, the mTLS worker
//! (`MtlsInterceptWorker::start_shared_owner` over the real
//! `HostMtlsIntercept`); a VM allocation is made intercept-live through
//! `start_alloc`, which registers its `inbound_destinations` tuple. The
//! node fixture is `vsock_legb_dial::node`. The far peer is a hand-built
//! namespace on a `TestCidrLease` `/24` whose only route to the guest prefix
//! points at the host — it models a remote client, not a production effect.
//!
//! # Not here
//!
//! - The same-node leg-B half of "a leg-C-diverted connection reaches
//!   leg-C" is `vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`.
//! - The gateway exclusion (`gateway:53` not matched) needs the gateway's
//!   host-local route (GW-LOCAL), which the control plane's boot convergence
//!   writes; it is a walking-skeleton cell (P6-S13).
//!
//! Ignored until DELIVER step 12.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    reason = "Tier-3 bodies: evidence goes to stderr; fixture preconditions panic with informative messages"
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::vsock_legb_dial::node::{Node, PeerNetns, established_sockets};
use super::vsock_mesh_egress_mtls::PEER_SNI;

/// A guest-prefix address no allocation holds.
const UNASSIGNED: Ipv4Addr = Ipv4Addr::new(100, 95, 255, 201);
/// A guest-prefix address the VM allocation of a cell holds.
const VM_ADDR: Ipv4Addr = Ipv4Addr::new(100, 95, 255, 202);
const PORT: u16 = 18_543;
/// The bound within which a host-local rejection must be reported.
const VERDICT_BOUND: Duration = Duration::from_secs(1);
/// How long a remote SYN to a dropped destination is given.
const DROP_WAIT: Duration = Duration::from_secs(2);

fn assert_no_accept(listener: &TcpListener, what: &str) {
    listener.set_nonblocking(true).expect("non-blocking listener");
    match listener.accept() {
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
        Ok((_stream, from)) => panic!("{what}: a host socket accepted a connection from {from}"),
        Err(e) => panic!("{what}: accept failed: {e}"),
    }
}

/// A new host-local TCP connection to a guest-prefix address is rejected at
/// once by the constant output reject (`ECONNREFUSED`), whether or not some
/// host socket listens on the port: a wildcard listener accepts nothing, and
/// the connection does not leave the host (the reject precedes routing out
/// of the uplink).
///
/// CONTRACT_SHAPE: bounded-change. Universe: the connect's result and
/// latency, the wildcard listener; expected: `ConnectionRefused` within the
/// bound, no accept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_local_connection_to_a_guest_prefix_address_is_rejected_at_once() {
    let node = Node::start("rules-host-tcp").await;
    let wildcard = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT))
        .expect("precondition: a wildcard host listener on the port");

    let started = Instant::now();
    let refused =
        TcpStream::connect_timeout(&SocketAddrV4::new(UNASSIGNED, PORT).into(), DROP_WAIT)
            .expect_err("a host-local connection into the guest prefix is refused");
    let elapsed = started.elapsed();
    node.record("connect", format_args!("{refused} after {elapsed:?}"));

    assert_eq!(refused.kind(), io::ErrorKind::ConnectionRefused, "the output reject answers");
    assert!(elapsed < VERDICT_BOUND, "rejected at once, not dropped: {elapsed:?}");
    assert_no_accept(&wildcard, "host-local TCP into the guest prefix");
    node.finish().await;
}

/// A host-local UDP datagram to a guest-prefix address is rejected at
/// `send()` with `EPERM` (plain reject) and reaches no host socket.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the send's result, a wildcard
/// UDP socket on the port; expected: `PermissionDenied`, nothing received.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_local_datagram_to_a_guest_prefix_address_fails_eperm() {
    let node = Node::start("rules-host-udp").await;
    let wildcard = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT))
        .expect("precondition: a wildcard UDP socket on the port");
    wildcard.set_read_timeout(Some(Duration::from_millis(300))).expect("read timeout");
    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).expect("sender");

    let refused = sender
        .send_to(b"into-the-prefix", SocketAddrV4::new(UNASSIGNED, PORT))
        .expect_err("a host-local datagram into the guest prefix is rejected");
    node.record("send_to", format_args!("{refused}"));
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "plain reject → EPERM");
    let mut buf = [0_u8; 64];
    match wildcard.recv_from(&mut buf) {
        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
        Ok((n, from)) => panic!("{n} bytes from {from} reached a host socket"),
        Err(e) => panic!("wildcard recv failed: {e}"),
    }
    node.finish().await;
}

/// A remote connection to a guest-prefix address that no allocation holds
/// is dropped by the constant prerouting drop: the remote's connect times
/// out (no reset), no host socket accepts it, and it is not forwarded (in
/// the dev VM a forwarded SYN would be answered by the user-mode network,
/// so the timeout also shows it never left).
///
/// CONTRACT_SHAPE: bounded-change. Universe: the remote's connect result,
/// the wildcard listener; expected: `TimedOut`, no accept.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_remote_connection_to_an_unassigned_guest_prefix_address_is_dropped() {
    let node = Node::start("rules-remote-drop").await;
    let peer = PeerNetns::create("rules-remote-drop");
    let wildcard = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT))
        .expect("precondition: a wildcard host listener on the port");

    let outcome = peer
        .connect_from(SocketAddrV4::new(UNASSIGNED, PORT), DROP_WAIT)
        .join()
        .expect("remote connect thread");
    node.record("remote_connect", format_args!("{:?}", outcome.as_ref().err()));
    let error = outcome.expect_err("the remote connection is not completed");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "dropped, never refused or forwarded");
    assert_no_accept(&wildcard, "remote TCP to an unassigned guest-prefix address");
    node.finish().await;
}

/// Every packet of a leg-C-diverted remote connection reaches leg-C, both
/// ways: a remote mesh client completes the mutual-TLS handshake with leg-C
/// for the VM allocation's registered tuple (its replies leave addressed
/// outside the prefix, so no rule matches them), and leg-C's accepted socket
/// is local to the VM's tuple with the remote's address as peer.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the remote's handshake, the
/// established sockets on the port; expected: a completed handshake against
/// the node's trust root, one leg-C socket `VM_ADDR:PORT` ↔ remote.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways() {
    let node = Node::start("rules-leg-c-remote").await;
    let _vm = node.serve_vm("rules-vm", VM_ADDR, PORT).await;
    let peer = PeerNetns::create("rules-leg-c-remote");
    let remote = peer.peer_addr();

    let mut roots = rustls::RootCertStore::empty();
    roots.add(node.pki.root.clone()).expect("the node's root is a trust anchor");
    roots.add(node.pki.client_chain[1].clone()).expect("the intermediate is a trust anchor");
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(node.pki.client_chain.clone(), node.pki.client_key.clone_key())
            .expect("client auth config"),
    );
    let (done_tx, done_rx) = std::sync::mpsc::channel::<io::Result<usize>>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let remote_thread = peer.spawn_in(move || {
        let handshake = || -> io::Result<(TcpStream, rustls::ClientConnection, usize)> {
            let mut tcp = TcpStream::connect_timeout(
                &SocketAddrV4::new(VM_ADDR, PORT).into(),
                Duration::from_secs(5),
            )?;
            tcp.set_read_timeout(Some(Duration::from_secs(5)))?;
            let name = rustls::pki_types::ServerName::try_from(PEER_SNI)
                .expect("the SNI is a valid server name");
            let mut tls = rustls::ClientConnection::new(config, name).map_err(io::Error::other)?;
            while tls.is_handshaking() {
                tls.complete_io(&mut tcp)?;
            }
            let presented = tls.peer_certificates().map_or(0, <[_]>::len);
            Ok((tcp, tls, presented))
        };
        match handshake() {
            Ok((tcp, tls, presented)) => {
                done_tx.send(Ok(presented)).expect("report the handshake");
                // Hold the connection while the main thread reads the
                // socket table; leg-S cannot reach the VM before the
                // attachment cut, so nothing else is exchanged.
                release_rx.recv().expect("release signal");
                drop((tls, tcp));
            }
            Err(e) => done_tx.send(Err(e)).expect("report the failure"),
        }
    });

    let presented = done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the remote reports its handshake")
        .expect("the mTLS handshake with leg-C completes");
    let sockets = established_sockets(PORT);
    release_tx.send(()).expect("release the remote");
    remote_thread.join().expect("remote thread");
    node.record("handshake", format_args!("{presented} certificates; ss {sockets:?}"));
    assert!(presented >= 1, "leg-C presented the VM allocation's SVID chain");
    assert!(
        sockets
            .iter()
            .any(|s| s.local == SocketAddrV4::new(VM_ADDR, PORT) && *s.peer.ip() == remote),
        "leg-C accepted the remote's connection on the VM's tuple: {sockets:?}"
    );
    node.finish().await;
}
