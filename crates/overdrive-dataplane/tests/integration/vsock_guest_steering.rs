//! P4 — guest-prefix steering kernel facts (Tier-3, `sk_lookup`).
//!
//! netns-density-295 vsock attachment replacement, DISTILL piece P4
//! (`docs/feature/netns-density-295/distill/test-scenarios-p4-steering-intake-boot.md`,
//! P4-S1..P4-S5). Contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Driven port — guest-prefix steering
//! `overdrive-dataplane::guest_steering`" (D8a-LOOKUP, D8a-LOAD-SWAP, D8a-CAP;
//! ADR-0171, ADR-0175, ADR-0176), V-26 (P-40).
//!
//! Adapter integration: every test drives the real `GuestPrefixSteering`
//! (`load` → `GuestSteeringCandidate::attach` → `steer`) against the real
//! kernel, with hand-bound listening TCPv4 sockets standing in for the owner's
//! intake listeners. The oracle is the kernel's landing decision for a real
//! `connect()` — refused (the program dropped the lookup, so TCP answers with a
//! reset) or accepted by the steered listener, with a byte-distinct
//! request/response exchange.
//!
//! The prefix is a `TestCidrLease` `/24`, never the production guest prefix:
//! the lease's gateway is the `gateway` argument, and the fixture installs the
//! lease's `local <prefix> dev lo` route — the precondition the shared
//! guest-network owner provides in production (boot step D8a-ROUTE). The pin
//! under `GUEST_PREFIX_STEERING_PIN_DIR` is removed by RAII.
//!
//! Lane: Tier-3, example-based (Mandate 11). The whole
//! `overdrive-dataplane` `integration` binary is in the `host-kernel-shared`
//! nextest group (`.config/nextest.toml`), which serializes this module's
//! node-global state (the root-namespace `sk_lookup` link and its pin).
//!
//! Every test carries the reasoned `#[ignore]` naming its activating DELIVER
//! step; the crafter removes only the marker.
//!
//! Not covered here (no pinned seam or owner-crate concern): a verifier
//! rejection at `load` (`Load`), a failing `BPF_LINK_UPDATE` (`Update`), a
//! failing first-attach pin (`Pin`), `KernelMemory` from `steer`, the
//! production nft guest-prefix rules (PREFIX-REPLY's constant rules are the
//! owner's), and a fill of all `GUEST_INTAKE_STEERING_MAX` entries (bounded
//! cell below).

#![allow(
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::unnecessary_debug_formatting
)]

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use ipnet::Ipv4Net;
use nix::sys::socket::{
    AddressFamily, Backlog, SockFlag, SockType, SockaddrIn, bind, connect, listen, socket,
};
use overdrive_core::guest_transport::guest_intake_host_port;
use overdrive_dataplane::guest_steering::{
    GUEST_PREFIX_STEERING_PIN_DIR, GuestPrefixSteering, GuestSteeringError,
};
use overdrive_testing::cidr_lease::TestCidrLease;

use super::helpers::guest_net::{PeerLink, in_netns, run_cleanup, run_ok, run_stdout};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A leased `/24` made local on `lo` (`ip route add local <cidr> dev lo table
/// local`): the guest prefix for one test. In production the shared owner
/// writes this route only over a pinned steering link (D8a-ROUTE); the test
/// supplies it directly because it exercises the steering adapter alone.
struct SteeringPrefix {
    prefix: Ipv4Net,
    lease: TestCidrLease,
}

impl SteeringPrefix {
    fn acquire(owner: &str) -> Self {
        let lease = TestCidrLease::acquire(owner)
            .unwrap_or_else(|e| panic!("precondition: test CIDR lease {owner}: {e}"));
        let prefix: Ipv4Net = lease.cidr().parse().expect("a lease CIDR parses as Ipv4Net");
        run_ok("ip", &["route", "add", "local", lease.cidr(), "dev", "lo", "table", "local"]);
        Self { prefix, lease }
    }

    /// The gateway the steering excludes (the lease's first host address).
    const fn gateway(&self) -> Ipv4Addr {
        self.lease.host_gateway()
    }

    /// A workload address inside the prefix (the lease's second address).
    const fn workload(&self) -> Ipv4Addr {
        self.lease.workload_addr()
    }

    /// The `n`th address of the prefix (`n` ≥ 3 is neither gateway nor the
    /// default workload address).
    fn host(&self, n: u8) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.prefix.network()) + u32::from(n))
    }
}

impl Drop for SteeringPrefix {
    fn drop(&mut self) {
        run_cleanup(
            "ip",
            &["route", "del", "local", self.lease.cidr(), "dev", "lo", "table", "local"],
        );
    }
}

/// Owns the pin directory's contents for one test: removes any link left
/// behind by an interrupted earlier run at creation (so a first `attach`
/// really is a first attach), and removes every pin on `Drop` — the links this
/// test pinned — so the root namespace carries no steering afterwards.
struct PinnedSteeringGuard;

impl PinnedSteeringGuard {
    fn claim() -> Self {
        for entry in pin_entries() {
            fs::remove_file(&entry).unwrap_or_else(|e| {
                panic!("precondition: leftover steering pin {entry:?} is removable: {e}")
            });
        }
        Self
    }
}

impl Drop for PinnedSteeringGuard {
    fn drop(&mut self) {
        for entry in pin_entries() {
            if let Err(e) = fs::remove_file(&entry) {
                eprintln!("cleanup: steering pin {entry:?} not removed: {e}");
            }
        }
    }
}

/// Every object pinned under `GUEST_PREFIX_STEERING_PIN_DIR`.
fn pin_entries() -> Vec<PathBuf> {
    match fs::read_dir(GUEST_PREFIX_STEERING_PIN_DIR) {
        Ok(dir) => dir.map(|e| e.expect("pin dir entry is readable").path()).collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => panic!("pin dir {GUEST_PREFIX_STEERING_PIN_DIR} is readable: {e}"),
    }
}

/// The pinned link's kernel identity, read with `bpftool` (never through the
/// adapter's private fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PinnedLink {
    link_id: u64,
    prog_id: u64,
}

/// The single link pinned under the pin dir.
fn pinned_link() -> PinnedLink {
    let entries = pin_entries();
    assert_eq!(entries.len(), 1, "exactly one steering link is pinned: {entries:?}");
    let path = entries[0].to_str().expect("pin path is UTF-8").to_owned();
    let json = run_stdout("bpftool", &["-j", "link", "show", "pinned", &path]);
    let v: serde_json::Value = serde_json::from_str(&json).expect("bpftool emits JSON");
    PinnedLink {
        link_id: v["id"].as_u64().expect("link has an id"),
        prog_id: v["prog_id"].as_u64().expect("link has a prog_id"),
    }
}

/// Boot steering for the prefix the way boot `converge_shared` does: `load`
/// then `attach`.
fn boot_steering(prefix: &SteeringPrefix) -> GuestPrefixSteering {
    GuestPrefixSteering::load(prefix.prefix, prefix.gateway())
        .expect("load: the verifier accepts this binary's steering program")
        .attach()
        .expect("attach: the steering link is attached and pinned")
}

/// Bind and listen an intake listener on `addr:guest_intake_host_port(index)`
/// (INTAKE-BIND: the intake host block, never the declared port).
fn intake_listener(addr: Ipv4Addr, index: u16) -> TcpListener {
    let port = guest_intake_host_port(index).expect("index lies in the intake host block");
    TcpListener::bind(SocketAddrV4::new(addr, port)).expect("precondition: intake listener binds")
}

/// A wildcard host service on an ephemeral port: the socket a prefix lookup
/// must never reach (B-2). Non-blocking, so "nothing accepted" is observable.
fn wildcard_service() -> (TcpListener, u16) {
    let l = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .expect("precondition: wildcard host service binds");
    l.set_nonblocking(true).expect("wildcard service non-blocking");
    let port = l.local_addr().expect("wildcard has a port").port();
    (l, port)
}

fn accepted_nothing(l: &TcpListener) -> bool {
    matches!(l.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
}

/// A host-local connect, classified by the kernel's landing decision.
fn connect_to(dst: SocketAddrV4) -> io::Result<TcpStream> {
    TcpStream::connect_timeout(&dst.into(), Duration::from_secs(1))
}

fn is_refused(r: &io::Result<TcpStream>) -> bool {
    matches!(r, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused)
}

const REQUEST: &[u8] = b"p4-steer-request";
const RESPONSE: &[u8] = b"p4-steer-reply!!";

/// Byte-distinct exchange both ways between `client` and the connection the
/// steered `listener` accepts.
fn exchange_both_ways(mut client: TcpStream, listener: &TcpListener) {
    let (mut server, _) = listener.accept().expect("the steered listener accepts the connection");
    client.write_all(REQUEST).expect("client writes");
    let mut got = [0u8; REQUEST.len()];
    server.read_exact(&mut got).expect("server reads the request");
    assert_eq!(&got, REQUEST, "the steered listener receives the client's bytes");
    server.write_all(RESPONSE).expect("server writes");
    let mut back = [0u8; RESPONSE.len()];
    client.read_exact(&mut back).expect("client reads the response");
    assert_eq!(&back, RESPONSE, "the client receives the steered listener's bytes");
}

/// Connect from source address `src` (bound before `connect`).
fn connect_from(src: Ipv4Addr, dst: SocketAddrV4) -> io::Result<TcpStream> {
    let fd = socket(AddressFamily::Inet, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .map_err(io::Error::from)?;
    bind(fd.as_raw_fd(), &SockaddrIn::from(SocketAddrV4::new(src, 0))).map_err(io::Error::from)?;
    connect(fd.as_raw_fd(), &SockaddrIn::from(dst)).map_err(io::Error::from)?;
    Ok(TcpStream::from(fd))
}

/// A TCP socket bound to `addr` but not listening.
fn bound_unlistening_tcp(addr: Ipv4Addr) -> OwnedFd {
    let fd = socket(AddressFamily::Inet, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .expect("precondition: TCP socket");
    bind(fd.as_raw_fd(), &SockaddrIn::from(SocketAddrV4::new(addr, 0)))
        .expect("precondition: TCP socket binds");
    fd
}

/// A listening TCP socket bound to `addr` on an ephemeral port.
fn listening_tcp(addr: Ipv4Addr) -> OwnedFd {
    let fd = bound_unlistening_tcp(addr);
    listen(&fd, Backlog::new(8).expect("backlog 8 is valid")).expect("precondition: listen");
    fd
}

// ---------------------------------------------------------------------------
// P4-S1 — only a steered listener is reachable at the prefix declared port
// (R5-1, R5-2 steering half; G-V8; V-26 K-L1/K-L5/K-L10)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn only_a_steered_listener_is_reachable_at_the_prefix_declared_port() {
    let prefix = SteeringPrefix::acquire("vsock-steering-s1");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);

    // GIVEN a wildcard host service on the declared port and no entry for it.
    let (wildcard, declared) = wildcard_service();
    let intake = SocketAddrV4::new(prefix.workload(), declared);

    // WHEN a host-local connect reaches the prefix with no entry,
    // THEN the lookup is dropped (reset) and never reaches the wildcard service.
    assert!(is_refused(&connect_to(intake)), "no entry -> refused");
    assert!(accepted_nothing(&wildcard), "the wildcard host service is never reached (B-2)");

    // WHEN an intake listener bound in the intake host block is steered,
    let listener = intake_listener(prefix.workload(), 0);
    steering.steer(intake, listener.as_fd()).expect("steer: the entry is present");

    // THEN a connect to the declared port reaches it and exchanges both ways,
    exchange_both_ways(connect_to(intake).expect("steered intake is reachable"), &listener);

    // AND repeating the steer for the same listener is Ok with no change,
    steering.steer(intake, listener.as_fd()).expect("re-steer of the same listener is Ok");
    exchange_both_ways(connect_to(intake).expect("still reachable after re-steer"), &listener);

    // AND the listener's own bound block port has no entry of its own: refused.
    let bound = match listener.local_addr().expect("listener address") {
        std::net::SocketAddr::V4(a) => a,
        std::net::SocketAddr::V6(a) => panic!("listener is IPv4, got {a}"),
    };
    assert!(is_refused(&connect_to(bound)), "bound block port -> refused (K-L10)");
    assert!(accepted_nothing(&wildcard), "the wildcard host service is never reached");
}

// ---------------------------------------------------------------------------
// P4-S2 — an unassigned prefix address reaches no host socket; the gateway and
// out-of-prefix destinations pass untouched; UDP and device ingress are dropped
// (R5-3, K-L1; G-V1/G-V8; D8a-LOOKUP decision)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn an_unassigned_prefix_address_reaches_no_host_socket() {
    let prefix = SteeringPrefix::acquire("vsock-steering-s2");
    let _pins = PinnedSteeringGuard::claim();
    let _steering = boot_steering(&prefix);
    let (wildcard, port) = wildcard_service();

    // A prefix address with no lease and no entry: refused, no host socket.
    let unassigned = SocketAddrV4::new(prefix.host(200), port);
    assert!(is_refused(&connect_to(unassigned)), "unassigned prefix address -> refused");
    assert!(accepted_nothing(&wildcard), "the wildcard host service is never reached");

    // The gateway is excluded: its lookup passes untouched to the host socket.
    let via_gateway = connect_to(SocketAddrV4::new(prefix.gateway(), port));
    assert!(via_gateway.is_ok(), "gateway passes untouched: {via_gateway:?}");
    assert!(!accepted_nothing(&wildcard), "the gateway connection lands on the host service");

    // A destination outside the prefix passes untouched.
    let outside = connect_to(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    assert!(outside.is_ok(), "out-of-prefix passes untouched: {outside:?}");
    assert!(!accepted_nothing(&wildcard), "the loopback connection lands on the host service");
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn udp_to_a_prefix_address_reaches_no_host_socket() {
    let prefix = SteeringPrefix::acquire("vsock-steering-udp");
    let _pins = PinnedSteeringGuard::claim();
    let _steering = boot_steering(&prefix);

    // GIVEN a wildcard UDP host service.
    let service = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        .expect("precondition: wildcard UDP service binds");
    service.set_read_timeout(Some(Duration::from_millis(500))).expect("read timeout");
    let port = service.local_addr().expect("UDP service port").port();
    let client = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("UDP client");
    let mut buf = [0u8; 64];

    // Control: outside the prefix the service receives the datagram.
    client
        .send_to(b"outside", SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
        .expect("send to loopback");
    let n = service.recv(&mut buf).expect("the host service receives out-of-prefix UDP");
    assert_eq!(&buf[..n], b"outside");

    // WHEN a datagram targets a prefix address, THEN no host socket receives it.
    client.send_to(b"prefix", SocketAddrV4::new(prefix.workload(), port)).expect("send to prefix");
    let got = service.recv(&mut buf);
    assert!(
        matches!(&got, Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)),
        "UDP to the prefix reaches no host socket: {got:?}",
    );
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn a_connection_arriving_on_a_device_is_dropped_even_for_a_steered_intake() {
    let prefix = SteeringPrefix::acquire("vsock-steering-dev");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);

    // GIVEN a steered intake that a host-local client reaches,
    let (wildcard, declared) = wildcard_service();
    let intake = SocketAddrV4::new(prefix.workload(), declared);
    let listener = intake_listener(prefix.workload(), 0);
    steering.steer(intake, listener.as_fd()).expect("steer");
    exchange_both_ways(
        connect_to(intake).expect("host-local client reaches the intake"),
        &listener,
    );

    // AND a remote client in another namespace, routed to the prefix over a veth.
    let link = PeerLink::create("vsock-steering-dev-link", 0);
    link.route_from_peer(&prefix.prefix.to_string());

    // Control: the remote path delivers TCP to the host (the gateway passes).
    let gateway_dst = SocketAddrV4::new(prefix.gateway(), declared);
    let control = in_netns(&link.netns.name, move || connect_to(gateway_dst).map(|_| ()));
    assert!(control.is_ok(), "the remote client reaches the host via the gateway: {control:?}");

    // WHEN the remote client connects to the steered intake (ingress = the veth),
    // THEN the lookup is dropped: only loopback ingress is steered.
    let remote = in_netns(&link.netns.name, move || connect_to(intake).map(|_| ()));
    assert!(
        matches!(&remote, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused),
        "device ingress to the prefix is refused even with an entry: {remote:?}",
    );
    listener.set_nonblocking(true).expect("listener non-blocking");
    assert!(accepted_nothing(&listener), "the steered listener accepted nothing remote");
    drop(wildcard);
}

// ---------------------------------------------------------------------------
// P4-S3 — close removes the entry in the same step; a freed slot is reusable
// (R5-22 close, D8a-CAP, K-L3; G-V8)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn closing_a_listener_frees_its_steering_slot_in_the_same_step() {
    let prefix = SteeringPrefix::acquire("vsock-steering-s3");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);
    let (_wildcard, declared) = wildcard_service();
    let intake = SocketAddrV4::new(prefix.workload(), declared);

    let first = intake_listener(prefix.workload(), 0);
    steering.steer(intake, first.as_fd()).expect("steer");
    exchange_both_ways(connect_to(intake).expect("steered -> reachable"), &first);

    // WHEN the listener closes, THEN the next connect is refused at once.
    drop(first);
    assert!(is_refused(&connect_to(intake)), "close removes the entry in the same step");

    // AND a fresh listener steered to the same intake is reachable at once.
    let second = intake_listener(prefix.workload(), 0);
    steering.steer(intake, second.as_fd()).expect("re-steer after close: the slot was freed");
    exchange_both_ways(connect_to(intake).expect("re-steered -> reachable"), &second);
}

/// Bounded capacity cycle. The full `GUEST_INTAKE_STEERING_MAX` (262,144)
/// fill needs one listening socket per entry — far beyond the test lane's
/// descriptor limit and memory — and over-capacity is a precondition the
/// owner's admission excludes (D8a-CAP: `steer` never fails for capacity), so
/// this cell proves the slot-recycling half at 256 entries over 4 addresses.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn closing_steered_listeners_frees_every_slot_for_new_listeners() {
    const ADDRESSES: u8 = 4;
    const PER_ADDRESS: u16 = 64;
    const DECLARED_BASE: u16 = 41_000;

    let prefix = SteeringPrefix::acquire("vsock-steering-cap");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);

    let intakes: Vec<(SocketAddrV4, Ipv4Addr, u16)> = (0..ADDRESSES)
        .flat_map(|a| {
            let addr = prefix.host(10 + a);
            (0..PER_ADDRESS).map(move |i| (SocketAddrV4::new(addr, DECLARED_BASE + i), addr, i))
        })
        .collect();

    let steer_all = || -> Vec<TcpListener> {
        intakes
            .iter()
            .map(|(intake, addr, index)| {
                let l = intake_listener(*addr, *index);
                steering.steer(*intake, l.as_fd()).expect("steer within capacity");
                l
            })
            .collect()
    };

    let generation = steer_all();
    let (sample, _, _) = intakes[intakes.len() / 2];
    exchange_both_ways(
        connect_to(sample).expect("sampled intake reachable"),
        &generation[intakes.len() / 2],
    );

    drop(generation);
    for (intake, _, _) in intakes.iter().step_by(37) {
        assert!(is_refused(&connect_to(*intake)), "closed listener's entry is gone: {intake}");
    }

    let regenerated = steer_all();
    for (i, (intake, _, _)) in intakes.iter().enumerate().step_by(37) {
        exchange_both_ways(
            connect_to(*intake).expect("re-steered intake reachable"),
            &regenerated[i],
        );
    }
}

// ---------------------------------------------------------------------------
// steer preconditions (IntakeOutsideGuestPrefix, IntakeAlreadySteered,
// UnsupportedSocket) — each refusal leaves the entry absent
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn an_intake_outside_the_prefix_or_at_the_gateway_is_refused() {
    let prefix = SteeringPrefix::acquire("vsock-steering-outside");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);

    // Outside the prefix (loopback).
    let outside = listening_tcp(Ipv4Addr::LOCALHOST);
    let intake = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41_500);
    let err = steering.steer(intake, outside.as_fd()).expect_err("outside the prefix");
    assert!(
        matches!(err, GuestSteeringError::IntakeOutsideGuestPrefix { intake: i } if i == intake),
        "outside -> IntakeOutsideGuestPrefix: {err:?}",
    );

    // The gateway.
    let at_gateway = listening_tcp(prefix.gateway());
    let intake = SocketAddrV4::new(prefix.gateway(), 41_501);
    let err = steering.steer(intake, at_gateway.as_fd()).expect_err("the gateway");
    assert!(
        matches!(err, GuestSteeringError::IntakeOutsideGuestPrefix { intake: i } if i == intake),
        "gateway -> IntakeOutsideGuestPrefix: {err:?}",
    );
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn a_socket_that_is_not_a_listener_bound_to_the_intake_address_is_refused() {
    let prefix = SteeringPrefix::acquire("vsock-steering-unsupported");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);
    let intake = SocketAddrV4::new(prefix.workload(), 41_600);

    let udp = UdpSocket::bind(SocketAddrV4::new(prefix.workload(), 0)).expect("UDP binds");
    let not_listening = bound_unlistening_tcp(prefix.workload());
    let other_address = listening_tcp(prefix.host(3));
    let wildcard = listening_tcp(Ipv4Addr::UNSPECIFIED);

    for (label, fd) in [
        ("UDP socket", udp.as_fd()),
        ("bound, not listening", not_listening.as_fd()),
        ("listening on another prefix address", other_address.as_fd()),
        ("listening on 0.0.0.0", wildcard.as_fd()),
    ] {
        let err = steering.steer(intake, fd).expect_err(label);
        assert!(matches!(err, GuestSteeringError::UnsupportedSocket), "{label}: {err:?}");
        assert!(is_refused(&connect_to(intake)), "{label}: the entry is absent");
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn an_intake_steered_to_another_open_listener_is_refused_without_change() {
    let prefix = SteeringPrefix::acquire("vsock-steering-already");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);
    let intake = SocketAddrV4::new(prefix.workload(), 41_700);

    let first = intake_listener(prefix.workload(), 0);
    steering.steer(intake, first.as_fd()).expect("first steer");
    let second = intake_listener(prefix.workload(), 1);

    // WHEN a second open listener is steered to the same intake,
    let err = steering.steer(intake, second.as_fd()).expect_err("already steered");
    assert!(
        matches!(err, GuestSteeringError::IntakeAlreadySteered { intake: i } if i == intake),
        "-> IntakeAlreadySteered: {err:?}",
    );
    // THEN nothing changed: the first listener still receives the connection.
    exchange_both_ways(connect_to(intake).expect("still steered to the first"), &first);

    // AND once the first closes, the second can be steered.
    drop(first);
    steering.steer(intake, second.as_fd()).expect("the freed intake is steerable");
    exchange_both_ways(connect_to(intake).expect("now steered to the second"), &second);
}

// ---------------------------------------------------------------------------
// P4-S4 — load, then swap: a first attach pins; a later boot swaps over the
// pinned link atomically and never detaches it (R5-24/R5-34, D8a-LOAD-SWAP,
// K-L9; G-V1)
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn a_first_attach_pins_a_link_that_outlives_the_handle() {
    let prefix = SteeringPrefix::acquire("vsock-steering-pin");
    let _pins = PinnedSteeringGuard::claim();
    let (wildcard, port) = wildcard_service();
    let dst = SocketAddrV4::new(prefix.workload(), port);

    // `load` attaches and pins nothing.
    let candidate = GuestPrefixSteering::load(prefix.prefix, prefix.gateway()).expect("load");
    assert!(pin_entries().is_empty(), "load pins nothing");
    let link = {
        let _steering = candidate.attach().expect("first attach");
        // A first attach pins one link.
        let link = pinned_link();
        assert_ne!(link.prog_id, 0, "the pinned link runs a program");
        link
    };

    // Dropping the handle (end of the block above) closes only this
    // process's descriptors: the pinned link keeps deciding the prefix.
    assert_eq!(pinned_link(), link, "the pin keeps the same link and program");
    assert!(is_refused(&connect_to(dst)), "the prefix stays steered after the handle drops");
    assert!(accepted_nothing(&wildcard), "the wildcard host service is never reached");
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 13 (steering load and swap)"]
fn boot_over_a_left_behind_pinned_link_swaps_atomically_and_never_detaches() {
    let prefix = SteeringPrefix::acquire("vsock-steering-swap");
    let _pins = PinnedSteeringGuard::claim();

    // GIVEN an earlier boot pinned a link and exited (its listeners closed —
    // K-L4: the earlier map holds no live socket when a later boot runs).
    let (_svc, declared) = wildcard_service();
    let intake = SocketAddrV4::new(prefix.workload(), declared);
    {
        let earlier = boot_steering(&prefix);
        let old_listener = intake_listener(prefix.workload(), 0);
        earlier.steer(intake, old_listener.as_fd()).expect("earlier steer");
        exchange_both_ways(connect_to(intake).expect("earlier steering serves"), &old_listener);
    }
    let before = pinned_link();

    // AND a wildcard host service the prefix must never reach, probed by a
    // connect loop running across the load and the swap.
    let (wildcard, port) = wildcard_service();
    let probe = SocketAddrV4::new(prefix.host(50), port);
    let stop = Arc::new(AtomicBool::new(false));
    let attempts = Arc::new(AtomicU64::new(0));
    let reached = Arc::new(AtomicU64::new(0));
    let loop_handle = {
        let (stop, attempts, reached) =
            (Arc::clone(&stop), Arc::clone(&attempts), Arc::clone(&reached));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if connect_to(probe).is_ok() {
                    reached.fetch_add(1, Ordering::AcqRel);
                }
                attempts.fetch_add(1, Ordering::AcqRel);
            }
        })
    };

    // WHEN a new boot loads its program and swaps it into the pinned link,
    let candidate = GuestPrefixSteering::load(prefix.prefix, prefix.gateway()).expect("load");
    let steering = candidate.attach().expect("attach: BPF_LINK_UPDATE over the pinned link");
    let at_swap = attempts.load(Ordering::Acquire);
    while attempts.load(Ordering::Acquire) < at_swap + 200 {
        std::thread::yield_now();
    }
    stop.store(true, Ordering::Release);
    loop_handle.join().expect("connect loop completes");

    // THEN the link was never detached nor replaced: same link, new program,
    let after = pinned_link();
    assert_eq!(after.link_id, before.link_id, "the pinned link is never detached (K-L9)");
    assert_ne!(after.prog_id, before.prog_id, "the link now runs this boot's program");

    // AND no connect across the load and swap reached the wildcard service,
    assert!(attempts.load(Ordering::Acquire) > 200, "the loop ran across the swap");
    assert_eq!(reached.load(Ordering::Acquire), 0, "no prefix connect reached a host socket");
    assert!(accepted_nothing(&wildcard), "the wildcard host service accepted nothing");

    // AND the swapped-in program steers with its own map.
    let listener = intake_listener(prefix.workload(), 1);
    steering.steer(intake, listener.as_fd()).expect("steer after the swap");
    exchange_both_ways(connect_to(intake).expect("steered after the swap"), &listener);
}

// ---------------------------------------------------------------------------
// P4-S5 — a host-local client whose source lies in the prefix completes to a
// steered intake (R5-43, PREFIX-REPLY steering half; G-V8). The constant nft
// reply-accept rules are the owner's (control-plane) and are not present here.
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER vsock new-step 9 (guest-prefix steering)"]
fn a_host_local_client_whose_source_lies_in_the_prefix_completes_to_a_steered_intake() {
    let prefix = SteeringPrefix::acquire("vsock-steering-s5");
    let _pins = PinnedSteeringGuard::claim();
    let steering = boot_steering(&prefix);
    let intake = SocketAddrV4::new(prefix.workload(), 41_800);
    let listener = intake_listener(prefix.workload(), 0);
    steering.steer(intake, listener.as_fd()).expect("steer");

    // WHEN a client sourced at the gateway connects to the steered intake,
    let client = connect_from(prefix.gateway(), intake).expect("in-prefix-source client completes");
    // THEN it exchanges both ways (the replies to the in-prefix source pass).
    exchange_both_ways(client, &listener);

    // AND a new connect to an unsteered port of the prefix is still refused.
    let fresh = connect_from(prefix.gateway(), SocketAddrV4::new(prefix.workload(), 41_801));
    assert!(
        matches!(&fresh, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused),
        "an unsteered prefix port is refused: {fresh:?}",
    );
}
