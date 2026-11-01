//! D-9 — the leg-F repair's kernel effect on the real `HostMtlsIntercept`
//! (`leg_f_repair_rebinds_transparent_listener`). Tier-3, Lima + root,
//! `host-kernel-shared`. GH #295 (netns-density-295).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § *Roadmap
//! impact* row 10-02 ("Fault entry points (D-9)", "What ends leg F's accept
//! task in production", "The cell's evidence", "Order within the cell") and
//! § *Validation plan* V-8. Leg-F loss has no production-reachable external
//! stimulus on the appliance and no test hook is added to `run_server`; the
//! recovery protocol is proven in the seeded-sim composed rig, and this cell
//! proves the kernel effect the recovery relies on.
//!
//! # Why a test binary of its own
//!
//! The stimulus lowers this process's `RLIMIT_NOFILE`, a process-wide
//! limit. The cell is the only test of this binary, so nothing else in the
//! process (another test's fixture, a parallel runtime) is starved by it,
//! whichever runner executes it.
//!
//! # The stimulus — what ends leg F's accept task in production
//!
//! An `accept(2)` error on leg F's listener: `shared_listener_task` returns
//! the error and the task ends `TaskFailed { leg: F }`. The reachable errors
//! fail before the connection is dequeued, so the listener stays `LISTEN`
//! with the connection still queued: descriptor exhaustion — `EMFILE` at
//! `get_unused_fd_flags` (`include/linux/file.h:198-201 @ v7.0`), reached
//! through `FD_ADD` at `net/socket.c:2022 @ v7.0` before `do_accept`. The
//! cell lowers its own soft `RLIMIT_NOFILE` to its open-descriptor count and
//! queues one connection to leg F: the worker's accept fails `EMFILE`.
//!
//! **Order (round 11).** Every descriptor the stimulus needs exists before
//! the limit is lowered: the client socket is created inside the fixture's
//! workload network namespace first (a socket created under the lowered limit
//! would itself fail `EMFILE`) and only `connect(2)` — which allocates no
//! descriptor — runs under the limit. The fixture first fills every hole in
//! the process descriptor table, so lowering the soft limit to the resulting
//! open-descriptor count leaves no allocatable descriptor number below the
//! limit. The connection enters through the production egress TPROXY rule
//! expression on the fixture's host-side veth; its dialled address is distinct
//! from leg F. `ss -ltn` runs only after the limit is restored (spawning it
//! needs pipes); it still shows `LISTEN`, because the worker's shared owner
//! keeps the listener in its `leg_f_listener` slot after the task ends.
//!
//! # The oracle — why "no `EADDRINUSE`" has teeth here
//!
//! The old listener is still `LISTEN` when the repair runs, and leg F binds
//! with `SO_REUSEADDR`: a second bind on the same address conflicts with a
//! listening `SO_REUSEADDR` socket (`inet_bind_conflict`,
//! `net/ipv4/inet_connection_sock.c:177-181 @ v7.0`). So a repair that bound
//! the new listener before releasing the old one fails `EADDRINUSE`; the
//! contract's repair releases first. Further: the old descriptor is closed,
//! the new listener is `LISTEN` on the same address with a new inode, a new
//! connection is accepted on it, and the post-repair audit passes.
//!
//! The EMFILE stimulus and post-repair connection both originate in a
//! test-owned workload network namespace, enter the host through its veth and
//! traverse the production outbound nft-TPROXY rule and fwmark policy route.
//! The dialled destination has no listener of its own, so a successful
//! handshake can only be the TPROXY delivery to leg F. After repair, the
//! unregistered connection must additionally end within the bound: leg F's
//! task accepted it and closed it fail-closed rather than merely leaving it in
//! the new listener's accept queue. The current node-shared owner has already
//! converged its table, chains, current exemptions and fwmark policy route.
//! The fixture therefore appends only the exact production egress TPROXY rule
//! expression; it never calls the legacy setup helper that would add two
//! differently tagged exemptions. Before repair or audit, the fixture removes
//! its one exact tagged rule by handle and proves zero fixture-owned workload
//! rules and zero legacy exemptions remain. Unknown foreign rules are neither
//! removed nor hidden; the strict owner observation still rejects them.
//!
//! Observables: the worker's typed results, `leg_f_addr()`, the production
//! outbound TPROXY guard and external packet path, the kernel's socket table
//! (`/proc/net/tcp`, `ss -ltn`), this process's descriptor table, and the
//! captured client's accepted-and-closed result. Pending DELIVER step 14-02.

#![cfg(all(feature = "integration-tests", target_os = "linux"))]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "Tier-3 body: fixture preconditions panic with informative messages; evidence goes to stderr"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::{MTLS_SHARED_NFT_TABLE, MTLS_TPROXY_MARK};
use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_netlink::nft;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::mtls_intercept::InterceptLeg;
use overdrive_worker::mtls_intercept_port::HostMtlsIntercept;
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptWorker, MtlsSharedOwnerError};

/// How long the failure observation may take after the stimulus.
const FAILURE_BOUND: Duration = Duration::from_secs(5);
/// How long one client connect to leg F may take.
const CONNECT_BOUND: Duration = Duration::from_secs(2);
/// The lock file every `overdrive-mtls` Tier-3 fixture serialises on
/// (`tests/integration/inbound_tproxy_harness.rs`, `KernelStateLock`), held
/// in addition to the `host-kernel-shared` nextest group.
const KERNEL_STATE_LOCK: &std::ffi::CStr = c"/tmp/overdrive-mtls-kernel-state.lock";
const CAPTURE_CHAIN: &str = "prerouting";
/// A destination with no listener. The fixture assigns this address to host
/// loopback; a successful workload-side dial therefore proves TPROXY delivery.
const CAPTURE_PORT: u16 = 24_443;

fn record(started: Instant, phase: &str, detail: impl std::fmt::Display) {
    eprintln!("[legf-repair][{phase}][+{}ms] {detail}", started.elapsed().as_millis());
}

/// Holds the cross-process `overdrive-mtls` kernel-state lock: closing the
/// descriptor (on drop) releases the `flock`.
struct KernelStateLock {
    _fd: OwnedFd,
}

impl KernelStateLock {
    fn acquire() -> Self {
        // SAFETY: open(2) with O_CREAT|O_RDWR on a fixed NUL-terminated path; the
        // returned descriptor is checked and adopted by `OwnedFd`.
        let raw = unsafe {
            libc::open(
                KERNEL_STATE_LOCK.as_ptr(),
                libc::O_CREAT | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        assert!(raw >= 0, "open the kernel-state lock: {}", std::io::Error::last_os_error());
        // SAFETY: `raw` is a fresh, owned, open descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: flock(2) on the owned descriptor; blocks until exclusive.
        let rc = unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX) };
        assert_eq!(rc, 0, "flock the kernel-state lock: {}", std::io::Error::last_os_error());
        Self { _fd: fd }
    }
}

/// The inode of the IPv4 TCP socket listening at `addr` (`/proc/net/tcp`,
/// state `0A`), or `None`.
fn listening_inode(addr: SocketAddrV4) -> Option<u64> {
    let table = std::fs::read_to_string("/proc/net/tcp").expect("read /proc/net/tcp");
    let wanted = format!("{:08X}:{:04X}", u32::from_le_bytes(addr.ip().octets()), addr.port());
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
        if *local == wanted && *state == "0A" { inode.parse().ok() } else { None }
    })
}

/// This process's descriptor of the socket with `inode`, or `None`.
fn descriptor_of(inode: u64) -> Option<RawFd> {
    let wanted = format!("socket:[{inode}]");
    std::fs::read_dir("/proc/self/fd").expect("read /proc/self/fd").flatten().find_map(|entry| {
        let target = std::fs::read_link(entry.path()).ok()?;
        if target.to_string_lossy() == wanted {
            entry.file_name().to_string_lossy().parse().ok()
        } else {
            None
        }
    })
}

/// The number of descriptors this process has open, not counting the one
/// the count itself opens.
fn open_descriptors() -> u64 {
    let entries = std::fs::read_dir("/proc/self/fd").expect("read /proc/self/fd").count();
    u64::try_from(entries).expect("fits") - 1
}

fn nofile() -> libc::rlimit {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit(2) into a correctly typed out-parameter.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) };
    assert_eq!(rc, 0, "getrlimit: {}", std::io::Error::last_os_error());
    limit
}

fn set_nofile(limit: libc::rlimit) {
    // SAFETY: setrlimit(2) with a fully initialised value; the soft limit is
    // only ever lowered below, or restored to, the original hard-bounded value.
    let rc = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) };
    assert_eq!(rc, 0, "setrlimit: {}", std::io::Error::last_os_error());
}

/// A TCP socket created now (its descriptor exists before the limit drops)
/// and connected later.
fn unconnected_socket() -> OwnedFd {
    // SAFETY: socket(2); the result is checked and adopted by `OwnedFd`.
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
    // SAFETY: `raw` is a fresh, owned, open descriptor.
    unsafe { OwnedFd::from_raw_fd(raw) }
}

/// Fill every hole in the process descriptor table and return reservations
/// plus the first descriptor number that cannot be allocated after lowering
/// `RLIMIT_NOFILE`. `RLIMIT_NOFILE` bounds descriptor numbers, not the number
/// of open files; using the raw open-file count without filling holes can leave
/// an allocatable low descriptor and let `accept(2)` succeed.
fn reserve_dense_fd_table() -> (Vec<OwnedFd>, libc::rlim_t) {
    const DEV_NULL: &std::ffi::CStr = c"/dev/null";

    let mut open_count = open_descriptors();
    let mut reservations = Vec::new();
    loop {
        // SAFETY: open(2) on a fixed NUL-terminated path; the result is
        // checked and immediately adopted by `OwnedFd`.
        let raw_fd = unsafe { libc::open(DEV_NULL.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        assert!(raw_fd >= 0, "reserve descriptor-table hole: {}", std::io::Error::last_os_error());
        let raw = u64::try_from(raw_fd).expect("a non-negative descriptor fits u64");
        // SAFETY: `raw` is a fresh, owned, open descriptor.
        reservations.push(unsafe { OwnedFd::from_raw_fd(raw_fd) });
        assert!(raw <= open_count, "lowest free descriptor {raw} exceeds open count {open_count}");
        open_count += 1;
        if raw + 1 == open_count {
            return (reservations, open_count);
        }
    }
}

/// `connect(2)` of `fd` to `addr` (allocates no descriptor).
fn connect(fd: &OwnedFd, addr: SocketAddrV4) -> std::io::Result<()> {
    let sin = libc::sockaddr_in {
        sin_family: libc::sa_family_t::try_from(libc::AF_INET).expect("fits"),
        sin_port: addr.port().to_be(),
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(addr.ip().octets()) },
        sin_zero: [0; 8],
    };
    // SAFETY: connect(2) on an owned socket with a correctly sized sockaddr_in.
    let rc = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            std::ptr::from_ref(&sin).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_in>()).expect("fits"),
        )
    };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn ip(args: &[&str]) {
    let output = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run ip for the leg-F capture fixture");
    assert!(
        output.status.success(),
        "ip {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn ip_cleanup(args: &[&str]) {
    match Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("cleanup: ip {} exited {status}", args.join(" ")),
        Err(source) => eprintln!("cleanup: ip {} could not run: {source}", args.join(" ")),
    }
}

/// A route-disjoint workload namespace whose TCP traffic enters the outer
/// guest through `host_if`. `destination` is assigned to host loopback and has
/// no listener; the egress TPROXY rule is therefore the only path by which a
/// dial can complete.
struct CapturedPath {
    namespace: String,
    host_if: String,
    destination: SocketAddrV4,
    backend_assigned: bool,
    workload_lease: TestCidrLease,
    _backend_lease: TestCidrLease,
}

impl CapturedPath {
    fn provision() -> Self {
        let workload_lease = TestCidrLease::acquire("worker-leg-f-repair-workload")
            .expect("acquire the workload-side CIDR lease");
        let backend_lease = TestCidrLease::acquire("worker-leg-f-repair-backend")
            .expect("acquire the capture-destination CIDR lease");
        let tag = format!("{:05x}", std::process::id() & 0xf_ffff);
        let namespace = format!("ovt-legf-{tag}");
        let host_if = format!("ovlfh{tag}");
        let peer_if = format!("ovlfn{tag}");
        let destination = SocketAddrV4::new(backend_lease.host_gateway(), CAPTURE_PORT);
        let mut fixture = Self {
            namespace,
            host_if,
            destination,
            backend_assigned: false,
            workload_lease,
            _backend_lease: backend_lease,
        };
        ip(&["netns", "add", &fixture.namespace]);
        ip(&["link", "add", &fixture.host_if, "type", "veth", "peer", "name", &peer_if]);
        ip(&["link", "set", &peer_if, "netns", &fixture.namespace]);
        let gateway = format!(
            "{}/{}",
            fixture.workload_lease.host_gateway(),
            fixture.workload_lease.prefix_len()
        );
        let workload = format!(
            "{}/{}",
            fixture.workload_lease.workload_addr(),
            fixture.workload_lease.prefix_len()
        );
        ip(&["addr", "add", &gateway, "dev", &fixture.host_if]);
        ip(&["link", "set", &fixture.host_if, "up"]);
        ip(&["-n", &fixture.namespace, "addr", "add", &workload, "dev", &peer_if]);
        ip(&["-n", &fixture.namespace, "link", "set", &peer_if, "up"]);
        ip(&["-n", &fixture.namespace, "link", "set", "lo", "up"]);
        ip(&[
            "-n",
            &fixture.namespace,
            "route",
            "add",
            "default",
            "via",
            &fixture.workload_lease.host_gateway().to_string(),
        ]);
        ip(&["addr", "add", &format!("{}/32", fixture.destination.ip()), "dev", "lo"]);
        fixture.backend_assigned = true;
        fixture
    }

    fn cleanup_kernel_state(&mut self) {
        ip_cleanup(&["link", "del", &self.host_if]);
        ip_cleanup(&["netns", "del", &self.namespace]);
        if self.backend_assigned {
            ip_cleanup(&["addr", "del", &format!("{}/32", self.destination.ip()), "dev", "lo"]);
            self.backend_assigned = false;
        }
    }

    fn unconnected_socket(&self) -> OwnedFd {
        let namespace = format!("/var/run/netns/{}", self.namespace);
        std::thread::spawn(move || {
            let file = std::fs::File::open(&namespace)
                .unwrap_or_else(|source| panic!("open workload netns {namespace}: {source}"));
            // SAFETY: `file` is a live network-namespace descriptor and this
            // fresh thread owns its namespace membership.
            let rc = unsafe { libc::setns(file.as_raw_fd(), libc::CLONE_NEWNET) };
            assert_eq!(
                rc,
                0,
                "enter workload netns {namespace}: {}",
                std::io::Error::last_os_error()
            );
            unconnected_socket()
        })
        .join()
        .expect("the workload socket thread completes")
    }
}

impl Drop for CapturedPath {
    fn drop(&mut self) {
        self.cleanup_kernel_state();
    }
}

/// The unregistered captured connection was dequeued by leg F and closed
/// fail-closed. A connection left queued without an accept times out instead.
fn accepted_and_closed(fd: OwnedFd) -> bool {
    let mut stream = TcpStream::from(fd);
    stream.set_read_timeout(Some(CONNECT_BOUND)).expect("set captured-client read bound");
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Err(source)
            if matches!(
                source.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            true
        }
        Ok(read) => panic!("an unregistered captured connection returned {read} unexpected bytes"),
        Err(source)
            if matches!(
                source.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            false
        }
        Err(source) => panic!("captured connection ended unexpectedly: {source}"),
    }
}

struct FixtureCaptureRule {
    handle: u64,
    userdata: Vec<u8>,
    normalized_program: Vec<u8>,
    removed: bool,
}

impl FixtureCaptureRule {
    fn install(host_if: &str, leg_f_port: u16) -> Self {
        let before = nft::list_rules(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN)
            .expect("read the current shared prerouting inventory before fixture capture");
        assert!(
            nft::workload_rule_handles(&before).is_empty(),
            "precondition: no per-workload legacy rule precedes this fixture: {before:#?}"
        );
        assert!(
            !nft::has_exemption(&before),
            "precondition: no legacy exemption precedes this fixture: {before:#?}"
        );

        let userdata = nft::userdata_egress(host_if, leg_f_port);
        assert_eq!(
            before.iter().filter(|rule| rule.userdata == userdata).count(),
            0,
            "the fixture capture tag is initially absent"
        );
        let program = nft::egress_tproxy_rule_exprs(
            host_if,
            Ipv4Addr::LOCALHOST,
            leg_f_port,
            MTLS_TPROXY_MARK,
        );
        let normalized_program = nft::normalized_rule_program_identity(&program)
            .expect("normalize the production egress TPROXY program");
        nft::append_rule(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN, &program, &userdata)
            .expect("append the fixture's exact production egress TPROXY rule");

        let after = nft::list_rules(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN)
            .expect("read back the fixture capture rule");
        let matching = after.iter().filter(|rule| rule.userdata == userdata).collect::<Vec<_>>();
        let [installed] = matching.as_slice() else {
            panic!("fixture capture must have exactly one tagged rule, got {matching:#?}");
        };
        assert!(installed.counter.is_some(), "fixture capture carries its production counter");
        assert_eq!(
            installed.normalized_program, normalized_program,
            "fixture capture uses the exact production egress TPROXY program"
        );
        assert_eq!(
            nft::workload_rule_handles(&after),
            vec![installed.handle],
            "the fixture adds exactly one per-workload rule"
        );
        assert!(
            !nft::has_exemption(&after),
            "the fixture adds no legacy exemption to the current shared program"
        );
        Self { handle: installed.handle, userdata, normalized_program, removed: false }
    }

    fn remove(mut self, label: &str) {
        let before =
            nft::list_rules(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN).unwrap_or_else(|source| {
                panic!("{label}: list fixture capture before removal: {source}")
            });
        let matching =
            before.iter().filter(|rule| rule.userdata == self.userdata).collect::<Vec<_>>();
        let [installed] = matching.as_slice() else {
            panic!("{label}: expected exactly one fixture capture rule, got {matching:#?}");
        };
        assert_eq!(installed.handle, self.handle, "{label}: fixture capture handle is stable");
        assert_eq!(
            installed.normalized_program, self.normalized_program,
            "{label}: fixture capture program is unchanged"
        );
        assert_eq!(
            nft::workload_rule_handles(&before),
            vec![self.handle],
            "{label}: exactly one fixture-owned workload rule exists"
        );
        assert!(!nft::has_exemption(&before), "{label}: the fixture created no legacy exemption");

        nft::delete_rule(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN, self.handle)
            .unwrap_or_else(|source| panic!("{label}: delete fixture capture by handle: {source}"));
        self.removed = true;
        let after =
            nft::list_rules(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN).unwrap_or_else(|source| {
                panic!("{label}: read capture cleanup postcondition: {source}")
            });
        assert_eq!(
            after.iter().filter(|rule| rule.userdata == self.userdata).count(),
            0,
            "{label}: fixture capture tag is absent after deletion"
        );
        assert!(
            nft::workload_rule_handles(&after).is_empty(),
            "{label}: no fixture-owned workload rule remains: {after:#?}"
        );
        assert!(
            !nft::has_exemption(&after),
            "{label}: no legacy exemption remains after fixture cleanup"
        );
    }
}

impl Drop for FixtureCaptureRule {
    fn drop(&mut self) {
        if !self.removed {
            let _ = nft::delete_rule(MTLS_SHARED_NFT_TABLE, CAPTURE_CHAIN, self.handle);
        }
    }
}

/// A direct control connection is dequeued and closed by leg F, leaving no
/// transient accepted descriptor before the EMFILE reservation is measured.
fn connects(addr: SocketAddrV4) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&SocketAddr::V4(addr), CONNECT_BOUND) else {
        return false;
    };
    stream.set_read_timeout(Some(CONNECT_BOUND)).expect("set control-client read bound");
    let mut byte = [0_u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => true,
        Err(source)
            if matches!(
                source.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) =>
        {
            true
        }
        Ok(_) | Err(_) => false,
    }
}

/// `ss -ltn` shows a listener at `addr`.
fn ss_shows_listen(addr: SocketAddrV4) -> (bool, String) {
    let out = Command::new("ss").args(["-H", "-ltn"]).output().expect("run ss -ltn");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let wanted = addr.to_string();
    let listening = text.lines().any(|line| {
        line.split_whitespace().next() == Some("LISTEN")
            && line.split_whitespace().any(|field| field == wanted)
    });
    (listening, text)
}

/// D-9 / V-8 — leg F's accept fails `EMFILE` with the listener still
/// `LISTEN`; the worker reports `TaskFailed` on leg F (component `LegF`); its
/// repair re-binds the transparent listener at the same address with no
/// `EADDRINUSE` (the old one released first), the old socket's descriptor is
/// closed, the new listener is a new socket, a connection to leg F is
/// accepted again, and the full audit passes.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the worker's failure and repair
/// results, `leg_f_addr()`, the LISTEN socket at that address and its
/// descriptor, a client connect, the post-repair audit; expected: `TaskFailed
/// { leg: F }`, the listener still `LISTEN` at the failure, repair `Ok`, the
/// same address, a new listening socket with the old descriptor gone, a
/// TPROXY-captured connection accepted and closed fail-closed, the audit `Ok`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leg_f_repair_rebinds_transparent_listener() {
    // SAFETY: getuid is always safe.
    assert!(unsafe { libc::getuid() } == 0, "Tier-3 evidence requires root and CAP_NET_ADMIN");
    let started = Instant::now();
    let kernel_release = Command::new("uname").arg("-r").output().expect("uname -r");
    record(started, "kernel", String::from_utf8_lossy(&kernel_release.stdout).trim());
    let _lock = KernelStateLock::acquire();
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        // Already installed in this process.
    }

    // The production worker over the real intercept and enforcement; the
    // shared owner converges the node-global table on publish.
    let identity: Arc<dyn IdentityRead> = Arc::new(SimIdentityRead::new(BTreeMap::new(), None));
    let enforcement = Arc::new(HostMtlsEnforcement::new(
        identity,
        MtlsLimits::default(),
        GUEST_NODE_PREFIX,
        Arc::new(SimGuestStreamDial::new()),
    ));
    enforcement.probe().await.expect("the enforcement probe passes");
    let resolve: Arc<dyn MtlsResolve> =
        Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh));
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement as Arc<dyn MtlsEnforcement>,
        resolve,
        Arc::new(SimClock::new()),
        Arc::new(HostMtlsIntercept::new()),
    ));
    worker.start_shared_owner().await.expect("publish the node-shared owner");

    let leg_f = worker.leg_f_addr().expect("the published owner records leg F's address");
    assert!(connects(leg_f), "control: leg F accepts before the fault");
    let old_inode =
        listening_inode(leg_f).expect("precondition: a LISTEN socket at leg F's address");
    let old_fd = descriptor_of(old_inode)
        .expect("precondition: leg F's listening socket is a descriptor of this process");
    record(started, "before", format_args!("leg F {leg_f} inode {old_inode} fd {old_fd}"));

    let capture = CapturedPath::provision();
    assert_eq!(
        listening_inode(capture.destination),
        None,
        "precondition: the dialled capture destination has no listener"
    );
    let capture_rule = FixtureCaptureRule::install(&capture.host_if, leg_f.port());

    // Stimulus. The workload-side socket exists before the limit drops; only
    // its TPROXY-captured connect runs while no descriptor can be allocated.
    let client = capture.unconnected_socket();
    let original = nofile();
    let (reservations, soft_limit) = reserve_dense_fd_table();
    set_nofile(libc::rlimit { rlim_cur: soft_limit, rlim_max: original.rlim_max });
    let queued = connect(&client, capture.destination);
    let failure = tokio::time::timeout(FAILURE_BOUND, worker.wait_shared_owner_failure()).await;
    set_nofile(original);
    drop(reservations);
    queued.expect("the TPROXY-captured connection is queued on leg F");
    let failure = failure.expect("the worker observes leg F's accept task end within the bound");
    record(started, "failure", format_args!("{failure:?} (soft NOFILE was {soft_limit})"));
    assert!(
        matches!(failure, MtlsSharedOwnerError::TaskFailed { leg: InterceptLeg::F, .. }),
        "leg F's EMFILE accept error ends its task as TaskFailed on leg F, got {failure:?}"
    );
    assert_eq!(failure.component(), SharedGuestNetworkComponent::LegF);

    // Only now (limit restored): the listener is still LISTEN, still ours.
    let (listening, table) = ss_shows_listen(leg_f);
    record(started, "ss-ltn", &table);
    assert!(listening, "the accept error leaves leg F's listener LISTEN (ss -ltn)");
    assert_eq!(listening_inode(leg_f), Some(old_inode), "the same socket is still listening");

    capture_rule.remove("before repair");
    // The repair: release first, then bind — or EADDRINUSE.
    let repaired = worker.converge_shared_owner().await;
    record(started, "repair", format_args!("{repaired:?}"));
    repaired.expect("the repair re-binds leg F at its recorded address (no EADDRINUSE)");
    assert_eq!(worker.leg_f_addr(), Some(leg_f), "the repair keeps leg F's address");

    let new_inode =
        listening_inode(leg_f).expect("a LISTEN socket at leg F's address after the repair");
    assert_ne!(new_inode, old_inode, "the listener at leg F is a new socket");
    assert_eq!(
        descriptor_of(old_inode),
        None,
        "the old leg-F socket's descriptor is closed (released, not leaked)"
    );
    let repaired_capture = FixtureCaptureRule::install(&capture.host_if, leg_f.port());
    let repaired_client = capture.unconnected_socket();
    connect(&repaired_client, capture.destination)
        .expect("a TPROXY-captured connection reaches repaired leg F");
    assert!(
        accepted_and_closed(repaired_client),
        "repaired leg F accepts the captured connection and closes the unregistered tuple"
    );
    repaired_capture.remove("before post-repair audit");
    worker.audit_shared_owner().await.expect("the post-repair audit passes: both tasks live");
    drop(client);
    worker.shutdown_owner().await.expect("the node-shared owner shuts down");
}
