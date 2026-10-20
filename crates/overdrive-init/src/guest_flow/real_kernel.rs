//! Real-kernel half of R5-59 (b), the guest seam's accept-path equivalence
//! (D-UP-19, PD-5; D-UP-18): `GuestFlowOwner` over `KernelGuestOwnerIo` and
//! `KernelFlowForwarder` serves the same connection table
//! ([`ACCEPT_PATH_CASES`]) with the same per-connection outcome as the sim half
//! (`tests::tcp_accept::accept_path_outcomes_by_first_record`, over
//! `SimGuestOwnerIo`): a connection with its `TcpAccept` request buffered is
//! served — the application accepts it and the bytes written after the
//! request reach it — and one with no byte yields no application connection
//! and its V_g is closed. Order-insensitive, compared per connection.
//!
//! **Lane.** Inside an Overdrive microVM guest, as the guest's operator
//! command, through `cargo xtask metal run --` with `kvm-tests` (see
//! `transport::real_kernel`): the production `overdrive-init` (PID 1) owns
//! `GUEST_INBOUND_PORT`; this cell opens the connections over the guest's vsock
//! loopback transport (`VMADDR_CID_LOCAL`), which delivers them to that
//! listener through the same `virtio_transport_recv_pkt` path a host
//! connection takes (`net/vmw_vsock/vsock_loopback.c:142-143 @ v7.0`), and
//! plays the application on `workload_addr:<port>`. It fails closed outside a
//! guest and when the guest kernel has no vsock loopback transport.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "real-kernel test cells: a failed precondition panics"
)]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddrV4, TcpListener};
use std::os::fd::{AsRawFd, OwnedFd};
use std::str::FromStr;
use std::time::{Duration, Instant};

use nix::sys::socket::{AddressFamily, SockFlag, SockType, VsockAddr, connect, shutdown, socket};
use overdrive_core::guest_transport::GUEST_INBOUND_PORT;
use overdrive_core::guest_transport::wire::{FlowId, FlowOrigin, FlowRequest};
use overdrive_core::net::ListenPort;

use super::test_support::{ACCEPT_PATH_CASES, AcceptPathCase};
use crate::transport::{GuestTransportToken, parse_transport_token};

/// The guest's transport token; panics (fails closed) outside a guest.
fn in_guest() -> GuestTransportToken {
    let cmdline = fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    assert!(
        cmdline.contains("overdrive.transport="),
        "not running inside an Overdrive guest: this cell runs as a microVM guest's operator command"
    );
    parse_transport_token(&cmdline).expect("the guest's token parses")
}

/// A vsock STREAM connection to the guest's own inbound listener over the
/// loopback transport.
fn connect_inbound() -> File {
    let fd: OwnedFd = socket(AddressFamily::Vsock, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .expect("a vsock socket");
    let addr = VsockAddr::new(nix::libc::VMADDR_CID_LOCAL, GUEST_INBOUND_PORT.as_u32());
    connect(fd.as_raw_fd(), &addr).unwrap_or_else(|e| {
        panic!("connect to the guest's own GUEST_INBOUND_PORT over vsock loopback failed ({e}): the guest kernel needs CONFIG_VSOCKETS_LOOPBACK")
    });
    File::from(fd)
}

/// The application's accept on `listener` within `bound`, or `None`.
fn accept_within(listener: &TcpListener, bound: Duration) -> Option<std::net::TcpStream> {
    listener.set_nonblocking(true).expect("non-blocking listener");
    let deadline = Instant::now() + bound;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) => panic!("accept failed: {e}"),
        }
    }
}

/// The per-connection outcome both halves compare: served with the payload,
/// or no application connection.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Served(Vec<u8>),
    NoApplicationConnection,
}

/// R5-59 (b), real half: the production guest owner's outcome per connection
/// of [`ACCEPT_PATH_CASES`] equals the sim half's — served or not, by its
/// first record.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_accept_path_matches_the_sim_per_connection() {
    let token = in_guest();
    let mut listeners: BTreeMap<u16, TcpListener> = BTreeMap::new();
    let mut outcomes = BTreeMap::new();
    for case in ACCEPT_PATH_CASES {
        let AcceptPathCase { seq, port, request_buffered, payload } = case;
        let listener = listeners.entry(port).or_insert_with(|| {
            TcpListener::bind(SocketAddrV4::new(token.workload_addr, port))
                .expect("the application listens")
        });
        let mut vsock = connect_inbound();
        let outcome = if request_buffered {
            let request = FlowRequest::TcpAccept {
                flow: FlowId::new(FlowOrigin::Host, seq).expect("small seq"),
                peer: token.gateway,
                listen_port: ListenPort::from_str(&port.to_string()).expect("non-zero"),
                launch: token.launch,
            };
            vsock.write_all(&request.encode()).expect("the request is written");
            let mut app =
                accept_within(listener, Duration::from_secs(5)).expect("the application accepts C");
            assert_eq!(
                app.peer_addr().expect("C's address").ip(),
                std::net::IpAddr::V4(token.gateway),
                "C is bound to the gateway"
            );
            vsock.write_all(payload).expect("the payload is written after the request");
            let mut got = vec![0u8; payload.len()];
            app.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
            app.read_exact(&mut got).expect("the payload reaches the application");
            app.shutdown(Shutdown::Both).ok();
            Outcome::Served(got)
        } else {
            shutdown(vsock.as_raw_fd(), nix::sys::socket::Shutdown::Write)
                .expect("the connection ends with no byte");
            let app = accept_within(listener, Duration::from_secs(2));
            assert!(app.is_none(), "{case:?}: no application connection");
            let mut rest = Vec::new();
            vsock.read_to_end(&mut rest).expect("V_g is closed by the guest owner");
            assert!(rest.is_empty());
            Outcome::NoApplicationConnection
        };
        outcomes.insert(seq, outcome);
    }
    for case in ACCEPT_PATH_CASES {
        let expected = if case.request_buffered {
            Outcome::Served(case.payload.to_vec())
        } else {
            Outcome::NoApplicationConnection
        };
        assert_eq!(outcomes.get(&case.seq), Some(&expected), "{case:?}");
    }
}
