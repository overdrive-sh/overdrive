//! Tier 3 — single-UDP-listener forward+reverse e2e (WALKING SKELETON)
//! (udp-service-support US-04; ADR-0060 § Enforcement Tier 3; K1).
//!
//! GREEN (step 01-03): follows the `reverse_nat_e2e` / `service_map_forward`
//! Tier-3 shape (real `EbpfDataplane` + `overdrive-testing`
//! `ThreeIfaceTopology` netns/veth fixtures) with the proto=`Udp` sibling
//! of the TCP reverse-NAT path. The kernel forward (`xdp_service_map_lookup`)
//! and reverse (`xdp_reverse_nat_lookup`) programs both handle proto=17 (UDP)
//! — landed by deps 01-01 (ServiceFrontend(vip,port,proto) threading) and
//! 01-02 (Tier-2 kernel proto=17 source rewrite). This e2e proves the full
//! UDP round-trip on a real kernel.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-04-A (WALKING SKELETON / driving adapter): the udp REVERSE_NAT key
//!   `(backend_ip, 5353, udp)` maps to the VIP + a real UDP datagram
//!   round-trip carries the VIP source (10.96.0.10:5353), not the backend
//!   IP. The subprocess `overdrive deploy` exit-0 + "Accepted." driving-
//!   adapter assertion lands in the companion direct-handler test under
//!   `crates/overdrive-cli/tests/` per the `exec_spec_walking_skeleton`
//!   precedent (the `overdrive-cli` crate forbids subprocess tests — see
//!   `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no
//!   subprocess"). This file owns the WIRE half.
//! - S-04-B: three datagrams each independently source-rewritten to VIP.
//! - S-04-C: a missing-backend response (no reply) is distinguished from
//!   a wrong-source response (reply with backend source) — only the
//!   latter is the #163 defect.
//!
//! Assertion discipline (`.claude/rules/testing.md` § "Assertion rules"):
//! assert on OBSERVABLE kernel side-effects (the REVERSE_NAT_MAP dump via
//! the proto-aware `reverse_nat_map_has_backend_proto` accessor, the
//! AF_PACKET/tcpdump wire capture source address) — NEVER on "the program
//! took branch X". Tier 3 (layer 4+) — example-only per Mandate 11;
//! traditional assertions per Mandate 8.
//!
//! Gated behind `integration-tests`; runs via `cargo xtask lima run --`.
//! Linux-only (real veth + bpffs + kernel). The whole `tests/integration`
//! binary is gated in `tests/integration.rs`.

// Fixture-wide allows: these lints fire pervasively across the netns /
// veth / subprocess plumbing helpers and the long Tier-3 scenario
// bodies; scoping each to a line would add ~30 annotations of pure
// noise. The cast lints are deliberately NOT listed — this file has no
// numeric casts, so listing them would suppress a future cast silently;
// leaving them off means a cast added later surfaces its lint.
#![allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::used_underscore_binding,
    clippy::type_complexity
)]

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use overdrive_core::SpiffeId;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_dataplane::EbpfDataplane;
use overdrive_dataplane::maps::ServiceKey;
use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;

use overdrive_testing::netns::{NetNsError, ThreeIfaceTopology, threeiface_ips};

/// The DNS-resolver UDP listener port from `dns-resolver.toml` (S-04).
const UDP_PORT: u16 = 5353;

/// Build a UDP `ServiceFrontend` for `vip` on the listener port. The
/// proto=`Udp` discriminator threads through `update_service` into the
/// REVERSE_NAT_MAP key (ADR-0060 D1a/D7), where the kernel
/// `xdp_reverse_nat_lookup` reads it back from the response packet's
/// proto byte (= 17) to rewrite the source 5-tuple to the VIP.
fn udp_frontend(vip: Ipv4Addr, port: u16) -> ServiceFrontend {
    let service_vip = ServiceVip::new(IpAddr::V4(vip)).expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(
        service_vip,
        std::num::NonZeroU16::new(port).expect("non-zero listener port"),
        Proto::Udp,
    )
    .expect("IPv4 ServiceFrontend constructs")
}

/// Enter `target_ns` via `setns(2)`. Returns an RAII guard reverting
/// the calling thread's netns on Drop. Mirrors `reverse_nat_e2e.rs`.
fn enter_netns(target_ns: &str) -> std::io::Result<NetNsGuard> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: `open(O_RDONLY)` on a kernel-managed path; owned fd, closed on Drop.
    let prior_fd = {
        let path = std::ffi::CString::new("/proc/self/ns/net").unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    let target_path = format!("/var/run/netns/{target_ns}");
    let cstr = std::ffi::CString::new(target_path).unwrap();
    // SAFETY: open(O_RDONLY) on a netns mount; closed on Drop.
    let target_fd = {
        let fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    // SAFETY: setns to a network namespace; the current thread moves into
    // the target ns; subsequent BPF / iface ops resolve within it.
    let rc = unsafe { libc::setns(target_fd.as_raw_fd(), libc::CLONE_NEWNET) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(NetNsGuard { prior_fd: Some(prior_fd) })
}

/// RAII guard reverting the calling thread's netns on Drop.
struct NetNsGuard {
    prior_fd: Option<std::os::fd::OwnedFd>,
}

impl Drop for NetNsGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        if let Some(fd) = self.prior_fd.take() {
            // Best-effort revert; the test process exits soon after either way.
            let _ = unsafe { libc::setns(fd.as_raw_fd(), libc::CLONE_NEWNET) };
        }
    }
}

/// Pre-flight: are we root (CAP_NET_ADMIN + CAP_BPF)? Skip on EUID != 0.
fn require_root_or_skip(test_name: &str) -> bool {
    // SAFETY: `geteuid` has no preconditions; reads a kernel-managed numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] {test_name} needs root (CAP_NET_ADMIN + CAP_BPF); euid={euid}");
        return false;
    }
    true
}

/// Per-test bpffs pin dir for SERVICE_MAP pin-by-name. RAII-cleaned.
struct PinDirGuard(PathBuf);
impl Drop for PinDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_pin_dir(tag: &str) -> (PathBuf, PinDirGuard) {
    let pin_dir =
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-rnatudp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let guard = PinDirGuard(pin_dir.clone());
    (pin_dir, guard)
}

/// Read `/sys/class/net/<iface>/address` inside a netns — used to
/// pre-populate the LB ARP table so the first datagram's
/// `bpf_fib_lookup` returns `RET_SUCCESS` instead of `RET_NO_NEIGH`
/// (which falls to XDP_PASS / slow path). Same flake-elimination as
/// the TCP S-2.2-17 precedent.
fn read_iface_mac(ns_name: &str, iface: &str) -> std::io::Result<String> {
    let out = Command::new("ip")
        .args(["netns", "exec", ns_name, "cat", &format!("/sys/class/net/{iface}/address")])
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "ip netns exec read MAC failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `ip neigh replace <ip> lladdr <mac> dev <iface> nud permanent` inside `ns`.
fn neigh_replace(ns: &str, ip: &str, mac: &str, dev: &str) {
    let out = Command::new("ip")
        .args([
            "netns",
            "exec",
            ns,
            "ip",
            "neigh",
            "replace",
            ip,
            "lladdr",
            mac,
            "dev",
            dev,
            "nud",
            "permanent",
        ])
        .output()
        .expect("ip neigh replace");
    assert!(
        out.status.success(),
        "ip neigh replace ({ip} on {dev}) failed: stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `xdp_pass` stub attached to a peer-side veth iface. Required for
/// XDP_TX/REDIRECT delivery into a veth peer (kernel patch v7 09/10).
/// Holds the pre-pinned SERVICE_MAP HoM alive for the stub's lifetime.
struct StubXdpHolder {
    _service_map: HashOfMapsHandle<ServiceKey, u32>,
    _bpf: aya::Ebpf,
    _link: aya::programs::xdp::XdpLinkId,
}

/// Load the `xdp_pass` program and attach to `iface` in the calling
/// thread's current netns. Caller must hold `setns(2)` on the target ns.
/// Mirrors `reverse_nat_e2e.rs::load_xdp_pass_stub`.
fn load_xdp_pass_stub(iface: &str, pin_dir: &std::path::Path) -> Result<StubXdpHolder, String> {
    use aya::programs::{Xdp, XdpFlags};

    const SERVICE_MAP_NAME: &str = "SERVICE_MAP";
    const SERVICE_MAP_OUTER_CAPACITY: u32 = 4096;
    const SERVICE_MAP_INNER_CAPACITY: u32 =
        overdrive_core::dataplane::MaglevTableSize::DEFAULT.get();
    let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        SERVICE_MAP_NAME,
        SERVICE_MAP_OUTER_CAPACITY,
        SERVICE_MAP_INNER_CAPACITY,
        pin_dir,
    )
    .map_err(|e| format!("pre-pin SERVICE_MAP for stub: {e}"))?;

    const STUB_OBJ: &[u8] = include_bytes!(env!("OVERDRIVE_BPF_OBJECT_PATH"));
    let bpf_temp_path =
        std::env::temp_dir().join(format!("overdrive_bpf_udpstub-{}.o", std::process::id()));
    std::fs::write(&bpf_temp_path, STUB_OBJ).map_err(|e| format!("write stub temp: {e}"))?;
    let load_result = aya::EbpfLoader::new()
        .map_pin_path(pin_dir)
        .allow_unsupported_maps()
        .load_file(&bpf_temp_path)
        .map_err(|e| format!("aya load stub: {e}"));
    let _ = std::fs::remove_file(&bpf_temp_path);
    let mut bpf = load_result?;

    let prog: &mut Xdp = bpf
        .program_mut("xdp_pass")
        .ok_or_else(|| "xdp_pass program not found".to_string())?
        .try_into()
        .map_err(|e| format!("xdp_pass program type: {e}"))?;
    prog.load().map_err(|e| format!("xdp_pass.load: {e}"))?;
    let link = match prog.attach(iface, XdpFlags::DRV_MODE) {
        Ok(link) => link,
        Err(_) => prog
            .attach(iface, XdpFlags::SKB_MODE)
            .map_err(|e| format!("xdp_pass.attach({iface}, SKB_MODE): {e}"))?,
    };
    Ok(StubXdpHolder { _service_map: service_map, _bpf: bpf, _link: link })
}

/// Wait for `child` to exit, polling at 50 ms up to `budget`.
fn wait_with_timeout(
    child: &mut std::process::Child,
    budget: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("try_wait: {e}"),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

/// Result of a UDP round-trip: how many of the `count` datagrams the
/// client received an echoed reply for, plus the source IPs of every
/// reply datagram (source port 5353) observed on the client veth.
///
/// **Two complementary observables, one primary, one load-bearing-for-C:**
///
/// 1. `replies_received` — the PRIMARY observable for the rewritten
///    (XDP) path (S-04-A / S-04-B). `nc -u <VIP> 5353` *connects* the
///    client UDP socket to `VIP:5353`; the kernel then delivers to the
///    application ONLY datagrams whose source is exactly `VIP:5353` and
///    silently drops any other source before `nc` can read it. The
///    backend's raw reply is sourced from `BACKEND_IP:5353`; it reaches
///    `nc` ONLY because the kernel `xdp_reverse_nat_lookup` rewrote the
///    source 5-tuple to `(VIP, 5353)`. So an echoed reply landing in
///    `nc`'s stdout is a kernel-enforced proof that the reverse-NAT
///    source rewrite to the VIP succeeded. This stays primary for the
///    XDP path because the rewritten reply is delivered into the client
///    veth via `XDP_REDIRECT`/`XDP_TX`, which an in-netns `tcpdump`
///    cannot reliably observe (debugging.md § 3, inspection-tool gap).
///    Mirrors the TCP precedent (`reverse_nat_e2e.rs`), which asserts on
///    `client_stdout.contains(PAYLOAD)`, not on a pcap.
///
/// 2. `reply_source_ips` — the source IP of every reply datagram
///    (source port 5353) captured by an **any-source** `tcpdump` on the
///    client veth. This is LOAD-BEARING for S-04-C: a *non-rewritten*
///    backend-sourced reply (the #163 defect) is an ordinary
///    normal-stack frame on the client veth, NOT an `XDP_REDIRECT`
///    frame, so the any-source capture reliably sees it. A positive
///    control (`target/probe/probe_capture.sh`, plain-routed UDP) has
///    been run and confirms the any-source client-veth capture surfaces
///    reply datagrams with their real source IP — so for S-04-C, the
///    ABSENCE of any reply datagram is a genuine, falsifiable
///    distinguisher: it would FAIL if a backend-sourced reply arrived.
struct RoundTripResult {
    /// Number of datagrams (of `count` sent) for which the client read
    /// back the echoed reply — i.e. the reply that survived the
    /// connected-socket source filter because its source was the VIP.
    replies_received: usize,
    /// Source IPs of every reply datagram (source port 5353) seen on the
    /// client veth by the any-source capture. For the rewritten XDP path
    /// this may be empty even on success (XDP_REDIRECT invisibility); for
    /// the backend-down / non-rewritten path it reliably reflects what
    /// actually landed on the wire. S-04-C asserts this is empty.
    reply_source_ips: Vec<Ipv4Addr>,
}

struct UdpFixture {
    topo: ThreeIfaceTopology,
    _pin_guard: PinDirGuard,
}

/// Build the 3-iface topology, pin dir, peer XDP stubs, ARP, and a
/// running `EbpfDataplane` with a single UDP backend installed. Returns
/// the live dataplane (held by the caller across the round-trip(s)) plus
/// the fixture handle. `backend_bound` controls whether a `nc -u -l`
/// listener is actually spawned on the backend (S-04-C exercises the
/// not-bound case).
fn build_udp_service(
    tag: &str,
    backend_bound: bool,
) -> Option<(EbpfDataplane, UdpFixture, Vec<StubXdpHolder>, Option<std::process::Child>, PathBuf)> {
    use threeiface_ips::{BACKEND_IP, CLIENT_IP, VIP};

    let topo = match ThreeIfaceTopology::create(tag) {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("[skip] udp e2e needs CAP_NET_ADMIN");
            return None;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    let (pin_dir, pin_guard) = make_pin_dir(tag);

    let pcap_dir = PathBuf::from(format!("/tmp/ovd-rnatudp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pcap_dir);
    std::fs::create_dir_all(&pcap_dir).expect("create pcap dir");

    let backend_mac =
        read_iface_mac(&topo.backend_ns.name, &topo.backend_veth).expect("read backend_veth MAC");
    let client_mac =
        read_iface_mac(&topo.client_ns.name, &topo.client_veth).expect("read client_veth MAC");

    // Peer XDP stubs so XDP_TX/REDIRECT delivery into the veth peers works.
    let mut stubs = Vec::new();
    {
        let _g = enter_netns(&topo.backend_ns.name).expect("setns backend-ns for stub");
        let stub_pin = pin_dir.join("backend-stub");
        let _ = std::fs::create_dir_all(&stub_pin);
        stubs.push(load_xdp_pass_stub(&topo.backend_veth, &stub_pin).expect("backend stub"));
    }
    {
        let _g = enter_netns(&topo.client_ns.name).expect("setns client-ns for stub");
        let stub_pin = pin_dir.join("client-stub");
        let _ = std::fs::create_dir_all(&stub_pin);
        stubs.push(load_xdp_pass_stub(&topo.client_veth, &stub_pin).expect("client stub"));
    }

    // Spawn the UDP echo server on the backend (when bound). `socat
    // UDP-LISTEN:5353,fork,reuseaddr PIPE` echoes every received datagram
    // straight back to its sender — a true UDP echo (`fork` handles each
    // datagram's source independently, which models the connectionless
    // per-datagram reply S-04-B asserts on). OpenBSD `nc -u -l` does NOT
    // echo (it only relays its own stdin), so socat is the right tool.
    // For the not-bound case (S-04-C) we skip the listener entirely so a
    // datagram to the VIP produces no reply.
    let backend_listener = if backend_bound {
        let child = topo
            .backend_ns
            .command("socat", [&format!("UDP4-LISTEN:{UDP_PORT},fork,reuseaddr"), "PIPE"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn backend socat UDP echo");
        std::thread::sleep(Duration::from_millis(300));
        Some(child)
    } else {
        None
    };

    // ARP pre-population on both LB ifaces.
    neigh_replace(&topo.lb_ns.name, &BACKEND_IP.to_string(), &backend_mac, &topo.lb_veth_b);
    neigh_replace(&topo.lb_ns.name, &CLIENT_IP.to_string(), &client_mac, &topo.lb_veth_a);

    // Construct EbpfDataplane in lb-ns and install the UDP service.
    let _ns_guard = enter_netns(&topo.lb_ns.name).expect("setns lb-ns");
    let dataplane = EbpfDataplane::new_with_pin_dir(
        &topo.lb_veth_a,
        &topo.lb_veth_b,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir on lb_veth_a + lb_veth_b");

    let backend_alloc =
        SpiffeId::new("spiffe://overdrive.local/job/dns/alloc/B1").expect("backend SpiffeId");
    let backend_addr = SocketAddr::new(IpAddr::V4(BACKEND_IP), UDP_PORT);
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");
    runtime
        .block_on(dataplane.update_service(
            udp_frontend(VIP, UDP_PORT),
            vec![Backend { alloc: backend_alloc, addr: backend_addr, weight: 1, healthy: true }],
        ))
        .expect("update_service (udp)");
    drop(_ns_guard);

    Some((dataplane, UdpFixture { topo, _pin_guard: pin_guard }, stubs, backend_listener, pcap_dir))
}

/// Send `count` UDP datagrams from the client to `VIP:UDP_PORT`,
/// capturing the wire on the client veth, and return what the client
/// observed. Each datagram is sent via a fresh `nc -u` so the
/// connected-socket source check is exercised per-datagram (UDP is
/// connectionless; each reply is independently rewritten — S-04-B).
fn run_round_trips(
    fixture: &UdpFixture,
    pcap_dir: &std::path::Path,
    count: usize,
) -> RoundTripResult {
    use threeiface_ips::VIP;

    // Any-source capture on the client veth. For the rewritten XDP path
    // (S-04-A/B) this is a best-effort diagnostic — `XDP_REDIRECT`-
    // delivered reply frames may be invisible to an in-netns tcpdump, so
    // the load-bearing observable there is `replies_received`. For the
    // backend-down / non-rewritten path (S-04-C) the capture is
    // LOAD-BEARING: any reply that DID arrive would be an ordinary
    // normal-stack frame the capture reliably sees, so an empty
    // `reply_source_ips` is a genuine "no reply on the wire" distinguisher
    // (positive control: target/probe/probe_capture.sh).
    let pcap = pcap_dir.join("client.pcap");
    let mut tcpdump = Command::new("ip")
        .args([
            "netns",
            "exec",
            &fixture.topo.client_ns.name,
            "tcpdump",
            "-l",
            "-n",
            "-i",
            fixture.topo.client_veth.as_str(),
            "-w",
            pcap.to_str().unwrap_or(""),
            "udp",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    std::thread::sleep(Duration::from_millis(300));

    let mut replies_received: usize = 0;
    for i in 0..count {
        let mut client = fixture
            .topo
            .client_ns
            .command("nc", ["-u", &VIP.to_string(), &UDP_PORT.to_string(), "-w", "2"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn client nc -u");
        {
            use std::io::Write;
            // Write the query datagram then CLOSE stdin (drop the handle)
            // so `nc -u` flushes the send and the `-w 2` read-timeout
            // governs how long it waits for the echoed reply. Holding
            // stdin open leaves nc blocked on stdin and it never exits /
            // never sends deterministically.
            let mut stdin = client.stdin.take().expect("client nc stdin");
            let _ = stdin.write_all(format!("dns-query-{i}\n").as_bytes());
        }
        let _ = wait_with_timeout(&mut client, Duration::from_secs(4));
        let mut out = String::new();
        if let Some(mut s) = client.stdout.take() {
            let _ = s.read_to_string(&mut out);
        }
        let mut err = String::new();
        if let Some(mut s) = client.stderr.take() {
            let _ = s.read_to_string(&mut err);
        }
        eprintln!("[diag] client nc datagram {i}: stdout=[{}] stderr=[{}]", out.trim(), err.trim());
        // The echo carries back the exact payload we sent; a non-empty
        // stdout means the connected socket accepted a VIP-sourced reply.
        if out.contains(&format!("dns-query-{i}")) {
            replies_received += 1;
        }
    }

    std::thread::sleep(Duration::from_millis(300));
    if let Some(mut t) = tcpdump.take() {
        let _ = t.kill();
        let _ = t.wait();
    }

    // Parse the any-source client-veth capture for reply datagrams
    // (source port 5353). For the rewritten XDP path this may be empty
    // even on success (XDP_REDIRECT invisibility). For the backend-down
    // path it reliably reflects the wire — S-04-C asserts it is empty,
    // which would FAIL if a backend-sourced reply (the #163 defect) had
    // arrived.
    let reply_source_ips = parse_pcap_udp_sources(&fixture.topo.client_ns.name, &pcap);
    eprintln!("[diag] client veth any-source reply source IPs = {reply_source_ips:?}");
    RoundTripResult { replies_received, reply_source_ips }
}

/// Parse the UDP datagrams arriving at the client veth from `VIP:5353`
/// (i.e. replies, source port 5353) and return their source IPs.
///
/// We read the capture back via `tcpdump -r` (text) and parse the
/// `IP <src>.<sport> > <dst>.<dport>` lines, keeping only datagrams
/// whose source port is the listener port (5353) — those are the
/// backend replies. The source IP on each is the observable assertion
/// surface: it MUST be the VIP, never the backend IP.
fn parse_pcap_udp_sources(ns_name: &str, pcap: &std::path::Path) -> Vec<Ipv4Addr> {
    let out = Command::new("ip")
        .args(["netns", "exec", ns_name, "tcpdump", "-n", "-r", pcap.to_str().unwrap_or(""), "udp"])
        .output();
    let Ok(out) = out else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut sources = Vec::new();
    for line in text.lines() {
        // Lines look like:
        //   12:00:00.000000 IP 10.96.0.10.5353 > 10.0.0.10.41234: UDP, length 13
        let Some(ip_idx) = line.find(" IP ") else { continue };
        let rest = &line[ip_idx + 4..];
        let Some(gt) = rest.find(" > ") else { continue };
        let src = &rest[..gt];
        // src is "<ip>.<port>" — split the LAST '.' (port) off.
        let Some(dot) = src.rfind('.') else { continue };
        let (ip_str, port_str) = (&src[..dot], &src[dot + 1..]);
        // Keep only replies (source port == listener port).
        if port_str.trim() != UDP_PORT.to_string() {
            continue;
        }
        if let Ok(ip) = ip_str.trim().parse::<Ipv4Addr>() {
            sources.push(ip);
        }
    }
    sources
}

/// S-04-A (WALKING SKELETON) — real UDP round-trip carries the VIP source.
///
/// Proves the walking-skeleton wire half:
///   1. `update_service(udp_frontend(VIP, 5353), [backend])` installs the
///      `(backend_ip, 5353, udp)` REVERSE_NAT key mapping to the VIP
///      (observable via `reverse_nat_map_has_backend_proto`).
///   2. A real UDP datagram round-trip through the kernel forward
///      (`xdp_service_map_lookup`) + reverse (`xdp_reverse_nat_lookup`)
///      path delivers a reply to the client whose source is the VIP
///      (10.96.0.10:5353), NOT the backend IP — observable via the
///      client-veth wire capture.
#[test]
fn single_udp_listener_round_trip_carries_vip_source() {
    if !require_root_or_skip("S-04-A") {
        return;
    }
    use threeiface_ips::{BACKEND_IP, VIP};

    let Some((dataplane, fixture, stubs, mut backend_listener, pcap_dir)) =
        build_udp_service("ua", true)
    else {
        return;
    };
    let _ns_guard = enter_netns(&fixture.topo.lb_ns.name).expect("setns lb-ns for map readback");

    // (1) Observable: the udp REVERSE_NAT key maps the backend to the VIP.
    // The proto-aware accessor distinguishes the UDP key from the TCP key
    // (a TCP-only lookup would MISS the proto=17 entry — the #163 gap).
    let udp_present = dataplane
        .reverse_nat_map_has_backend_proto(BACKEND_IP, UDP_PORT, Proto::Udp)
        .expect("REVERSE_NAT_MAP udp key readback");
    assert!(
        udp_present,
        "REVERSE_NAT_MAP must contain the (backend_ip={BACKEND_IP}, {UDP_PORT}, udp) key after \
         update_service(udp_frontend, [backend]) — this is the source rewrite the kernel \
         uses to map proto=17 responses back to the VIP"
    );
    // The same backend under TCP must be ABSENT — proves the proto byte
    // is load-bearing in the key (not silently TCP).
    let tcp_present = dataplane
        .reverse_nat_map_has_backend_proto(BACKEND_IP, UDP_PORT, Proto::Tcp)
        .expect("REVERSE_NAT_MAP tcp key readback");
    assert!(
        !tcp_present,
        "REVERSE_NAT_MAP must NOT contain a TCP key for a UDP-only service — the proto byte \
         is part of the key (ADR-0060 D7); a TCP hit here means proto was ignored"
    );
    drop(_ns_guard);

    // (2) Real UDP round-trip — the reply carries the VIP source.
    let result = run_round_trips(&fixture, &pcap_dir, 1);

    if let Some(mut l) = backend_listener.take() {
        let _ = l.kill();
        let _ = l.wait();
    }
    drop(stubs);
    drop(dataplane);

    assert_eq!(
        result.replies_received,
        1,
        "client must receive the backend's UDP reply through the VIP. `nc -u {VIP} 5353` \
         connects the socket to {VIP}:5353; the kernel delivers ONLY datagrams whose source \
         is {VIP}:5353 and drops anything else (e.g. a reply still sourced from the backend \
         {BACKEND_IP} — the #163 defect) before nc reads it. An echoed reply landing in \
         stdout therefore proves the kernel reverse-NAT rewrote the source to the VIP. \
         pcaps: {}",
        pcap_dir.display()
    );
    let _ = fixture;
}

/// S-04-B — every UDP reply is independently source-rewritten to the VIP.
///
/// UDP is connectionless; the kernel rewrites EACH response packet's
/// source 5-tuple independently. Three datagrams must each produce a
/// reply captured with the VIP as source.
#[test]
fn every_udp_reply_independently_source_rewritten() {
    if !require_root_or_skip("S-04-B") {
        return;
    }
    use threeiface_ips::{BACKEND_IP, VIP};

    let Some((dataplane, fixture, stubs, mut backend_listener, pcap_dir)) =
        build_udp_service("ub", true)
    else {
        return;
    };

    let result = run_round_trips(&fixture, &pcap_dir, 3);

    if let Some(mut l) = backend_listener.take() {
        let _ = l.kill();
        let _ = l.wait();
    }
    drop(stubs);
    drop(dataplane);

    // UDP is connectionless: the kernel rewrites EACH response packet's
    // source 5-tuple independently. All three replies must therefore
    // survive the per-datagram connected-socket source filter (= all
    // three were VIP-sourced). A reverse path that rewrote only the
    // first reply (e.g. some spurious connection-tracking shortcut)
    // would deliver 1, not 3.
    assert_eq!(
        result.replies_received,
        3,
        "all three datagrams must each produce a VIP-sourced echoed reply (UDP is \
         connectionless; each reply is independently rewritten to {VIP}); the connected \
         socket received {} of 3. A reply still sourced from {BACKEND_IP} is the #163 defect \
         and would be dropped by the client kernel before nc reads it. pcaps: {}",
        result.replies_received,
        pcap_dir.display()
    );
    let _ = fixture;
}

/// S-04-C — a missing-backend response (genuinely no reply on the wire)
/// is distinguished from a wrong-source response (a backend-sourced
/// reply, the #163 defect).
///
/// The backend is NOT bound on the listener port: no echo server runs.
/// A datagram to the VIP therefore produces NO reply. The distinguisher
/// is made genuinely falsifiable by observing replies REGARDLESS of
/// source via an any-source `tcpdump` on the client veth: the test
/// asserts that ZERO reply datagrams (source port 5353) arrive from ANY
/// source.
///
/// Why the any-source capture is reliable HERE specifically (where it is
/// only best-effort for S-04-A/B): a #163-defect reply is a
/// *non-rewritten* backend-sourced frame that traverses the normal
/// kernel stack into the client veth — NOT an `XDP_REDIRECT`-delivered
/// frame — so the in-netns capture reliably sees it. A positive control
/// (`target/probe/probe_capture.sh`, plain-routed UDP with no XDP) has
/// been run and confirms the any-source client-veth capture surfaces
/// reply datagrams with their real source IP.
///
/// Falsifiability: if a backend-sourced reply DID arrive (the #163
/// defect surfacing through some spurious path), `reply_source_ips`
/// would be non-empty and this test would FAIL — which is exactly the
/// "wrong-source response" the scenario name promises to catch. The
/// connected-socket `replies_received == 0` assertion is kept as a
/// secondary belt-and-suspenders check (it alone could not see a
/// backend-sourced reply, since the connected socket silently drops it).
#[test]
fn missing_backend_response_distinguished_from_wrong_source() {
    if !require_root_or_skip("S-04-C") {
        return;
    }

    // backend_bound = false — no listener on the backend.
    let Some((dataplane, fixture, stubs, _backend_listener, pcap_dir)) =
        build_udp_service("uc", false)
    else {
        return;
    };

    let result = run_round_trips(&fixture, &pcap_dir, 1);

    drop(stubs);
    drop(dataplane);

    // PRIMARY distinguisher (load-bearing, genuinely falsifiable): the
    // any-source client-veth capture must show NO reply datagram from
    // ANY source. With no backend bound, nothing is echoed, so nothing
    // lands on the wire. A backend-sourced reply (the #163 defect) is a
    // normal-stack frame the any-source capture reliably observes — so a
    // non-empty `reply_source_ips` here would FAIL the test, which is
    // precisely the "wrong-source response" this scenario distinguishes
    // from genuine "no response".
    assert!(
        result.reply_source_ips.is_empty(),
        "with no backend bound, the client veth must see NO reply datagram from any source — \
         a captured reply would be the wrong-source (#163) failure this scenario distinguishes \
         from 'no response'. Captured reply sources = {:?}. pcaps: {}",
        result.reply_source_ips,
        pcap_dir.display()
    );

    // SECONDARY belt-and-suspenders: the connected socket also reads
    // nothing. (This alone could not catch a backend-sourced reply — the
    // connected-socket filter silently drops it — so it is not the
    // distinguisher; the any-source capture above is.)
    assert_eq!(
        result.replies_received,
        0,
        "with no backend bound, the connected socket must read no reply. Got {} replies. \
         pcaps: {}",
        result.replies_received,
        pcap_dir.display()
    );
    let _ = fixture;
}
