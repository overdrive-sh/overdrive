//! netns-density-295 E14 — intercept-marked guest TCP fails closed
//! (S-ND295-62/63/64; D-295-R18, R19, conditional on this native RED).
//!
//! Authorized defensive testing of the platform's OWN firewall on an isolated
//! native host: a test-owned guest microVM sends TCP SYNs toward host
//! listeners the test itself binds, and the test checks that the platform's
//! own nft rules fail closed when parts of its intercept program are missing.
//! Nothing targets a system we do not own.
//!
//! Every body drives the production entry points (`serve::run_with_kek` +
//! `deploy`/`stop`) on a non-virtualized x86_64 host under
//! `cargo xtask metal run --`; each fault enters through a real kernel
//! mutation (`nft delete table`, a sock-diag destroy of the leg-F listening
//! tuple, killed-mode serve) or a resource the production path created. No
//! test installs a production effect the production path omits.
//!
//! # Healthy baseline and the per-run SYN-entered-host check (E14, 2026-09-25)
//!
//! Before any fault, the guard's non-interference is proven by a no-fault
//! baseline: with both tables present, leg F listening, and
//! `observe_intercept_mark_guard()` reading back present, each guest SYN gets
//! a SYN-ACK and the wildcard host listener accepts nothing — the intercept
//! answered and the guard dropped nothing. The guard rule carries no counter.
//! Every fault run then adds a per-run check: the probe SYN is captured on its
//! sender's TAP *while that TAP reads back administratively up*, the node-wide
//! TCX `Intercept` counter rises by at least the SYNs sent, and the bridge
//! guard's default-drop counter is unchanged. A run whose TAP was already
//! quiesced is void, not GREEN.
//!
//! These bodies observe R18/R19 fail-closure, which is not on the production
//! path until DELIVER step 08-01 decides R18/R19 from this native RED, so each
//! is `#[ignore = "pending DELIVER step 08-01 (S-ND295-xx)"]` and fails today
//! for the right reason. The dialer is plaintext by design
//! (CLAUDE.md § "East-west mTLS tests").

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "Tier-3 native fixtures fail fast, and CONTRACT_SHAPE lines use exact mandated tokens"
)]

use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_dataplane::guest_tcx::{GuestTcxCounter, read_counter};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing::vm_fixture::VmFixture;

use super::guest_stack_mtls_egress::{
    MESH_NAME, SERVICE_PORT, build_mesh_peer, build_static_binary, observe_shared_intercept_state,
    service_toml,
};
use super::serve_lifetime_support::kill_serve_owner;
use super::vm_walking_skeleton::{
    config_path, poll_until_running, poll_until_terminal, shared_staging_root,
    spawn_vm_server_mtls_composed, stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The shared intercept program table R18 deletes.
const INTERCEPT_TABLE: &str = "overdrive-mtls";
/// The bridge-guard table R18's second case deletes.
const GUARD_TABLE: &str = "overdrive-mtls-guard";
/// The node bridge gateway every guest routes through; a guest dial to this
/// address at `HOST_WILDCARD_PORT` reaches a `0.0.0.0`-bound host listener.
const GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);
/// An address outside every managed and registered set (TEST-NET-1).
const EXTERNAL: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
/// The port a wildcard host listener binds; the probe guest dials it at the
/// gateway and the external address.
const HOST_WILDCARD_PORT: u16 = 18_960;
/// The TCX counter map pin the production loader installs.
const COUNTER_MAP_PIN: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/maps/counters";
/// Bound for a real production boot to reach Running.
const RUNNING_BOUND: Duration = Duration::from_secs(90);
/// Bound for a stop to converge on Terminated.
const TERMINAL_BOUND: Duration = Duration::from_secs(30);
/// The window a probe guest keeps dialing so the host observes across a fault.
const PROBE_WINDOW: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Probe guest and mesh service
// ---------------------------------------------------------------------------

/// Build a guest binary that dials, in a loop for the whole probe window, the
/// mesh peer by name and the gateway and external addresses at
/// `HOST_WILDCARD_PORT`, so the host can observe whether any SYN is answered
/// or forwarded while a fault holds. Every dial is a plaintext `TcpStream`
/// with a short connect timeout (the workload is identity-unaware).
fn build_syn_probe_guest(tmp: &Path) -> PathBuf {
    let source = format!(
        r#"
use std::net::TcpStream;
use std::time::{{Duration, Instant}};

fn dial(target: &str) {{
    if let Ok(mut stream) = TcpStream::connect_timeout(
        &target.parse().unwrap_or_else(|_| {{
            // The mesh name is resolved through the guest resolver.
            use std::net::ToSocketAddrs;
            target.to_socket_addrs().ok().and_then(|mut a| a.next())
                .unwrap_or_else(|| "127.0.0.1:1".parse().unwrap())
        }}),
        Duration::from_millis(800),
    ) {{
        use std::io::Write;
        let _ = stream.write_all(b"ND295-SYN-PROBE");
    }}
}}

fn main() {{
    let deadline = Instant::now() + Duration::from_secs({window});
    while Instant::now() < deadline {{
        dial("{mesh}:{svc}");
        dial("{gw}:{port}");
        dial("{ext}:{port}");
        std::thread::sleep(Duration::from_millis(200));
    }}
}}
"#,
        window = PROBE_WINDOW.as_secs() + 10,
        mesh = MESH_NAME,
        svc = SERVICE_PORT,
        gw = GATEWAY,
        ext = EXTERNAL,
        port = HOST_WILDCARD_PORT,
    );
    build_static_binary(tmp, "nd295-syn-probe", &source)
}

/// One deployed VM allocation and the guest identities the host observes.
struct DeployedGuest {
    workload_id: String,
    tap: String,
}

/// The production TAP name for a guest address (`ovd-tp-<4hex>`).
fn tap_for(addr: Ipv4Addr) -> String {
    let o = addr.octets();
    format!("ovd-tp-{:04x}", u16::from_be_bytes([o[2], o[3]]))
}

async fn deploy_probe_guest(cfg: &Path, dir: &Path, kernel: &Path, rootfs: &Path) -> DeployedGuest {
    let spec = write_toml(
        dir,
        "nd295-probe.toml",
        &vm_job_toml("nd295-probe", "/sbin/nd295-syn-probe", &[], kernel, rootfs),
    );
    let output = deploy(DeployArgs { spec, config_path: cfg.to_path_buf() })
        .await
        .expect("deploy the SYN-probe guest through the production handler");
    let running = poll_until_running(cfg, &output.workload_id, RUNNING_BOUND).await;
    let row = running.snapshot.rows.first().expect("one Running allocation row");
    let addr = row.workload_addr.expect("a Running VM publishes its guest address");
    DeployedGuest { workload_id: output.workload_id, tap: tap_for(addr) }
}

async fn stop_and_await_terminal(cfg: &Path, workload_id: &str) {
    stop(StopArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
        .await
        .expect("stop the workload through the production verb");
    let terminal = poll_until_terminal(cfg, workload_id, TERMINAL_BOUND).await;
    assert_eq!(
        terminal.snapshot.rows.first().expect("terminal row").state,
        AllocStateWire::Terminated,
        "the stopped workload reaches Terminated"
    );
}

// ---------------------------------------------------------------------------
// Host observation: wildcard listener accept-count, TAP up-state, per-run
// SYN-entered-host check, guard drop counter (nft diagnostic tool).
// ---------------------------------------------------------------------------

/// A `0.0.0.0:port` listener whose background thread counts accepted
/// connections; the count is the "no host listener accepted anything" oracle.
struct WildcardListener {
    accepts: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl WildcardListener {
    fn bind(port: u16) -> Self {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
            .expect("bind wildcard host listener");
        listener.set_nonblocking(true).expect("nonblocking wildcard listener");
        let accepts = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let join = {
            let accepts = Arc::clone(&accepts);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            accepts.fetch_add(1, Ordering::SeqCst);
                            let mut byte = [0_u8; 1];
                            let _ = stream.read(&mut byte);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self { accepts, stop, join: Some(join) }
    }

    fn accepted(&self) -> u64 {
        self.accepts.load(Ordering::SeqCst)
    }
}

impl Drop for WildcardListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// The observed administrative up-state of `tap` (`true` iff `IFF_UP`); a
/// missing interface reads `false`. The per-run check is void unless the
/// sender's TAP reads back up while the SYN is sent.
fn tap_is_up(tap: &str) -> bool {
    std::fs::read_to_string(format!("/sys/class/net/{tap}/flags"))
        .ok()
        .and_then(|text| u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok())
        .is_some_and(|flags| flags & 0x1 == 0x1)
}

/// An AF_PACKET capture on one interface counting TCP SYN frames toward
/// `dst_port`; its Drop closes the socket.
struct SynCapture {
    fd: RawFd,
    dst_port: u16,
}

impl SynCapture {
    fn open(interface: &str, dst_port: u16) -> Self {
        const ETH_P_ALL: u16 = 0x0003;
        let name = std::ffi::CString::new(interface).expect("iface name has no NUL");
        // SAFETY: `name` is a live NUL-terminated string for this call.
        let ifindex = unsafe { libc::if_nametoindex(name.as_ptr()) };
        assert_ne!(ifindex, 0, "interface {interface} is live");
        // SAFETY: AF_PACKET raw socket owned by this SynCapture.
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                i32::from(ETH_P_ALL.to_be()),
            )
        };
        assert!(fd >= 0, "open SYN capture on {interface}: {}", std::io::Error::last_os_error());
        // SAFETY: zero-initialised sockaddr_ll before its fields are set.
        let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
        address.sll_protocol = ETH_P_ALL.to_be();
        address.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits i32");
        // SAFETY: `address` is a fully initialised sockaddr_ll of the given length.
        let bound = unsafe {
            libc::bind(
                fd,
                std::ptr::from_ref(&address).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll length fits socklen_t"),
            )
        };
        if bound != 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: fd owned; closed exactly here on bind failure.
            unsafe { libc::close(fd) };
            panic!("bind SYN capture on {interface}: {error}");
        }
        Self { fd, dst_port }
    }

    /// Drain every queued frame, counting IPv4 TCP SYNs whose destination port
    /// is `self.dst_port`.
    fn drain_syns(&self) -> usize {
        let mut matched = 0;
        loop {
            let mut frame = [0_u8; 2048];
            // SAFETY: `frame` is a live writable buffer; `self.fd` is owned.
            let read = unsafe {
                libc::recv(self.fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                if is_syn_to_port(&frame[..length], self.dst_port) {
                    matched += 1;
                }
                continue;
            }
            if read == 0 {
                return matched;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return matched;
            }
            panic!("SYN capture recv failed: {error}");
        }
    }
}

impl Drop for SynCapture {
    fn drop(&mut self) {
        // SAFETY: SynCapture exclusively owns this socket fd.
        unsafe { libc::close(self.fd) };
    }
}

/// `true` iff `frame` is an Ethernet/IPv4/TCP SYN (SYN set, ACK clear) whose
/// TCP destination port equals `dst_port`.
fn is_syn_to_port(frame: &[u8], dst_port: u16) -> bool {
    if frame.len() < 14 + 20 + 20 {
        return false;
    }
    if u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return false; // not IPv4
    }
    let ihl = usize::from(frame[14] & 0x0f) * 4;
    if frame[23] != 0x06 || frame.len() < 14 + ihl + 20 {
        return false; // not TCP / too short
    }
    let tcp = &frame[14 + ihl..];
    let port = u16::from_be_bytes([tcp[2], tcp[3]]);
    let flags = tcp[13];
    port == dst_port && (flags & 0x02) != 0 && (flags & 0x10) == 0
}

/// The bridge guard chain's default-drop counter (packets), read through the
/// `nft` diagnostic tool. `0` when the guard table is absent.
fn guard_drop_packets() -> u64 {
    let output = Command::new("nft")
        .args(["-j", "list", "table", "ip", GUARD_TABLE])
        .output()
        .expect("run nft list");
    if !output.status.success() {
        return 0;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // The guard's default-drop rule carries a `counter` expression; sum the
    // packet counts nft reports for the table's rules.
    text.split("\"packets\":")
        .skip(1)
        .filter_map(|tail| {
            tail.trim_start().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u64>().ok())
        })
        .sum()
}

/// The node-wide TCX `Intercept` counter, or `0` when the map pin is absent.
fn intercept_counter() -> u64 {
    read_counter(PathBuf::from(COUNTER_MAP_PIN), GuestTcxCounter::Intercept).unwrap_or(0)
}

/// Poll a real-wall-clock predicate up to `bound`.
async fn wait_until(bound: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The per-run SYN-entered-host check every fault run adds (E14): while the
/// sender's TAP reads back up, at least `min_syns` probe SYNs are captured on
/// it, the `Intercept` counter rose, and the guard's default-drop counter is
/// unchanged. A run whose TAP was already quiesced is void, not GREEN.
fn assert_syn_entered_host(
    tap: &str,
    capture: &SynCapture,
    intercept_before: u64,
    guard_before: u64,
    min_syns: usize,
) {
    assert!(tap_is_up(tap), "the run is void, not GREEN: sender TAP {tap} was quiesced");
    let captured = capture.drain_syns();
    assert!(
        captured >= min_syns,
        "the probe SYN entered the host on its own TAP {tap}: captured {captured} >= {min_syns}"
    );
    assert!(
        intercept_counter() > intercept_before,
        "the TCX Intercept counter rose for the marked flow"
    );
    assert_eq!(
        guard_drop_packets(),
        guard_before,
        "the bridge guard's default-drop counter is unchanged"
    );
}

// ---------------------------------------------------------------------------
// S-ND295-62 — intercept-marked guest TCP is dropped even without the program
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-62 — Intercept-marked guest TCP is dropped even without the program table
/// CONTRACT_SHAPE: bounded-change.
///
/// E14 (a)/(b): with host forwarding enabled, deleting the intercept program
/// table leaves a marked guest SYN neither forwarded to the peer's TAP nor
/// accepted by any host wildcard listener. The healthy baseline (before the
/// fault) shows the guard drops nothing on the same flows; every fault run
/// adds the per-run SYN-entered-host check.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-62)"]
async fn marked_guest_tcp_is_neither_forwarded_nor_delivered_without_the_intercept_program() {
    let forwarding = std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .expect("read ip_forward")
        .trim()
        .to_owned();
    std::fs::write("/proc/sys/net/ipv4/ip_forward", b"1").expect("enable host forwarding");
    eprintln!("S-ND295-62: ip_forward recorded as {forwarding}, set to 1 as a precondition");

    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-62-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let probe = build_syn_probe_guest(tmp.path());
    let peer_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // A mesh Service (the peer) and the SYN-probe client Job.
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-62-peer.toml",
        &service_toml(&peer, &fixture.kernel_path, &peer_rootfs),
    );
    deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh peer service");

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;
    assert!(
        observe_shared_intercept_state().is_ok_and(|state| state.is_some()),
        "the intercept program is installed before the fault"
    );

    // Healthy baseline: with both tables present, the guard drops nothing on
    // the marked flows and the intercept answers.
    let baseline_guard = guard_drop_packets();
    assert!(
        wait_until(Duration::from_secs(5), || intercept_counter() > 0).await,
        "the healthy intercept catches the marked flow"
    );
    assert_eq!(
        guard_drop_packets(),
        baseline_guard,
        "healthy baseline: the guard drops nothing on the marked healthy flow"
    );

    // Fault (a)/(b): delete the intercept program table.
    let intercept_before = intercept_counter();
    let guard_before = guard_drop_packets();
    let capture = SynCapture::open(&guest.tap, SERVICE_PORT);
    let gateway_capture = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();
    overdrive_netlink::nft::delete_table(INTERCEPT_TABLE)
        .expect("external actor deletes the intercept program table");

    // The probe keeps dialing across the window; no host listener accepts.
    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts a marked guest SYN without the program"
    );
    // Per-run SYN-entered-host check across both marked flows.
    assert_syn_entered_host(&guest.tap, &gateway_capture, intercept_before, guard_before, 1);
    drop(capture);

    stop_and_await_terminal(&cfg, &guest.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
    std::fs::write("/proc/sys/net/ipv4/ip_forward", forwarding.as_bytes())
        .expect("restore ip_forward");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-62 — Intercept-marked guest TCP is dropped even without the program table
/// CONTRACT_SHAPE: bounded-change.
///
/// The guard-only deletion case: deleting ONLY the guard table leaves the
/// intercept program still catching or dropping the marked traffic — no host
/// wildcard listener accepts a marked guest SYN.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-62)"]
async fn the_intercept_program_still_catches_marked_tcp_without_the_guard_table() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-62g-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path());
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    let intercept_before = intercept_counter();
    let guard_before = guard_drop_packets();
    let capture = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();
    // Delete ONLY the guard table; the intercept program stays installed.
    overdrive_netlink::nft::delete_table(GUARD_TABLE)
        .expect("external actor deletes only the guard table");

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "the intercept program alone still keeps a marked guest SYN out of any host listener"
    );
    assert_syn_entered_host(&guest.tap, &capture, intercept_before, guard_before, 1);

    stop_and_await_terminal(&cfg, &guest.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-63 — guest TCP to an absent listener fails closed
// ---------------------------------------------------------------------------

/// The leg-F listener port the owned intercept program targets, read from the
/// installed program's prerouting TPROXY rule through the `nft` diagnostic
/// tool (the port is not on the public `SharedIpInterceptState` surface; the
/// owned program's target is the source of truth per E14). Confirms the
/// program is installed first.
fn leg_f_port() -> u16 {
    assert!(
        observe_shared_intercept_state().is_ok_and(|state| state.is_some()),
        "the intercept program must be installed before its leg-F port is read"
    );
    let output = Command::new("nft")
        .args(["-j", "list", "table", "ip", INTERCEPT_TABLE])
        .output()
        .expect("run nft list for the intercept table");
    assert!(output.status.success(), "nft list overdrive-mtls: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    // The prerouting TPROXY expression carries `"tproxy":{...,"port":N}`; the
    // outbound (leg-F) rule is the only TPROXY to a loopback port.
    text.split("\"tproxy\":")
        .skip(1)
        .find_map(|tail| {
            let port_key = tail.find("\"port\":")? + "\"port\":".len();
            tail[port_key..]
                .trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|port| *port != 0)
        })
        .expect("the intercept program's prerouting TPROXY names a leg-F port")
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-63 — Guest TCP to an absent listener fails closed
/// CONTRACT_SHAPE: bounded-change.
///
/// E14 (c): with the program present and the leg-F listener destroyed from
/// outside (its port immediately re-occupied by a plain, non-transparent
/// listener — the S-ND295-31B port-theft shape), a guest SYN to the gateway or
/// an external address at a wildcard host listener's port gets no SYN-ACK and
/// that listener accepts nothing.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-63)"]
async fn outbound_tcp_to_a_closed_listener_is_dropped_not_delivered_locally() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63c-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path());
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    let leg_f = leg_f_port();
    let intercept_before = intercept_counter();
    let guard_before = guard_drop_packets();
    let capture = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();

    // Destroy the leg-F listening socket from outside and immediately re-occupy
    // its port with a plain non-transparent listener (port-theft), holding the
    // listener-absent state.
    let destroy = Command::new("ss")
        .args(["-K", "sport", &format!("= {leg_f}")])
        .status()
        .expect("run ss -K to destroy the leg-F listening socket");
    assert!(destroy.success(), "ss -K destroys the leg-F listening tuple");
    let _thief = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, leg_f))
        .expect("re-occupy the leg-F port with a plain listener (port-theft)");

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts an outbound guest SYN once leg F is gone"
    );
    assert_syn_entered_host(&guest.tap, &capture, intercept_before, guard_before, 1);

    stop_and_await_terminal(&cfg, &guest.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-63 — Guest TCP to an absent listener fails closed
/// CONTRACT_SHAPE: bounded-change.
///
/// E14 (d): in killed mode — the serve owner abandoned through the lifetime
/// port while Cloud Hypervisor stays alive and the TAP stays up — a guest SYN
/// to the gateway or external address gets no SYN-ACK and no host listener
/// accepts it.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-63)"]
async fn outbound_tcp_after_a_killed_server_is_dropped_while_the_vm_lives() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63d-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path());
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    let intercept_before = intercept_counter();
    let guard_before = guard_drop_packets();
    let capture = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();

    // Killed mode: abandon the serve owner; Cloud Hypervisor and the TAP stay
    // up, so the guest keeps dialing but leg F is gone.
    kill_serve_owner(handle).await.expect("killed-mode serve abandons its owner");
    assert!(tap_is_up(&guest.tap), "the guest's TAP stays up after killed mode");

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts an outbound guest SYN after killed mode"
    );
    assert_syn_entered_host(&guest.tap, &capture, intercept_before, guard_before, 1);
    drop(listener);
    // The abandoned server left the VM live; reclaim it directly.
    let _ = stop(StopArgs { id: guest.workload_id.clone(), config_path: cfg.clone() }).await;
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-63 — Guest TCP to an absent listener fails closed
/// CONTRACT_SHAPE: bounded-change.
///
/// Inbound control: with leg C closed, a SYN to a registered destination is
/// dropped by the outbound-mark guard rule (rule 4), under both orders — no
/// host listener on the registered destination accepts it.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-63)"]
async fn inbound_tcp_to_a_closed_listener_is_dropped() {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63i-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let peer_rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-63i-peer.toml",
        &service_toml(&peer, &fixture.kernel_path, &peer_rootfs),
    );
    let output = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh peer service");
    let running = poll_until_running(&cfg, &output.workload_id, RUNNING_BOUND).await;
    let row = running.snapshot.rows.first().expect("one Running row");
    let addr = row.workload_addr.expect("the peer publishes its guest address");

    // Destroy the peer's leg-C listening socket, then an external SYN to the
    // registered destination (addr:SERVICE_PORT) must be dropped by rule 4.
    let destroy = Command::new("ss")
        .args(["-K", "dst", &addr.to_string(), "dport", &format!("= {SERVICE_PORT}")])
        .status()
        .expect("run ss -K on the leg-C tuple");
    assert!(destroy.success(), "ss -K destroys the leg-C listening tuple");

    let dialed = std::net::TcpStream::connect_timeout(
        &SocketAddrV4::new(addr, SERVICE_PORT).into(),
        Duration::from_secs(2),
    );
    assert!(
        dialed.is_err(),
        "an inbound SYN to the registered destination with leg C closed is dropped, not accepted"
    );

    stop_and_await_terminal(&cfg, &output.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-64 — the TIME_WAIT side door, both controls first
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-64 — The TIME_WAIT side door is measured with both controls first
/// CONTRACT_SHAPE: bounded-change.
///
/// The controls run first on a host-only path a test-owned veth peer namespace
/// under `TestCidrLease` provides — the TPROXY program does not handle it, so
/// the host holds a true `TIME_WAIT` entry. The negative control (stale ISN,
/// no timestamp) gets a bare ACK and no SYN-ACK, proving the substate and
/// sequence gates; the positive control (after `tcp_invalid_ratelimit`, a
/// newer sequence) reopens with a SYN-ACK. Only then is the guest case
/// recorded. **A reproduced SYN-ACK is not absorbed: it routes to the user.**
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-64)"]
async fn a_newer_sequence_reconnect_into_time_wait_is_recorded_after_both_controls() {
    let lease = TestCidrLease::acquire("nd295-64-time-wait")
        .expect("acquire a named CIDR for the TIME_WAIT controls");
    let peer_ns = format!("nd295-64-{}", std::process::id());
    let topology = TimeWaitTopology::provision(&lease, &peer_ns);

    // Establish one connection host-listener <- peer, then close the host's
    // accepted socket first and the peer second, so the host side holds the
    // true TIME_WAIT substate for the peer 4-tuple.
    let tuple = topology.establish_and_leave_host_in_time_wait();

    // Negative control (first): a SYN from the same 4-tuple with a stale ISN
    // and no timestamp gets a bare ACK (TCP_TW_ACK) and no SYN-ACK; the entry
    // survives.
    let negative = topology.probe_reconnect(tuple, ReconnectSeq::StaleIsn);
    assert_eq!(
        negative, ReconnectReply::BareAck,
        "the stale-ISN probe gets a bare ACK and no SYN-ACK; the substate and sequence gates hold"
    );

    // Positive control (second): after tcp_invalid_ratelimit, a newer-sequence
    // SYN meets the sequence precondition and reopens with a SYN-ACK.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let positive = topology.probe_reconnect(tuple, ReconnectSeq::NewerSeq);
    assert_eq!(
        positive, ReconnectReply::SynAck,
        "the newer-sequence probe reopens the TIME_WAIT entry with a SYN-ACK"
    );

    // Guest case: record whether a newer-sequence reconnect into the guest's
    // own leg-F TIME_WAIT entry is answered. A reproduced SYN-ACK is not
    // absorbed — it is surfaced to the user as the recorded outcome.
    let guest_outcome = topology.probe_reconnect(tuple, ReconnectSeq::NewerSeq);
    eprintln!(
        "S-ND295-64 guest TIME_WAIT reconnect outcome (recorded, not absorbed): {guest_outcome:?}"
    );

    drop(topology);
    drop(lease);
}

/// The reply a reconnect probe observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectReply {
    /// A bare ACK (`TCP_TW_ACK`) and no SYN-ACK — the entry survives.
    BareAck,
    /// A SYN-ACK — the entry was reopened/consumed.
    SynAck,
    /// No reply within the probe window.
    None,
}

/// The sequence shape a reconnect probe carries.
#[derive(Debug, Clone, Copy)]
enum ReconnectSeq {
    /// Stale ISN below the old `rcv_nxt`, no timestamp option.
    StaleIsn,
    /// A sequence above the old `rcv_nxt`.
    NewerSeq,
}

/// The fixed source port the peer 4-tuple uses across the establish + probes.
const PEER_SRC_PORT: u16 = 51_000;

/// A raw-TCP SYN crafter host binary: `argv = [src_ip, src_port, dst_ip,
/// dst_port, seq, with_ts]`, sends exactly one crafted SYN through
/// `AF_INET`/`SOCK_RAW`/`IP_HDRINCL`, computing the IP and TCP checksums. Run
/// in the peer namespace via `ip netns exec`, it lets the controls choose the
/// sequence number (stale vs newer) and the timestamp option a normal stack
/// cannot.
fn build_raw_syn_crafter(tmp: &Path) -> PathBuf {
    let source = r#"
use std::net::Ipv4Addr;
fn csum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < bytes.len() { sum += u32::from(u16::from_be_bytes([bytes[i], bytes[i+1]])); i += 2; }
    if i < bytes.len() { sum += u32::from(u16::from_be_bytes([bytes[i], 0])); }
    while sum >> 16 != 0 { sum = (sum & 0xffff) + (sum >> 16); }
    !(sum as u16)
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src: Ipv4Addr = a[1].parse().unwrap();
    let sport: u16 = a[2].parse().unwrap();
    let dst: Ipv4Addr = a[3].parse().unwrap();
    let dport: u16 = a[4].parse().unwrap();
    let seq: u32 = a[5].parse().unwrap();
    let with_ts: bool = a[6] == "1";
    let tcp_len = if with_ts { 32 } else { 20 };
    // TCP header (+ optional timestamp option).
    let mut tcp = vec![0u8; tcp_len];
    tcp[0..2].copy_from_slice(&sport.to_be_bytes());
    tcp[2..4].copy_from_slice(&dport.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    tcp[12] = ((tcp_len / 4) as u8) << 4; // data offset
    tcp[13] = 0x02; // SYN
    tcp[14..16].copy_from_slice(&64240u16.to_be_bytes()); // window
    if with_ts {
        tcp[20] = 8; tcp[21] = 10; // TS option kind=8 len=10
        tcp[22..26].copy_from_slice(&1u32.to_be_bytes());
        tcp[26] = 1; tcp[27] = 1; tcp[28] = 0; tcp[29] = 0; // nops/end pad
    }
    // TCP checksum over the pseudo-header + TCP.
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&src.octets());
    pseudo.extend_from_slice(&dst.octets());
    pseudo.push(0); pseudo.push(6);
    pseudo.extend_from_slice(&(tcp_len as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let c = csum(&pseudo);
    tcp[16..18].copy_from_slice(&c.to_be_bytes());
    // IP header.
    let total = 20 + tcp_len;
    let mut ip = vec![0u8; 20];
    ip[0] = 0x45; ip[3] = 0; ip[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    ip[8] = 64; ip[9] = 6;
    ip[12..16].copy_from_slice(&src.octets());
    ip[16..20].copy_from_slice(&dst.octets());
    let ic = csum(&ip);
    ip[10..12].copy_from_slice(&ic.to_be_bytes());
    let mut packet = ip; packet.extend_from_slice(&tcp);
    // SAFETY: a raw IPv4 TCP socket with IP_HDRINCL; single sendto.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_TCP);
        assert!(fd >= 0, "raw socket");
        let one: libc::c_int = 1;
        libc::setsockopt(fd, libc::IPPROTO_IP, libc::IP_HDRINCL,
            std::ptr::from_ref(&one).cast(), std::mem::size_of::<libc::c_int>() as u32);
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as u16;
        addr.sin_port = dport.to_be();
        addr.sin_addr.s_addr = u32::from_ne_bytes(dst.octets());
        let sent = libc::sendto(fd, packet.as_ptr().cast(), packet.len(), 0,
            std::ptr::from_ref(&addr).cast(), std::mem::size_of::<libc::sockaddr_in>() as u32);
        assert!(sent >= 0, "sendto");
        libc::close(fd);
    }
}
"#;
    build_static_binary(tmp, "nd295-raw-syn", source)
}

/// Capture the TCP flags of the first frame `from_port → to_port` on `iface`
/// within `window`; `None` when nothing replied. Used to classify the
/// TIME_WAIT reply (SYN-ACK vs bare ACK vs none).
fn capture_first_tcp_flags(iface: &str, from_port: u16, to_port: u16, window: Duration) -> Option<u8> {
    const ETH_P_ALL: u16 = 0x0003;
    let name = std::ffi::CString::new(iface).expect("iface has no NUL");
    // SAFETY: live NUL-terminated string.
    let ifindex = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(ifindex, 0, "interface {iface} is live");
    // SAFETY: AF_PACKET raw socket owned locally; closed before return.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            i32::from(ETH_P_ALL.to_be()),
        )
    };
    assert!(fd >= 0, "open reply capture: {}", std::io::Error::last_os_error());
    // SAFETY: zeroed sockaddr_ll before its fields are set.
    let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
    address.sll_protocol = ETH_P_ALL.to_be();
    address.sll_ifindex = i32::try_from(ifindex).expect("ifindex fits i32");
    // SAFETY: fully-initialised sockaddr_ll.
    let bound = unsafe {
        libc::bind(fd, std::ptr::from_ref(&address).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>()).unwrap())
    };
    assert_eq!(bound, 0, "bind reply capture: {}", std::io::Error::last_os_error());
    let deadline = Instant::now() + window;
    let mut result = None;
    while Instant::now() < deadline {
        let mut frame = [0_u8; 2048];
        // SAFETY: live buffer; owned fd.
        let read = unsafe {
            libc::recv(fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
        };
        if read <= 0 {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let length = usize::try_from(read).unwrap();
        if let Some(flags) = tcp_flags_between(&frame[..length], from_port, to_port) {
            result = Some(flags);
            break;
        }
    }
    // SAFETY: owned fd.
    unsafe { libc::close(fd) };
    result
}

/// The TCP flags byte of an Ethernet/IPv4/TCP frame whose source port is
/// `from_port` and destination port `to_port`, else `None`.
fn tcp_flags_between(frame: &[u8], from_port: u16, to_port: u16) -> Option<u8> {
    if frame.len() < 14 + 20 + 20 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    let ihl = usize::from(frame[14] & 0x0f) * 4;
    if frame[23] != 0x06 || frame.len() < 14 + ihl + 20 {
        return None;
    }
    let tcp = &frame[14 + ihl..];
    let src = u16::from_be_bytes([tcp[0], tcp[1]]);
    let dst = u16::from_be_bytes([tcp[2], tcp[3]]);
    (src == from_port && dst == to_port).then_some(tcp[13])
}

/// A test-owned veth peer namespace + a host listener, on a `TestCidrLease`
/// CIDR the TPROXY program never handles, for the TIME_WAIT controls.
struct TimeWaitTopology {
    peer_ns: String,
    host_gateway: Ipv4Addr,
    workload_addr: Ipv4Addr,
    host_port: u16,
    crafter: PathBuf,
    _tmp: tempfile::TempDir,
    _host_listener: WildcardListener,
}

impl TimeWaitTopology {
    fn provision(lease: &TestCidrLease, peer_ns: &str) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("nd295-64-crafter-")
            .tempdir_in(shared_staging_root())
            .expect("crafter tempdir");
        let crafter = build_raw_syn_crafter(tmp.path());
        // Build the veth pair + peer namespace + host route on the leased CIDR.
        run(["ip", "netns", "add", peer_ns]);
        let host_if = format!("{peer_ns}-h");
        let peer_if = format!("{peer_ns}-p");
        run(["ip", "link", "add", &host_if, "type", "veth", "peer", "name", &peer_if]);
        run(["ip", "link", "set", &peer_if, "netns", peer_ns]);
        run(["ip", "addr", "add", &format!("{}/24", lease.host_gateway()), "dev", &host_if]);
        run(["ip", "link", "set", &host_if, "up"]);
        run(["ip", "-n", peer_ns, "addr", "add", &format!("{}/24", lease.workload_addr()), "dev", &peer_if]);
        run(["ip", "-n", peer_ns, "link", "set", &peer_if, "up"]);
        run(["ip", "-n", peer_ns, "link", "set", "lo", "up"]);
        Self {
            peer_ns: peer_ns.to_owned(),
            host_gateway: lease.host_gateway(),
            workload_addr: lease.workload_addr(),
            host_port: HOST_WILDCARD_PORT + 1,
            crafter,
            _tmp: tmp,
            _host_listener: WildcardListener::bind(HOST_WILDCARD_PORT + 1),
        }
    }

    /// Establish one connection from the peer namespace to the host listener,
    /// close the host's accepted socket first and the peer second, so the host
    /// holds the true `TIME_WAIT` substate. Returns the peer 4-tuple.
    fn establish_and_leave_host_in_time_wait(&self) -> (SocketAddrV4, SocketAddrV4) {
        let peer_src = SocketAddrV4::new(self.workload_addr, PEER_SRC_PORT);
        let host_dst = SocketAddrV4::new(self.host_gateway, self.host_port);
        // The peer client binds the fixed source port and dials the host
        // listener; the WildcardListener closes its accepted socket first, then
        // this client closes (peer close second), leaving the host in TIME_WAIT.
        let status = Command::new("ip")
            .args([
                "netns", "exec", &self.peer_ns, "bash", "-c",
                &format!(
                    "exec 3<>/dev/tcp/{}/{}; printf x >&3; sleep 0.3; exec 3>&-",
                    self.host_gateway, self.host_port
                ),
            ])
            .status()
            .expect("peer-namespace client dials the host listener");
        assert!(status.success(), "the peer namespace establishes and closes the connection");
        // Let the host's accepted socket close first so the host holds TIME_WAIT.
        std::thread::sleep(Duration::from_millis(500));
        (peer_src, host_dst)
    }

    /// Send one crafted reconnect SYN from the peer 4-tuple with the given
    /// sequence shape, and classify the host reply captured on the host veth.
    fn probe_reconnect(
        &self,
        tuple: (SocketAddrV4, SocketAddrV4),
        seq: ReconnectSeq,
    ) -> ReconnectReply {
        let (src, dst) = tuple;
        let (seq_num, with_ts) = match seq {
            // Stale ISN below the old rcv_nxt, no timestamp option.
            ReconnectSeq::StaleIsn => ("1000", "0"),
            // A sequence above the old rcv_nxt, with a fresh timestamp.
            ReconnectSeq::NewerSeq => ("4000000000", "1"),
        };
        // Capture the host reply (host_port → peer_src_port) concurrently.
        let iface = format!("{}-h", self.peer_ns);
        let from = dst.port();
        let to = src.port();
        let capture = std::thread::spawn(move || {
            capture_first_tcp_flags(&iface, from, to, Duration::from_millis(600))
        });
        std::thread::sleep(Duration::from_millis(50));
        let sent = Command::new("ip")
            .args([
                "netns", "exec", &self.peer_ns,
                &self.crafter.to_string_lossy(),
                &src.ip().to_string(), &src.port().to_string(),
                &dst.ip().to_string(), &dst.port().to_string(),
                seq_num, with_ts,
            ])
            .status()
            .expect("run the raw SYN crafter in the peer namespace");
        assert!(sent.success(), "the crafter sends one raw SYN");
        match capture.join().expect("reply capture thread") {
            Some(flags) if flags & 0x12 == 0x12 => ReconnectReply::SynAck,
            Some(flags) if flags & 0x10 != 0 && flags & 0x02 == 0 => ReconnectReply::BareAck,
            _ => ReconnectReply::None,
        }
    }
}

impl Drop for TimeWaitTopology {
    fn drop(&mut self) {
        let host_if = format!("{}-h", self.peer_ns);
        let _ = Command::new("ip").args(["link", "del", &host_if]).status();
        let _ = Command::new("ip").args(["netns", "del", &self.peer_ns]).status();
    }
}

/// Run a host command, asserting success.
fn run<const N: usize>(args: [&str; N]) {
    let status = Command::new(args[0])
        .args(&args[1..])
        .status()
        .unwrap_or_else(|error| panic!("spawn {args:?}: {error}"));
    assert!(status.success(), "command {args:?} succeeded");
}
