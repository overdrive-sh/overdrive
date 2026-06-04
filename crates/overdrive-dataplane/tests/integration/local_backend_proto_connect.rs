//! S-02-02 — `LOCAL_BACKEND_MAP` keys on `(vip, vip_port, proto)`:
//! a TCP `connect(VIP:port)` and a UDP `connect(VIP:port)` to the SAME
//! `(vip, port)` each reach the proto-correct backend.
//!
//! Tags: `@US-udp` `@slice-02-02` `@real-io @adapter-integration`.
//! Tier: Tier 3 (real kernel — THE GATE for the cgroup path).
//!
//! ADR-0053 rev 2026-06-03 widened the `LOCAL_BACKEND_MAP` key from
//! `(vip, vip_port)` to `(vip, vip_port, proto)` IPVS-style so a service
//! co-locating tcp/53 + udp/53 on the same VIP routes each protocol to
//! its own backend. The `cgroup_connect4_service` program sources proto
//! from `bpf_sock_addr.protocol` (the IANA byte — 6 for TCP, 17 for UDP,
//! zero translation) and slots it into the lookup key.
//!
//! There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for
//! `cgroup_sock_addr` programs (ENOTSUPP ≤ 6.8 — see
//! `.claude/rules/development.md` § "`bpf_sock_addr.user_port`"). This
//! Tier-3 test is the structural defense for the proto-source
//! correctness; the handle's proto-roundtrip proptest compensates at
//! the userspace edge.
//!
//! # What this proves
//!
//! With a TCP backend AND a UDP backend registered under the SAME
//! `(vip, port)` but DISTINCT proto:
//!   1. `bpftool`-equivalent map dump (`local_backend_map_entries`)
//!      shows BOTH `(vip, port, tcp)` and `(vip, port, udp)` keys —
//!      neither overwrites the other (the pre-02-02 2-byte-pad key would
//!      collapse them into one slot).
//!   2. A real `connect(VIP:port)` over TCP rewrites to the TCP backend
//!      and echoes through it.
//!   3. A real connected-UDP `connect(VIP:port)` rewrites to the UDP
//!      backend and echoes through it.
//!
//! The test process must run as a descendant of the configured
//! `cgroup_attach_path` (`/sys/fs/cgroup`) so `connect4` fires —
//! `cargo xtask lima run --` runs the nextest process as root under
//! that ancestor, matching the BDB walking-skeleton harness.
//!
//! Capability gating mirrors `service_map_forward.rs`: requires
//! `CAP_NET_ADMIN` for veth setup; bails with a skip message on
//! `EPERM` rather than failing.

#![allow(clippy::missing_panics_doc)]
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::items_after_statements,
    clippy::doc_markdown
)]

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_dataplane::EbpfDataplane;

use super::helpers::veth::{VethError, VethPair};

/// VIP both protocols share. Distinct from any host-assigned address.
const VIP: Ipv4Addr = Ipv4Addr::new(10, 99, 0, 1);
/// Shared VIP listener port — the DNS co-location case (tcp/53 + udp/53
/// would be the canonical operator shape; 5353 avoids privileged-port
/// bind requirements in the test).
const VIP_PORT: u16 = 5353;

/// Spawn a one-shot TCP echo listener bound to `addr`. Echoes the first
/// payload back to the connecting client, then closes. Returns the
/// bound `SocketAddrV4`.
fn spawn_tcp_echo(addr: SocketAddrV4) -> SocketAddrV4 {
    let listener = TcpListener::bind(addr).expect("bind TCP echo backend");
    let bound = match listener.local_addr().expect("tcp local_addr") {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => unreachable!("bound IPv4 backend"),
    };
    thread::spawn(move || {
        if let Ok((mut stream, _peer)) = listener.accept() {
            let mut buf = [0u8; 64];
            if let Ok(n) = stream.read(&mut buf) {
                let _ = stream.write_all(&buf[..n]);
                let _ = stream.flush();
            }
        }
    });
    bound
}

/// Spawn a UDP echo listener bound to `addr`. Echoes each datagram back
/// to its sender. Returns the bound `SocketAddrV4`.
fn spawn_udp_echo(addr: SocketAddrV4) -> SocketAddrV4 {
    let sock = UdpSocket::bind(addr).expect("bind UDP echo backend");
    let bound = match sock.local_addr().expect("udp local_addr") {
        std::net::SocketAddr::V4(v4) => v4,
        std::net::SocketAddr::V6(_) => unreachable!("bound IPv4 backend"),
    };
    thread::spawn(move || {
        let mut buf = [0u8; 64];
        // One echo round is enough for the assertion.
        if let Ok((n, src)) = sock.recv_from(&mut buf) {
            let _ = sock.send_to(&buf[..n], src);
        }
    });
    bound
}

/// S-02-02 — proto-distinct local backends co-located on one `(vip, port)`.
#[test]
#[serial_test::serial(env)]
fn tcp_and_udp_connect_to_same_vip_port_reach_proto_correct_backend() {
    let host = "ovd-lbp0";
    let peer = "ovd-lbp1";

    let veth = match VethPair::create(host, peer) {
        Ok(v) => v,
        Err(VethError::CapNetAdminRequired) => {
            eprintln!(
                "skip: S-02-02 needs CAP_NET_ADMIN for veth setup — \
                 run via `cargo xtask lima run --` (default-root)"
            );
            return;
        }
        Err(e) => panic!("veth setup failed: {e}"),
    };

    // The veth pair only exists so `EbpfDataplane::new_with_pin_dir`
    // has real ifaces to attach its XDP programs to; the cgroup
    // connect4 path under test rewrites to the backend's own address
    // and never traverses the veth. Backends therefore bind loopback
    // (always reachable from the test process). This test drives
    // `register_local_backend` directly, bypassing the hydrator's
    // loopback guard — loopback backends are a test convenience here,
    // not a production shape.
    let _ = &veth;
    let host_ip = Ipv4Addr::LOCALHOST;

    // Two distinct local backends, each bound to a distinct loopback
    // port — proves the connect reached the proto-correct one.
    let tcp_backend = spawn_tcp_echo(SocketAddrV4::new(host_ip, 0));
    let udp_backend = spawn_udp_echo(SocketAddrV4::new(host_ip, 0));
    assert_ne!(
        tcp_backend.port(),
        udp_backend.port(),
        "fixture sanity: TCP and UDP backends must bind distinct ports"
    );

    // Per-test bpffs pin dir, cleaned pre + post.
    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-lbp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_dir_guard = PinDirGuard(pin_dir.clone());

    // Construct the production dataplane with the cgroup_connect4 hook
    // attached at /sys/fs/cgroup (the test process is a descendant).
    let dataplane = EbpfDataplane::new_with_pin_dir(
        &veth.host,
        &veth.peer,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir with cgroup attach");

    let rt = tokio::runtime::Runtime::new().expect("tokio rt");

    // Register BOTH protos at the SAME (vip, port). Pre-02-02 the second
    // register would overwrite the first (2-byte pad key collapses proto);
    // post-02-02 they are distinct (vip, port, proto) slots.
    rt.block_on(async {
        dataplane
            .register_local_backend(VIP, VIP_PORT, tcp_backend, Proto::Tcp)
            .await
            .expect("register TCP local backend");
        dataplane
            .register_local_backend(VIP, VIP_PORT, udp_backend, Proto::Udp)
            .await
            .expect("register UDP local backend");
    });

    // AC[9] — bpftool-equivalent dump shows BOTH (vip, port, tcp) and
    // (vip, port, udp) keys; neither overwrote the other.
    let entries = dataplane.local_backend_map_entries().expect("dump LOCAL_BACKEND_MAP");
    let tcp_key_present = entries.iter().any(|(k, v)| {
        k.vip_host == u32::from(VIP)
            && k.port_host == VIP_PORT
            && k.proto == Proto::Tcp.as_u8()
            && v.backend_port_host == tcp_backend.port()
    });
    let udp_key_present = entries.iter().any(|(k, v)| {
        k.vip_host == u32::from(VIP)
            && k.port_host == VIP_PORT
            && k.proto == Proto::Udp.as_u8()
            && v.backend_port_host == udp_backend.port()
    });
    assert!(
        tcp_key_present,
        "LOCAL_BACKEND_MAP must carry the (vip, port, tcp) key → tcp backend; entries={entries:?}"
    );
    assert!(
        udp_key_present,
        "LOCAL_BACKEND_MAP must carry the (vip, port, udp) key → udp backend; entries={entries:?}"
    );

    // proto-correct accessor: each (vip, port, proto) reads back its own
    // backend.
    let tcp_entry = dataplane
        .local_backend_for(VIP, VIP_PORT, Proto::Tcp)
        .expect("local_backend_for tcp")
        .expect("tcp entry present");
    assert_eq!(tcp_entry.backend_port_host, tcp_backend.port(), "tcp slot → tcp backend port");
    let udp_entry = dataplane
        .local_backend_for(VIP, VIP_PORT, Proto::Udp)
        .expect("local_backend_for udp")
        .expect("udp entry present");
    assert_eq!(udp_entry.backend_port_host, udp_backend.port(), "udp slot → udp backend port");

    // AC[9] — real TCP connect(VIP:port) rewrites to the TCP backend.
    let probe = b"proto-connect-probe";
    let tcp_ok = poll_until(Duration::from_secs(2), || {
        let mut stream = TcpStream::connect((VIP, VIP_PORT)).ok()?;
        stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
        stream.write_all(probe).ok()?;
        let mut buf = [0u8; 64];
        let n = stream.read(&mut buf).ok()?;
        (&buf[..n] == probe).then_some(())
    });
    assert!(
        tcp_ok.is_some(),
        "TCP connect({VIP}:{VIP_PORT}) did not echo through the TCP backend within 2s — \
         cgroup_connect4 proto-source or (vip, port, tcp) rewrite regression"
    );

    // AC[9] — real connected-UDP connect(VIP:port) rewrites to the UDP
    // backend.
    let udp_ok = poll_until(Duration::from_secs(2), || {
        let sock = UdpSocket::bind(SocketAddrV4::new(host_ip, 0)).ok()?;
        sock.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
        sock.connect((VIP, VIP_PORT)).ok()?;
        sock.send(probe).ok()?;
        let mut buf = [0u8; 64];
        let n = sock.recv(&mut buf).ok()?;
        (&buf[..n] == probe).then_some(())
    });
    assert!(
        udp_ok.is_some(),
        "connected-UDP connect({VIP}:{VIP_PORT}) did not echo through the UDP backend within 2s — \
         cgroup_connect4 proto-source (bpf_sock_addr.protocol) or (vip, port, udp) rewrite regression"
    );

    drop(dataplane);
}

/// Poll `f` until it returns `Some` or the deadline elapses.
fn poll_until<T>(budget: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}
