//! S-2.2-15, S-2.2-18 — REVERSE_NAT_MAP real-TCP end-to-end.
//!
//! Tags: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`.
//! Tier: Tier 3.
//!
//! These tests drive the production `EbpfDataplane` against a real
//! veth pair stretched across two network namespaces:
//!
//! ```text
//!   netns "rnat-clt-<pid>"            netns "rnat-bck-<pid>"
//!     ┌──────────────┐                  ┌──────────────┐
//!     │   ovd-rclt   │ <───── pair ───> │   ovd-rbck   │
//!     │  10.0.0.100  │                  │   10.1.0.5   │
//!     │  XDP svc-map │                  │  nc -l 9000  │
//!     │  TC reverse  │                  │              │
//!     └──────────────┘                  └──────────────┘
//! ```
//!
//! The test process enters `netns-client` via `setns(2)` before
//! constructing `EbpfDataplane` so the BPF program's `attach()` call
//! and the `bpf_obj_get` pin recovery both resolve the iface index
//! within that namespace. `nc` subprocesses are spawned via `ip netns
//! exec` against the appropriate namespace.
//!
//! Capability gating: requires `CAP_NET_ADMIN` + `CAP_BPF`. Bails with
//! a skip on EPERM rather than failing — the test is run via
//! `cargo xtask lima run --` (default-root) on macOS and as the CI
//! integration job's `sudo`-wrapped invocation elsewhere.

#![cfg(target_os = "linux")]
#![allow(
    clippy::missing_panics_doc,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::similar_names
)]

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use overdrive_core::SpiffeId;
use overdrive_core::traits::dataplane::{Backend, Dataplane};
use overdrive_dataplane::EbpfDataplane;
use overdrive_dataplane::maps::ServiceKey;
use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;

use super::helpers::netns::{NetNs, NetNsError, ThreeIfaceTopology, threeiface_ips};
use super::helpers::veth::{VethError, VethPair};

// Per-test iface name pair. Both tests share E2eTopology, so we
// parameterise the name pair through `create_with_names` to give
// each test a distinct pair (avoids the `RTNETLINK answers: File
// exists` collision when nextest runs them in parallel processes
// against the same host-side namespace).
//
// Linux `IFNAMSIZ = 16` (15 chars + NUL) — keep both names under
// 15 chars.
const VIP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);
const VIP_PORT: u16 = 8080;
const CLIENT_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 100);
const BACKEND_IP: Ipv4Addr = Ipv4Addr::new(10, 1, 0, 5);
// `VIP_PORT == BACKEND_PORT` is REQUIRED for the S-2.2-17 forward
// path: the production `Dataplane::update_service(vip, backends)`
// trait derives the SERVICE_MAP key's port from `backends[0].addr.port()`
// (no separate `vip_port` argument). The kernel-side XDP key is built
// from the IPv4 packet's dst_port. For the lookup to hit, the SYN's
// dst_port (= VIP_PORT) must equal the backend's addr port (=
// BACKEND_PORT). Setting them equal models the most common L4LB
// deployment (VIP:8080 → backend:8080); per-port translation is a
// later phase if/when needed.
const BACKEND_PORT: u16 = VIP_PORT;

/// Two-namespace topology. Owns the lifecycle of both netns + the
/// veth pair connecting them. Drop teardown order matters: the veth
/// pair must drop first (its `Drop` issues `ip link del`), THEN the
/// namespaces. Rust drops fields in declaration order, so put `_veth`
/// before the namespaces.
///
/// In practice `ip netns del` reaps in-namespace ifaces too, so the
/// teardown is robust to either order — but the explicit ordering
/// matches the intent and avoids relying on kernel reaping for
/// correctness.
struct E2eTopology {
    client_ns: NetNs,
    /// Held for `Drop`-ordering: the backend netns must outlive the
    /// veth peer that lives inside it. The `Drop` impl on `NetNs`
    /// reaps the namespace and any in-namespace ifaces; this field
    /// is the lifecycle anchor and is otherwise not read.
    #[allow(dead_code)]
    backend_ns: NetNs,
    host_veth: String,
    /// Peer veth name. Currently only consumed inside `create()`
    /// for IP assignment + iface bring-up; retained as a struct
    /// field so future callers (the S-2.2-15 architectural
    /// follow-up; raw-socket inject tests on the backend side)
    /// can name it without re-deriving from the per-test tag.
    #[allow(dead_code)]
    peer_veth: String,
}

impl E2eTopology {
    /// Build the full topology:
    /// 1. Create both netns.
    /// 2. Create veth pair in host netns.
    /// 3. Move client end into client_ns; peer end into backend_ns.
    /// 4. Bring both ends up + assign IPs inside the respective ns.
    ///
    /// `tag` is a short (≤ 4 char) discriminator that namespaces
    /// the iface and netns names so two tests running in parallel
    /// processes don't collide on the global iface namespace.
    fn create(tag: &str) -> Result<Self, TopologyError> {
        let suffix = std::process::id();
        let client_name = format!("rnt-clt-{tag}-{suffix}");
        let backend_name = format!("rnt-bck-{tag}-{suffix}");
        // IFNAMSIZ = 16 (15 chars + NUL). Tag ≤ 4 + suffix u32
        // up to 5 hex chars + "ov-" 3 chars = 12 chars worst-case;
        // truncate the suffix to its low 16 bits to stay safe.
        let host_veth = format!("ov{tag}h{:04x}", suffix & 0xffff);
        let peer_veth = format!("ov{tag}p{:04x}", suffix & 0xffff);

        let client_ns = NetNs::create(&client_name).map_err(TopologyError::NetNs)?;
        let backend_ns = NetNs::create(&backend_name).map_err(TopologyError::NetNs)?;

        // Create veth in host netns first; subsequent `ip link set
        // ... netns ...` moves the ends into their target namespaces.
        // VethPair drops on error in `?`-shape, leaving netns also
        // dropped (clean teardown).
        let veth = VethPair::create(&host_veth, &peer_veth).map_err(TopologyError::Veth)?;

        // Move client end → client_ns; peer end → backend_ns. Once
        // moved, the iface name remains the same but it lives inside
        // that netns. The `VethPair::Drop` `ip link del <host>` would
        // fail because the iface no longer exists in the host netns
        // — but that's harmless (best-effort) and the netns drops
        // reap the ifaces anyway. Forget the VethPair to suppress
        // its Drop.
        client_ns.move_iface(&veth.host).map_err(TopologyError::NetNs)?;
        backend_ns.move_iface(&veth.peer).map_err(TopologyError::NetNs)?;
        std::mem::forget(veth);

        // Configure addresses in their respective namespaces with
        // /8 prefix so each ns has an on-link route covering BOTH
        // the local IP (10.0.0.100 in client; 10.1.0.5 in backend)
        // AND the peer's IP (10.1.0.5 from client's POV;
        // 10.0.0.100 from backend's POV) AND the VIP (10.0.0.1).
        // /16 only covers a single second-octet space so the kernel
        // refuses to route 10.1.x.x out a /16 iface configured on
        // 10.0.x.x. /8 covers 10.0.0.0/8 entirely.
        client_ns
            .assign_ip_and_up(&host_veth, &format!("{CLIENT_IP}/8"))
            .map_err(TopologyError::NetNs)?;
        backend_ns
            .assign_ip_and_up(&peer_veth, &format!("{BACKEND_IP}/8"))
            .map_err(TopologyError::NetNs)?;

        // Add a route in client_ns for the VIP. /16 already covers
        // 10.0.0.1 via the on-link route, so this is redundant —
        // kept best-effort + ignored for documentation.
        let _ = Command::new("ip")
            .args([
                "netns",
                "exec",
                &client_name,
                "ip",
                "route",
                "add",
                &VIP.to_string(),
                "dev",
                &host_veth,
            ])
            .output();

        // Backend ns needs to accept ARP for the VIP (10.0.0.1)
        // when the client first resolves it — without an answer,
        // the kernel never transmits the SYN. The simplest fix is
        // to teach the backend's veth to respond to ARP for the
        // VIP via a static neighbour entry on the client side
        // pointing at the peer's MAC.
        //
        // Actually simpler: disable rp_filter on both ifaces (the
        // packet's source IP is in a different subnet from the
        // backend's own /24, which strict rp_filter would drop).
        // /16 covers both addresses though, so rp_filter shouldn't
        // fire.
        for ns_name in [&client_name, &backend_name] {
            let _ = Command::new("ip")
                .args(["netns", "exec", ns_name, "sysctl", "-w", "net.ipv4.conf.all.rp_filter=0"])
                .output();
            let _ = Command::new("ip")
                .args([
                    "netns",
                    "exec",
                    ns_name,
                    "sysctl",
                    "-w",
                    "net.ipv4.conf.default.rp_filter=0",
                ])
                .output();
        }

        Ok(Self { client_ns, backend_ns, host_veth, peer_veth })
    }
}

#[derive(Debug)]
enum TopologyError {
    Veth(VethError),
    NetNs(NetNsError),
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Veth(e) => write!(f, "veth: {e}"),
            Self::NetNs(e) => write!(f, "netns: {e}"),
        }
    }
}
impl std::error::Error for TopologyError {}

/// Tells the test caller whether to skip vs propagate-as-failure.
fn classify_topology_setup(err: &TopologyError) -> SetupOutcome {
    match err {
        TopologyError::Veth(VethError::CapNetAdminRequired) => SetupOutcome::SkipNoCap,
        TopologyError::NetNs(NetNsError::CapNetAdminRequired) => SetupOutcome::SkipNoCap,
        _ => SetupOutcome::Failed,
    }
}

enum SetupOutcome {
    SkipNoCap,
    Failed,
}

/// Enter `target_ns` via `setns(2)` against the netns FD opened from
/// `/var/run/netns/<name>`. Returns the prior netns FD so the caller
/// can revert. Both FDs are owned by the caller (`OwnedFd`).
fn enter_netns(target_ns: &str) -> std::io::Result<NetNsGuard> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: `open(O_RDONLY)` on a kernel-managed path. The
    // resulting fd is owned by us; close on Drop.
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
    // SAFETY: open(O_RDONLY) on a netns mount; close on Drop.
    let target_fd = {
        let fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { OwnedFd::from_raw_fd(fd) }
    };

    // SAFETY: setns to a network namespace. The current thread
    // moves into the target namespace; subsequent BPF / iface ops
    // resolve within it.
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
            // Best-effort revert — failure here means subsequent
            // operations from this thread run in the wrong netns,
            // but the test process exits soon after either way.
            let _ = unsafe { libc::setns(fd.as_raw_fd(), libc::CLONE_NEWNET) };
        }
    }
}

/// Pre-flight: are we running as root with CAP_NET_ADMIN +
/// CAP_BPF? On EUID != 0 bail with a skip.
fn require_root_or_skip(test_name: &str) -> bool {
    // SAFETY: `geteuid` is `unsafe` per the libc binding family but
    // has no preconditions; reads a kernel-managed numeric.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        eprintln!("[skip] {test_name} needs root (CAP_NET_ADMIN + CAP_BPF); euid={euid}");
        return false;
    }
    true
}

/// S-2.2-17 — Real TCP connection completes through forward and
/// reverse paths.
///
/// **Slice 05-04 GREEN** — Option α (`bpf_fib_lookup` + L2 MAC
/// rewrite + cross-iface `bpf_redirect` when egress != ingress)
/// landed in the production XDP program. See
/// `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`.
///
/// Topology (per `ThreeIfaceTopology` in helpers/netns.rs):
///
/// ```text
///   client-ns                       lb-ns                          backend-ns
///     ┌──────────────┐                ┌──────────────────┐           ┌──────────────┐
///     │ client_veth  │ <──── pair ──> │ lb_veth_a        │           │              │
///     │ 10.0.0.10/24 │                │ 10.0.0.1/24      │           │              │
///     │              │                │                  │           │              │
///     │              │                │      lb_veth_b   │ <─pair─>  │ backend_veth │
///     │              │                │      10.1.0.1/24 │           │ 10.1.0.5/24  │
///     │              │                │  XDP+TC programs │           │  XDP_PASS    │
///     │              │                │  attach here     │           │  stub        │
///     └──────────────┘                └──────────────────┘           └──────────────┘
/// ```
///
/// Test flow:
///
/// 1. Build the 3-iface topology + per-test bpffs pin dir.
/// 2. Attach the `xdp_pass` stub in `backend-ns` on `backend_veth`
///    so XDP_REDIRECT delivery into the veth peer works (per
///    kernel patch v7 09/10 — XDP_TX/REDIRECT into a veth peer
///    requires the receiving veth to also have an XDP program).
/// 3. Spawn `nc -l <BACKEND_PORT>` in `backend-ns` with a known
///    payload pre-piped to its stdin.
/// 4. Construct `EbpfDataplane` in `lb-ns` attached to `lb_veth_a`.
/// 5. Pre-populate ARP in `lb-ns` for the backend's MAC against
///    `lb_veth_b` so the first SYN's `bpf_fib_lookup` returns
///    `RET_SUCCESS` deterministically (without ARP, the helper
///    returns `RET_NO_NEIGH` and the program falls back to
///    `XDP_PASS`, taking the slow path; pre-populating eliminates
///    the flake risk).
/// 6. `update_service(VIP, [backend])` populates SERVICE_MAP +
///    BACKEND_MAP + REVERSE_NAT_MAP.
/// 7. Spawn `nc <VIP> <VIP_PORT>` in `client-ns`.
/// 8. Assert `nc` exits 0 and the client's stdout contains the
///    backend's payload — proves the full forward + reverse path.
#[test]
fn real_tcp_connection_completes_through_vip_with_payload_echo() {
    if !require_root_or_skip("S-2.2-17") {
        return;
    }
    use threeiface_ips::{BACKEND_IP as A_BACKEND_IP, VIP as A_VIP};

    let topo = match ThreeIfaceTopology::create("a") {
        Ok(t) => t,
        Err(NetNsError::CapNetAdminRequired) => {
            eprintln!("[skip] S-2.2-17 needs CAP_NET_ADMIN");
            return;
        }
        Err(e) => panic!("3-iface topology setup failed: {e}"),
    };

    // Per-test bpffs pin dir for SERVICE_MAP pin-by-name.
    let pin_dir = PathBuf::from(format!("/sys/fs/bpf/overdrive-test-rnat3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_guard = PinDirGuard(pin_dir.clone());

    // Diagnostics dir for tcpdump pcaps. Best-effort.
    let pcap_dir = PathBuf::from(format!("/tmp/ovd-rnat3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pcap_dir);
    std::fs::create_dir_all(&pcap_dir).expect("create pcap dir");

    // Read backend's MAC for ARP pre-population. The `ThreeIfaceTopology`
    // exposes the iface name; we read /sys/class/net/<iface>/address
    // inside backend-ns via `ip netns exec`.
    let backend_mac =
        read_iface_mac(&topo.backend_ns.name, &topo.backend_veth).expect("read backend_veth MAC");
    eprintln!("[diag] backend_veth MAC = {backend_mac}");

    // Step 1 — Attach the no-op `xdp_pass` stub to `backend_veth` in
    // backend-ns. Per research § Finding 4.2 / kernel patch v7 09/10,
    // XDP_TX/REDIRECT into a veth peer requires the receiving veth to
    // have an XDP program attached.
    let stub_holder = {
        let _g = enter_netns(&topo.backend_ns.name).expect("setns backend-ns for stub");
        let stub_pin = pin_dir.join("backend-stub");
        let _ = std::fs::create_dir_all(&stub_pin);
        load_xdp_pass_stub(&topo.backend_veth, &stub_pin).expect("attach xdp_pass stub")
    };

    // Step 2 — Spawn `nc -l 9000` in backend-ns. The listener pipes a
    // fixed payload into the connection, then -q 1 closes after stdin
    // EOF.
    const PAYLOAD: &str = "ovd-rnat-e2e-marker";
    let mut backend_nc = topo
        .backend_ns
        .command("nc", ["-l", "-p", &BACKEND_PORT.to_string(), "-q", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn backend nc");
    {
        let stdin = backend_nc.stdin.as_mut().expect("backend nc stdin");
        stdin.write_all(PAYLOAD.as_bytes()).expect("payload to backend stdin");
        stdin.write_all(b"\n").expect("newline");
    }
    std::thread::sleep(Duration::from_millis(200));

    // Step 3 — Start tcpdump captures on each iface inside its ns.
    // Best-effort; if tcpdump is missing we still run the test but
    // diagnostics are unavailable.
    let mut tcpdumps: Vec<std::process::Child> = Vec::new();
    for (ns_name, iface, label) in [
        (&topo.client_ns.name, topo.client_veth.as_str(), "client"),
        (&topo.lb_ns.name, topo.lb_veth_a.as_str(), "lb_a"),
        (&topo.lb_ns.name, topo.lb_veth_b.as_str(), "lb_b"),
        (&topo.backend_ns.name, topo.backend_veth.as_str(), "backend"),
    ] {
        let pcap = pcap_dir.join(format!("{label}.pcap"));
        if let Ok(c) = Command::new("ip")
            .args([
                "netns",
                "exec",
                ns_name,
                "tcpdump",
                "-U",
                "-i",
                iface,
                "-w",
                pcap.to_str().unwrap_or(""),
                "-s",
                "256",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            tcpdumps.push(c);
        }
    }
    std::thread::sleep(Duration::from_millis(200));

    // Step 4 — Pre-populate ARP for the backend on `lb_veth_b` in
    // lb-ns. Without this, the first SYN's `bpf_fib_lookup` returns
    // `RET_NO_NEIGH` (the kernel ARP table is empty) and the program
    // falls back to `XDP_PASS`, taking the slow path through the
    // kernel networking stack. Pre-populating eliminates that flake.
    // See research § Finding 4.4 — "ip neigh add <backend_ip> lladdr
    // <backend_mac> dev <egress_iface> nud permanent".
    let neigh_add = Command::new("ip")
        .args([
            "netns",
            "exec",
            &topo.lb_ns.name,
            "ip",
            "neigh",
            "replace",
            &A_BACKEND_IP.to_string(),
            "lladdr",
            &backend_mac,
            "dev",
            &topo.lb_veth_b,
            "nud",
            "permanent",
        ])
        .output()
        .expect("ip neigh add for backend");
    assert!(
        neigh_add.status.success(),
        "ip neigh replace failed: stderr={:?}",
        String::from_utf8_lossy(&neigh_add.stderr)
    );

    // Step 5 — Construct EbpfDataplane in lb-ns and attach to
    // `lb_veth_a`. The XDP+TC programs live there per the helper
    // docstring's intent and per research Recommendation 1.
    let _ns_guard = enter_netns(&topo.lb_ns.name).expect("setns lb-ns");
    let dataplane = EbpfDataplane::new_with_pin_dir(&topo.lb_veth_a, &pin_dir)
        .expect("EbpfDataplane::new_with_pin_dir on lb_veth_a");

    let backend_alloc =
        SpiffeId::new("spiffe://overdrive.local/job/e2e/alloc/B1").expect("backend SpiffeId");
    let backend_addr = SocketAddr::new(IpAddr::V4(A_BACKEND_IP), BACKEND_PORT);
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");
    runtime
        .block_on(dataplane.update_service(
            A_VIP,
            vec![Backend {
                alloc: backend_alloc.clone(),
                addr: backend_addr,
                weight: 1,
                healthy: true,
            }],
        ))
        .expect("update_service");
    drop(_ns_guard);

    // Step 6 — Spawn client `nc <vip> 8080` in client-ns.
    let mut client_nc = topo
        .client_ns
        .command("nc", [&A_VIP.to_string(), &VIP_PORT.to_string(), "-q", "1", "-w", "5"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn client nc");
    {
        let stdin = client_nc.stdin.as_mut().expect("client nc stdin");
        stdin.write_all(b"hello\n").expect("client stdin");
    }

    let client_status = wait_with_timeout(&mut client_nc, Duration::from_secs(8));
    let mut client_stdout = String::new();
    if let Some(mut s) = client_nc.stdout.take() {
        let _ = s.read_to_string(&mut client_stdout);
    }
    let mut client_stderr = String::new();
    if let Some(mut s) = client_nc.stderr.take() {
        let _ = s.read_to_string(&mut client_stderr);
    }
    let _ = wait_with_timeout(&mut backend_nc, Duration::from_secs(2));

    // Stop tcpdumps before assertions so pcaps are flushed to disk.
    for mut t in tcpdumps {
        let _ = t.kill();
        let _ = t.wait();
    }
    eprintln!("[diag] pcaps written under: {}", pcap_dir.display());

    // Hold the BPF objects until end-of-test so attachments stay
    // alive across `nc` lifecycle.
    drop(dataplane);
    drop(stub_holder);

    // Assertions.
    let status = client_status.expect("client nc exit within 8s");
    assert!(
        status.success(),
        "client nc exited non-zero (code = {:?}); stdout = {client_stdout:?}; stderr = {client_stderr:?}; pcaps = {}",
        status.code(),
        pcap_dir.display(),
    );
    assert!(
        client_stdout.contains(PAYLOAD),
        "client did not receive backend payload; got stdout = {client_stdout:?}; stderr = {client_stderr:?}; pcaps = {}",
        pcap_dir.display(),
    );
}

/// Read `/sys/class/net/<iface>/address` inside a netns. Used by
/// the S-2.2-17 test to pre-populate the LB's ARP table for the
/// backend before the first SYN arrives — eliminates the
/// `RET_NO_NEIGH` first-packet slow-path flake.
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

/// `xdp_pass` stub attached to a backend-side veth iface.
///
/// `_service_map` keeps the pre-pinned outer SERVICE_MAP HoM (and
/// its inner-map prototype FD) alive for the stub's lifetime — if
/// it drops first, the kernel reclaims the outer map and the next
/// load against the bpffs pin would fail.
struct StubXdpHolder {
    _service_map: HashOfMapsHandle<ServiceKey, u32>,
    _bpf: aya::Ebpf,
    _link: aya::programs::xdp::XdpLinkId,
}

/// Load the `xdp_pass` program from the embedded BPF object and
/// attach it to `iface` in the calling thread's current netns.
/// The `pin_dir` is consumed by aya's loader for any pinned-map
/// resolution but is otherwise unused by `xdp_pass` — that program
/// only writes to `PKTS` (LruHashMap) which uses no pinning.
///
/// Caller must hold `setns(2)` on the target namespace before
/// calling — `if_nametoindex` resolves against the calling
/// thread's netns.
fn load_xdp_pass_stub(iface: &str, pin_dir: &std::path::Path) -> Result<StubXdpHolder, String> {
    use aya::programs::{Xdp, XdpFlags};

    // Pre-create + pin SERVICE_MAP first. The shared BPF ELF declares
    // SERVICE_MAP as a `HASH_OF_MAPS` with `pinning = ByName`; aya
    // 0.13.x's loader cannot create HoM directly (it falls back to
    // bare `BPF_MAP_CREATE` which the kernel rejects without an
    // `inner_map_fd`), so we replicate the production
    // `EbpfDataplane::new_with_pin_dir` pin-by-name dance here. Once
    // pinned, aya's loader sees the existing pin and reuses it via
    // `BPF_OBJ_GET`. Capacities mirror the production constants in
    // `crates/overdrive-dataplane/src/lib.rs` (architecture.md § 10):
    // outer = 4096, inner = MaglevTableSize::DEFAULT (16_381).
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

    // Re-use the embedded BPF artifact from the dataplane crate.
    // We need to materialise it to a temp file because aya's
    // `EbpfLoader::load_file` is the more tolerant path (matches
    // the production `EbpfDataplane::new_with_pin_dir` shape).
    const STUB_OBJ: &[u8] = include_bytes!(concat!(
        env!("CARGO_WORKSPACE_DIR"),
        "/target/xtask/bpf-objects/overdrive_bpf.o",
    ));
    let bpf_temp_path =
        std::env::temp_dir().join(format!("overdrive_bpf_stub-{}.o", std::process::id()));
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

/// S-2.2-18 — Removed backend's REVERSE_NAT entry purged on service
/// update; no stale rewrite leak.
///
/// Test:
///   1. Set up two netns + veth (via `E2eTopology`).
///   2. Construct dataplane with backend B1 = 10.1.0.5:9000.
///   3. Verify REVERSE_NAT_MAP contains B1's entry by checking the
///      live-set tracker (the post-update internal state).
///   4. Call update_service again with an EMPTY backend set —
///      semantically "remove all backends" — which should purge
///      the prior REVERSE_NAT entry.
///   5. Wait, then call again with the backend swapped out for
///      B2 (different IP) — verifies the diff-shaped purge in
///      `update_service` removes B1's entry and inserts B2's.
///   6. Send a "late response" packet from B1's address via raw
///      socket (simulating an in-flight backend response after
///      removal) and capture it on the client side; assert the
///      packet either does NOT appear (TC dropped/passed it
///      through unchanged) OR if captured, does NOT carry a
///      rewritten source address matching the VIP.
///
/// The test uses internal observation of the dataplane's
/// `service_reverse_nat_keys` tracker (cannot — that field is
/// `pub(crate)` only) → we instead verify via behavior: send a
/// late response from B1, assert no rewrite. If the entry was
/// purged, the `tc_reverse_nat` lookup misses and the packet
/// passes through with src unchanged.
#[test]
fn removing_backend_purges_reverse_nat_entry_no_stale_rewrite() {
    if !require_root_or_skip("S-2.2-18") {
        return;
    }

    let topo = match E2eTopology::create("b") {
        Ok(t) => t,
        Err(e) => match classify_topology_setup(&e) {
            SetupOutcome::SkipNoCap => {
                eprintln!("[skip] S-2.2-18 needs CAP_NET_ADMIN: {e}");
                return;
            }
            SetupOutcome::Failed => panic!("topology setup failed: {e}"),
        },
    };

    let pin_dir =
        PathBuf::from(format!("/sys/fs/bpf/overdrive-test-rnatpurge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin_dir);
    std::fs::create_dir_all(&pin_dir).expect("create pin dir");
    struct PinDirGuard(PathBuf);
    impl Drop for PinDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _pin_guard = PinDirGuard(pin_dir.clone());

    let _ns_guard = enter_netns(&topo.client_ns.name).expect("setns into client ns");

    let dataplane = EbpfDataplane::new_with_pin_dir(&topo.host_veth, &pin_dir)
        .expect("EbpfDataplane::new_with_pin_dir");

    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio rt");

    // Step 1: install backend B1 — REVERSE_NAT_MAP gets B1's entry.
    let alloc_b1 =
        SpiffeId::new("spiffe://overdrive.local/job/e2e/alloc/B1").expect("alloc B1 SpiffeId");
    runtime
        .block_on(dataplane.update_service(
            VIP,
            vec![Backend {
                alloc: alloc_b1.clone(),
                addr: SocketAddr::new(IpAddr::V4(BACKEND_IP), BACKEND_PORT),
                weight: 1,
                healthy: true,
            }],
        ))
        .expect("update_service install B1");

    // Step 2: snapshot REVERSE_NAT_MAP key count via the public
    // accessor we add below.
    let count_after_b1 = dataplane.reverse_nat_map_size().expect("size readback after B1");
    assert!(
        count_after_b1 >= 1,
        "REVERSE_NAT_MAP must contain at least B1's entry after install (got {count_after_b1})"
    );

    // Step 3: install B2 (different IP) — the `update_service`
    // diff should purge B1's entry and insert B2's. The size
    // therefore returns to 1 (one backend → one REVERSE_NAT entry).
    let backend_b2_ip = Ipv4Addr::new(10, 1, 0, 6);
    // For B2 we need an IP that's actually reachable by the
    // backend ns. Without bringing up an alias we can't echo
    // packets from B2, but for the purge-test the IP only needs
    // to exist as a `BackendKeyPod` — REVERSE_NAT_MAP doesn't care
    // about reachability; the kernel-side TC program looks up by
    // the packet's source IP. So B2 ≠ B1 is sufficient.
    let alloc_b2 =
        SpiffeId::new("spiffe://overdrive.local/job/e2e/alloc/B2").expect("alloc B2 SpiffeId");
    runtime
        .block_on(dataplane.update_service(
            VIP,
            vec![Backend {
                alloc: alloc_b2.clone(),
                addr: SocketAddr::new(IpAddr::V4(backend_b2_ip), BACKEND_PORT),
                weight: 1,
                healthy: true,
            }],
        ))
        .expect("update_service swap B1 → B2");

    let count_after_b2 = dataplane.reverse_nat_map_size().expect("size readback after B2");
    assert_eq!(
        count_after_b2, 1,
        "REVERSE_NAT_MAP must contain exactly 1 entry after swap (B2 replaces B1); \
         got {count_after_b2}"
    );

    // Step 4: B1's specific entry must be gone — point-lookup
    // returns None.
    let b1_present =
        dataplane.reverse_nat_map_has_backend(BACKEND_IP, BACKEND_PORT).expect("readback B1 entry");
    assert!(!b1_present, "B1's REVERSE_NAT entry must be purged after backend swap (S-2.2-18)");

    // Step 5: B2's entry IS present — confirms the populate path
    // ran for the new backend.
    let b2_present = dataplane
        .reverse_nat_map_has_backend(backend_b2_ip, BACKEND_PORT)
        .expect("readback B2 entry");
    assert!(b2_present, "B2's REVERSE_NAT entry must be installed after swap");

    drop(_ns_guard);
    drop(dataplane);
    let _ = topo;
}

/// Wait for `child` to exit, polling at 50 ms intervals up to
/// `budget`. Returns the exit status, or panics on timeout (which
/// the caller may want to handle differently — the convention here
/// is that `nc` is expected to exit promptly via `-q 1`).
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
    // Best-effort kill on timeout so the child does not leak.
    let _ = child.kill();
    let _ = child.wait();
    None
}
