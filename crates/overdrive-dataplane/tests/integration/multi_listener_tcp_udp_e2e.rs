//! Tier 3 — multi-listener (TCP + UDP) forward+reverse e2e
//! (udp-service-support US-05; ADR-0060; K4).
//!
//! GREEN (step 01-04): the two-listener generalisation of the
//! single-UDP-listener walking skeleton (`reverse_nat_udp_e2e.rs`, step
//! 01-03) and the TCP reverse-NAT e2e (`reverse_nat_e2e.rs`). A
//! two-listener service is installed as TWO per-listener
//! `update_service` calls — `tcp_frontend(VIP, 8080)` and
//! `udp_frontend(VIP, 8081)` — each carrying its own `(vip, port,
//! proto)` `ServiceFrontend` and installing its own REVERSE_NAT key
//! with its own declared proto byte (ADR-0060 D4 per-proto, per-listener
//! fan-out; D7 the BackendKey proto byte distinguishes the tcp/8080 key
//! from the udp/8081 key for the same backend). This is exactly the
//! per-listener fan-out the production CLI→control-plane→reconciler→
//! `EbpfDataplane` chain emits for a multi-listener `edge.toml` (one
//! `update_service` per listener); the Tier-3 e2e exercises the
//! observable kernel side-effects that chain produces.
//!
//! Scenario SSOT:
//! `docs/feature/udp-service-support/distill/test-scenarios.md`
//! - S-05-A: a two-listener service installs BOTH protocols' reverse
//!   paths — the `(backend, 8080, tcp)` AND `(backend, 8081, udp)`
//!   REVERSE_NAT keys are present (one per listener, each with its own
//!   declared proto byte), and a tcp round-trip + a udp round-trip both
//!   deliver a reply sourced from the VIP.
//! - S-05-B: each listener's reverse path is independently VIP-sourced —
//!   the tcp reply and the udp reply are each delivered through the VIP.
//! - S-05-C: re-submitting with an added udp/8082 listener converges so
//!   the third `(backend, 8082, udp)` key is present, the new udp/8082
//!   reverse path works, AND the existing two paths still work.
//!
//! ASSERTION-BOUNDARY DISCIPLINE (architect review, roadmap reviews[0]
//! high finding): a subprocess e2e cannot observe internal control-plane
//! reachability — do NOT assert white-box hydrator call counts ("emits
//! one update_service per listener"). The DISTILL prose carries that
//! white-box phrasing; it is replaced here by the OBSERVABLE PROXY: the
//! `bpftool`-equivalent REVERSE_NAT_MAP dump (via the proto-aware
//! `reverse_nat_map_has_backend_proto` accessor) shows ONE key per
//! listener, each with its own proto byte, plus the dual VIP-sourced
//! wire captures. Tier 3 (layer 4+) — example-only per Mandate 11;
//! observable kernel side-effects only (`.claude/rules/testing.md`
//! § "Tier 3 → Assertion rules"), never internal program reachability.
//!
//! Gated behind `integration-tests`; runs via `cargo xtask lima run --`.
//! Linux-only (real veth + bpffs + kernel). The whole `tests/integration`
//! binary is gated in `tests/integration.rs`.

// Fixture-wide allows: these lints fire pervasively across the netns /
// veth / subprocess plumbing helpers and the long Tier-3 scenario
// bodies; scoping each to a line would add ~30 annotations of pure
// noise. The cast lints are deliberately NOT listed — this file has no
// numeric casts, so listing them would suppress a future cast silently.
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

use std::io::{Read, Write};
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

/// The `edge.toml` TCP listener port (S-05). The backend binds the same
/// port (production `update_service` derives the SERVICE_MAP key port
/// from `backends[0].addr.port()`; VIP_port == backend_port models the
/// common L4LB deployment — same constraint as `reverse_nat_e2e.rs`).
const TCP_PORT: u16 = 8080;
/// The `edge.toml` UDP listener port (S-05).
const UDP_PORT: u16 = 8081;
/// The `edge.toml` added-on-resubmit UDP listener port (S-05-C).
const UDP_PORT_2: u16 = 8082;

/// Build a TCP `ServiceFrontend` for `vip` on `port`. The proto=`Tcp`
/// discriminator threads through `update_service` into the REVERSE_NAT
/// key (ADR-0060 D1a/D7) with proto byte = 6.
fn tcp_frontend(vip: Ipv4Addr, port: u16) -> ServiceFrontend {
    let service_vip = ServiceVip::new(IpAddr::V4(vip)).expect("valid IPv4 ServiceVip");
    ServiceFrontend::new(
        service_vip,
        std::num::NonZeroU16::new(port).expect("non-zero listener port"),
        Proto::Tcp,
    )
    .expect("IPv4 ServiceFrontend constructs")
}

/// Build a UDP `ServiceFrontend` for `vip` on `port`. The proto=`Udp`
/// discriminator threads through `update_service` into the REVERSE_NAT
/// key (ADR-0060 D1a/D7) with proto byte = 17 — the byte that
/// distinguishes the udp/8081 key from the tcp/8080 key for the same
/// backend.
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
/// the calling thread's netns on Drop. Mirrors `reverse_nat_udp_e2e.rs`.
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
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-multi-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    let guard = PinDirGuard(pin_dir.clone());
    (pin_dir, guard)
}

/// Read `/sys/class/net/<iface>/address` inside a netns — used to
/// pre-populate the LB ARP table so the first packet's `bpf_fib_lookup`
/// returns `RET_SUCCESS` instead of `RET_NO_NEIGH` (slow-path fallback).
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
/// Mirrors `reverse_nat_udp_e2e.rs::load_xdp_pass_stub`.
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
        std::env::temp_dir().join(format!("overdrive_bpf_multistub-{}.o", std::process::id()));
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

/// The live multi-listener fixture: the 3-iface topology, the per-test
/// pin dir, the peer XDP stubs, the running `EbpfDataplane` with the
/// listeners installed, the backend listener children, and the pcap dir.
/// Held by the caller across the round-trips; dropped after assertions.
struct MultiListenerFixture {
    topo: ThreeIfaceTopology,
    dataplane: EbpfDataplane,
    _pin_guard: PinDirGuard,
    _stubs: Vec<StubXdpHolder>,
    backend_children: Vec<std::process::Child>,
    pcap_dir: PathBuf,
}

impl Drop for MultiListenerFixture {
    fn drop(&mut self) {
        for mut c in self.backend_children.drain(..) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Spawn a TCP echo listener on the backend that pipes `payload` back to
/// each connection. `nc -l -p <port> -q 1` (OpenBSD/traditional nc)
/// relays its stdin to the connected client then closes 1 s after stdin
/// EOF — the same shape `reverse_nat_e2e.rs` uses for the TCP reply.
fn spawn_tcp_echo(topo: &ThreeIfaceTopology, port: u16, payload: &str) -> std::process::Child {
    let mut child = topo
        .backend_ns
        .command("nc", ["-l", "-p", &port.to_string(), "-q", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn backend tcp nc");
    {
        let stdin = child.stdin.as_mut().expect("backend tcp nc stdin");
        stdin.write_all(payload.as_bytes()).expect("tcp payload to backend stdin");
        stdin.write_all(b"\n").expect("newline");
    }
    child
}

/// Spawn a UDP echo listener on the backend. `socat
/// UDP4-LISTEN:<port>,fork,reuseaddr PIPE` echoes each received datagram
/// straight back to its sender — a true per-datagram UDP echo (the
/// connectionless reply S-05 asserts on). Same tool the single-UDP
/// walking skeleton uses.
fn spawn_udp_echo(topo: &ThreeIfaceTopology, port: u16) -> std::process::Child {
    topo.backend_ns
        .command("socat", [&format!("UDP4-LISTEN:{port},fork,reuseaddr"), "PIPE"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn backend socat UDP echo")
}

/// Build the 3-iface topology + pin dir + peer XDP stubs + ARP + a
/// running `EbpfDataplane`, then install each `(frontend, port)` listener
/// in `listeners` via a separate per-listener `update_service` call
/// (ADR-0060 D4 per-proto, per-listener fan-out). For each listener whose
/// proto is TCP a `nc -l` echo is spawned with `tcp_payload`; for each
/// UDP listener a `socat` echo is spawned. Returns the fixture, or None
/// on CAP_NET_ADMIN-unavailable (skip).
///
/// `listeners` is a slice of `(ServiceFrontend, backend_port)` — the
/// backend binds `backend_port` for that listener's proto. VIP and
/// backend IP come from `threeiface_ips`.
fn build_multi_listener_service(
    tag: &str,
    listeners: &[(ServiceFrontend, Proto, u16)],
    tcp_payload: &str,
) -> Option<MultiListenerFixture> {
    use threeiface_ips::{BACKEND_IP, CLIENT_IP, VIP};

    let topo = match ThreeIfaceTopology::create(tag) {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("[skip] multi-listener e2e needs CAP_NET_ADMIN");
            return None;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    let (pin_dir, pin_guard) = make_pin_dir(tag);

    let pcap_dir = PathBuf::from(format!("/tmp/ovd-multi-{tag}-{}", std::process::id()));
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

    // Spawn the per-listener backend echo servers.
    let mut backend_children = Vec::new();
    for (_, proto, backend_port) in listeners {
        match proto {
            Proto::Tcp => backend_children.push(spawn_tcp_echo(&topo, *backend_port, tcp_payload)),
            Proto::Udp => backend_children.push(spawn_udp_echo(&topo, *backend_port)),
        }
    }
    std::thread::sleep(Duration::from_millis(300));

    // ARP pre-population on both LB ifaces.
    neigh_replace(&topo.lb_ns.name, &BACKEND_IP.to_string(), &backend_mac, &topo.lb_veth_b);
    neigh_replace(&topo.lb_ns.name, &CLIENT_IP.to_string(), &client_mac, &topo.lb_veth_a);

    // Construct EbpfDataplane in lb-ns and install each listener via a
    // SEPARATE per-listener update_service call (the production
    // per-listener fan-out — ADR-0060 D4).
    let _ns_guard = enter_netns(&topo.lb_ns.name).expect("setns lb-ns");
    let dataplane = EbpfDataplane::new_with_pin_dir(
        &topo.lb_veth_a,
        &topo.lb_veth_b,
        &pin_dir,
        std::path::Path::new("/sys/fs/cgroup"),
    )
    .expect("EbpfDataplane::new_with_pin_dir on lb_veth_a + lb_veth_b");

    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");
    for (idx, (frontend, _proto, backend_port)) in listeners.iter().enumerate() {
        let alloc = SpiffeId::new(&format!("spiffe://overdrive.local/job/edge/alloc/L{idx}"))
            .expect("backend SpiffeId");
        let backend_addr = SocketAddr::new(IpAddr::V4(BACKEND_IP), *backend_port);
        runtime
            .block_on(dataplane.update_service(
                *frontend,
                vec![Backend { alloc, addr: backend_addr, weight: 1, healthy: true }],
            ))
            .expect("update_service (per-listener)");
    }
    drop(_ns_guard);
    let _ = VIP;

    Some(MultiListenerFixture {
        topo,
        dataplane,
        _pin_guard: pin_guard,
        _stubs: stubs,
        backend_children,
        pcap_dir,
    })
}

/// Run a single TCP round-trip from the client to `VIP:port`, returning
/// whether the client received the backend's echoed `payload`. `nc <VIP>
/// <port> -q 1 -w 5` connects, sends a line, and reads the reply; the
/// payload landing in stdout proves the reverse-NAT rewrote the reply
/// source to the VIP (a reply still sourced from the backend would be
/// dropped by the client kernel on the connected socket / would not
/// complete the TCP handshake through the VIP). Mirrors
/// `reverse_nat_e2e.rs`'s `client_stdout.contains(PAYLOAD)` assertion.
fn run_tcp_round_trip(fixture: &MultiListenerFixture, port: u16, payload: &str) -> bool {
    use threeiface_ips::VIP;

    let mut client = fixture
        .topo
        .client_ns
        .command("nc", [&VIP.to_string(), &port.to_string(), "-q", "1", "-w", "5"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn client tcp nc");
    {
        let stdin = client.stdin.as_mut().expect("client tcp nc stdin");
        let _ = stdin.write_all(b"hello\n");
    }
    let _ = wait_with_timeout(&mut client, Duration::from_secs(8));
    let mut out = String::new();
    if let Some(mut s) = client.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    let mut err = String::new();
    if let Some(mut s) = client.stderr.take() {
        let _ = s.read_to_string(&mut err);
    }
    eprintln!("[diag] tcp client (VIP:{port}): stdout=[{}] stderr=[{}]", out.trim(), err.trim());
    out.contains(payload)
}

/// Run a single UDP round-trip from the client to `VIP:port`, returning
/// whether the client received the backend's echoed reply. `nc -u <VIP>
/// <port>` connects the client UDP socket to `VIP:port`; the client
/// kernel then delivers ONLY datagrams sourced from `VIP:port` and drops
/// any other source. The backend's raw reply is sourced from
/// `BACKEND:port`; it reaches `nc` ONLY because the kernel
/// `xdp_reverse_nat_lookup` rewrote the source 5-tuple to `(VIP, port)`.
/// So an echoed datagram in stdout is a kernel-enforced proof the udp
/// reverse-NAT source rewrite to the VIP succeeded — the same
/// load-bearing observable the single-UDP walking skeleton uses.
fn run_udp_round_trip(fixture: &MultiListenerFixture, port: u16, marker: &str) -> bool {
    use threeiface_ips::VIP;

    let mut client = fixture
        .topo
        .client_ns
        .command("nc", ["-u", &VIP.to_string(), &port.to_string(), "-w", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn client udp nc");
    {
        // Write the query then CLOSE stdin so `nc -u` flushes the send and
        // the `-w 2` read-timeout governs the wait for the echoed reply.
        let mut stdin = client.stdin.take().expect("client udp nc stdin");
        let _ = stdin.write_all(format!("{marker}\n").as_bytes());
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
    eprintln!("[diag] udp client (VIP:{port}): stdout=[{}] stderr=[{}]", out.trim(), err.trim());
    out.contains(marker)
}

/// Assert (observable proxy) that the REVERSE_NAT_MAP contains the
/// per-listener key for `(BACKEND_IP, port, proto)` and that the SAME
/// backend+port is ABSENT under the opposite proto — proving the proto
/// byte is load-bearing in the key (ADR-0060 D7), not silently defaulted.
/// Caller must hold `setns(2)` on lb-ns for the map readback.
fn assert_reverse_nat_key_present_proto_distinguished(
    dataplane: &EbpfDataplane,
    backend_ip: Ipv4Addr,
    port: u16,
    proto: Proto,
) {
    let present = dataplane
        .reverse_nat_map_has_backend_proto(backend_ip, port, proto)
        .expect("REVERSE_NAT_MAP key readback");
    assert!(
        present,
        "REVERSE_NAT_MAP must contain the (backend_ip={backend_ip}, {port}, {proto:?}) key — one \
         reverse path is installed per listener with its own declared proto byte (ADR-0060 D4/D7)"
    );
    let opposite = match proto {
        Proto::Tcp => Proto::Udp,
        Proto::Udp => Proto::Tcp,
    };
    let opposite_present = dataplane
        .reverse_nat_map_has_backend_proto(backend_ip, port, opposite)
        .expect("REVERSE_NAT_MAP opposite-proto readback");
    assert!(
        !opposite_present,
        "REVERSE_NAT_MAP must NOT contain a {opposite:?} key for the {proto:?}/{port} listener — \
         the proto byte is part of the key (ADR-0060 D7); a hit under the opposite proto means \
         the proto byte was ignored / defaulted"
    );
}

/// S-05-A — a two-listener service installs BOTH protocols' reverse
/// paths, and both a tcp and a udp round-trip are VIP-sourced.
///
/// Observable proxy (architect assertion-boundary discipline): the
/// REVERSE_NAT_MAP shows ONE key per listener, each with its own proto
/// byte — `(backend, 8080, tcp)` AND `(backend, 8081, udp)` — and each
/// the opposite-proto lookup MISSES (the proto byte is load-bearing).
/// Then a real tcp round-trip and a real udp round-trip each deliver a
/// reply sourced from the VIP.
#[test]
fn two_listener_service_installs_both_protocol_paths() {
    if !require_root_or_skip("S-05-A") {
        return;
    }
    use threeiface_ips::{BACKEND_IP, VIP};

    const TCP_MARKER: &str = "ovd-multi-tcp-marker";
    let Some(fixture) = build_multi_listener_service(
        "a",
        &[
            (tcp_frontend(VIP, TCP_PORT), Proto::Tcp, TCP_PORT),
            (udp_frontend(VIP, UDP_PORT), Proto::Udp, UDP_PORT),
        ],
        TCP_MARKER,
    ) else {
        return;
    };

    // (1) Observable proxy: both per-listener REVERSE_NAT keys present,
    // each distinguished by its proto byte.
    {
        let _ns_guard =
            enter_netns(&fixture.topo.lb_ns.name).expect("setns lb-ns for map readback");
        assert_reverse_nat_key_present_proto_distinguished(
            &fixture.dataplane,
            BACKEND_IP,
            TCP_PORT,
            Proto::Tcp,
        );
        assert_reverse_nat_key_present_proto_distinguished(
            &fixture.dataplane,
            BACKEND_IP,
            UDP_PORT,
            Proto::Udp,
        );
    }

    // (2) Both round-trips are VIP-sourced.
    let tcp_ok = run_tcp_round_trip(&fixture, TCP_PORT, TCP_MARKER);
    let udp_ok = run_udp_round_trip(&fixture, UDP_PORT, "ovd-multi-udp-marker");

    assert!(
        tcp_ok,
        "tcp listener (VIP:{TCP_PORT}) must deliver the backend's echoed reply through the VIP — \
         a reply still sourced from {BACKEND_IP} would not complete the connection through the \
         VIP. pcaps: {}",
        fixture.pcap_dir.display()
    );
    assert!(
        udp_ok,
        "udp listener (VIP:{UDP_PORT}) must deliver the backend's echoed datagram through the VIP \
         — `nc -u {VIP} {UDP_PORT}` drops any reply not sourced from {VIP}:{UDP_PORT}, so an \
         echoed datagram proves the udp reverse-NAT rewrote the source to the VIP. pcaps: {}",
        fixture.pcap_dir.display()
    );
}

/// S-05-B — each listener's reverse path is independently VIP-sourced.
///
/// Distinct from S-05-A's "both installed": this asserts the two reverse
/// paths are INDEPENDENT — exercising the tcp listener does not depend on
/// the udp listener and vice versa. Each round-trip is run and asserted
/// VIP-sourced in isolation (the proto byte in each REVERSE_NAT key keeps
/// the kernel `xdp_reverse_nat_lookup` matching the right path per
/// response proto).
#[test]
fn each_listener_reverse_path_independently_vip_sourced() {
    if !require_root_or_skip("S-05-B") {
        return;
    }
    use threeiface_ips::VIP;

    const TCP_MARKER: &str = "ovd-multi-b-tcp";
    let Some(fixture) = build_multi_listener_service(
        "b",
        &[
            (tcp_frontend(VIP, TCP_PORT), Proto::Tcp, TCP_PORT),
            (udp_frontend(VIP, UDP_PORT), Proto::Udp, UDP_PORT),
        ],
        TCP_MARKER,
    ) else {
        return;
    };

    // The udp reply is independently VIP-sourced.
    let udp_ok = run_udp_round_trip(&fixture, UDP_PORT, "ovd-multi-b-udp");
    // The tcp reply is independently VIP-sourced.
    let tcp_ok = run_tcp_round_trip(&fixture, TCP_PORT, TCP_MARKER);

    assert!(
        udp_ok,
        "the udp listener's reply must be independently VIP-sourced (VIP:{UDP_PORT}); pcaps: {}",
        fixture.pcap_dir.display()
    );
    assert!(
        tcp_ok,
        "the tcp listener's reply must be independently VIP-sourced (VIP:{TCP_PORT}); pcaps: {}",
        fixture.pcap_dir.display()
    );
}

/// S-05-C — adding a udp/8082 listener on re-submit converges so the new
/// path works AND the existing two paths still work.
///
/// Convergence is driven by an ADDITIONAL per-listener `update_service`
/// for udp/8082 against the already-running two-listener fixture — the
/// per-proto, per-listener fan-out (ADR-0060 D4) means each listener is
/// an independent call installing its own REVERSE_NAT key; adding one
/// must NOT disturb the other two (the empty-backends purge is per-proto,
/// so a new listener's install touches only its own key).
///
/// Observable proxy: after the add, the REVERSE_NAT_MAP shows all THREE
/// keys — `(backend, 8080, tcp)`, `(backend, 8081, udp)`, `(backend,
/// 8082, udp)` — and the new udp/8082 round-trip is VIP-sourced AND both
/// pre-existing listeners still deliver VIP-sourced replies.
#[test]
fn adding_listener_on_resubmit_converges_without_breaking_existing() {
    if !require_root_or_skip("S-05-C") {
        return;
    }
    use threeiface_ips::{BACKEND_IP, VIP};

    const TCP_MARKER: &str = "ovd-multi-c-tcp";
    let Some(mut fixture) = build_multi_listener_service(
        "c",
        &[
            (tcp_frontend(VIP, TCP_PORT), Proto::Tcp, TCP_PORT),
            (udp_frontend(VIP, UDP_PORT), Proto::Udp, UDP_PORT),
        ],
        TCP_MARKER,
    ) else {
        return;
    };

    // Re-submit: add the udp/8082 listener as an ADDITIONAL per-listener
    // update_service call (the convergence the production chain produces
    // on an edge.toml re-submit with one more listener), plus its backend
    // echo. The backend binds 8082 for udp.
    fixture.backend_children.push(spawn_udp_echo(&fixture.topo, UDP_PORT_2));
    std::thread::sleep(Duration::from_millis(300));
    {
        let _ns_guard = enter_netns(&fixture.topo.lb_ns.name).expect("setns lb-ns for add");
        let runtime =
            tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");
        let alloc = SpiffeId::new("spiffe://overdrive.local/job/edge/alloc/L2-added")
            .expect("added backend SpiffeId");
        runtime
            .block_on(fixture.dataplane.update_service(
                udp_frontend(VIP, UDP_PORT_2),
                vec![Backend {
                    alloc,
                    addr: SocketAddr::new(IpAddr::V4(BACKEND_IP), UDP_PORT_2),
                    weight: 1,
                    healthy: true,
                }],
            ))
            .expect("update_service add udp/8082 listener");
    }

    // (1) Observable proxy: all THREE per-listener REVERSE_NAT keys
    // present after the add (the new one installed, the existing two
    // untouched — per-proto fan-out, ADR-0060 D4).
    {
        let _ns_guard = enter_netns(&fixture.topo.lb_ns.name).expect("setns lb-ns for readback");
        assert_reverse_nat_key_present_proto_distinguished(
            &fixture.dataplane,
            BACKEND_IP,
            TCP_PORT,
            Proto::Tcp,
        );
        assert_reverse_nat_key_present_proto_distinguished(
            &fixture.dataplane,
            BACKEND_IP,
            UDP_PORT,
            Proto::Udp,
        );
        assert_reverse_nat_key_present_proto_distinguished(
            &fixture.dataplane,
            BACKEND_IP,
            UDP_PORT_2,
            Proto::Udp,
        );
    }

    // (2) The new udp/8082 path works AND the existing two still work.
    let new_udp_ok = run_udp_round_trip(&fixture, UDP_PORT_2, "ovd-multi-c-udp2");
    let old_udp_ok = run_udp_round_trip(&fixture, UDP_PORT, "ovd-multi-c-udp1");
    let old_tcp_ok = run_tcp_round_trip(&fixture, TCP_PORT, TCP_MARKER);

    assert!(
        new_udp_ok,
        "the newly-added udp/{UDP_PORT_2} listener must deliver a VIP-sourced reply after \
         re-submit; pcaps: {}",
        fixture.pcap_dir.display()
    );
    assert!(
        old_udp_ok,
        "the pre-existing udp/{UDP_PORT} listener must STILL deliver a VIP-sourced reply after \
         adding udp/{UDP_PORT_2} — adding a listener must not break existing paths (ADR-0060 D4 \
         per-proto fan-out); pcaps: {}",
        fixture.pcap_dir.display()
    );
    assert!(
        old_tcp_ok,
        "the pre-existing tcp/{TCP_PORT} listener must STILL deliver a VIP-sourced reply after \
         adding udp/{UDP_PORT_2}; pcaps: {}",
        fixture.pcap_dir.display()
    );
}
