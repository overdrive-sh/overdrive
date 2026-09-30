//! Tier-3 EGRESS capture walking proof (step 03-03) — the egress half of the
//! ADR-0071 § Enforcement Tier-3 obligations (a)+(b), composing these surfaces
//! on the live kernel:
//!
//!   - the production outbound divert of the GH #295 shared design, composed on
//!     the host veth by the shared fixture `SharedOutboundDivert`
//!     (`mtls_intercept_install.rs`): `HostMtlsIntercept::converge_shared` at
//!     the two bound legs' targets (the node's constant `overdrive-mtls`
//!     program, plus the fwmark rule and the table-100 local route),
//!     `HostMtlsIntercept::install_outbound(workload, leg_f_port)` (the
//!     workload's managed-guest and outbound-source elements), and the guest
//!     TCX ingress classifier (`GuestTcxProgram`, test-scoped pins) attached
//!     first at the host veth with the workload's endpoint registered. Shared
//!     prerouting rule 1 TPROXYs TCP that carries the classifier's intercept
//!     mark `0x295a` from an admitted source to leg F. No per-interface rule is
//!     installed;
//!   - `HostMtlsIntercept::bind_transparent(addr)` — the `MtlsIntercept` port's
//!     leg-F (and leg-C) listener. Leg-F MUST be `IP_TRANSPARENT` because TPROXY
//!     delivers packets whose dst is the orig-dst (NOT leg-F's bound addr); a
//!     non-transparent socket cannot receive them. The listener also marks its
//!     sockets with the leg-S mark, so its replies to the managed guest pass the
//!     shared output chain's exemption;
//!   - the port listener's accept, read through the `LegListener` bridge
//!     (`leg_listener.rs`, GH #295 S-ND295-70) — the accepted connection's local
//!     address is the workload's dialed orig-dst (`getsockname` on the
//!     TPROXY-intercepted leg-F socket). Before the DELIVER step that changes
//!     `bind_transparent`'s return type (gap B-7) the bridge reaches the
//!     production outbound accept helper; after it, the host listener's
//!     `accept`. The body is the same on both sides.
//!
//! NO new production code — this step is test-only, composing the above on a
//! REAL kernel through the real netns + veth topology proven by the increment-b
//! spike (`docs/feature/.../spike/findings-egress-tproxy.md`, VERDICT WORKS,
//! kernel 7.0.0-22-generic).
//!
//! Topology:
//!
//!   netns nsW:  workload client; vethW <lease workload>/24, MAC 02:00:<workload
//!                 octets>; default via <lease gateway>
//!                 connect(10.200.0.1:18777)
//!     <== veth ==>
//!   host netns: vethH <lease gateway>/24
//!                 TCX ingress: guest classifier; endpoint {workload, its MAC,
//!                   bridge MAC}; guest TCP -> meta mark 0x295a
//!                 table ip overdrive-mtls (converge_shared):
//!                   prerouting 0: meta mark 0x2 accept          <- leg-S exemption
//!                   prerouting 1: meta mark 0x295a, saddr @outbound_sources, tcp
//!                                 -> tproxy to 127.0.0.1:<legF>, meta mark set 0x1
//!                   output 0:     meta mark 0x2 accept          <- leg-S exemption
//!                   output 2:     daddr @managed_guest_ips, tcp -> drop
//!                   @managed_guest_ips = @outbound_sources = { <workload> }
//!                 ip rule fwmark 0x1 lookup 100
//!                 ip route local 0.0.0.0/0 dev lo table 100
//!                 leg-F IP_TRANSPARENT 127.0.0.1:<legF>, SO_MARK 0x2
//!                 real backend 10.200.0.1:18777    (host lo)
//!
//! Port-to-port: every assertion enters through public surfaces (the
//! `MtlsIntercept` port's `converge_shared`, `install_outbound`, and
//! `bind_transparent` listener and its accept; the dataplane's guest TCX
//! classifier) and asserts at the kernel/socket boundary: `getsockname`
//! orig-dst recovery, the classifier's intercept count, and which listener
//! received the connection. Litmus:
//!   - gut the port listener's accept-side orig-dst recovery (the accepted
//!     connection's `local`) → the `getsockname == dialed-dst` assertion goes RED
//!     (the orig-dst is recovered by production code, not the fixture);
//!   - drop the outbound-source element from `install_outbound`, or the
//!     classifier's intercept mark → the leg-F accept times out (RED): shared
//!     rule 1 needs both, and the without-divert control shows the dial
//!     otherwise reaches the backend directly.
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, nft, ip netns,
//! ip rule, BPF TCX attach); the body asserts it. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run` (a compile-only gate is
//! green even when every fixture refuses at boot).
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design (it is
//! node-global converge-on-boot), so each test scrubs ALL `overdrive-mtls` nft
//! state + the fwmark rule/route + the test netns/veth/lo-backend at START
//! (tolerate pre-existing) AND, through the [`EgressTopology`] guard, at END —
//! also when the body panics. The divert's release asserts each of its own
//! steps (classifier detached and pins removed, elements removed, program
//! removed). The node-global sysctls the topology relaxes
//! (`net.ipv4.ip_forward`, `net.ipv4.conf.{all,lo}.rp_filter`) are snapshotted
//! and restored by the same guard. A cross-PROCESS `flock(2)` lock
//! (`KernelStateLock`) serialises the kernel-touching tests — nextest runs each
//! `#[test]` in a separate process, so an in-process `serial_test` lock cannot
//! serialise node-global kernel state.

#![allow(
    clippy::doc_markdown,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::match_wildcard_for_single_variants,
    clippy::option_if_let_else,
    reason = "Test bodies; skip messages + evidence go to stderr; failures must panic with informative messages; the SO_MARK/AF_INET casts are FFI-width on compile-time constants; the composed walking proof is a single long scenario; the SocketAddr wildcard arm is the V6 case a v4-only fixture cannot hit; the so_mark match reads clearer than map_or_else"
)]

use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::os::fd::AsRawFd as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_worker::mtls_intercept_port::{HostMtlsIntercept, MtlsIntercept};

use super::leg_listener::{LegListener, accept_leg_within};
use super::mtls_intercept_install::{
    NodeSysctls, SharedOutboundDivert, mac_text, set_link_sysctl, workload_mac,
};

// ---- topology constants (mirror the increment-b spike recipe) ----
const NS_W: &str = "nsW-egr0303";
const VETH_W: &str = "vethW-egr03";
const VETH_H: &str = "vethH-egr03";
// The topology requests a `/24` from overdrive-testing's global
// `10.250.0.0/16` pool under this stable owner name. The pool is disjoint from
// production's `10.99.0.0/16`; the lease supplies the gateway/workload pair.
const CIDR_LEASE_NAME: &str = "worker-egress-tproxy-capture";
/// The "real backend" the workload dials — a host-side lo-bound address the
/// workload routes to via the gateway, so its egress genuinely INGRESSES vethH
/// and hits PREROUTING (not loopback-to-self inside the netns).
const BACKEND_IP: &str = "10.200.0.1";
const BACKEND_PORT: u16 = 18777;

/// Cross-PROCESS exclusion for the shared host-netns kernel state. The
/// `overdrive-mtls` nft table, the fwmark ip-rule, and the table-100 local route
/// are NODE-GLOBAL: every test touching them touches the SAME kernel state.
/// nextest runs each `#[test]` in a SEPARATE PROCESS, so an in-process lock
/// cannot serialise them — an `flock(2)` on a fixed path spans processes. The
/// path is SHARED with `mtls_intercept_install.rs` so the egress + inbound
/// suites cannot race each other's chain dumps.
struct KernelStateLock {
    fd: std::os::fd::OwnedFd,
}

impl KernelStateLock {
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
        unsafe {
            libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// True iff this process is uid 0 (root). IP_TRANSPARENT, nft, `ip netns`, and
/// `ip rule` all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; the body asserts it,
/// so a non-root run fails rather than passing vacuously.
fn is_root() -> bool {
    // SAFETY: getuid is always safe; takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

fn backend_addr() -> SocketAddrV4 {
    SocketAddrV4::new(BACKEND_IP.parse().expect("backend ip"), BACKEND_PORT)
}

// ===================================================================
// command shims
// ===================================================================

/// Run `ip <args>`; panic on non-zero exit (the fixture precondition is "this
/// topology step must succeed"). Returns nothing — the side effect is the point.
fn ip(args: &[&str]) {
    let out = Command::new("ip")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn ip");
    assert!(
        out.status.success(),
        "ip {args:?} exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
}

/// Best-effort `ip <args>` — failure is the "nothing to clean" signal in
/// teardown; non-zero exits are intentionally ignored.
fn ip_quiet(args: &[&str]) {
    let _ = Command::new("ip").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// `nft list table ip overdrive-mtls` (verbatim dump). The listing must
/// succeed: an unreadable table is not an empty one.
fn nft_dump_table() -> String {
    let output = Command::new("nft")
        .args(["list", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn the nft table listing");
    assert!(
        output.status.success(),
        "listing table ip overdrive-mtls failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `uname -r`, which pins the verdict to a kernel (spike.md discipline).
fn kernel_release() -> String {
    let output = Command::new("uname").arg("-r").output().expect("spawn uname -r");
    assert!(output.status.success(), "uname -r failed");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Scrub ALL `overdrive-mtls` nft state + the shared fwmark rule/route so a
/// clean-kernel ground-truth run is reproducible. Run at test START (tolerate
/// pre-existing) AND END. Best-effort: every failure is "nothing to clean".
fn clean_shared_infra() {
    // Drain however many fwmark rules a prior run may have stacked.
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
    ip_quiet(&["route", "del", "local", "0.0.0.0/0", "dev", "lo", "table", "100"]);
    let _ = Command::new("nft")
        .args(["delete", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Tear down the per-test netns + veth pair + lo-backend address. The shared
/// `overdrive-mtls` infra is handled by `clean_shared_infra`.
fn teardown_topology() {
    // Deleting one veth side removes the pair; deleting the netns also frees the
    // workload side. Both best-effort.
    ip_quiet(&["link", "del", VETH_H]);
    ip_quiet(&["netns", "del", NS_W]);
    ip_quiet(&["addr", "del", &format!("{BACKEND_IP}/32"), "dev", "lo"]);
}

/// The per-test topology and the node-global state it relaxes, owned for the
/// body's duration. `Drop` tears the topology down and scrubs the shared
/// intercept infra, then (field order) releases the CIDR lease and restores
/// the node-global sysctls, so a panic mid-body leaves nothing behind.
struct EgressTopology {
    _lease: TestCidrLease,
    _node_sysctls: NodeSysctls,
}

impl EgressTopology {
    /// Stand up the topology of `setup_topology` on the leased `/24`.
    fn provision(lease: TestCidrLease) -> Self {
        setup_topology(&lease);
        // Host-side routing hygiene (NOT a TPROXY concession; spike § Edge
        // cases): forwarding so the host routes the workload's packet to the
        // lo-bound backend; rp_filter relaxation so the asymmetric ingress is
        // not dropped (which would mask the test as a false "no fire"). The
        // node-global keys are restored on drop; the host veth's key goes with
        // the veth.
        let node_sysctls = NodeSysctls::set(
            "03-03",
            &[
                ("net.ipv4.ip_forward", "1"),
                ("net.ipv4.conf.all.rp_filter", "0"),
                ("net.ipv4.conf.lo.rp_filter", "0"),
            ],
        );
        set_link_sysctl("03-03", &format!("net.ipv4.conf.{VETH_H}.rp_filter"), "0");
        Self { _lease: lease, _node_sysctls: node_sysctls }
    }
}

impl Drop for EgressTopology {
    fn drop(&mut self) {
        teardown_topology();
        clean_shared_infra();
    }
}

/// Stand up the netns + veth pair + addresses as the increment-b spike does,
/// with the workload veth carrying [`workload_mac`] of its address (the MAC the
/// guest classifier's endpoint record names). The real backend lives on host
/// `lo`; the workload routes to it via the gateway so its egress ingresses vethH
/// — where the classifier runs — and hits PREROUTING. Every step must succeed
/// ([`ip`]).
fn setup_topology(lease: &TestCidrLease) {
    // Start from a clean slate (a prior crashed run leaves residue).
    teardown_topology();

    let host_gateway = lease.host_gateway().to_string();
    let workload_addr = lease.workload_addr().to_string();
    let prefix_len = lease.prefix_len().to_string();
    let workload_mac_text = mac_text(workload_mac(lease.workload_addr()));

    ip(&["netns", "add", NS_W]);
    ip(&["link", "add", VETH_W, "type", "veth", "peer", "name", VETH_H]);
    ip(&["link", "set", VETH_W, "address", &workload_mac_text]);
    ip(&["link", "set", VETH_W, "netns", NS_W]);

    // Host side: address + up.
    ip(&["addr", "add", &format!("{host_gateway}/{prefix_len}"), "dev", VETH_H]);
    ip(&["link", "set", VETH_H, "up"]);

    // Workload side (inside netns): lo up + address + up + default route.
    ip(&["netns", "exec", NS_W, "ip", "link", "set", "lo", "up"]);
    ip(&[
        "netns",
        "exec",
        NS_W,
        "ip",
        "addr",
        "add",
        &format!("{workload_addr}/{prefix_len}"),
        "dev",
        VETH_W,
    ]);
    ip(&["netns", "exec", NS_W, "ip", "link", "set", VETH_W, "up"]);
    ip(&["netns", "exec", NS_W, "ip", "route", "add", "default", "via", &host_gateway]);

    // The real-backend address lives on host lo so the host can bind+listen on
    // it; the workload routes to it via the gateway.
    ip(&["addr", "add", &format!("{BACKEND_IP}/32"), "dev", "lo"]);

    // No TX-checksum-offload change: the TPROXY divert rewrites no header, so
    // no checksum base is needed (`.claude/rules/bpf.md` Rule 2 concerns an
    // incremental checksum after a NAT rewrite).
}

/// Run a `/dev/tcp` client INSIDE the workload netns: connect to `dst`, send a
/// marker, read one line of echo. Optionally stamp `SO_MARK` on the client
/// socket via Python (bash `/dev/tcp` cannot set sockopts; the self-exempt probe
/// needs a real SO_MARK from inside the netns). Returns the client's stdout.
///
/// `so_mark = None` → plain bash `/dev/tcp` client.
/// `so_mark = Some(m)` → a Python client that sets `SO_MARK = m` before connect,
///   proving a workload CANNOT self-exempt: the mark is skb-local metadata that
///   does not cross the veth/netns boundary, and the guest classifier at the
///   host veth stamps the intercept mark on the guest's TCP, so the shared
///   divert still captures the connection.
fn run_client_in_netns(dst: SocketAddrV4, so_mark: Option<u32>) -> String {
    let (prog, script): (&str, String) = match so_mark {
        None => (
            "bash",
            // Success == connect + send succeeded (`WL-SENT`); the read is
            // best-effort because the server side asserts on the bytes it
            // RECEIVED and does not always echo. A connect failure prints
            // `CLIENT-FAIL`.
            format!(
                "{{ exec 3<>/dev/tcp/{ip}/{port} && printf 'HELLO-FROM-WORKLOAD' >&3 && \
                 echo WL-SENT; }} || echo CLIENT-FAIL",
                ip = dst.ip(),
                port = dst.port(),
            ),
        ),
        Some(mark) => (
            "python3",
            // Built line-by-line to avoid backslash-continuation escape pitfalls;
            // SO_MARK is sockopt 36 (SOL_SOCKET). The mark is set INSIDE the
            // workload netns — it is skb-local and does NOT cross the veth, and
            // the host veth's classifier stamps the intercept mark, so the
            // shared divert still captures the connection.
            // Success == connect + send succeeded (`WL-MARKED-SENT`); the recv
            // is best-effort (the leg-F side asserts via getsockname and does
            // not echo). A connect failure prints `CLIENT-FAIL`.
            [
                "import socket".to_owned(),
                "s=socket.socket(socket.AF_INET,socket.SOCK_STREAM)".to_owned(),
                format!("s.setsockopt(socket.SOL_SOCKET,36,{mark})"),
                "s.settimeout(3)".to_owned(),
                "try:".to_owned(),
                format!("    s.connect(('{}',{}))", dst.ip(), dst.port()),
                "    s.sendall(b'HELLO-MARKED-WL')".to_owned(),
                "    print('WL-MARKED-SENT')".to_owned(),
                "except Exception as e:".to_owned(),
                "    print('CLIENT-FAIL:'+str(e))".to_owned(),
            ]
            .join("\n"),
        ),
    };
    let out = Command::new("ip")
        .args(["netns", "exec", NS_W, prog, "-c", &script])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) => format!(
            "[exit={:?}] stdout={} stderr={}",
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => format!("spawn client failed: {e}"),
    }
}

/// Accept on `listener` within `timeout` by polling a non-blocking accept.
/// Returns the accepted connection or a TimedOut error (the failure shape that
/// would mean the connection went to the OTHER listener).
fn accept_with_timeout(
    listener: &TcpListener,
    timeout: Duration,
) -> std::io::Result<(TcpStream, std::net::SocketAddr)> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok(pair) => {
                pair.0.set_nonblocking(false)?;
                return Ok(pair);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "no connection within timeout",
                    ));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Dial `addr` with `SO_MARK = mark` set BEFORE connect (the shape the agent's
/// own leg dial uses). Mirrors the sibling test's `dial_with_so_mark`. Returns
/// the connected stream.
fn dial_with_so_mark(
    addr: SocketAddrV4,
    mark: u32,
    timeout: Duration,
) -> std::io::Result<TcpStream> {
    use std::os::fd::FromRawFd as _;
    // SAFETY: a fresh AF_INET stream socket; SO_MARK is set before connect; the
    // fd is adopted by TcpStream::from_raw_fd which owns it.
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
    let sa: libc::sockaddr_in = {
        let mut s: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        s.sin_family = libc::AF_INET as libc::sa_family_t;
        s.sin_port = addr.port().to_be();
        s.sin_addr.s_addr = u32::from_ne_bytes(addr.ip().octets());
        s
    };
    stream.set_read_timeout(Some(timeout))?;
    // SAFETY: stream owns a live AF_INET socket fd; sa is a correctly-sized
    // sockaddr_in for the connect target.
    let rc = unsafe {
        libc::connect(
            stream.as_raw_fd(),
            std::ptr::from_ref(&sa).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// THE deliverable (ADR-0071 Tier-3 (a) + (b)): compose the production shared
/// outbound divert (`SharedOutboundDivert`: `converge_shared`,
/// `install_outbound`, and the guest TCX ingress classifier) with the
/// `MtlsIntercept` port's leg-F listener (`HostMtlsIntercept::bind_transparent`)
/// and its accept (through `LegListener::accept_leg`) on the REAL kernel.
///
/// Proves, in order:
///   AC4 (without-divert control): with NO shared program and NO classifier,
///        the workload's `connect(backend)` reaches the REAL backend directly —
///        isolating "fired" from "passed through" (debugging.md §5/§11).
///   AC1 (with-divert redirect + getsockname recovery): with the shared program
///        converged at the two legs, the workload's source admitted, and the
///        guest classifier attached at the host veth, the workload's `connect`
///        is diverted to the leg-F IP_TRANSPARENT listener; the port listener's
///        accept recovers orig-dst via getsockname == the dialed (ip,port); the
///        classifier's intercept count advances with the dial.
///   AC2-a (agent HOST dial reaches the backend — by TOPOLOGY, NOT the leg-S
///        exemption): the agent's HOST-netns dial carrying
///        `SO_MARK = MTLS_LEG_S_DIAL_MARK` reaches the REAL backend directly
///        (NOT diverted to leg-F) because it originates host-side and never
///        ingresses the workload veth: the guest classifier never sees it, so it
///        carries no intercept mark, and its source is no admitted outbound
///        source, so shared rule 1 cannot match it — WITH OR WITHOUT the leg-S
///        exemption. Its destination is no managed guest, so no drop rule
///        applies either. See the inline gap note at the AC2-a block for why a
///        load-bearing *dial-direction* exemption control is out of 03-03's
///        scope.
///   AC2-b (self-exempt-impossible — the SAFE negative control): a WORKLOAD dial
///        that sets `SO_MARK` INSIDE its own netns is STILL captured to leg-F —
///        the mark is skb-local and does not cross the veth/netns boundary, and
///        the host veth's classifier stamps the intercept mark on the guest's
///        TCP, so a workload cannot self-exempt against the host-side divert.
#[test]
fn workload_egress_redirects_to_legf_and_getsockname_recovers_orig_dst() {
    assert!(
        is_root(),
        "the egress capture proof requires root and CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, \
         nft, ip netns, ip rule)"
    );

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = kernel_release();
    eprintln!("[03-03] uname -r = {kr}");

    // Cross-process exclusion: hold the shared-kernel-state lock for the whole
    // body (the lock is shared with the inbound suite). The topology guard is
    // declared after the lock, so it tears down (and restores the node-global
    // sysctls) before the lock is released, panic or not.
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    let lease =
        TestCidrLease::acquire(CIDR_LEASE_NAME).expect("acquire egress topology CIDR lease");
    let workload_addr = lease.workload_addr();
    let _topology = EgressTopology::provision(lease);

    let backend = backend_addr();

    // ----------------------------------------------------------------
    // AC4 — WITHOUT-divert control: no shared program and no classifier yet.
    // The workload's connect reaches the REAL backend directly. This isolates
    // "redirect fired" from "passed through" and proves the divert (not the
    // topology) is what redirects.
    // ----------------------------------------------------------------
    let control_backend = TcpListener::bind(backend).expect("bind real backend (control)");
    let control_client = std::thread::spawn(move || run_client_in_netns(backend, None));
    let (mut conn, control_peer) = accept_with_timeout(&control_backend, Duration::from_secs(8))
        .expect(
            "WITHOUT-divert control: workload connect must reach the REAL backend directly \
             (nothing installed). A timeout here means the topology itself is broken.",
        );
    let mut buf = [0u8; 19];
    conn.read_exact(&mut buf).expect("read control marker");
    assert_eq!(&buf, b"HELLO-FROM-WORKLOAD", "control: backend must receive the workload's bytes");
    let control_out = control_client.join().expect("control client thread");
    eprintln!("[03-03][AC4 without-divert control] backend accepted peer={control_peer}");
    eprintln!("[03-03][AC4 without-divert control] client: {control_out}");
    // The accepted peer is the workload's veth address (it came through the veth,
    // not loopback-to-self) — confirms a genuine remote dial.
    assert!(
        matches!(control_peer, std::net::SocketAddr::V4(v4) if *v4.ip() == workload_addr),
        "control: backend peer must be the workload's veth addr {workload_addr}, got {control_peer}"
    );
    drop(control_backend); // free the port before the redirect phase rebinds it

    // ----------------------------------------------------------------
    // AC1 — WITH the shared divert: converge the shared program, admit the
    // workload's source, attach the guest classifier, drive the SAME dial, and
    // prove the divert to leg-F + getsockname orig-dst recovery.
    // ----------------------------------------------------------------
    // leg-F MUST be IP_TRANSPARENT (TPROXY delivers orig-dst-addressed packets).
    // Both legs are bound through the `MtlsIntercept` port, whose host adapter
    // owns the transparent sockets; leg-F is read and accepted through the
    // `LegListener` bridge so this body is unchanged when the port's listener
    // type changes (gap B-7). Leg-C is the shared program's inbound target; no
    // connection reaches it here.
    let intercept = HostMtlsIntercept::new();
    let leg_f = Arc::new(
        intercept
            .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("HostMtlsIntercept::bind_transparent leg-F"),
    );
    let leg_c = intercept
        .bind_transparent(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("HostMtlsIntercept::bind_transparent leg-C");
    let outbound_target = leg_f.bound_v4().expect("leg-F bound IPv4 address");
    let inbound_target = leg_c.bound_v4().expect("leg-C bound IPv4 address");
    let leg_f_port = outbound_target.port();

    // The driving ports under test: `converge_shared` at the two legs,
    // `install_outbound(workload, leg-F port)`, and the guest classifier at
    // vethH's TCX ingress with the workload's endpoint. The fixture asserts
    // each step (program identity, the workload's two elements, the endpoint
    // read-back, exactly one attached program).
    let divert = SharedOutboundDivert::install(
        "03-03",
        &intercept,
        outbound_target,
        inbound_target,
        workload_addr,
        workload_mac(workload_addr),
        VETH_H,
    );
    let intercepted_before = divert.intercepted();
    eprintln!("[03-03][AC1] shared divert installed; classifier intercepted {intercepted_before}");
    eprintln!("[03-03][AC1] nft table after the shared divert:\n{}", nft_dump_table());

    // No listener waits at the backend during the divert phases. A dial the
    // divert missed could not complete there either — shared output rule 2
    // drops an unmarked listener's SYN-ACK to the managed guest — so a listener
    // at the destination cannot tell the two apart. The leg-F accept and the
    // classifier's intercept count are the witnesses.
    let redirect_client = std::thread::spawn(move || run_client_in_netns(backend, None));

    // The port listener's accept drives the production getsockname recovery on
    // the TPROXY-intercepted leg-F socket: the accepted connection's `local` is
    // the recovered orig-dst (the resolve consumer that classifies it is 04-02's
    // default-lane DST job — here we prove the kernel-side capture +
    // getsockname recovery). The wait is bounded: if the divert silently
    // failed, the accept reports a clean timeout after 8 s instead of hanging
    // to the 120 s slow-timeout SIGKILL.
    let (leg, _peer, got) =
        accept_leg_within(&leg_f, Duration::from_secs(8)).unwrap_or_else(|error| {
            panic!(
                "the port listener's accept must recover orig-dst from the shared divert. A \
                 timeout here (no connection within 8 s) means the divert did NOT deliver to \
                 leg-F ({error}); classifier counters: {}",
                divert.counter_snapshot()
            )
        });

    // AC1: the divert fired (leg-F accepted) AND getsockname recovered the
    // dialed orig-dst.
    eprintln!("[03-03][AC1] getsockname(leg-F accepted) = {got}");
    eprintln!("[03-03][AC1] expected dialed backend    = {backend}");
    assert_eq!(
        got, backend,
        "getsockname-recovered orig-dst must equal the dialed backend {backend}"
    );
    assert_ne!(
        got.port(),
        leg_f_port,
        "recovered orig-dst port must be the backend port, NOT leg-F's bound port"
    );
    assert_ne!(
        u32::from(*got.ip()),
        u32::from(Ipv4Addr::LOCALHOST),
        "recovered orig-dst must be the backend addr, NOT leg-F's loopback bind addr"
    );
    drop(leg);
    let redirect_out = redirect_client.join().expect("redirect client thread");
    eprintln!("[03-03][AC1] redirect-phase client: {redirect_out}");
    let intercepted_after_ac1 = divert.intercepted();
    assert!(
        intercepted_after_ac1 > intercepted_before,
        "AC1: the workload's TCP reached leg-F through the classifier's intercept mark — the \
         Intercept count must advance past {intercepted_before}, got {intercepted_after_ac1}"
    );

    // ----------------------------------------------------------------
    // AC2-a — agent HOST dial reaches the backend by TOPOLOGY (NOT the leg-S
    // exemption): the agent's HOST-netns dial carrying
    // SO_MARK = MTLS_LEG_S_DIAL_MARK reaches the REAL backend directly (NOT
    // diverted to leg-F) because it originates host-side and never ingresses
    // the workload veth. The guest classifier never sees it, so it carries no
    // intercept mark `0x295a`, and its source is no admitted outbound source —
    // shared rule 1 cannot match it, WITH OR WITHOUT the leg-S exemption. Its
    // destination (10.200.0.1, on host `lo`) is no managed guest, so neither
    // managed-guest drop applies. This does NOT exercise the leg-S exemption.
    //
    // GAP NOTE (honest, per the 03-03 review): the exemption's
    // load-bearingness FOR THE AGENT'S leg-S re-dial is a SEPARATE, UNPROVEN
    // (possibly inapplicable) claim that touches ADR-0071's obligation-(b)
    // framing and depends on how leg-S is wired in 04-01/04-02. For EGRESS, the
    // ADR-0071 Tier-3 obligation (b) is satisfied HERE by AC2-b
    // (self-exempt-impossible) alone. A load-bearing *dial-direction* exemption
    // control would require a dial that actually INGRESSES the workload veth
    // carrying the leg-S mark (the agent's real leg-S dial path, wired in
    // 04-01/04-02) — a host-`lo` dial carries no intercept mark and so cannot
    // exercise the exemption. That is explicitly out of 03-03's scope. In the
    // reply direction the exemption IS load-bearing on this path: leg-F's
    // replies to the workload carry the leg-S mark, and shared output rule 0
    // accepts them ahead of the managed-guest drop (output rule 2), so AC1's
    // completed handshake depends on it. (The exemption's other role is on the
    // shared program's inbound-destination rules, where a host-originated
    // marked dial to a registered destination DOES match and WOULD loop
    // without it.)
    let agent_backend = TcpListener::bind(backend).expect("bind real backend (AC2-a topology)");
    let agent_dial = std::thread::spawn(move || -> std::io::Result<()> {
        use std::io::Write as _;
        let mut s = dial_with_so_mark(backend, MTLS_LEG_S_DIAL_MARK, Duration::from_secs(8))?;
        s.write_all(b"AGENT-MARKED")?;
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    });
    let (mut agent_conn, agent_peer) = accept_with_timeout(&agent_backend, Duration::from_secs(5))
        .expect(
            "AC2-a topology: the agent's HOST dial must reach the REAL backend directly because \
             it originates host-side and never passes the workload veth's classifier, so shared \
             rule 1 cannot match it (with or without the leg-S exemption). A timeout here means \
             the host-side routing/topology is broken — NOT that the exemption is broken (this \
             path does not exercise the exemption).",
        );
    let mut abuf = [0u8; 12];
    agent_conn.read_exact(&mut abuf).expect("read agent marker");
    assert_eq!(
        &abuf, b"AGENT-MARKED",
        "AC2-a topology: backend must receive the agent's marked bytes (a host dial is never \
         intercept-marked)"
    );
    agent_dial
        .join()
        .expect("agent dial thread")
        .expect("AC2-a: the agent's marked HOST dial connects and sends its marker");
    eprintln!(
        "[03-03][AC2-a topology] agent HOST dial reached backend directly (never \
         intercept-marked, NOT via the leg-S exemption), peer={agent_peer}"
    );
    // The agent dial originates in the HOST netns (NOT via the veth), so its peer
    // is the loopback source the host kernel picks for a lo-bound dst — proving
    // it never traversed the veth and never met the classifier. This is WHY it
    // reaches the backend: the topology non-match, not the leg-S exemption. See
    // the AC2-a gap note above.
    drop(agent_backend);

    // ----------------------------------------------------------------
    // AC2-b — SELF-EXEMPT-IMPOSSIBLE (safe negative control): a WORKLOAD dial
    // that sets SO_MARK INSIDE its own netns is STILL captured to leg-F. SO_MARK
    // is skb-local metadata that does NOT cross the veth/netns boundary, and
    // the classifier at vethH stamps the intercept mark on the guest's TCP —
    // a workload cannot self-exempt. We prove capture by getsockname recovery
    // on leg-F again, and by the intercept count advancing with the dial.
    // ----------------------------------------------------------------
    let selfexempt_client =
        std::thread::spawn(move || run_client_in_netns(backend, Some(MTLS_LEG_S_DIAL_MARK)));
    // The same bounded accept through the port listener.
    let (leg2, _peer2, got2) =
        accept_leg_within(&leg_f, Duration::from_secs(8)).unwrap_or_else(|error| {
            panic!(
                "self-exempt-impossible: a workload's SO_MARK-stamped dial must STILL be \
                 captured to leg-F (the mark does not cross the netns boundary). A timeout here \
                 (no connection within 8 s) means the workload self-exempted — a security hole \
                 ({error}); classifier counters: {}",
                divert.counter_snapshot()
            )
        });
    eprintln!(
        "[03-03][AC2-b self-exempt-impossible] workload marked dial STILL captured; getsockname = {got2}"
    );
    assert_eq!(
        got2, backend,
        "self-exempt-impossible: the workload's marked dial is still captured to leg-F \
         and getsockname recovers the dialed backend {backend}"
    );
    drop(leg2);
    let selfexempt_out = selfexempt_client.join().expect("self-exempt client thread");
    eprintln!("[03-03][AC2-b self-exempt-impossible] client: {selfexempt_out}");
    let intercepted_after_ac2b = divert.intercepted();
    assert!(
        intercepted_after_ac2b > intercepted_after_ac1,
        "AC2-b: the workload's marked dial passed the classifier's intercept mark — the \
         Intercept count must advance past {intercepted_after_ac1}, got {intercepted_after_ac2b}"
    );

    eprintln!(
        "[03-03] VERDICT: WORKS — shared-divert redirect + getsockname recovery + \
         self-exempt-impossible (ADR-0071 obligation (b) for egress) validated on kernel {kr}. \
         AC2-a's agent HOST dial reaches the backend by topology non-match (never \
         intercept-marked), NOT via the leg-S exemption — a load-bearing dial-direction exemption \
         control needs the real leg-S veth-ingress dial wired in 04-01/04-02 (out of 03-03 scope)."
    );

    // Teardown: release the divert in reverse, asserting each step (classifier
    // detached and its pins removed, the workload's elements removed, the
    // member-free program removed by the node guard's drop), then the legs;
    // the topology guard then scrubs the shared infra + topology and restores
    // the node-global sysctls, so a clean-kernel re-run reproduces.
    divert.release();
    drop((leg_c, leg_f));
}
