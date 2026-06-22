//! Tier-3 EGRESS capture walking proof (step 03-03) — the egress half of the
//! ADR-0071 § Enforcement Tier-3 obligations (a)+(b), composing two production
//! functions that ALREADY landed (03-01 / 03-02) on the live kernel:
//!
//!   - `install_outbound_tproxy(host_veth, leg_f_port)` (03-01) — appends an
//!     `iifname <host_veth> meta l4proto tcp tproxy to 127.0.0.1:<leg_f>` rule to
//!     the shared `overdrive-mtls` PREROUTING chain (after the F5 exemption) and
//!     ensures the shared fwmark rule / local route / table idempotently.
//!   - `accept_outbound_leg(leg_f_listener, alloc, _peer)` (03-02) — recovers the
//!     workload's dialed orig-dst via `getsockname` on the TPROXY-intercepted
//!     leg-F socket; builds `Routed::Outbound { peer }` from the RECOVERED addr.
//!   - `make_transparent_listener(addr)` — leg-F MUST be `IP_TRANSPARENT` because
//!     TPROXY delivers packets whose dst is the orig-dst (NOT leg-F's bound addr);
//!     a non-transparent socket cannot receive them.
//!
//! NO new production code — this step is test-only, composing the above on a
//! REAL kernel through the real netns + veth + nft + ip-rule topology proven by
//! the increment-b spike (`docs/feature/.../spike/findings-egress-tproxy.md`,
//! VERDICT WORKS, kernel 7.0.0-22-generic).
//!
//! Topology (mirrors the spike EXACTLY):
//!
//!   netns nsW:  workload client; vethW 10.99.0.2/24; default via .1
//!                 connect(10.200.0.1:18777)
//!     <== veth ==>
//!   host netns: vethH 10.99.0.1/24
//!                 PREROUTING (priority mangle):
//!                   meta mark 0x2 accept            <- F5 exemption (chain head)
//!                   iifname vethH meta l4proto tcp tproxy to 127.0.0.1:<legF>
//!                                                   meta mark set 0x1 accept
//!                 ip rule fwmark 0x1 lookup 100
//!                 ip route local 0.0.0.0/0 dev lo table 100
//!                 leg-F IP_TRANSPARENT 127.0.0.1:<legF>
//!                 real backend 10.200.0.1:18777    (host lo)
//!
//! Port-to-port: every assertion enters through the `mtls_intercept` module's
//! public driving-port fns (`install_outbound_tproxy` / `accept_outbound_leg` /
//! `make_transparent_listener`) and asserts at the kernel/socket boundary:
//! `getsockname` orig-dst recovery, the accepted-socket peer, and which listener
//! (leg-F vs the real backend) received the connection. Litmus:
//!   - gut `accept_outbound_leg`'s body → the `getsockname == dialed-dst`
//!     assertion goes RED (the orig-dst is recovered by production code, not the
//!     fixture);
//!   - remove the `iifname` rule append in `install_outbound_tproxy` → the
//!     redirect assertion goes RED (the without-TPROXY control proves this — the
//!     workload reaches the backend directly instead of leg-F).
//!
//! Requires root + CAP_NET_ADMIN/CAP_SYS_ADMIN (IP_TRANSPARENT, nft, ip netns,
//! ip rule). A non-root run SKIPs. Run via
//! `cargo xtask lima run -- cargo nextest run -p overdrive-worker
//! --features integration-tests`. NEVER `--no-run` (a compile-only gate is
//! green even when every fixture refuses at boot).
//!
//! Hygiene: the shared `overdrive-mtls` routing infra PERSISTS by design (it is
//! node-global converge-on-boot), so each test scrubs ALL `overdrive-mtls` nft
//! state + the fwmark rule/route + the test netns/veth/lo-backend at START
//! (tolerate pre-existing) AND END. A cross-PROCESS `flock(2)` lock
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
use std::time::{Duration, Instant};

use overdrive_core::dataplane::MTLS_LEG_S_DIAL_MARK;
use overdrive_worker::mtls_intercept::{
    accept_outbound_and_recover_orig_dst, install_outbound_tproxy, make_transparent_listener,
};

// ---- topology constants (mirror the increment-b spike recipe) ----
const NS_W: &str = "nsW-egr0303";
const VETH_W: &str = "vethW-egr03";
const VETH_H: &str = "vethH-egr03";
const HOST_GW: &str = "10.99.0.1";
const WL_ADDR: &str = "10.99.0.2";
const SUBNET_LEN: &str = "24";
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
/// `ip rule` all need root + CAP_NET_ADMIN/CAP_SYS_ADMIN; a non-root run cannot
/// stand up the fixture, so we SKIP rather than fail.
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

/// Best-effort `sysctl -w <kv>` for host-side routing hygiene.
fn sysctl_w(kv: &str) {
    let _ = Command::new("sysctl")
        .args(["-w", kv])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// `nft list table ip overdrive-mtls` (verbatim dump) for evidence.
fn nft_dump_table() -> String {
    Command::new("nft")
        .args(["list", "table", "ip", "overdrive-mtls"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
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

/// Stand up the netns + veth pair + addresses + host routing hygiene EXACTLY as
/// the increment-b spike does. The real backend lives on host `lo`; the workload
/// routes to it via the gateway so its egress ingresses vethH and hits
/// PREROUTING.
fn setup_topology() {
    // Start from a clean slate (a prior crashed run leaves residue).
    teardown_topology();

    ip(&["netns", "add", NS_W]);
    ip(&["link", "add", VETH_W, "type", "veth", "peer", "name", VETH_H]);
    ip(&["link", "set", VETH_W, "netns", NS_W]);

    // Host side: address + up.
    ip(&["addr", "add", &format!("{HOST_GW}/{SUBNET_LEN}"), "dev", VETH_H]);
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
        &format!("{WL_ADDR}/{SUBNET_LEN}"),
        "dev",
        VETH_W,
    ]);
    ip(&["netns", "exec", NS_W, "ip", "link", "set", VETH_W, "up"]);
    ip(&["netns", "exec", NS_W, "ip", "route", "add", "default", "via", HOST_GW]);

    // The real-backend address lives on host lo so the host can bind+listen on
    // it; the workload routes to it via the gateway.
    ip(&["addr", "add", &format!("{BACKEND_IP}/32"), "dev", "lo"]);

    // Host-side routing hygiene (NOT a TPROXY concession; spike § Edge cases):
    // forwarding so the host routes the workload's packet to the lo-bound
    // backend; rp_filter relaxation so the asymmetric ingress is not dropped
    // (which would mask the test as a false "no fire").
    sysctl_w("net.ipv4.ip_forward=1");
    sysctl_w(&format!("net.ipv4.conf.{VETH_H}.rp_filter=0"));
    sysctl_w("net.ipv4.conf.all.rp_filter=0");
    sysctl_w("net.ipv4.conf.lo.rp_filter=0");

    // bpf.md Rule 2 / spike: disable TX-checksum-offload on the host veth (the
    // veth CHECKSUM_PARTIAL invariant). Best-effort — ethtool may be absent, and
    // for a pure TPROXY redirect (no NAT rewrite) this is belt-and-braces.
    let _ = Command::new("ethtool")
        .args(["-K", VETH_H, "tx", "off"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Run a `/dev/tcp` client INSIDE the workload netns: connect to `dst`, send a
/// marker, read one line of echo. Optionally stamp `SO_MARK` on the client
/// socket via Python (bash `/dev/tcp` cannot set sockopts; the self-exempt probe
/// needs a real SO_MARK from inside the netns). Returns the client's stdout.
///
/// `so_mark = None` → plain bash `/dev/tcp` client.
/// `so_mark = Some(m)` → a Python client that sets `SO_MARK = m` before connect,
///   proving a workload CANNOT self-exempt: the mark is skb-local metadata that
///   does not cross the veth/netns boundary, so the host-side `iifname` rule
///   still captures the connection.
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
            // workload netns — it is skb-local and does NOT cross the veth, so
            // the host-side iifname rule still captures the connection.
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
                pair.0.set_nonblocking(false).ok();
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
    stream.set_read_timeout(Some(timeout)).ok();
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
    stream.set_nodelay(true).ok();
    Ok(stream)
}

/// Bound a blocking `accept()` on `listener` to `timeout` by setting
/// `SO_RCVTIMEO` on the listener socket. On Linux, `SO_RCVTIMEO` applies to
/// `accept(2)` — a `listen`ing socket with the timeout set returns
/// `EAGAIN`/`EWOULDBLOCK` after `timeout` with no incoming connection. This lets
/// the PRODUCTION `accept_outbound_leg`'s internal blocking `leg_f.accept()`
/// (mtls_intercept.rs) return a clean error after a bounded wait instead of
/// hanging forever to nextest's 120 s slow-timeout SIGKILL (which reads
/// identically to "VM hung / infra broke" per debugging.md § 11). The happy path
/// (a connection arrives in well under `timeout`) is unaffected; only the
/// silent-redirect-failure path clean-fails. The production API is UNCHANGED —
/// this is a test-side socket-option tweak applied before handing `&leg_f` to
/// the production fn.
///
/// If `SO_RCVTIMEO`-on-listener does not reliably bound `accept()` on this
/// kernel, the bound is best-effort and the production accept may still block;
/// the diagnostic `.expect()` messages at the call sites document the
/// hang-on-failure shape so a future failure is still diagnosable.
fn bound_listener_accept(listener: &TcpListener, timeout: Duration) {
    let tv = libc::timeval {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_usec: libc::suseconds_t::from(timeout.subsec_micros()),
    };
    // SAFETY: listener owns a live socket fd; SO_RCVTIMEO takes a `timeval` of
    // the size passed. A non-zero return is a best-effort failure (the bound is
    // not load-bearing for correctness — only for the diagnostic failure shape),
    // so we log rather than panic.
    let rc = unsafe {
        libc::setsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            std::ptr::from_ref(&tv).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        eprintln!(
            "[03-03] warn: SO_RCVTIMEO on leg-F listener failed ({}); production accept may \
             hang to slow-timeout on a silent redirect failure",
            std::io::Error::last_os_error()
        );
    }
}

/// THE deliverable (ADR-0071 Tier-3 (a) + (b)): compose `install_outbound_tproxy`
/// + `accept_outbound_leg` + `make_transparent_listener` on the REAL kernel.
///
/// Proves, in order:
///   AC4 (without-TPROXY control): with NO egress rule, the workload's
///        `connect(backend)` reaches the REAL backend directly — isolating
///        "fired" from "passed through" (debugging.md §5/§11).
///   AC1 (with-TPROXY redirect + getsockname recovery): `install_outbound_tproxy`
///        appends the `iifname <host_veth>` rule; the workload's `connect` is
///        redirected to the leg-F IP_TRANSPARENT listener; `accept_outbound_leg`
///        recovers orig-dst via getsockname == the dialed (ip,port).
///   AC2-a (agent HOST dial reaches the backend — by TOPOLOGY, NOT the F5
///        exemption): the agent's HOST-netns dial carrying
///        `SO_MARK = MTLS_LEG_S_DIAL_MARK` reaches the REAL backend directly
///        (NOT re-captured to leg-F) because it originates host-side and never
///        ingresses the workload veth, so the production `iifname <host_veth>`
///        egress rule cannot match it — WITH OR WITHOUT the F5 exemption. This
///        path does NOT exercise the egress F5 exemption: the exemption is
///        irrelevant to the `iifname` rule (it matches on ingress interface, not
///        destination), and its load-bearing role is on the SHARED chain's
///        INBOUND `ip daddr`/`tcp dport` rules — where a host-originated marked
///        dial to a virt DOES match and WOULD loop. See the inline gap note at
///        the AC2-a block for why a genuinely load-bearing egress F5 *positive*
///        control is out of 03-03's scope.
///   AC2-b (self-exempt-impossible — the SAFE negative control): a WORKLOAD dial
///        that sets `SO_MARK` INSIDE its own netns is STILL captured to leg-F —
///        the mark is skb-local and does not cross the veth/netns boundary, so a
///        workload cannot self-exempt against the host-side `iifname` rule.
#[test]
fn workload_egress_redirects_to_legf_and_getsockname_recovers_orig_dst() {
    if !is_root() {
        eprintln!(
            "SKIP workload_egress_redirects_to_legf_and_getsockname_recovers_orig_dst: not root"
        );
        return;
    }

    // Pin the verdict to a kernel (spike.md discipline).
    let kr = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    eprintln!("[03-03] uname -r = {kr}");

    // Cross-process exclusion: hold the shared-kernel-state lock for the whole
    // body (the lock is shared with the inbound suite).
    let _kernel_lock = KernelStateLock::acquire();
    clean_shared_infra();
    setup_topology();

    let backend = backend_addr();

    // ----------------------------------------------------------------
    // AC4 — WITHOUT-TPROXY control: no egress rule installed yet. The
    // workload's connect reaches the REAL backend directly. This isolates
    // "redirect fired" from "passed through" and proves the install (not the
    // topology) is what redirects.
    // ----------------------------------------------------------------
    let control_backend = TcpListener::bind(backend).expect("bind real backend (control)");
    let control_client = std::thread::spawn(move || run_client_in_netns(backend, None));
    let (mut conn, control_peer) = accept_with_timeout(&control_backend, Duration::from_secs(8))
        .expect(
            "WITHOUT-TPROXY control: workload connect must reach the REAL backend directly \
             (no rule installed). A timeout here means the topology itself is broken.",
        );
    let mut buf = [0u8; 19];
    conn.read_exact(&mut buf).expect("read control marker");
    assert_eq!(&buf, b"HELLO-FROM-WORKLOAD", "control: backend must receive the workload's bytes");
    let control_out = control_client.join().expect("control client thread");
    eprintln!("[03-03][AC4 without-TPROXY control] backend accepted peer={control_peer}");
    eprintln!("[03-03][AC4 without-TPROXY control] client: {control_out}");
    // The accepted peer is the workload's veth address (it came through the veth,
    // not loopback-to-self) — confirms a genuine remote dial.
    assert!(
        matches!(control_peer, std::net::SocketAddr::V4(v4) if *v4.ip() == WL_ADDR.parse::<Ipv4Addr>().unwrap()),
        "control: backend peer must be the workload's veth addr {WL_ADDR}, got {control_peer}"
    );
    drop(control_backend); // free the port before the redirect phase rebinds it

    // ----------------------------------------------------------------
    // AC1 — WITH-TPROXY: install the egress rule, drive the SAME dial, prove the
    // redirect to leg-F + getsockname orig-dst recovery.
    // ----------------------------------------------------------------
    // leg-F MUST be IP_TRANSPARENT (TPROXY delivers orig-dst-addressed packets).
    // FORWARD-NOTE (04-01): make_transparent_listener's rustdoc
    // (mtls_intercept.rs:127) still says "inbound leg-C", but the fn is
    // direction-agnostic and 03-03 exercises it here for leg-F (egress). 04-01
    // wires both legs and touches that file — broaden the rustdoc there to cover
    // leg-C (inbound) / leg-F (egress). (Test-only step: cannot touch src/ here.)
    let leg_f = make_transparent_listener(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .expect("make_transparent_listener leg-F");
    let leg_f_port = match leg_f.local_addr().expect("leg-F local_addr") {
        std::net::SocketAddr::V4(a) => a.port(),
        other => panic!("expected V4 leg-F addr, got {other}"),
    };

    // The driving port under test: install the egress rule matching
    // `iifname VETH_H` → redirect ALL the workload's egress TCP to leg-F.
    let guard = install_outbound_tproxy(VETH_H, leg_f_port)
        .expect("install_outbound_tproxy must append the iifname egress rule + shared infra");

    let dump = nft_dump_table();
    eprintln!("[03-03] nft table after install_outbound_tproxy:\n{dump}");
    assert!(
        dump.contains(&format!("iifname \"{VETH_H}\"")) && dump.contains("tproxy to"),
        "the iifname egress rule must be installed in the shared chain, got:\n{dump}"
    );

    // Re-bind a real backend so that IF the redirect failed to fire, the dial
    // would land here (the with/without contrast — a hung accept on leg-F with
    // the backend silent is the unambiguous "redirect fired" signal).
    let backend_fallback = TcpListener::bind(backend).expect("bind real backend (redirect phase)");
    backend_fallback.set_nonblocking(true).ok();

    let redirect_client = std::thread::spawn(move || run_client_in_netns(backend, None));

    // accept_outbound_and_recover_orig_dst drives the production getsockname
    // recovery on the TPROXY-intercepted leg-F socket and returns the recovered
    // orig-dst (the resolve consumer that classifies it is 04-02's default-lane
    // DST job — here we prove the kernel-side capture + getsockname recovery).
    // Bound the PRODUCTION accept's internal blocking accept(): if the redirect
    // silently failed (the dial landed on the fallback backend instead of
    // leg-F), this returns a clean error after 8 s instead of hanging to the
    // 120 s slow-timeout SIGKILL.
    bound_listener_accept(&leg_f, Duration::from_secs(8));
    let (leg, got) = accept_outbound_and_recover_orig_dst(&leg_f).expect(
        "accept_outbound_and_recover_orig_dst must recover orig-dst from the TPROXY redirect. A \
             clean error here (EAGAIN/timeout after 8 s) means the redirect did NOT deliver to \
             leg-F — egress capture did not fire (the dial reached the fallback backend instead). \
             If this HANGS instead of clean-failing, SO_RCVTIMEO did not bound the production \
             accept on this kernel; treat a 120 s slow-timeout SIGKILL as the same \
             redirect-did-not-fire signal.",
    );

    // AC1: the redirect fired (leg-F accepted, NOT the fallback backend) AND
    // getsockname recovered the dialed orig-dst.
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

    // The fallback backend must NOT have accepted — the redirect took the dial.
    assert!(
        backend_fallback.accept().is_err(),
        "redirect fired: the real backend must NOT have accepted the workload's dial \
         (it was redirected to leg-F)"
    );
    let redirect_out = redirect_client.join().expect("redirect client thread");
    eprintln!("[03-03][AC1] redirect-phase client: {redirect_out}");
    drop(backend_fallback);

    // ----------------------------------------------------------------
    // AC2-a — agent HOST dial reaches the backend by TOPOLOGY (NOT the F5
    // exemption): the agent's HOST-netns dial carrying
    // SO_MARK = MTLS_LEG_S_DIAL_MARK reaches the REAL backend directly (NOT
    // re-captured to leg-F) because it originates host-side and never ingresses
    // the workload veth, so the production `iifname VETH_H` egress rule cannot
    // match it — WITH OR WITHOUT the F5 exemption. The SO_MARK is decorative on
    // THIS path: the dst (10.200.0.1) lives on host `lo`, so the packet ingresses
    // with `iif=lo`, never `iif=VETH_H`. This does NOT exercise the egress F5
    // exemption.
    //
    // GAP NOTE (honest, per the 03-03 review): the egress F5 exemption's
    // load-bearingness FOR THE AGENT'S leg-S re-dial is a SEPARATE, UNPROVEN
    // (possibly inapplicable) claim that touches ADR-0071's obligation-(b)
    // framing and depends on how leg-S is wired in 04-01/04-02. For EGRESS, the
    // ADR-0071 Tier-3 obligation (b) is satisfied HERE by AC2-b
    // (self-exempt-impossible) alone. A genuinely load-bearing egress F5
    // *positive* control would require a dial that actually INGRESSES the
    // workload veth carrying the leg-S mark (the agent's real leg-S dial path,
    // wired in 04-01/04-02) — a host-`lo` dial cannot match the production
    // `iifname` rule and so cannot exercise the exemption. That is explicitly out
    // of 03-03's scope. (The exemption's real load-bearing role is on the SHARED
    // chain's INBOUND `ip daddr`/`tcp dport` rules, where a host-originated marked
    // dial to a virt DOES match and WOULD loop without it.)
    let agent_backend = TcpListener::bind(backend).expect("bind real backend (AC2-a topology)");
    let agent_dial = std::thread::spawn(move || {
        let s = dial_with_so_mark(backend, MTLS_LEG_S_DIAL_MARK, Duration::from_secs(8));
        if let Ok(mut s) = s {
            use std::io::Write as _;
            let _ = s.write_all(b"AGENT-MARKED");
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    let (mut agent_conn, agent_peer) = accept_with_timeout(&agent_backend, Duration::from_secs(5))
        .expect(
            "AC2-a topology: the agent's HOST dial must reach the REAL backend directly because \
             it originates host-side and ingresses with iif=lo, so the production `iifname VETH_H` \
             egress rule cannot match it (with or without the F5 exemption). A timeout here means \
             the host-side routing/topology is broken — NOT that the exemption is broken (this \
             path does not exercise the exemption).",
        );
    let mut abuf = [0u8; 12];
    agent_conn.read_exact(&mut abuf).expect("read agent marker");
    assert_eq!(
        &abuf, b"AGENT-MARKED",
        "AC2-a topology: backend must receive the agent's marked bytes (host dial never \
         iifname-matched)"
    );
    agent_dial.join().expect("agent dial thread");
    eprintln!(
        "[03-03][AC2-a topology] agent HOST dial reached backend directly (never iifname-matched, \
         NOT via the F5 exemption), peer={agent_peer}"
    );
    // The agent dial originates in the HOST netns (NOT via the veth), so its peer
    // is the loopback source the host kernel picks for a lo-bound dst — proving
    // it never traversed the veth and was never iifname-matched. This is WHY it
    // reaches the backend: the topology non-match, not the F5 exemption (which is
    // irrelevant to the egress iifname rule). See the AC2-a gap note above.
    drop(agent_backend);

    // ----------------------------------------------------------------
    // AC2-b — SELF-EXEMPT-IMPOSSIBLE (safe negative control): a WORKLOAD dial
    // that sets SO_MARK INSIDE its own netns is STILL captured to leg-F. SO_MARK
    // is skb-local metadata that does NOT cross the veth/netns boundary, so the
    // host-side `iifname VETH_H` rule still matches — a workload cannot
    // self-exempt. We prove capture by getsockname recovery on leg-F again.
    // ----------------------------------------------------------------
    let selfexempt_client =
        std::thread::spawn(move || run_client_in_netns(backend, Some(MTLS_LEG_S_DIAL_MARK)));
    // Bound the production accept again (the SO_RCVTIMEO set above persists on the
    // listener fd; re-apply defensively in case a prior accept reset it).
    bound_listener_accept(&leg_f, Duration::from_secs(8));
    let (leg2, got2) = accept_outbound_and_recover_orig_dst(&leg_f).expect(
        "self-exempt-impossible: a workload's SO_MARK-stamped dial must STILL be captured to \
         leg-F (the mark does not cross the netns boundary). A clean error here (EAGAIN/timeout \
         after 8 s) means the workload self-exempted — a security hole. If this HANGS instead, \
         SO_RCVTIMEO did not bound the production accept on this kernel; a 120 s slow-timeout \
         SIGKILL is the same self-exempt-leaked signal.",
    );
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

    eprintln!(
        "[03-03] VERDICT: WORKS — egress redirect + getsockname recovery + self-exempt-impossible \
         (ADR-0071 obligation (b) for egress) validated on kernel {kr}. AC2-a's agent HOST dial \
         reaches the backend by topology non-match (never iifname-matched), NOT via the F5 \
         exemption — a load-bearing egress F5 *positive* control needs the real leg-S veth-ingress \
         dial wired in 04-01/04-02 (out of 03-03 scope)."
    );

    // Teardown: drop the per-workload guard (removes ONLY the iifname rule), then
    // scrub the shared infra + topology so a clean-kernel re-run reproduces.
    drop(guard);
    drop(leg_f);
    teardown_topology();
    clean_shared_infra();
}
