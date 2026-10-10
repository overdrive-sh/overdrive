//! P5-S8 — the host-internal output rules of the production shared firewall
//! table reject marked owner sockets, structurally (D26, D26-PINS F-1..F-6).
//! Tier-3, Lima + root, `host-kernel-shared`.
//!
//! netns-density-295 (`docs/feature/netns-density-295/feature-delta.md`
//! § "Owner and provisioner" → *Kernel enforcement (D26, D26-PINS,
//! ADR-0167)*; *Composition* boot step 8; DG-9). Scenario:
//! `distill/test-scenarios-p5-egress-unframe-hostinternal.md` P5-S8;
//! obligations R5-48 and the kernel-rule half of R5-7; gate G-V6. Anchored on
//! P-47 (V-21, `spike/findings-v21.md`).
//!
//! # Production path
//!
//! The shared nft table (`MTLS_SHARED_NFT_TABLE`, family `ip`) is written by
//! its owner, the mTLS worker: every body boots it through
//! `MtlsInterceptWorker::start_shared_owner` over the production
//! `HostMtlsIntercept`, and the replacement body replaces it through the same
//! call on a successor worker (a `serve` restart: the sealed shutdown retains
//! the table, the next boot replaces it). No body writes an nft rule, a set
//! or a chain itself. A marked userspace socket (`SO_MARK =
//! OWNER_EGRESS_MARK`, DG-9) stands in for the owner's directly connected host
//! socket H; the forwarder-redirect transmit on H (R5-7, F-4) is P6's walking
//! skeleton through `serve` + `deploy`.
//!
//! # Topology
//!
//! A hand-built veth pair to one test namespace on a [`TestCidrLease`] `/24`
//! (route-disjoint from production's `10.99.0.0/16`): the host side carries
//! the lease gateway (a host interface address, a *local* destination, with
//! its subnet-directed broadcast); the namespace side carries the lease
//! workload address (a *neighbour*, unicast) and the fixture-only link-local
//! service address `169.254.250.2/32`, reached over a host `/32` route on the
//! veth (a unicast route, so only the explicit `169.254.0.0/16` match can
//! reject it). The fixture models host services and peers; it installs no
//! effect production installs.
//!
//! # Not here
//!
//! - `gateway:53` accepted and the guest prefix's broadcast address rejected
//!   need the shared `local 100.95.0.0/16 dev lo src 100.95.0.1` route, which
//!   only `overdrive-control-plane`'s `converge_shared` writes (D8a); a
//!   worker-crate body would have to hand-install it.
//!
//! # Lane and isolation
//!
//! Integration lane (`integration-tests`), root: real nft, real marked
//! sockets, a real namespace. The whole `overdrive-worker` integration binary
//! is in the `host-kernel-shared` nextest group; every body also holds the
//! cross-process [`KernelStateLock`] its siblings hold, and an
//! [`EnforceSandbox`] scrubs the node-global intercept state on entry and on
//! drop. Evidence is append-only: one stderr line per observation, stamped
//! with the scenario cell, the phase and the elapsed time.
//!
//! RED until DELIVER lands the D26 output rules: against today's table the
//! marked dials are not rejected.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "Tier-3 bodies: evidence goes to stderr; fixture preconditions must panic with \
              informative messages; the libc socket calls take FFI-width integers"
)]

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::mtls_mark::{
    MTLS_LEG_S_DIAL_MARK, MTLS_SHARED_NFT_TABLE, MTLS_TPROXY_MARK, OWNER_EGRESS_MARK,
};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use super::inbound_tproxy_harness::{KernelStateLock, build_worker, is_root, record_uname};

/// The bound within which a rejected dial must fail (F-3: never a hang).
const VERDICT_BOUND: Duration = Duration::from_secs(1);
/// How long a service is given to receive a datagram that must not arrive.
const SETTLE: Duration = Duration::from_millis(300);

const NS: &str = "nd295-hostint";
const VETH_HOST: &str = "nd295hiH";
const VETH_NS: &str = "nd295hiW";
const LEASE_NAME: &str = "vsock_host_internal_enforce";
/// The fixture-only link-local service address in the test namespace.
const LINK_LOCAL_SERVICE: Ipv4Addr = Ipv4Addr::new(169, 254, 250, 2);
/// An off-host unicast destination (TEST-NET-1), via the default route.
const OFF_HOST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 1), 9);

// ---------------------------------------------------------------------------
// Append-only evidence
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Evidence {
    cell: &'static str,
    started: Instant,
}

impl Evidence {
    fn begin(cell: &'static str) -> Self {
        assert!(is_root(), "{cell}: P5-S8 Tier-3 evidence requires root and CAP_NET_ADMIN");
        record_uname(cell);
        Self { cell, started: Instant::now() }
    }

    fn record(&self, phase: &str, detail: impl std::fmt::Display) {
        eprintln!(
            "[P5-S8][{}][{phase}][+{}ms] {detail}",
            self.cell,
            self.started.elapsed().as_millis()
        );
    }
}

/// Run one host command, record it, and report whether it succeeded.
fn run_recorded(evidence: Evidence, phase: &str, program: &str, args: &[&str]) -> bool {
    match Command::new(program).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).output() {
        Ok(output) => {
            evidence.record(
                phase,
                format_args!(
                    "{program} {args:?} -> {:?} stderr={:?}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            );
            output.status.success()
        }
        Err(error) => {
            evidence.record(phase, format_args!("{program} {args:?} could not spawn: {error}"));
            false
        }
    }
}

/// A fixture step that must succeed.
fn fixture(evidence: Evidence, program: &str, args: &[&str]) {
    assert!(
        run_recorded(evidence, "fixture", program, args),
        "[{}] fixture step `{program} {args:?}` must succeed",
        evidence.cell
    );
}

/// The JSON listing of the production shared table, or `None` when absent.
fn shared_table_json() -> Option<Vec<u8>> {
    let output = Command::new("nft")
        .args(["-j", "list", "table", "ip", MTLS_SHARED_NFT_TABLE])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn an nft JSON table listing");
    output.status.success().then_some(output.stdout)
}

// ---------------------------------------------------------------------------
// Node-global sandbox and the hand-built topology
// ---------------------------------------------------------------------------

/// Scrubs every node-global object these bodies can leave — both
/// `overdrive-mtls` families, the guard table, the `fwmark 0x1 lookup 100`
/// rules, table 100's local route, and the test topology — on entry and on
/// drop, holding the cross-process kernel-state lock throughout. Field order:
/// the scrub runs in `drop`, then the lease and the lock are released.
struct EnforceSandbox {
    evidence: Evidence,
    lease: TestCidrLease,
    _lock: KernelStateLock,
}

impl EnforceSandbox {
    fn provision(evidence: Evidence) -> Self {
        let lock = KernelStateLock::acquire();
        let lease = TestCidrLease::acquire(LEASE_NAME).expect("acquire the P5-S8 CIDR lease");
        scrub(evidence, "setup-scrub");
        let gateway = format!("{}/{}", lease.host_gateway(), lease.prefix_len());
        let workload = format!("{}/{}", lease.workload_addr(), lease.prefix_len());
        let link_local = format!("{LINK_LOCAL_SERVICE}/32");
        let host_gateway = lease.host_gateway().to_string();
        fixture(evidence, "ip", &["netns", "add", NS]);
        fixture(
            evidence,
            "ip",
            &["link", "add", VETH_HOST, "type", "veth", "peer", "name", VETH_NS],
        );
        fixture(evidence, "ip", &["link", "set", VETH_NS, "netns", NS]);
        fixture(evidence, "ip", &["addr", "add", &gateway, "brd", "+", "dev", VETH_HOST]);
        fixture(evidence, "ip", &["link", "set", VETH_HOST, "up"]);
        fixture(evidence, "ip", &["netns", "exec", NS, "ip", "link", "set", "lo", "up"]);
        fixture(
            evidence,
            "ip",
            &["netns", "exec", NS, "ip", "addr", "add", &workload, "dev", VETH_NS],
        );
        fixture(
            evidence,
            "ip",
            &["netns", "exec", NS, "ip", "addr", "add", &link_local, "dev", VETH_NS],
        );
        fixture(evidence, "ip", &["netns", "exec", NS, "ip", "link", "set", VETH_NS, "up"]);
        fixture(
            evidence,
            "ip",
            &["netns", "exec", NS, "ip", "route", "add", "default", "via", &host_gateway],
        );
        fixture(evidence, "ip", &["route", "add", &link_local, "dev", VETH_HOST]);
        Self { evidence, lease, _lock: lock }
    }

    /// A host interface address (routed `local`).
    const fn host_address(&self) -> Ipv4Addr {
        self.lease.host_gateway()
    }

    /// The neighbour in the test namespace (routed unicast).
    const fn neighbour(&self) -> Ipv4Addr {
        self.lease.workload_addr()
    }

    /// The host interface's subnet-directed broadcast address.
    fn subnet_broadcast(&self) -> Ipv4Addr {
        let host_bits = u32::MAX >> self.lease.prefix_len();
        Ipv4Addr::from(u32::from(self.lease.network()) | host_bits)
    }
}

impl Drop for EnforceSandbox {
    fn drop(&mut self) {
        scrub(self.evidence, "cleanup-scrub");
    }
}

fn scrub(evidence: Evidence, phase: &str) {
    run_recorded(evidence, phase, "ip", &["link", "del", VETH_HOST]);
    run_recorded(evidence, phase, "ip", &["netns", "del", NS]);
    let link_local = format!("{LINK_LOCAL_SERVICE}/32");
    run_recorded(evidence, phase, "ip", &["route", "del", &link_local]);
    for (family, table) in [
        ("ip", MTLS_SHARED_NFT_TABLE),
        ("bridge", MTLS_SHARED_NFT_TABLE),
        ("ip", "overdrive-mtls-guard"),
    ] {
        run_recorded(evidence, phase, "nft", &["delete", "table", family, table]);
    }
    for _ in 0..64 {
        if !run_recorded(evidence, phase, "ip", &["rule", "del", "fwmark", "0x1", "lookup", "100"])
        {
            break;
        }
    }
    run_recorded(
        evidence,
        phase,
        "ip",
        &["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"],
    );
}

/// Boot the production shared table through its owner.
async fn boot_shared_table(evidence: Evidence) -> Arc<MtlsInterceptWorker> {
    let worker = build_worker();
    worker.start_shared_owner().await.expect("the shared owner publishes on a scrubbed kernel");
    assert!(
        shared_table_json().is_some(),
        "precondition: start_shared_owner wrote the `ip {MTLS_SHARED_NFT_TABLE}` table"
    );
    evidence.record("boot", "the production shared table is present");
    worker
}

async fn shutdown(worker: &Arc<MtlsInterceptWorker>) {
    worker.shutdown_owner().await.expect("the owner shuts down cleanly");
}

// ---------------------------------------------------------------------------
// Host services (in the root namespace or the test namespace)
// ---------------------------------------------------------------------------

/// Enter the test namespace on the calling thread (the thread's sockets are
/// created there).
fn enter_netns(name: &str) {
    let file = std::fs::File::open(format!("/var/run/netns/{name}"))
        .unwrap_or_else(|error| panic!("open netns {name}: {error}"));
    // SAFETY: a valid netns fd; CLONE_NEWNET moves only this thread.
    let rc = unsafe { libc::setns(file.as_raw_fd(), libc::CLONE_NEWNET) };
    assert_eq!(rc, 0, "setns({name}): {}", io::Error::last_os_error());
}

/// A host TCP service that accepts every connection and counts what reaches
/// it.
struct TcpService {
    addr: SocketAddrV4,
    accepted: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TcpService {
    fn bind(netns: Option<&'static str>, addr: SocketAddrV4) -> Self {
        let accepted = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (bound_tx, bound_rx) = mpsc::channel();
        let thread = {
            let (accepted, bytes, stop) =
                (Arc::clone(&accepted), Arc::clone(&bytes), Arc::clone(&stop));
            std::thread::spawn(move || {
                if let Some(name) = netns {
                    enter_netns(name);
                }
                let listener = TcpListener::bind(addr).expect("bind the host TCP service");
                listener.set_nonblocking(true).expect("non-blocking service listener");
                let local = local_v4(listener.local_addr().expect("service address"));
                bound_tx.send(local).expect("report the bound address");
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _peer)) => {
                            accepted.fetch_add(1, Ordering::SeqCst);
                            stream.set_nonblocking(false).expect("blocking accepted stream");
                            stream
                                .set_read_timeout(Some(Duration::from_millis(500)))
                                .expect("read timeout");
                            let mut buf = [0_u8; 512];
                            while let Ok(n) = io::Read::read(&mut stream, &mut buf) {
                                if n == 0 {
                                    break;
                                }
                                bytes.fetch_add(n as u64, Ordering::SeqCst);
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("service accept: {error}"),
                    }
                }
            })
        };
        let addr = bound_rx.recv().expect("the TCP service binds");
        Self { addr, accepted, bytes, stop, thread: Some(thread) }
    }

    fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::SeqCst)
    }

    fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::SeqCst)
    }
}

impl Drop for TcpService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

/// A host UDP service that counts the datagrams reaching it, echoing each when
/// `echo` is set.
struct UdpService {
    addr: SocketAddrV4,
    datagrams: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl UdpService {
    fn bind(addr: SocketAddrV4, echo: bool) -> Self {
        let socket = UdpSocket::bind(addr).expect("bind the host UDP service");
        socket.set_read_timeout(Some(Duration::from_millis(20))).expect("read timeout");
        if addr.ip().is_unspecified() {
            socket.set_broadcast(true).expect("receive broadcasts");
        }
        let bound = local_v4(socket.local_addr().expect("service address"));
        let datagrams = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (datagrams, stop) = (Arc::clone(&datagrams), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut buf = [0_u8; 512];
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((n, peer)) = socket.recv_from(&mut buf) {
                        datagrams.fetch_add(1, Ordering::SeqCst);
                        if echo {
                            let _sent = socket.send_to(&buf[..n], peer);
                        }
                    }
                }
            })
        };
        Self { addr: bound, datagrams, stop, thread: Some(thread) }
    }

    fn datagrams(&self) -> u64 {
        self.datagrams.load(Ordering::SeqCst)
    }
}

impl Drop for UdpService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

fn local_v4(addr: std::net::SocketAddr) -> SocketAddrV4 {
    match addr {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(v6) => panic!("an IPv4 fixture bound {v6}"),
    }
}

// ---------------------------------------------------------------------------
// Marked sockets (the owner's host socket H)
// ---------------------------------------------------------------------------

const fn sockaddr(addr: SocketAddrV4) -> libc::sockaddr_in {
    // SAFETY: sockaddr_in is plain old data; zero is a valid starting value.
    let mut raw: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    raw.sin_family = libc::AF_INET as libc::sa_family_t;
    raw.sin_port = addr.port().to_be();
    raw.sin_addr.s_addr = u32::from_ne_bytes(addr.ip().octets());
    raw
}

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn set_int_opt(
    fd: &OwnedFd,
    level: libc::c_int,
    name: libc::c_int,
    value: libc::c_int,
) -> io::Result<()> {
    // SAFETY: a live socket fd and a correctly sized int option value.
    check(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            level,
            name,
            std::ptr::from_ref(&value).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
}

/// How a test socket is opened.
#[derive(Clone, Copy, Default)]
struct SocketOpts {
    /// `SO_MARK`, set before any connect or transmit.
    mark: Option<u32>,
    /// `SO_BROADCAST`.
    broadcast: bool,
    /// `SO_REUSEADDR` and a bind to this source.
    source: Option<SocketAddrV4>,
}

fn open_socket(kind: libc::c_int, opts: SocketOpts) -> io::Result<OwnedFd> {
    // SAFETY: a fresh AF_INET socket; ownership passes to OwnedFd.
    let raw = unsafe { libc::socket(libc::AF_INET, kind | libc::SOCK_CLOEXEC, 0) };
    check(raw)?;
    // SAFETY: `raw` is a live descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if let Some(mark) = opts.mark {
        set_int_opt(&fd, libc::SOL_SOCKET, libc::SO_MARK, mark as libc::c_int)?;
    }
    if opts.broadcast {
        set_int_opt(&fd, libc::SOL_SOCKET, libc::SO_BROADCAST, 1)?;
    }
    if let Some(source) = opts.source {
        set_int_opt(&fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, 1)?;
        let raw_source = sockaddr(source);
        // SAFETY: a live socket and a correctly sized sockaddr_in.
        check(unsafe {
            libc::bind(
                fd.as_raw_fd(),
                std::ptr::from_ref(&raw_source).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        })?;
    }
    Ok(fd)
}

fn connect_fd(fd: &OwnedFd, dst: SocketAddrV4) -> io::Result<()> {
    let raw = sockaddr(dst);
    // SAFETY: a live socket and a correctly sized sockaddr_in.
    check(unsafe {
        libc::connect(
            fd.as_raw_fd(),
            std::ptr::from_ref(&raw).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    })
}

/// A TCP connect from a socket opened with `opts`, bounded by `bound`: the
/// outcome and how long it took. A connect still pending at `bound` is
/// `TimedOut` (a hang).
fn tcp_connect(
    dst: SocketAddrV4,
    opts: SocketOpts,
    bound: Duration,
) -> (io::Result<OwnedFd>, Duration) {
    let started = Instant::now();
    let outcome = (|| {
        let fd = open_socket(libc::SOCK_STREAM | libc::SOCK_NONBLOCK, opts)?;
        match connect_fd(&fd, dst) {
            Ok(()) => return Ok(fd),
            Err(error) if error.raw_os_error() == Some(libc::EINPROGRESS) => {}
            Err(error) => return Err(error),
        }
        let mut pfd = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&raw mut pfd, 1, bound.as_millis() as libc::c_int) };
        check(ready)?;
        if ready == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        let mut error: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: a live socket; SO_ERROR writes one int.
        check(unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                std::ptr::from_mut(&mut error).cast(),
                &raw mut len,
            )
        })?;
        if error == 0 { Ok(fd) } else { Err(io::Error::from_raw_os_error(error)) }
    })();
    (outcome, started.elapsed())
}

/// A UDP socket opened with `opts`, connected to `dst`, transmitting `payload`
/// once: the outcome of the connect-then-first-transmit.
fn udp_first_transmit(dst: SocketAddrV4, opts: SocketOpts, payload: &[u8]) -> io::Result<OwnedFd> {
    let fd = open_socket(libc::SOCK_DGRAM, opts)?;
    connect_fd(&fd, dst)?;
    // SAFETY: a live connected socket and a valid buffer.
    let sent = unsafe { libc::send(fd.as_raw_fd(), payload.as_ptr().cast(), payload.len(), 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn owner_marked() -> SocketOpts {
    SocketOpts { mark: Some(OWNER_EGRESS_MARK), ..SocketOpts::default() }
}

/// Assert a TCP outcome is the plain-`reject` verdict: `ECONNREFUSED`, fast.
fn assert_tcp_refused(cell: &str, outcome: &(io::Result<OwnedFd>, Duration)) {
    let (result, elapsed) = outcome;
    assert!(
        matches!(result, Err(error) if error.raw_os_error() == Some(libc::ECONNREFUSED)),
        "{cell}: a marked TCP connect is refused ECONNREFUSED, got {result:?}",
    );
    assert!(
        *elapsed < VERDICT_BOUND,
        "{cell}: the refusal is immediate, never a hang ({elapsed:?})"
    );
}

/// Assert a UDP outcome is the plain-`reject` verdict: `EPERM` on the first
/// transmit.
fn assert_udp_rejected(cell: &str, result: &io::Result<OwnedFd>) {
    assert!(
        matches!(result, Err(error) if error.raw_os_error() == Some(libc::EPERM)),
        "{cell}: a marked UDP first transmit fails EPERM, got {result:?}",
    );
}

// ---------------------------------------------------------------------------
// F-3 / 127.0.0.0/8 — marked TCP to a loopback service: ECONNREFUSED, no hang
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_tcp_connect_to_a_loopback_service_is_refused_econnrefused_without_hang() {
    let evidence = Evidence::begin("F-3-tcp-loopback");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    // 127.20.0.1: inside 127.0.0.0/8, not 127.0.0.1.
    let service = TcpService::bind(None, SocketAddrV4::new(Ipv4Addr::new(127, 20, 0, 1), 0));
    let table_before = shared_table_json();

    let outcome = tcp_connect(service.addr, owner_marked(), VERDICT_BOUND * 2);
    evidence.record("dial", format_args!("{:?} in {:?}", outcome.0, outcome.1));

    assert_tcp_refused("F-3-tcp-loopback", &outcome);
    assert_eq!(service.accepted(), 0, "the loopback service accepted nothing");
    assert_eq!(shared_table_json(), table_before, "the dial left the production table unchanged");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// fib daddr type local — marked TCP to a host interface address
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_tcp_connect_to_a_host_interface_address_is_refused() {
    let evidence = Evidence::begin("local-host-address");
    let sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(None, SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
    let dst = SocketAddrV4::new(sandbox.host_address(), service.addr.port());

    let outcome = tcp_connect(dst, owner_marked(), VERDICT_BOUND * 2);
    evidence.record("dial", format_args!("{dst}: {:?} in {:?}", outcome.0, outcome.1));

    assert_tcp_refused("local-host-address", &outcome);
    assert_eq!(service.accepted(), 0, "the wildcard host service accepted nothing");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-3 — marked UDP to a local service: EPERM on the first transmit
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_first_transmit_to_a_local_service_fails_eperm() {
    let evidence = Evidence::begin("F-3-udp-local");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = UdpService::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), false);

    let result = udp_first_transmit(service.addr, owner_marked(), b"nd295-f3");
    evidence.record("transmit", format_args!("{result:?}"));
    std::thread::sleep(SETTLE);

    assert_udp_rejected("F-3-udp-local", &result);
    assert_eq!(service.datagrams(), 0, "the local UDP service received nothing");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-1 — the D26 rules precede every accept: an established tuple is no bypass
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_socket_reusing_an_unmarked_flows_source_port_is_still_rejected() {
    let evidence = Evidence::begin("F-1-reuse");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = UdpService::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0), true);

    // An earlier UNMARKED flow from a fixed source: request and echoed reply,
    // so conntrack holds the tuple as established.
    let source = SocketAddrV4::new(Ipv4Addr::LOCALHOST, {
        let probe = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("pick a source port");
        let picked = local_v4(probe.local_addr().expect("probe address"));
        picked.port()
    });
    let unmarked = udp_first_transmit(
        service.addr,
        SocketOpts { source: Some(source), ..SocketOpts::default() },
        b"nd295-unmarked",
    )
    .expect("precondition: the unmarked flow transmits");
    // SAFETY: the fd is owned and live; a short receive timeout bounds the wait.
    let echoed = unsafe {
        let timeout = libc::timeval { tv_sec: 1, tv_usec: 0 };
        libc::setsockopt(
            unmarked.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&timeout).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
        let mut buf = [0_u8; 64];
        libc::recv(unmarked.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0)
    };
    assert!(echoed > 0, "precondition: the unmarked flow's reply arrived (established tuple)");
    drop(unmarked);
    let before = service.datagrams();
    assert_eq!(before, 1, "precondition: the service received the unmarked datagram");

    // A MARKED socket on the same source port, to the same destination.
    let result = udp_first_transmit(
        service.addr,
        SocketOpts { mark: Some(OWNER_EGRESS_MARK), source: Some(source), ..SocketOpts::default() },
        b"nd295-marked",
    );
    evidence.record("transmit", format_args!("marked reuse of {source}: {result:?}"));
    std::thread::sleep(SETTLE);

    assert_udp_rejected("F-1-reuse", &result);
    assert_eq!(service.datagrams(), before, "the marked reuse delivered nothing more");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-5 — fib daddr type != unicast, with and without SO_BROADCAST
// ---------------------------------------------------------------------------

/// A marked transmit to a non-unicast `dst` reaches no host service: with
/// `SO_BROADCAST` the rule rejects it `EPERM`; without it the transmit fails
/// too. The wildcard host service and the neighbour's service count nothing.
async fn non_unicast_reaches_no_service(
    cell: &'static str,
    dst_of: impl Fn(&EnforceSandbox) -> Ipv4Addr,
) {
    let evidence = Evidence::begin(cell);
    let sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let host_service = UdpService::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), false);
    let dst = SocketAddrV4::new(dst_of(&sandbox), host_service.addr.port());

    for broadcast in [true, false] {
        let result = udp_first_transmit(
            dst,
            SocketOpts { mark: Some(OWNER_EGRESS_MARK), broadcast, ..SocketOpts::default() },
            b"nd295-f5",
        );
        evidence.record("transmit", format_args!("{dst} SO_BROADCAST={broadcast}: {result:?}"));
        std::thread::sleep(SETTLE);
        if broadcast {
            assert_udp_rejected(cell, &result);
        } else {
            assert!(
                result.is_err(),
                "{cell}: without SO_BROADCAST the transmit fails too, got {result:?}"
            );
        }
        assert_eq!(
            host_service.datagrams(),
            0,
            "{cell}: the host service received nothing (SO_BROADCAST={broadcast})"
        );
    }
    shutdown(&worker).await;
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_transmit_to_a_subnet_directed_broadcast_reaches_no_service() {
    non_unicast_reaches_no_service("F-5-subnet-broadcast", EnforceSandbox::subnet_broadcast).await;
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_transmit_to_the_limited_broadcast_reaches_no_service() {
    non_unicast_reaches_no_service("F-5-limited-broadcast", |_| Ipv4Addr::BROADCAST).await;
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_transmit_to_the_all_hosts_multicast_group_is_rejected_eperm() {
    let evidence = Evidence::begin("F-5-multicast");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let host_service = UdpService::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), false);
    let dst = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 1), host_service.addr.port());

    let result = udp_first_transmit(dst, owner_marked(), b"nd295-f5-mcast");
    evidence.record("transmit", format_args!("{dst}: {result:?}"));
    std::thread::sleep(SETTLE);

    assert_udp_rejected("F-5-multicast", &result);
    assert_eq!(host_service.datagrams(), 0, "the host service received nothing");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// 169.254.0.0/16 — a link-local service over a unicast route is refused
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_tcp_connect_to_a_link_local_service_is_refused() {
    let evidence = Evidence::begin("link-local");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(Some(NS), SocketAddrV4::new(LINK_LOCAL_SERVICE, 0));
    // Control: the unmarked dial reaches it (the route is unicast and live).
    let (unmarked, _elapsed) = tcp_connect(service.addr, SocketOpts::default(), VERDICT_BOUND * 2);
    assert!(
        unmarked.is_ok(),
        "precondition: an unmarked dial reaches the link-local service: {unmarked:?}"
    );
    drop(unmarked);
    let before = service.accepted();

    let outcome = tcp_connect(service.addr, owner_marked(), VERDICT_BOUND * 2);
    evidence.record("dial", format_args!("{}: {:?} in {:?}", service.addr, outcome.0, outcome.1));

    assert_tcp_refused("link-local", &outcome);
    std::thread::sleep(SETTLE);
    assert_eq!(service.accepted(), before, "the link-local service accepted no marked connection");
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-5 controls — unicast stays reachable for marked sockets
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_tcp_connect_to_a_neighbour_stays_reachable() {
    let evidence = Evidence::begin("F-5-neighbour");
    let sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(Some(NS), SocketAddrV4::new(sandbox.neighbour(), 0));

    let (result, elapsed) = tcp_connect(service.addr, owner_marked(), VERDICT_BOUND * 2);
    evidence.record("dial", format_args!("{}: {result:?} in {elapsed:?}", service.addr));
    let fd = result.expect("a marked connect to a unicast neighbour is not rejected");
    let payload = b"nd295-neighbour";
    // SAFETY: a live connected socket and a valid buffer.
    let sent = unsafe { libc::send(fd.as_raw_fd(), payload.as_ptr().cast(), payload.len(), 0) };
    assert_eq!(sent, payload.len() as isize, "the payload is sent");
    drop(fd);
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.bytes() < payload.len() as u64 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        service.bytes(),
        payload.len() as u64,
        "the neighbour received the payload byte-exact"
    );
    shutdown(&worker).await;
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_udp_transmit_to_an_off_host_address_is_not_rejected() {
    let evidence = Evidence::begin("F-5-off-host");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;

    let result = udp_first_transmit(OFF_HOST, owner_marked(), b"nd295-off-host");
    evidence.record("transmit", format_args!("{OFF_HOST}: {result:?}"));

    assert!(
        result.is_ok(),
        "a marked transmit to an off-host unicast address leaves the host: {result:?}"
    );
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-6 — the owner bit under its mask; leg-S and TPROXY marks unaffected
// ---------------------------------------------------------------------------

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_packet_carrying_the_owner_bit_among_other_bits_is_refused() {
    let evidence = Evidence::begin("F-6-masked");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(None, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let mark = OWNER_EGRESS_MARK | 0x0F00_0000;

    let outcome = tcp_connect(
        service.addr,
        SocketOpts { mark: Some(mark), ..SocketOpts::default() },
        VERDICT_BOUND * 2,
    );
    evidence.record("dial", format_args!("mark {mark:#x}: {:?} in {:?}", outcome.0, outcome.1));

    assert_tcp_refused("F-6-masked", &outcome);
    assert_eq!(service.accepted(), 0, "the local service accepted nothing");
    shutdown(&worker).await;
}

/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service() {
    let evidence = Evidence::begin("F-6-other-marks");
    let _sandbox = EnforceSandbox::provision(evidence);
    let worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(None, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));

    for (name, mark) in
        [("MTLS_LEG_S_DIAL_MARK", MTLS_LEG_S_DIAL_MARK), ("MTLS_TPROXY_MARK", MTLS_TPROXY_MARK)]
    {
        let before = service.accepted();
        let (result, elapsed) = tcp_connect(
            service.addr,
            SocketOpts { mark: Some(mark), ..SocketOpts::default() },
            VERDICT_BOUND * 2,
        );
        evidence.record("dial", format_args!("{name} {mark:#x}: {result:?} in {elapsed:?}"));
        assert!(result.is_ok(), "{name}: the D26 rules do not match it: {result:?}");
        drop(result);
        let deadline = Instant::now() + VERDICT_BOUND;
        while service.accepted() == before && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(service.accepted(), before + 1, "{name}: the local service accepted it");
    }
    shutdown(&worker).await;
}

// ---------------------------------------------------------------------------
// F-2 — the table is replaced in one transaction: a storm leaks nothing
// ---------------------------------------------------------------------------

/// A marked-connect storm on its own thread.
struct Storm {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<(u64, u64)>,
}

impl Storm {
    fn start(dst: SocketAddrV4) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let (mut attempts, mut connected) = (0_u64, 0_u64);
                while !stop.load(Ordering::Relaxed) {
                    attempts += 1;
                    if tcp_connect(dst, owner_marked(), Duration::from_millis(200)).0.is_ok() {
                        connected += 1;
                    }
                }
                (attempts, connected)
            })
        };
        Self { stop, thread }
    }

    /// `(attempts, connects that reached the service)`.
    fn stop(self) -> (u64, u64) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().expect("the storm thread ends")
    }
}

/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
#[ignore = "pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)"]
async fn a_marked_connect_storm_across_repeated_table_replacements_reaches_no_local_service() {
    const RESTARTS: usize = 12;
    let evidence = Evidence::begin("F-2-replacement");
    let _sandbox = EnforceSandbox::provision(evidence);
    let mut worker = boot_shared_table(evidence).await;
    let service = TcpService::bind(None, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let mut table = shared_table_json().expect("the production table is present");

    let storm = Storm::start(service.addr);
    let mut replacements = 0_usize;
    for restart in 0..RESTARTS {
        shutdown(&worker).await;
        let successor = build_worker();
        successor
            .start_shared_owner()
            .await
            .expect("a successor owner replaces the retained table");
        let replaced = shared_table_json().expect("the replaced table is present");
        if replaced != table {
            replacements += 1;
        }
        evidence
            .record("replace", format_args!("restart {restart}: replaced={}", replaced != table));
        table = replaced;
        worker = successor;
    }
    let (attempts, connected) = storm.stop();
    evidence.record("storm", format_args!("attempts={attempts} connected={connected}"));

    assert!(
        replacements >= RESTARTS - 1,
        "precondition: the table was replaced on restart ({replacements})"
    );
    assert!(attempts > 0, "precondition: the storm dialled during the replacements");
    assert_eq!(connected, 0, "no marked connect reached the local service across the replacements");
    assert_eq!(service.accepted(), 0, "the local service accepted nothing");
    shutdown(&worker).await;
}
