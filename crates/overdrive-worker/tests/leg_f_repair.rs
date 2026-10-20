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
//! the limit is lowered: the client socket is created first (a socket
//! created under the lowered limit would itself fail `EMFILE`) and only
//! `connect(2)` — which allocates no descriptor — runs under the limit. `ss
//! -ltn` runs only after the limit is restored (spawning it needs pipes);
//! it still shows `LISTEN`, because the worker's shared owner keeps the
//! listener in its `leg_f_listener` slot after the task ends.
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
//! The connection is a direct connect to leg F's address rather than a
//! TPROXY-captured one: the accept error the cell provokes happens at
//! `accept(2)` on the listener, which is the same for both (the capture's
//! subject is covered by the `mtls` Tier-3 suite).
//!
//! Observables: the worker's typed results, `leg_f_addr()`, the kernel's
//! socket table (`/proc/net/tcp`, `ss -ltn`), this process's descriptor
//! table, and a client connect to leg F. Pending DELIVER step 10-02.

#![cfg(all(feature = "integration-tests", target_os = "linux"))]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "Tier-3 body: fixture preconditions panic with informative messages; evidence goes to stderr"
)]

use std::collections::BTreeMap;
use std::net::{SocketAddr, SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::guest_network::SharedGuestNetworkComponent;
use overdrive_core::guest_transport::GUEST_NODE_PREFIX;
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve};
use overdrive_dataplane::mtls::HostMtlsEnforcement;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::guest_stream_dial::SimGuestStreamDial;
use overdrive_sim::adapters::{SimIdentityRead, SimMtlsResolve};
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

/// A client connect to leg F succeeds (a socket listens at the address).
fn connects(addr: SocketAddrV4) -> bool {
    TcpStream::connect_timeout(&SocketAddr::V4(addr), CONNECT_BOUND).is_ok()
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
/// same address, a new listening socket with the old descriptor gone, the
/// connect accepted, the audit `Ok`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "pending DELIVER step 10-02"]
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

    // Stimulus. The client socket exists before the limit drops.
    let client = unconnected_socket();
    let original = nofile();
    let in_use = open_descriptors();
    set_nofile(libc::rlimit { rlim_cur: in_use, rlim_max: original.rlim_max });
    let queued = connect(&client, leg_f);
    let failure = tokio::time::timeout(FAILURE_BOUND, worker.wait_shared_owner_failure()).await;
    set_nofile(original);
    queued.expect("the connection is queued on leg F (connect allocates no descriptor)");
    let failure = failure.expect("the worker observes leg F's accept task end within the bound");
    record(started, "failure", format_args!("{failure:?} (soft NOFILE was {in_use})"));
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
    assert!(connects(leg_f), "a connection to leg F is accepted after the repair");
    worker.audit_shared_owner().await.expect("the post-repair audit passes: both tasks live");
    drop(client);
    worker.shutdown_owner().await.expect("the node-shared owner shuts down");
}
