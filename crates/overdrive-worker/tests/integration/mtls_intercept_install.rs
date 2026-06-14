//! Tier-3 acceptance test for the worker's intercept-install + leg-acquire
//! role (`overdrive_worker::mtls_intercept`, D-MTLS-14 / SD-1(a)).
//!
//! Proves the four production free functions against REAL kernel side
//! effects on the Lima 6.18 kernel — no mocks, no synthetic ctx:
//!
//!   AC1 `make_transparent_listener` → a listener whose socket has
//!        `IP_TRANSPARENT` set (proven by `getsockopt(SOL_IP,
//!        IP_TRANSPARENT) == 1` on the real bound fd).
//!   AC2 `install_inbound_tproxy` → the nft-TPROXY rule + `ip rule fwmark`
//!        + `ip route local … table` companions are present in the live
//!        kernel state; dropping `TproxyInterceptGuard` removes them.
//!   AC3 `accept_inbound_leg` on a TPROXY-redirected connection recovers
//!        orig-dst via `getsockname` and builds
//!        `Routed::Inbound { orig_dst }` equal to the client's intended
//!        `virt`.
//!   AC4 `accept_outbound_leg` builds `Routed::Outbound { peer }` with the
//!        pre-programmed peer; the owned leg is handed by value.
//!
//! Port-to-port: every assertion enters through the `mtls_intercept`
//! module's public driving-port fns and asserts at the kernel boundary
//! (`getsockopt`, `nft list`, `ip rule`, a real redirected connect →
//! `getsockname`). Deleting the body of `accept_inbound_leg` MUST keep
//! AC3 RED — the orig-dst is recovered by production code, not the
//! fixture.
//!
//! Requires root + `CAP_NET_ADMIN` (IP_TRANSPARENT, nft, ip rule/route):
//! run via `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. A non-root run SKIPs (returns early).

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::match_wildcard_for_single_variants,
    reason = "Test bodies; skip messages go to stderr; failures must panic with informative messages; size_of casts are FFI-width on compile-time constants; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit"
)]

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{Direction, Routed};
use overdrive_worker::mtls_intercept::{
    accept_inbound_leg, accept_outbound_leg, install_inbound_tproxy, make_transparent_listener,
};

/// `IP_TRANSPARENT` sockopt — libc 0.2 does not name it (same as the
/// reference harness `mtls_roles.rs`).
const IP_TRANSPARENT: libc::c_int = 19;

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

/// `nft list table ip <name>` — Ok(stdout) on a present table, Err on absent.
fn nft_list_table(name: &str) -> Result<String, String> {
    let out = Command::new("nft")
        .args(["list", "table", "ip", name])
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

/// True iff an `ip rule` line for `fwmark <mark>` lookup `<table>` exists.
fn ip_rule_fwmark_present(mark: u32, table: u32) -> bool {
    ip_rule_fwmark_count(mark, table) > 0
}

/// Count of `ip rule` lines matching `fwmark <mark>` lookup `<table>`. Each
/// `ip rule add fwmark … lookup …` stacks a distinct rule, so repeated adds
/// (from aborted runs) accumulate; this counts them so a hygiene test can
/// assert the install left EXACTLY ONE.
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

/// Stack `n` extra `ip rule add fwmark <mark> lookup <table>` rules, mimicking
/// the GLOBAL state a prior SIGKILL'd run leaks (the inbound state is not
/// netns-scoped, so the guard's `Drop` never reclaimed it). Returns the number
/// successfully added so the test can assert the leak actually took.
fn leak_stale_fwmark_rules(mark: u32, table: u32, n: usize) -> usize {
    let mut added = 0;
    for _ in 0..n {
        let status = Command::new("ip")
            .args(["rule", "add", "fwmark", &format!("{mark:#x}"), "lookup", &table.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(s) if s.success()) {
            added += 1;
        }
    }
    added
}

/// Bounded blocking accept hand-off via a connecting client thread is done in
/// each scenario directly; this helper just dials `addr` once and returns the
/// connected stream so the production `accept_*` fn has a peer to accept.
fn dial(addr: SocketAddrV4, timeout: Duration) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect_timeout(&std::net::SocketAddr::V4(addr), timeout)?;
    stream.set_nodelay(true).ok();
    Ok(stream)
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

use std::os::fd::FromRawFd as _;

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

/// AC2 + AC3: inbound install + TPROXY-redirected leg acquire.
///
/// Stands up the agent's IP_TRANSPARENT leg-C listener, installs the inbound
/// nft-TPROXY intercept for a loopback `virt`, asserts the rule/route/table
/// are present, then dials `virt` (TPROXY-redirected to leg C) and drives
/// `accept_inbound_leg`, asserting orig-dst == virt. Finally drops the guard
/// and asserts the kernel state is gone.
#[test]
fn worker_intercept_install_leg_acquire_inbound() {
    if !is_root() {
        eprintln!("SKIP worker_intercept_install_leg_acquire_inbound: not root");
        return;
    }

    // Pre-clean any global inbound state a prior SIGKILL'd run leaked
    // (fwmark rule / local route / table) so this run starts from a clean
    // kernel. Mirrors the reference `preclean_global_inbound_state` discipline.
    preclean_global_inbound_state();

    let leg_c = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-C");
    let agent_port = match leg_c.local_addr().expect("leg-C local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-C addr, got {other}"),
    };

    // The client aims at this loopback virtual addr; the prerouting TPROXY
    // rule redirects a connection to `virt` onto leg C on `agent_port`.
    let virt = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 5), 18555);

    // AC2 (hygiene): `install_inbound_tproxy` precleans leaked GLOBAL state
    // before adding its own rule. Global inbound state (the fwmark rule, the
    // local route, the nft table) is NOT netns-scoped, so a prior SIGKILL'd run
    // (nextest slow-timeout, Bash cap, user cancel) leaves it behind — the
    // guard's `Drop` never ran. Leak TWO duplicate fwmark rules here, mimicking
    // two such aborted runs, then let `install_inbound_tproxy` (which opens with
    // `preclean_global_inbound_state`) run. We assert EXACTLY ONE fwmark rule
    // survives:
    //   - preclean intact  → drains the two stale rules, install adds one → 1.
    //   - preclean → `()`  → two stale rules survive, install stacks a third → 3 ≠ 1.
    //   - drain-loop `!` deleted (break on first success) → one stale survives,
    //     install stacks another → 2 ≠ 1.
    // Either preclean mutant flips this count off 1 and is CAUGHT.
    let leaked = leak_stale_fwmark_rules(0x1, 100, 2);
    assert_eq!(leaked, 2, "precondition: two stale fwmark rules must be leaked pre-install");
    assert_eq!(
        ip_rule_fwmark_count(0x1, 100),
        2,
        "precondition: exactly the two leaked fwmark rules are present pre-install"
    );

    let guard = install_inbound_tproxy(virt, agent_port).expect(
        "install_inbound_tproxy must preclean leaked state then install nft-TPROXY + ip rule/route",
    );

    // AC2 (preclean): the install's preclean drained the two leaked duplicates
    // so it left EXACTLY ONE fwmark rule, not a stacked pile.
    assert_eq!(
        ip_rule_fwmark_count(0x1, 100),
        1,
        "preclean must drain the leaked duplicates so install leaves EXACTLY ONE fwmark rule"
    );

    // AC2 (present): the production table shows the tproxy rule; the ip rule
    // fwmark companion + ip route local table companion are present.
    let table_dump =
        nft_list_table("overdrive-mtls").expect("nft table overdrive-mtls must be present");
    assert!(
        table_dump.contains(&format!("tproxy to 127.0.0.1:{agent_port}")),
        "nft table must carry the tproxy redirect to leg C, got:\n{table_dump}"
    );
    assert!(
        table_dump.contains("127.0.0.5") && table_dump.contains("18555"),
        "nft rule must match the virt daddr/dport, got:\n{table_dump}"
    );
    assert!(
        ip_rule_fwmark_present(0x1, 100),
        "ip rule fwmark 0x1 lookup 100 companion must be present"
    );

    // AC3: a real TPROXY-redirected connect lands on leg C; production
    // accept_inbound_leg recovers orig-dst via getsockname == virt.
    let client = std::thread::spawn(move || {
        // The TPROXY rule fires in prerouting on the host; a loopback connect
        // to `virt` is redirected to leg C transparently. Bounded so a failed
        // intercept never hangs the test.
        let s = dial(virt, Duration::from_secs(8));
        // Keep the client socket alive briefly so the server side can read
        // orig-dst before the connection is torn down.
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

    // AC2 (removed on Drop): dropping the guard removes the table + ip rule.
    drop(guard);
    assert!(
        nft_list_table("overdrive-mtls").is_err(),
        "TproxyInterceptGuard Drop must remove the nft table"
    );
    assert!(
        !ip_rule_fwmark_present(0x1, 100),
        "TproxyInterceptGuard Drop must remove the ip rule fwmark companion"
    );
}

/// Idempotent pre-clean of the GLOBAL inbound state (fwmark rule, local
/// route, nft table) a prior aborted run may have leaked. Global inbound
/// state is NOT netns-scoped and survives a SIGKILL; reuse the reference
/// `preclean_global_inbound_state` discipline so a leaked run does not red
/// this test. Best-effort: every command's failure is the "nothing to clean"
/// signal, so we intentionally ignore non-zero exits here.
fn preclean_global_inbound_state() {
    // Up to 64 stacked fwmark rules from repeated aborted runs.
    for _ in 0..64 {
        let status = Command::new("ip")
            .args(["rule", "del", "fwmark", "0x1", "lookup", "100"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !matches!(status, Ok(s) if s.success()) {
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
