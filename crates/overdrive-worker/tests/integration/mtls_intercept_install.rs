//! Tier-3 acceptance test for the worker's intercept-install + leg-acquire
//! role (`overdrive_worker::mtls_intercept`, D-MTLS-14 / SD-1(a)).
//!
//! Proves the four production free functions against REAL kernel side
//! effects on the Lima 6.18 kernel — no mocks, no synthetic ctx:
//!
//!   AC1 `make_transparent_listener` → a listener whose socket has
//!        `IP_TRANSPARENT` set (proven by `getsockopt(SOL_IP,
//!        IP_TRANSPARENT) == 1` on the real bound fd).
//!   AC2 `install_inbound_tproxy` (the (b)-refined multi-virt model) → a
//!        per-virt TPROXY rule is APPENDED to the SHARED `prerouting` chain;
//!        a second install for a different virt COEXISTS (the second install
//!        does NOT raze the first); dropping ONE guard removes ONLY that
//!        virt's rule by handle, leaving the sibling's rule + the shared
//!        chain/exemption/ip-rule/route intact.
//!   AC3 `accept_inbound_leg` on a TPROXY-redirected connection recovers
//!        orig-dst via `getsockname` and builds
//!        `Routed::Inbound { orig_dst }` equal to the client's intended
//!        `virt`.
//!   AC4 `accept_outbound_leg` builds `Routed::Outbound { peer }` with the
//!        pre-programmed peer; the owned leg is handed by value.
//!   D3  the F5 `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption is present
//!        in the shared chain AND ordered BEFORE any tproxy rule; a dial with
//!        `SO_MARK = MTLS_LEG_S_DIAL_MARK` is NOT redirected to leg C (the
//!        exemption accepts it, no recursion).
//!
//! Port-to-port: every assertion enters through the `mtls_intercept`
//! module's public driving-port fns and asserts at the kernel boundary
//! (`getsockopt`, `nft -a list chain`, `ip rule`, a real redirected connect →
//! `getsockname`). Deleting the body of `accept_inbound_leg` MUST keep
//! AC3 RED — the orig-dst is recovered by production code, not the
//! fixture.
//!
//! Requires root + `CAP_NET_ADMIN` (IP_TRANSPARENT, nft, ip rule/route):
//! run via `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. A non-root run SKIPs (returns early).
//!
//! Hygiene: the shared routing infra (`ip rule`, `ip route`, nft
//! table/chain/exemption) now PERSISTS by design (node-global converge-on-boot
//! per the (b)-refined model), so each test tolerates pre-existing shared
//! infra at setup and scrubs ALL `overdrive-mtls` nft state + the fwmark
//! rule/route at start AND end via `clean_shared_infra()` so a clean-kernel
//! ground-truth run is reproducible. These tests mutate process-global kernel
//! state (the shared host-netns routing tables); a cross-process
//! `flock(2)` lock (`KernelStateLock`, below) serialises the
//! kernel-touching tests so concurrent installs do not race each other's
//! chain dumps. nextest runs each test in a SEPARATE PROCESS, so an
//! in-process `serial_test` lock cannot serialise node-global kernel
//! state — hence the file lock.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::match_wildcard_for_single_variants,
    reason = "Test bodies; skip messages go to stderr; failures must panic with informative messages; size_of/AF_INET casts are FFI-width on compile-time constants; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit"
)]

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::process::{Command, Stdio};
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_core::traits::mtls_enforcement::{Direction, Routed};
use overdrive_worker::mtls_intercept::{
    accept_inbound_leg, accept_outbound_leg, install_inbound_tproxy, make_transparent_listener,
};

/// Cross-PROCESS exclusion for the shared host-netns kernel state.
///
/// The `overdrive-mtls` nft table, the `fwmark` ip-rule, and the `table 100`
/// local route are NODE-GLOBAL: every test that installs/asserts on them
/// touches the SAME kernel state. nextest runs each `#[test]` in a SEPARATE
/// PROCESS, so an in-process lock (`serial_test`) does NOT serialise them —
/// two test processes concurrently in `ensure_shared_routing_infra`'s
/// check-then-add window each add the fwmark rule (→ 2, not 1) and interleave
/// chain dumps. An `flock(2)` on a fixed lock file spans processes; the guard
/// holds the exclusive lock for the whole test body and releases on Drop.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
    /// Acquire the exclusive cross-process lock (blocking). The lock file is a
    /// fixed well-known path so every test process contends on the same lock.
    fn acquire() -> Self {
        use std::os::fd::FromRawFd as _;
        let path = c"/tmp/overdrive-mtls-kernel-state.lock";
        // SAFETY: open with O_CREAT|O_RDWR on a fixed path; the returned fd is
        // adopted by OwnedFd. flock blocks until the exclusive lock is held.
        let fd = unsafe {
            let raw = libc::open(path.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600);
            assert!(raw >= 0, "open kernel-state lock file: {}", std::io::Error::last_os_error());
            let rc = libc::flock(raw, libc::LOCK_EX);
            assert!(rc == 0, "flock LOCK_EX: {}", std::io::Error::last_os_error());
            std::os::fd::OwnedFd::from_raw_fd(raw)
        };
        Self { fd }
    }
}

impl Drop for KernelStateLock {
    fn drop(&mut self) {
        // SAFETY: fd is the live lock fd; LOCK_UN releases the advisory lock.
        // (Dropping the fd would release it too, but be explicit.)
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// `IP_TRANSPARENT` sockopt — libc 0.2 does not name it (same as the
/// reference harness `mtls_roles.rs`).
const IP_TRANSPARENT: libc::c_int = 19;

/// The shared fixed fwmark + routing-policy table the (b)-refined model uses.
const TPROXY_FWMARK: u32 = 0x1;
const TPROXY_RT_TABLE: u32 = 100;

/// True iff this process is uid 0 (root). The IP_TRANSPARENT setopt, nft,
/// and `ip rule`/`route` all need root + CAP_NET_ADMIN; a non-root run
/// cannot stand up the fixture, so we SKIP rather than fail.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Read `getsockopt(SOL_IP, IP_TRANSPARENT)` on `fd`. Returns the raw int
/// value (1 == set). Panics on syscall failure — the fixture precondition
/// is "the fd is a real bound socket".
fn getsockopt_ip_transparent(fd: i32) -> libc::c_int {
    let mut val: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: fd is a live socket from the production listener; val/len are
    // correctly sized for an int sockopt.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_IP,
            IP_TRANSPARENT,
            std::ptr::from_mut(&mut val).cast(),
            std::ptr::from_mut(&mut len),
        )
    };
    assert!(rc == 0, "getsockopt(IP_TRANSPARENT): {}", std::io::Error::last_os_error());
    val
}

/// `nft -a list chain ip overdrive-mtls prerouting` — Ok(dump) on a present
/// chain, Err(stderr) on absent. `-a` emits the per-rule `# handle <N>` so the
/// test can assert on rule presence/ordering and handles.
fn nft_list_chain() -> Result<String, String> {
    let out = Command::new("nft")
        .args(["-a", "list", "chain", "ip", "overdrive-mtls", "prerouting"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn nft: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// True iff the chain dump contains a per-virt tproxy rule for `virt`.
fn chain_has_virt_rule(dump: &str, virt: SocketAddrV4) -> bool {
    let daddr = format!("ip daddr {}", virt.ip());
    let dport = format!("tcp dport {}", virt.port());
    dump.lines().any(|l| l.contains(&daddr) && l.contains(&dport) && l.contains("tproxy to"))
}

/// True iff an `ip rule` line for `fwmark <mark>` lookup `<table>` exists.
fn ip_rule_fwmark_present(mark: u32, table: u32) -> bool {
    ip_rule_fwmark_count(mark, table) > 0
}

/// Count of `ip rule` lines matching `fwmark <mark>` lookup `<table>`. The
/// (b)-refined `ensure_shared_routing_infra` adds the rule only when missing,
/// so a green run leaves EXACTLY ONE regardless of how many virts install.
fn ip_rule_fwmark_count(mark: u32, table: u32) -> usize {
    let out = Command::new("ip")
        .args(["rule", "show"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return 0 };
    let text = String::from_utf8_lossy(&out.stdout);
    let needle_mark = format!("fwmark {mark:#x}");
    let needle_mark_dec = format!("fwmark {mark}");
    text.lines()
        .filter(|l| {
            (l.contains(&needle_mark) || l.contains(&needle_mark_dec))
                && l.contains(&format!("lookup {table}"))
        })
        .count()
}

/// True iff `ip route show table <table>` carries the shared local catch-all
/// loopback route. The kernel CANONICALISES `local 0.0.0.0/0 dev lo` to
/// `local default dev lo scope host` on read, so the needle must match
/// `default`, not `0.0.0.0/0`.
fn ip_route_local_present(table: u32) -> bool {
    let out = Command::new("ip")
        .args(["route", "show", "table", &table.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return false };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().any(|l| l.contains("local") && l.contains("default") && l.contains("lo"))
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing shared infra) AND END. Best-effort: every command's failure is
/// the "nothing to clean" signal, so non-zero exits are intentionally ignored.
fn clean_shared_infra() {
    // Drain however many fwmark rules a prior run may have stacked (a healthy
    // (b) run leaves exactly one; an old buggy run may have stacked several).
    for _ in 0..64 {
        let ok = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            break;
        }
    }
    let _ = Command::new("ip")
        .args(["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Dial `addr` once and return the connected stream so the production
/// `accept_*` fn has a peer to accept.
fn dial(addr: SocketAddrV4, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), timeout)?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

/// Dial `addr` with `SO_MARK = mark` set on the socket BEFORE connect (the
/// shape the agent's own leg-S dial uses). Returns the connected stream.
fn dial_with_so_mark(
    addr: SocketAddrV4,
    mark: u32,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    // SAFETY: a fresh AF_INET stream socket; SO_MARK is set on it before
    // connect; the fd is adopted by TcpStream::from_raw_fd which owns it.
    let stream = unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mark_val: libc::c_int = mark as libc::c_int;
        let rc = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_MARK,
            std::ptr::from_ref(&mark_val).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        TcpStream::from_raw_fd(fd)
    };
    let sa = std::net::SocketAddr::V4(addr);
    // std has no connect_timeout-on-existing-fd; the loopback connect is
    // immediate. Set a short read timeout so a hung connect cannot stall.
    stream.connect_timeout_compat(&sa, timeout)?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

/// `TcpStream` does not expose connect on an existing fd; emulate it for the
/// SO_MARK case via a raw `connect(2)` with a bounded poll.
trait ConnectCompat {
    fn connect_timeout_compat(
        &self,
        addr: &std::net::SocketAddr,
        timeout: Duration,
    ) -> std::io::Result<()>;
}

impl ConnectCompat for TcpStream {
    fn connect_timeout_compat(
        &self,
        addr: &std::net::SocketAddr,
        timeout: Duration,
    ) -> std::io::Result<()> {
        let std::net::SocketAddr::V4(v4) = addr else {
            return Err(std::io::Error::other("v4 only"));
        };
        let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        sa.sin_family = libc::AF_INET as libc::sa_family_t;
        sa.sin_port = v4.port().to_be();
        sa.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
        self.set_read_timeout(Some(timeout)).ok();
        // SAFETY: self owns a live AF_INET socket fd; sa is a correctly-sized
        // sockaddr_in for the connect target.
        let rc = unsafe {
            libc::connect(
                self.as_raw_fd(),
                std::ptr::from_ref(&sa).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("valid allocation id")
}

/// AC1 + AC4: outbound leg acquire. `make_transparent_listener` is NOT used
/// for leg F (leg F is a plain loopback listener — the design states leg F
/// needs no IP_TRANSPARENT), so this scenario stands up a plain
/// `std::net::TcpListener` on `127.0.0.1:0`, dials it, and drives
/// `accept_outbound_leg`, asserting the routing fact is `Outbound { peer }`
/// with the pre-programmed peer and the leg is handed by value (an OwnedFd).
#[test]
fn worker_intercept_install_leg_acquire_outbound() {
    if !is_root() {
        eprintln!("SKIP worker_intercept_install_leg_acquire_outbound: not root");
        return;
    }

    // leg-F listener: plain loopback, no IP_TRANSPARENT (per D-MTLS-14).
    let leg_f = std::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("bind leg-F loopback listener");
    let leg_f_addr = match leg_f.local_addr().expect("leg-F local_addr") {
        std::net::SocketAddr::V4(a) => a,
        other => panic!("expected V4 leg-F addr, got {other}"),
    };

    // The pre-programmed real peer leg B would dial (handed verbatim into
    // accept_outbound_leg). Arbitrary stable addr; not actually connected to.
    let peer = SocketAddrV4::new(Ipv4Addr::new(10, 9, 8, 7), 4443);
    let alloc_id = alloc("alloc-outbound-leg");

    // A client thread dials the leg-F listener so the production accept has a
    // pending connection. The byte exchange proves the returned OwnedFd is the
    // genuine accepted leg (we write through it and the client reads it back).
    let client = std::thread::spawn(move || {
        let mut s = dial(leg_f_addr, Duration::from_secs(5)).expect("dial leg-F");
        let mut buf = [0u8; 4];
        s.read_exact(&mut buf).expect("read leg-F probe byte");
        buf
    });

    let intercepted = accept_outbound_leg(&leg_f, alloc_id.clone(), peer)
        .expect("accept_outbound_leg must build InterceptedConnection");

    // AC4: routing fact is Outbound { peer } with the pre-programmed peer.
    match intercepted.routed {
        Routed::Outbound { peer: got } => assert_eq!(got, peer, "Outbound peer must round-trip"),
        Routed::Inbound { orig_dst } => panic!("expected Outbound, got Inbound {{ {orig_dst} }}"),
    }
    assert_eq!(intercepted.routed.direction(), Direction::Outbound);
    assert_eq!(intercepted.alloc, alloc_id, "alloc must round-trip");
    assert!(intercepted.expected_peer.is_none(), "v1 authn-only: expected_peer is None");

    // Prove the owned leg is the genuine accepted socket: write through a dup
    // of it (an independent fd), the client reads it back byte-exact. We dup
    // so the production type keeps owning `intercepted.leg`.
    {
        let dup_fd = raw_dup(intercepted.leg.as_raw_fd());
        // SAFETY: dup_fd is an independent owned fd over the accepted TCP leg.
        let mut leg = unsafe { TcpStream::from_raw_fd(dup_fd) };
        leg.write_all(b"PING").expect("write through owned leg F");
        leg.flush().ok();
        // `leg` drops here, closing the dup; `intercepted.leg` stays owned.
    }
    let echoed = client.join().expect("client thread");
    assert_eq!(&echoed, b"PING", "client must read the byte written through the owned leg");
}

/// Duplicate a raw fd (so the test can write through a copy without consuming
/// the OwnedFd the production type owns). Returns the new fd.
fn raw_dup(fd: i32) -> i32 {
    // SAFETY: dup of a live fd; the returned fd is owned by the caller.
    let new = unsafe { libc::dup(fd) };
    assert!(new >= 0, "dup: {}", std::io::Error::last_os_error());
    new
}

/// AC1: `make_transparent_listener` sets IP_TRANSPARENT on the real socket.
#[test]
fn worker_make_transparent_listener_sets_ip_transparent() {
    if !is_root() {
        eprintln!("SKIP worker_make_transparent_listener_sets_ip_transparent: not root");
        return;
    }
    let listener = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener must bind an IP_TRANSPARENT socket");
    let val = getsockopt_ip_transparent(listener.as_raw_fd());
    assert_eq!(val, 1, "IP_TRANSPARENT must be set on the production leg-C listener");
    // And it is actually bound to a loopback addr with an assigned port.
    let addr = listener.local_addr().expect("local_addr");
    assert!(addr.is_ipv4(), "must be a v4 loopback listener");
    assert_ne!(addr.port(), 0, "kernel must have assigned a port");
}

/// AC2 (multi-virt coexistence + per-virt by-handle teardown) + D3 (F5
/// exemption present and ordered first).
///
/// The D2-proof: install for virt A, then virt B; assert BOTH per-virt rules
/// coexist in the ONE shared chain (the second install did NOT raze the
/// first). Drop A's guard; assert A's rule is gone AND B's rule + the shared
/// chain/exemption/ip-rule/route all REMAIN. This is the honest AC the old
/// single-table razes-the-table model could never pass.
#[test]
fn worker_inbound_multi_virt_coexist_and_per_virt_teardown() {
    if !is_root() {
        eprintln!("SKIP worker_inbound_multi_virt_coexist_and_per_virt_teardown: not root");
        return;
    }
    // Cross-process exclusion: hold the shared-kernel-state lock for the whole
    // body so a sibling test process cannot mutate the nft chain / fwmark rule
    // concurrently.
    let _kernel_lock = KernelStateLock::acquire();
    // Tolerate pre-existing shared infra; start from a clean kernel.
    clean_shared_infra();

    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    let virt_a = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 5), 18555);
    let virt_b = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 6), 18666);

    let guard_a = install_inbound_tproxy(virt_a, agent_port)
        .expect("install_inbound_tproxy(virt_a) must append a per-virt rule to the shared chain");

    // Second install for a DIFFERENT virt must NOT raze the first.
    let guard_b = install_inbound_tproxy(virt_b, agent_port)
        .expect("install_inbound_tproxy(virt_b) must coexist with virt_a's rule");

    // AC2 (coexistence): BOTH per-virt rules present in the ONE shared chain.
    let dump = nft_list_chain().expect("shared overdrive-mtls prerouting chain must be present");
    assert!(
        chain_has_virt_rule(&dump, virt_a),
        "virt_a's tproxy rule must survive virt_b's install (no raze), got:\n{dump}"
    );
    assert!(
        chain_has_virt_rule(&dump, virt_b),
        "virt_b's tproxy rule must coexist with virt_a's, got:\n{dump}"
    );
    assert!(
        dump.contains(&format!("tproxy to 127.0.0.1:{agent_port}")),
        "both rules redirect to the agent leg-C port, got:\n{dump}"
    );

    // D3: the F5 leg-S-dial exemption is present AND ordered BEFORE any tproxy
    // rule (so the agent's own marked dial is accepted before a redirect can
    // match it).
    // nft renders the mark zero-padded 8-hex (e.g. `meta mark 0x00000002
    // accept`), NOT `0x2` / decimal `2`. Match the canonical rendering.
    let exemption_needle = format!("meta mark {MTLS_LEG_S_DIAL_MARK:#010x} accept");
    let exemption_idx = dump
        .lines()
        .position(|l| l.contains(&exemption_needle))
        .unwrap_or_else(|| panic!("F5 leg-S exemption missing from chain, got:\n{dump}"));
    let first_tproxy_idx = dump
        .lines()
        .position(|l| l.contains("tproxy to"))
        .unwrap_or_else(|| panic!("expected at least one tproxy rule, got:\n{dump}"));
    assert!(
        exemption_idx < first_tproxy_idx,
        "F5 exemption (line {exemption_idx}) must precede every tproxy rule (first at {first_tproxy_idx}), got:\n{dump}"
    );

    // Shared infra present.
    assert_eq!(
        ip_rule_fwmark_count(TPROXY_FWMARK, TPROXY_RT_TABLE),
        1,
        "idempotent ensure leaves EXACTLY ONE shared fwmark rule across two installs"
    );
    assert!(
        ip_route_local_present(TPROXY_RT_TABLE),
        "shared local route in table 100 must be present"
    );

    // AC2 (per-virt teardown): drop A only.
    drop(guard_a);
    let dump_after =
        nft_list_chain().expect("shared chain must STILL be present after dropping one guard");
    assert!(
        !chain_has_virt_rule(&dump_after, virt_a),
        "dropping guard_a must remove ONLY virt_a's rule, got:\n{dump_after}"
    );
    // The sibling's rule + the shared infra (chain, exemption, ip-rule, route)
    // must all REMAIN — the by-handle delete touched only virt_a.
    assert!(
        chain_has_virt_rule(&dump_after, virt_b),
        "virt_b's rule must survive virt_a's guard drop, got:\n{dump_after}"
    );
    assert!(
        dump_after.lines().any(|l| l.contains(&exemption_needle)),
        "F5 exemption must survive a per-virt guard drop, got:\n{dump_after}"
    );
    assert!(
        ip_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE),
        "shared fwmark rule must survive a per-virt guard drop"
    );
    assert!(
        ip_route_local_present(TPROXY_RT_TABLE),
        "shared local route must survive a per-virt guard drop"
    );

    drop(guard_b);
    clean_shared_infra();
}

/// AC3 (the PRIMARY deliverable): a REAL TPROXY-redirected connect to a virt →
/// `accept_inbound_leg` → `Routed::Inbound { orig_dst }` with `orig_dst ==
/// virt` (getsockname recovery). Deleting the body of `accept_inbound_leg`
/// keeps this RED.
#[test]
fn worker_inbound_tproxy_redirect_recovers_orig_dst() {
    if !is_root() {
        eprintln!("SKIP worker_inbound_tproxy_redirect_recovers_orig_dst: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    let virt = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 5), 18555);
    let guard = install_inbound_tproxy(virt, agent_port)
        .expect("install_inbound_tproxy must append the per-virt TPROXY rule");

    // AC2 sanity: the rule + companions are live before we drive the connect.
    let dump = nft_list_chain().expect("nft chain overdrive-mtls prerouting must be present");
    assert!(chain_has_virt_rule(&dump, virt), "virt's tproxy rule must be installed, got:\n{dump}");
    assert!(
        ip_rule_fwmark_present(TPROXY_FWMARK, TPROXY_RT_TABLE),
        "shared fwmark rule must be present"
    );

    // AC3: a real TPROXY-redirected connect lands on leg C; production
    // accept_inbound_leg recovers orig-dst via getsockname == virt.
    let client = std::thread::spawn(move || {
        let s = dial(virt, Duration::from_secs(8));
        if let Ok(mut s) = s {
            let _ = s.write_all(b"HELLO");
            std::thread::sleep(Duration::from_millis(200));
        }
    });

    let alloc_id = alloc("alloc-inbound-leg");
    let intercepted = accept_inbound_leg(&leg_c, alloc_id.clone())
        .expect("accept_inbound_leg must build InterceptedConnection from TPROXY redirect");

    match intercepted.routed {
        Routed::Inbound { orig_dst } => {
            assert_eq!(orig_dst, virt, "getsockname orig-dst must equal the client's virt");
        }
        Routed::Outbound { peer } => panic!("expected Inbound, got Outbound {{ {peer} }}"),
    }
    assert_eq!(intercepted.routed.direction(), Direction::Inbound);
    assert_eq!(intercepted.alloc, alloc_id, "alloc must round-trip");
    assert!(intercepted.expected_peer.is_none(), "v1 authn-only: expected_peer is None");

    client.join().expect("inbound client thread");

    drop(guard);
    clean_shared_infra();
}

/// D3 (recursion bypass): a dial carrying `SO_MARK = MTLS_LEG_S_DIAL_MARK`
/// (the shape the agent's own leg-S dial uses) is NOT TPROXY-redirected onto
/// leg C — the F5 exemption at the chain head accepts it first. We prove this
/// by binding a REAL server on `virt` and asserting the marked dial reaches
/// THAT server (not leg C): if the redirect had fired, the marked connection
/// would have landed on leg C and the real server would never accept it.
#[test]
fn worker_inbound_leg_s_marked_dial_bypasses_redirect() {
    if !is_root() {
        eprintln!("SKIP worker_inbound_leg_s_marked_dial_bypasses_redirect: not root");
        return;
    }
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();

    // leg-C listener: the redirect TARGET. If the exemption is broken, the
    // marked dial would be redirected here.
    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    // A REAL server bound on the virt addr — the leg-S-marked dial must reach
    // THIS, proving it bypassed the redirect. Use a concrete loopback addr +
    // port we can bind directly.
    let virt = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 7), 18777);
    let real_server = std::net::TcpListener::bind(virt).expect("bind real server on virt");

    let guard = install_inbound_tproxy(virt, agent_port)
        .expect("install_inbound_tproxy must append the per-virt TPROXY rule");

    // The marked dial: SO_MARK = MTLS_LEG_S_DIAL_MARK. The F5 exemption
    // (ordered first) accepts it in prerouting before the tproxy rule matches,
    // so it is delivered to the REAL server on virt, not redirected to leg C.
    let client = std::thread::spawn(move || {
        let s = dial_with_so_mark(virt, MTLS_LEG_S_DIAL_MARK, Duration::from_secs(8));
        if let Ok(mut s) = s {
            let _ = s.write_all(b"MARKED");
            std::thread::sleep(Duration::from_millis(200));
        }
    });

    // The real server accepts (proving bypass). leg C is NOT polled — if the
    // redirect had wrongly fired, the marked dial would have landed on leg C
    // and this accept would time out.
    real_server.set_nonblocking(false).expect("blocking accept on real server");
    let (mut conn, _peer) = accept_with_timeout(&real_server, Duration::from_secs(5))
        .expect("F5 exemption: leg-S-marked dial must reach the REAL server on virt, not leg C");
    let mut buf = [0u8; 6];
    conn.read_exact(&mut buf).expect("read the marked-dial payload");
    assert_eq!(&buf, b"MARKED", "the real server must receive the marked dial's bytes");

    client.join().expect("marked-dial client thread");
    // leg_c is held only so the redirect target exists; never accepted from.
    drop(leg_c);
    drop(guard);
    clean_shared_infra();
}

/// Accept on `listener` within `timeout` by polling with a short read-timeout
/// loop on a non-blocking accept. Returns the accepted connection or an error
/// if nothing arrives within the budget (the failure shape that would mean the
/// marked dial was wrongly redirected to leg C).
fn accept_with_timeout(
    listener: &std::net::TcpListener,
    timeout: Duration,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false).ok();
                return Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no inbound connection within timeout (marked dial may have been redirected)",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}
