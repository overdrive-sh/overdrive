//! netns-density-295 E14 — intercept-marked guest TCP fails closed
//! (S-ND295-62/63/64; D-295-R18; D-295-R19 ordering withdrawn 2026-10-03).
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
//! Step 08-01 activates the native R18 program-loss and guard-only bodies plus
//! the retained-order listener and TIME_WAIT cases. R18 remains required after
//! its reproduced IP-program-loss exposure; the conditional R19 rule reorder
//! was withdrawn on native evidence, and no body assumes or installs it.
//!
//! The R18 hazard body reads no guard-table presence: its healthy baseline
//! asserts only that each probe SYN receives a SYN-ACK at the guest's TAP and
//! that the wildcard host listener accepts nothing, then deletes
//! `table ip overdrive-mtls` and asserts E14 (a)/(b). At 08-01 it runs once
//! before the guard exists (the RED that decides R18) and again after (GREEN).
//! The guard's own non-interference control — `observe_intercept_mark_guard()`
//! reading back `Ok(true)` with the same SYN-ACKs — lives in the guard-only
//! body, which is removed at 08-01 if R18 is withdrawn.
//!
//! The dialer is plaintext by design (CLAUDE.md § "East-west mTLS tests").

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

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::VmRunDir;
use overdrive_dataplane::guest_tcx::{GuestTcxCounter, read_counter};
use overdrive_netlink::nft::bridge::{BridgeGuardSpec, observe as observe_bridge_guard};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing::vm_fixture::VmFixture;

use super::guest_stack_mtls_egress::{
    MESH_NAME, SERVICE_PORT, WireCapture, build_mesh_peer, build_static_binary, interface_index,
    observe_shared_intercept_state, service_toml,
};
use super::serve_lifetime_support::kill_serve_owner;
use super::vm_walking_skeleton::{
    TeardownBound, config_path, guard_default_drop_packets, poll_until_running,
    poll_until_terminal, shared_staging_root, spawn_vm_server_mtls_composed,
    stage_rootfs_with_extra_binaries, stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
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
/// The base sequence number the TIME_WAIT guest crafts its reconnect SYNs at —
/// retained lower bound for the authored in-run witness. The actual base is
/// selected above the real FIN-derived `rcv_nxt` under TCP's modulo comparison.
const TW_CRAFT_SEQ_BASE: u32 = 0x5000_0000;
/// The base TSval the TIME_WAIT guest stamps its crafted reconnect SYNs with —
/// retained lower bound; the actual TSval is selected newer than the real FIN.
const TW_CRAFT_TSVAL_BASE: u32 = 0x0100_0000;
/// Test-private host-to-guest event gate, used only after the guest has
/// completed its close and the actual serve owner has entered killed mode.
const TW_CONTROL_PORT: u16 = 41_064;
/// The cgroup slice every VM allocation's scope lives under; the killed-server
/// residue guard reaps the scopes that appeared under it.
const WORKLOADS_SLICE: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";
/// The bpffs directory the production TCX loader pins its per-endpoint links
/// under; the residue guard removes the pins that appeared in it.
const TCX_LINK_PIN_DIR: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/links";

// ---------------------------------------------------------------------------
// Probe guest and mesh service
// ---------------------------------------------------------------------------

/// E14 prescribes scripted SYNs, not a completed application connection.
/// Completing a TcpStream handshake would make the healthy intercept dial
/// the wildcard backend and invalidate the healthy zero-accept control.
const SYN_PROBE_RAW_SOURCE: &str = r#"
extern "C" {
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
    fn close(fd: i32) -> i32;
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0_u32;
    for pair in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
    }
    while sum >> 16 != 0 { sum = (sum & 0xffff) + (sum >> 16); }
    !(sum as u16)
}

fn scripted_syns(targets: &[std::net::SocketAddrV4], window: u64) {
    let route = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    // A connect selects the source address without sending a UDP packet.
    route.connect(targets[0]).unwrap();
    let source = match route.local_addr().unwrap() {
        std::net::SocketAddr::V4(v4) => *v4.ip(),
        _ => std::process::exit(31),
    };
    let fd = unsafe { socket(2, 3, 255) }; // AF_INET/SOCK_RAW/IPPROTO_RAW, IP_HDRINCL
    if fd < 0 { std::process::exit(21); }
    let deadline = Instant::now() + Duration::from_secs(window);
    let mut n = 0_u32;
    while Instant::now() < deadline {
        for (index, target) in targets.iter().enumerate() {
            let mut tcp = [0_u8; 20];
            // Each ordinary probe is a fresh TCP 4-tuple, as with the original
            // connect attempts. Only E14(e) deliberately reuses a source port.
            let source_port = 41_000_u16 + (n as u16 * targets.len() as u16) + index as u16;
            tcp[0..2].copy_from_slice(&source_port.to_be_bytes());
            tcp[2..4].copy_from_slice(&target.port().to_be_bytes());
            tcp[4..8].copy_from_slice(&0x295a_0000_u32.wrapping_add(n * 4_000).to_be_bytes());
            tcp[12] = 0x50;
            tcp[13] = 0x02;
            tcp[14..16].copy_from_slice(&64_240_u16.to_be_bytes());
            let mut pseudo = Vec::new();
            pseudo.extend_from_slice(&source.octets());
            pseudo.extend_from_slice(&target.ip().octets());
            pseudo.extend_from_slice(&[0, 6, 0, 20]);
            pseudo.extend_from_slice(&tcp);
            tcp[16..18].copy_from_slice(&checksum(&pseudo).to_be_bytes());
            let mut packet = [0_u8; 40];
            packet[0] = 0x45;
            packet[2..4].copy_from_slice(&40_u16.to_be_bytes());
            packet[8] = 64;
            packet[9] = 6;
            packet[12..16].copy_from_slice(&source.octets());
            packet[16..20].copy_from_slice(&target.ip().octets());
            let check = checksum(&packet[..20]);
            packet[10..12].copy_from_slice(&check.to_be_bytes());
            packet[20..].copy_from_slice(&tcp);
            let mut address = [0_u8; 16];
            address[0] = 2;
            address[2..4].copy_from_slice(&target.port().to_be_bytes());
            address[4..8].copy_from_slice(&target.ip().octets());
            let sent = unsafe { sendto(fd, packet.as_ptr(), packet.len(), 0, address.as_ptr(), 16) };
            if sent != packet.len() as isize { unsafe { close(fd); } std::process::exit(22); }
        }
        n += 1;
        // The original full probe window is unchanged. This workload cadence
        // lets both named SYNs occur during the owner's actual live exposure;
        // it neither postpones nor changes the owner's quiescence.
        std::thread::sleep(Duration::from_millis(10));
    }
    unsafe { close(fd); }
}
"#;

/// Build the probe guest. Its active R18/R19 modes send scripted TCP SYNs
/// from fresh source ports across the whole window to the exact case's
/// peer/gateway/host-interface or gateway/external destinations. `host` is
/// the real host interface address the R18 (b)
/// "another host interface address" case dials; it is read from the kernel at
/// test time (never a literal).
fn build_syn_probe_guest(tmp: &Path, host: Ipv4Addr) -> PathBuf {
    let source = format!(
        r#"
use std::net::TcpStream;
use std::time::{{Duration, Instant}};

{raw_probe}

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
    // E14(a) names the actual peer allocation address, not the Service VIP
    // returned by its mesh name. The host supplies that observed address.
    let peer = std::env::args().nth(1).unwrap_or_else(|| "{mesh}:{svc}".to_owned());
    let r18 = std::env::args().nth(2).as_deref() == Some("r18");
    let r19 = std::env::args().nth(1).as_deref() == Some("r19");
    if r18 || r19 {{
        let targets = if r18 {{
            vec![peer, "{gw}:{port}".to_owned(), "{host}:{port}".to_owned()]
        }} else {{
            vec!["{gw}:{port}".to_owned(), "{ext}:{port}".to_owned()]
        }};
        let targets = targets.iter().map(|target| target.parse().unwrap()).collect::<Vec<_>>();
        scripted_syns(&targets, {window});
        return;
    }}
    let deadline = Instant::now() + Duration::from_secs({window});
    while Instant::now() < deadline {{
        dial(&peer);
        dial("{gw}:{port}");
        dial("{host}:{port}");
        // E14(a)/(b)'s control universe is peer, gateway, and host interface.
        // The external address belongs to the independent R19 cases.
        if !r18 {{ dial("{ext}:{port}"); }}
        std::thread::sleep(Duration::from_millis(200));
    }}
}}
"#,
        window = PROBE_WINDOW.as_secs() + 10,
        mesh = MESH_NAME,
        svc = SERVICE_PORT,
        gw = GATEWAY,
        host = host,
        ext = EXTERNAL,
        port = HOST_WILDCARD_PORT,
        raw_probe = SYN_PROBE_RAW_SOURCE,
    );
    build_static_binary(tmp, "nd295-syn-probe", &source)
}

/// One deployed VM allocation and the guest identities the host observes.
struct DeployedGuest {
    workload_id: String,
    addr: Ipv4Addr,
    tap: String,
}

/// The production TAP name for a guest address (`ovd-tp-<4hex>`).
fn tap_for(addr: Ipv4Addr) -> String {
    let o = addr.octets();
    format!("ovd-tp-{:04x}", u16::from_be_bytes([o[2], o[3]]))
}

async fn deploy_probe_guest_with_args(
    cfg: &Path,
    dir: &Path,
    kernel: &Path,
    rootfs: &Path,
    args: &[String],
) -> DeployedGuest {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    let spec = write_toml(
        dir,
        "nd295-probe.toml",
        &vm_job_toml("nd295-probe", "/sbin/nd295-syn-probe", &args, kernel, rootfs),
    );
    let output = deploy(DeployArgs { spec, config_path: cfg.to_path_buf() })
        .await
        .expect("deploy the SYN-probe guest through the production handler");
    let running = poll_until_running(cfg, &output.workload_id, RUNNING_BOUND).await;
    let row = running.snapshot.rows.first().expect("one Running allocation row");
    let addr = row.workload_addr.expect("a Running VM publishes its guest address");
    DeployedGuest { workload_id: output.workload_id, addr, tap: tap_for(addr) }
}

/// A real host interface IPv4 address distinct from the bridge gateway — the
/// R18 (b) "another host interface address" case's destination, read from the
/// kernel's own routing table (never a literal, and never TEST-NET-1, which is
/// not a host address). Uses the source address the kernel would select to
/// reach a routable destination: that address is bound to a real host
/// interface. Fails loudly if the host has no such route.
fn host_interface_address() -> Ipv4Addr {
    let output = Command::new("ip")
        .args(["-j", "route", "get", "1.1.1.1"])
        .output()
        .expect("run `ip route get` to read a real host interface address");
    assert!(
        output.status.success(),
        "ip route get 1.1.1.1: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let key = "\"prefsrc\":\"";
    let start = text.find(key).expect("ip route get names a prefsrc host address") + key.len();
    let addr = text[start..]
        .split('"')
        .next()
        .expect("prefsrc value is quoted")
        .parse::<Ipv4Addr>()
        .expect("prefsrc is a valid IPv4 host address");
    assert!(addr != GATEWAY, "the R18 (b) host address {addr} must differ from the gateway");
    addr
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
/// The thread records the last non-`WouldBlock` accept error rather than
/// exiting on it, so "accepts nothing" is never vacuously true because the
/// accept loop had silently died: [`WildcardListener::assert_healthy`] proves
/// the loop was still running (alive, no recorded error) when the oracle read
/// the accept count.
struct WildcardListener {
    accepts: Arc<AtomicU64>,
    /// The last non-`WouldBlock` accept-loop error, as an `errno`, or `0` when
    /// the loop has hit none. Read by [`WildcardListener::assert_healthy`].
    accept_error: Arc<AtomicU64>,
    /// Set by the accept loop each time it observes it is still running, so a
    /// health read can prove the thread reached the check after the oracle.
    alive_ticks: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl WildcardListener {
    fn bind(port: u16) -> Self {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
            .expect("bind wildcard host listener");
        listener.set_nonblocking(true).expect("nonblocking wildcard listener");
        let accepts = Arc::new(AtomicU64::new(0));
        let accept_error = Arc::new(AtomicU64::new(0));
        let alive_ticks = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let join = {
            let accepts = Arc::clone(&accepts);
            let accept_error = Arc::clone(&accept_error);
            let alive_ticks = Arc::clone(&alive_ticks);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    alive_ticks.fetch_add(1, Ordering::SeqCst);
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // The accept itself is the evidence the oracle
                            // reads. The stream then consumes the peer's first
                            // byte (bounded wait) before it closes, so the
                            // close is an orderly FIN, not a reset for unread
                            // data: the TIME_WAIT controls read that close
                            // sequence off the wire. The read's outcome is not
                            // evidence for any oracle.
                            accepts.fetch_add(1, Ordering::SeqCst);
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .expect("bound the accepted stream's first read");
                            let mut byte = [0_u8; 1];
                            match stream.read(&mut byte) {
                                Ok(_) => {}
                                Err(ref e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                            | std::io::ErrorKind::ConnectionReset
                                    ) => {}
                                Err(e) => panic!("wildcard listener: first read failed: {e}"),
                            }
                            drop(stream);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(20));
                        }
                        Err(ref e) => {
                            // Record the error and keep looping; a dead accept
                            // loop must not make "accepts nothing" vacuous.
                            let errno = e.raw_os_error().unwrap_or(-1).unsigned_abs();
                            accept_error.store(u64::from(errno), Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    }
                }
            })
        };
        Self { accepts, accept_error, alive_ticks, stop, join: Some(join) }
    }

    fn accepted(&self) -> u64 {
        self.accepts.load(Ordering::SeqCst)
    }

    /// Assert the accept loop is still healthy at the moment the oracle reads
    /// the accept count: it has recorded no error, and it advances past a tick
    /// it had reached before this call — so the loop is provably alive, not a
    /// thread that died and left a frozen count. Makes "accepts nothing" a real
    /// oracle rather than a vacuous one.
    fn assert_healthy(&self) {
        assert_eq!(
            self.accept_error.load(Ordering::SeqCst),
            0,
            "the wildcard listener's accept loop recorded an error; 'accepts nothing' is not vacuous"
        );
        let before = self.alive_ticks.load(Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.alive_ticks.load(Ordering::SeqCst) <= before {
            assert!(
                Instant::now() < deadline,
                "the wildcard listener's accept loop is not advancing; the thread is dead"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
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

/// Use the kernel packet timestamps' CLOCK_REALTIME domain for cuts around
/// externally applied faults. A dequeue time must never relabel a healthy
/// pre-fault packet as a fault-period response.
fn packet_clock_now() -> i128 {
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    // SAFETY: `now` is writable timespec storage for the duration of the call.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut now) };
    assert_eq!(rc, 0, "read the packet timestamp clock: {}", std::io::Error::last_os_error());
    i128::from(now.tv_sec) * 1_000_000_000 + i128::from(now.tv_nsec)
}

/// Confirm the same production TAP's state without raising it. Two successive
/// up samples bound packet-event timestamps conservatively. The actual owner
/// quiesces this TAP once after listener loss and cannot restore it while the
/// exact-port thief holds the failed leg, so a down sample ends the exposure;
/// observation nevertheless continues for the original complete probe window.
fn tap_state_sample(tap: &str, ifindex: u32) -> (i128, bool) {
    assert_eq!(interface_index(tap), ifindex, "the capture still names the same production TAP");
    let up = tap_is_up(tap);
    (packet_clock_now(), up)
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
        // SAFETY: AF_PACKET raw socket owned by this SynCapture. Created with
        // protocol 0 so it receives nothing until the `bind` below sets
        // `ETH_P_ALL` on the target ifindex — no frame from another interface
        // is queued in the pre-bind window.
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
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

    /// Drain every queued frame and return `true` if any is an IPv4 TCP SYN-ACK
    /// from `source` at `source_port` — the handshake reply from one specific
    /// destination the guest dialed. For more than one destination on the same
    /// capture use [`SynCapture::syn_acks_seen`], which classifies every
    /// destination in one drain.
    fn syn_ack_from(&self, source: Ipv4Addr, source_port: u16) -> bool {
        !self.syn_acks_seen(&[(source, source_port)]).is_empty()
    }

    /// Drain every queued frame once and return the members of `wanted`
    /// (IPv4 source address, TCP source port) that sent at least one IPv4 TCP
    /// SYN-ACK. One drain classifies every wanted destination, so a reply from
    /// one destination is never consumed and discarded while the capture is
    /// scanned for another (healthy baseline: every R18 destination answers;
    /// fault: none does).
    fn syn_acks_seen(&self, wanted: &[(Ipv4Addr, u16)]) -> BTreeSet<(Ipv4Addr, u16)> {
        let mut seen = BTreeSet::new();
        loop {
            let mut frame = [0_u8; 2048];
            // SAFETY: `frame` is a live writable buffer; `self.fd` is owned.
            let read = unsafe {
                libc::recv(self.fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                let slice = &frame[..length];
                for &(source, source_port) in wanted {
                    if is_syn_ack_from_port(slice, source_port)
                        && frame_ipv4_src(slice) == Some(source)
                    {
                        seen.insert((source, source_port));
                    }
                }
                continue;
            }
            if read == 0 {
                return seen;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return seen;
            }
            panic!("SYN capture recv failed: {error}");
        }
    }

    /// The kernel's `PACKET_STATISTICS.tp_drops` for this capture — frames the
    /// kernel could not enqueue (socket buffer overflow) since the socket was
    /// opened. A zero-match oracle (e.g. "zero forwarded frames") is vacuous if
    /// the kernel silently dropped the frames before the drain read them, so the
    /// caller reads this exactly once after its final drain and asserts it is 0.
    /// Reading `PACKET_STATISTICS` resets the kernel counter, so call it once.
    fn packet_drops(&self) -> u64 {
        // Declared locally rather than reaching for `libc::tpacket_stats`, whose
        // re-export module differs across libc versions; the layout is stable
        // kernel UAPI (two `__u32`s, packets then drops).
        #[repr(C)]
        #[allow(dead_code)]
        struct TpacketStats {
            tp_packets: libc::c_uint,
            tp_drops: libc::c_uint,
        }
        const PACKET_STATISTICS: libc::c_int = 6;
        let mut stats = TpacketStats { tp_packets: 0, tp_drops: 0 };
        let mut len = libc::socklen_t::try_from(std::mem::size_of::<TpacketStats>())
            .expect("TpacketStats length fits socklen_t");
        // SAFETY: getsockopt into a live, correctly sized TpacketStats on the
        // AF_PACKET fd this SynCapture owns.
        let rc = unsafe {
            libc::getsockopt(
                self.fd,
                libc::SOL_PACKET,
                PACKET_STATISTICS,
                std::ptr::from_mut(&mut stats).cast(),
                std::ptr::from_mut(&mut len),
            )
        };
        assert_eq!(rc, 0, "read PACKET_STATISTICS: {}", std::io::Error::last_os_error());
        u64::from(stats.tp_drops)
    }
}

impl Drop for SynCapture {
    fn drop(&mut self) {
        // SAFETY: SynCapture exclusively owns this socket fd.
        unsafe { libc::close(self.fd) };
    }
}

/// The IPv4 source address of an Ethernet/IPv4 frame, else `None`.
fn frame_ipv4_src(frame: &[u8]) -> Option<Ipv4Addr> {
    if frame.len() < 14 + 20 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    Some(Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]))
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

/// `true` iff `frame` is an Ethernet/IPv4/TCP SYN-ACK (SYN set, ACK set) whose
/// TCP SOURCE port equals `src_port` — a reopen reply from the original
/// destination (S-ND295-64).
fn is_syn_ack_from_port(frame: &[u8], src_port: u16) -> bool {
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
    let port = u16::from_be_bytes([tcp[0], tcp[1]]);
    let flags = tcp[13];
    port == src_port && (flags & 0x02) != 0 && (flags & 0x10) != 0
}

/// Exact request nonce for a guest SYN: destination, guest source port, and
/// sequence. The source port is fresh for every ordinary E14 probe.
fn probe_request(frame: &[u8], guest: Ipv4Addr, port: u16) -> Option<(Ipv4Addr, u16, u32)> {
    if !is_syn_to_port(frame, port) || frame_ipv4_src(frame) != Some(guest) {
        return None;
    }
    let tcp = &frame[14 + usize::from(frame[14] & 0x0f) * 4..];
    Some((
        Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]),
        u16::from_be_bytes([tcp[0], tcp[1]]),
        u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]),
    ))
}

/// Reject a delayed healthy reply as fault evidence. A fault-period SYN-ACK
/// must acknowledge one exact guest SYN actually observed after the fault.
fn probe_reply(
    frame: &[u8],
    port: u16,
    requests: &BTreeSet<(Ipv4Addr, u16, u32)>,
) -> Option<(Ipv4Addr, u16)> {
    if !is_syn_ack_from_port(frame, port) {
        return None;
    }
    let source = frame_ipv4_src(frame)?;
    let tcp = &frame[14 + usize::from(frame[14] & 0x0f) * 4..];
    let guest_port = u16::from_be_bytes([tcp[2], tcp[3]]);
    let ack = u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]]);
    requests.contains(&(source, guest_port, ack.wrapping_sub(1))).then_some((source, port))
}

/// The production shared **bridge** guard's default-drop counter (packets),
/// read through the netlink observe surface. The R18 `ip overdrive-mtls-guard`
/// table carries no counter (FD § "R18-B contract"), so the in-run
/// non-interference oracle reads the D9 bridge guard's `DefaultDrop` counter
/// instead (`vm_walking_skeleton::guard_default_drop_packets`). Fails loudly on
/// any observe/read error — never silently `0`, which would make the
/// "unchanged" assertion vacuous.
fn bridge_guard_drop_packets() -> u64 {
    let spec = BridgeGuardSpec::new(
        "overdrive-mtls".to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical production bridge guard specification");
    let observation = observe_bridge_guard(&spec, &BTreeSet::new())
        .expect("observe the production shared bridge guard through netlink");
    guard_default_drop_packets(&observation)
}

/// The node-wide TCX `Intercept` counter. Fails loudly on a read error naming
/// the map pin precondition — never silently `0`, which would make the "the
/// counter rose" witness vacuous.
fn intercept_counter() -> u64 {
    read_counter(PathBuf::from(COUNTER_MAP_PIN), GuestTcxCounter::Intercept)
        .expect("read the node-wide TCX Intercept counter from its production map pin")
}

/// Destroy the exact listening socket bound to `ip:port` via a sock-diag
/// destroy (`ss -K -l`), and assert it found and killed a socket. `ss -K`
/// prints the sockets it destroyed, so a killed socket's tuple appears in
/// stdout; an empty match prints only the header. `ss -K` (unlike a bare
/// `ss -K` on established sockets) needs `-l` to reach listening sockets. Fails
/// on any run error and on a match that killed nothing.
fn destroy_listening_tuple(ip: Ipv4Addr, port: u16) {
    let filter = format!("src {ip} and sport = :{port}");
    let output = Command::new("ss")
        .args(["-K", "-l", "-n", &filter])
        .output()
        .expect("run ss -K -l to destroy the exact listening tuple");
    assert!(
        output.status.success(),
        "ss -K -l {filter}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(&format!(":{port}")),
        "ss -K -l found and killed the {ip}:{port} listening socket (killed set: {text:?})"
    );
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

/// Positive control on the same production TAP and exact destinations before
/// the fault. Scripted SYNs receive SYN-ACKs without completing a handshake.
async fn assert_healthy_probe_synacks(tap: &str, targets: &[(Ipv4Addr, u16)]) {
    let capture = SynCapture::open(tap, HOST_WILDCARD_PORT);
    let mut answered = BTreeSet::new();
    assert!(
        wait_until(Duration::from_secs(8), || {
            answered.extend(capture.syn_acks_seen(targets));
            answered.len() == targets.len()
        })
        .await,
        "healthy positive control: every exact R19 destination answers its SYN; {answered:?} of {targets:?}"
    );
    assert!(tap_is_up(tap), "the positive control runs on the live production TAP");
    assert_eq!(capture.packet_drops(), 0, "the positive-control capture drops no packets");
}

/// R19's stated path is rule 1 -> rule 2 with the owned program, source
/// membership, and fwmark/local route still present. A missing prerequisite
/// must not be misreported as proof that the absent listener fails closed.
async fn assert_r19_program_live(
    source: Ipv4Addr,
) -> overdrive_netlink::nft::SharedIpInterceptState {
    let state = observe_shared_intercept_state()
        .expect("read the R19 owned program")
        .expect("the R19 program remains present");
    assert!(state.managed_guest_ips().contains(&source), "the R19 source remains a managed guest");
    assert!(
        state.outbound_sources().contains(&source),
        "the R19 source still selects outbound rule 1"
    );
    let client =
        overdrive_netlink::Client::new().expect("open the existing typed host netlink reader");
    assert!(
        client.fib_rule_fwmark_present(1, 100).await.expect("read the fwmark rule"),
        "R19 still has fwmark 0x1 lookup 100"
    );
    assert!(
        client.local_route_present(100, "lo").await.expect("read the local route"),
        "R19 still has table 100's local default route"
    );
    eprintln!(
        "R19 owned-program prerequisite: source={source}, managed={}, outbound={}, fwmark_rule=true, local_route=true",
        state.managed_guest_ips().contains(&source),
        state.outbound_sources().contains(&source)
    );
    state
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
        bridge_guard_drop_packets(),
        guard_before,
        "the bridge guard's default-drop counter is unchanged"
    );
}

// ---------------------------------------------------------------------------
// Killed-mode residue cleanup (RAII) — the killed server leaves live VMs, TAPs,
// cgroup scopes, bpffs pins, and nft tables; no second `serve` boot cleans them
// (a boot refuses on stale members before 08-02). The guard reaps exactly the
// host state that appeared after it was installed.
// ---------------------------------------------------------------------------

/// A snapshot of the host state a killed server can leave behind. The residue
/// guard captures one at install and one at Drop; the set difference is what it
/// reaps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostResidue {
    ch_pids: BTreeSet<u32>,
    ovd_links: BTreeSet<String>,
    alloc_scopes: BTreeSet<String>,
    tcx_pins: BTreeSet<String>,
    intercept_tables: BTreeSet<String>,
}

impl HostResidue {
    fn capture() -> Self {
        Self {
            ch_pids: cloud_hypervisor_pids(),
            ovd_links: directory_names(Path::new("/sys/class/net"))
                .into_iter()
                .filter(|name| name.starts_with("ovd-"))
                .collect(),
            alloc_scopes: directory_names(Path::new(WORKLOADS_SLICE))
                .into_iter()
                .filter(|name| name.starts_with("alloc-"))
                .collect(),
            tcx_pins: directory_names(Path::new(TCX_LINK_PIN_DIR)),
            intercept_tables: intercept_ip_tables(),
        }
    }
}

/// The immediate directory entry names under `dir`, or an empty set when the
/// directory is absent (a fallible read that is not "absent" panics).
fn directory_names(dir: &Path) -> BTreeSet<String> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|entry| {
                entry.expect("read directory entry").file_name().to_string_lossy().into_owned()
            })
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => panic!("read {}: {error}", dir.display()),
    }
}

/// Every live `cloud-hypervisor` pid, matched on `argv[0]` basename (never the
/// `TASK_COMM_LEN`-truncated `comm`).
fn cloud_hypervisor_pids() -> BTreeSet<u32> {
    let mut pids = BTreeSet::new();
    for entry in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry: {error}"),
        };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline = match std::fs::read(entry.path().join("cmdline")) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc/{pid}/cmdline: {error}"),
        };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        let argv0 = String::from_utf8_lossy(argv0);
        if Path::new(argv0.as_ref()).file_name() == Some(std::ffi::OsStr::new("cloud-hypervisor")) {
            pids.insert(pid);
        }
    }
    pids
}

/// The `ip`-family nft tables the intercept owner installs (`overdrive-mtls`
/// and the R18 guard `overdrive-mtls-guard`).
fn intercept_ip_tables() -> BTreeSet<String> {
    let output = Command::new("nft")
        .args(["list", "tables", "ip"])
        .output()
        .expect("run nft list tables ip");
    assert!(
        output.status.success(),
        "nft list tables ip failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("table ip ").map(str::to_owned))
        .filter(|name| name == INTERCEPT_TABLE || name == GUARD_TABLE)
        .collect()
}

/// An RAII guard that reaps exactly the killed-server residue that appeared
/// after it was installed: the Cloud Hypervisor pids (SIGKILL, waited to exit),
/// then the cgroup scopes, TAPs, bpffs pins, and nft tables. Every step's
/// result is appended to an append-only diagnostic log; on the non-panicking
/// path the guard also asserts the residue is gone (a genuine cleanup failure
/// fails the test), while during unwinding it only logs, so it never
/// double-panics.
struct KilledServerResidueGuard {
    baseline: HostResidue,
    label: String,
    adopted_program: Option<overdrive_netlink::nft::SharedIpInterceptState>,
}

impl KilledServerResidueGuard {
    fn install(label: &str) -> Self {
        Self { baseline: HostResidue::capture(), label: label.to_owned(), adopted_program: None }
    }

    fn record_owned_program(
        &mut self,
        program: &overdrive_netlink::nft::SharedIpInterceptState,
        managed: BTreeSet<Ipv4Addr>,
        inbound: BTreeSet<SocketAddrV4>,
    ) {
        assert_eq!(program.managed_guest_ips(), &managed, "exact test-owned managed sources");
        assert_eq!(program.outbound_sources(), &managed, "exact test-owned outbound sources");
        assert_eq!(
            program.inbound_destinations(),
            &inbound,
            "exact test-owned registered destinations"
        );
        if self.baseline.intercept_tables.contains(INTERCEPT_TABLE) {
            self.adopted_program = Some(program.clone());
        }
    }

    fn release_adopted_members(
        program: &overdrive_netlink::nft::SharedIpInterceptState,
    ) -> Result<(), String> {
        let observed = observe_shared_intercept_state()?;
        if observed.as_ref() != Some(program) {
            return Err(
                "the witnessed test-owned program changed; cleanup refuses mutation".to_owned()
            );
        }
        for source in program.managed_guest_ips() {
            overdrive_netlink::nft::delete_shared_ip_intercept_elements_atomically(
                program.identity(),
                Some(*source),
                &[],
            )
            .map_err(|error| error.to_string())?;
        }
        let destinations = program.inbound_destinations().iter().copied().collect::<Vec<_>>();
        if !destinations.is_empty() {
            overdrive_netlink::nft::delete_shared_ip_intercept_elements_atomically(
                program.identity(),
                None,
                &destinations,
            )
            .map_err(|error| error.to_string())?;
        }
        let after = observe_shared_intercept_state()?
            .ok_or_else(|| "cleanup removed the pre-existing constant program".to_owned())?;
        if after.identity() != program.identity()
            || !after.managed_guest_ips().is_empty()
            || !after.outbound_sources().is_empty()
            || !after.inbound_destinations().is_empty()
        {
            return Err(
                "cleanup did not preserve the constant program with zero owned members".to_owned()
            );
        }
        Ok(())
    }
}

impl Drop for KilledServerResidueGuard {
    fn drop(&mut self) {
        // A failing host read while the body is already unwinding must not
        // panic again (that aborts the process and loses every other guard's
        // cleanup): log it and leave the residue to the next run's sweep.
        let now = if std::thread::panicking() {
            let Ok(now) = std::panic::catch_unwind(HostResidue::capture) else {
                eprintln!(
                    "{}: killed-server residue cleanup skipped: the host residue read failed \
                     while unwinding (its panic message is above)",
                    self.label
                );
                return;
            };
            now
        } else {
            HostResidue::capture()
        };
        let mut log: Vec<String> = Vec::new();

        for pid in now.ch_pids.difference(&self.baseline.ch_pids) {
            // SAFETY: SIGKILL to a Cloud Hypervisor process this body's server
            // started; the pid was read from /proc above.
            let rc =
                unsafe { libc::kill(i32::try_from(*pid).expect("pid fits i32"), libc::SIGKILL) };
            let exited = wait_for_pid_exit(*pid, Duration::from_secs(5));
            log.push(format!("ch pid {pid}: kill rc={rc}, exited_within_5s={exited}"));
        }
        for scope in now.alloc_scopes.difference(&self.baseline.alloc_scopes) {
            let path = Path::new(WORKLOADS_SLICE).join(scope);
            let killed = std::fs::write(path.join("cgroup.kill"), "1");
            let drained = wait_for_cgroup_drain(&path, Duration::from_secs(5));
            let removed = std::fs::remove_dir(&path);
            log.push(format!("scope {scope}: kill={killed:?} drained={drained} rmdir={removed:?}"));
        }
        for link in now.ovd_links.difference(&self.baseline.ovd_links) {
            let removed = Command::new("ip").args(["link", "del", link]).status();
            log.push(format!("link {link}: del={removed:?}"));
        }
        for pin in now.tcx_pins.difference(&self.baseline.tcx_pins) {
            let removed = std::fs::remove_file(Path::new(TCX_LINK_PIN_DIR).join(pin));
            log.push(format!("tcx pin {pin}: rm={removed:?}"));
        }
        // A sealed normal shutdown retains an empty owned constant program.
        // A later killed test can adopt that table, so table-name difference
        // alone misses the members this test created. After its complete
        // observation window, release only the exact witnessed test members;
        // retain the same pre-existing program, targets, and foreign complement.
        let adopted_cleanup = self.adopted_program.as_ref().map(Self::release_adopted_members);
        if let Some(result) = &adopted_cleanup {
            log.push(format!("adopted program: exact owned member release={result:?}"));
        }
        for table in now.intercept_tables.difference(&self.baseline.intercept_tables) {
            let removed = overdrive_netlink::nft::delete_table(table);
            log.push(format!("nft table ip {table}: delete={removed:?}"));
        }

        // Append-only diagnostic history (never overwrites an earlier line).
        for line in &log {
            eprintln!("{}: killed-server residue cleanup: {line}", self.label);
        }

        // On the success path, a genuine leak fails the test; during unwinding
        // we only log, so the guard never double-panics.
        if !std::thread::panicking() {
            assert!(
                adopted_cleanup.as_ref().is_none_or(Result::is_ok),
                "{}: adopted constant program's test-owned member cleanup failed: {adopted_cleanup:?}",
                self.label
            );
            let after = HostResidue::capture();
            assert!(
                after.ch_pids.difference(&self.baseline.ch_pids).next().is_none(),
                "{}: a killed-server Cloud Hypervisor process survived cleanup",
                self.label
            );
            assert!(
                after.alloc_scopes.difference(&self.baseline.alloc_scopes).next().is_none(),
                "{}: a killed-server cgroup scope survived cleanup",
                self.label
            );
            assert!(
                after.intercept_tables.difference(&self.baseline.intercept_tables).next().is_none(),
                "{}: a killed-server nft table survived cleanup",
                self.label
            );
        }
    }
}

/// Poll `/proc/<pid>` until it is gone, up to `bound`; `true` iff it exited.
fn wait_for_pid_exit(pid: u32, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    while Path::new("/proc").join(pid.to_string()).exists() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// Poll a cgroup scope's `cgroup.procs` until it is empty, up to `bound`;
/// `true` iff it drained. A `NotFound` read means the scope is already gone
/// (drained); any other read error keeps polling until the deadline (it is a
/// cleanup poll, not a load-bearing observation).
fn wait_for_cgroup_drain(scope: &Path, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        match std::fs::read_to_string(scope.join("cgroup.procs")) {
            Ok(procs) if procs.split_whitespace().next().is_none() => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Ok(_) | Err(_) => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------
// S-ND295-62 — intercept-marked guest TCP is dropped even without the program
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-62 — Intercept-marked guest TCP is dropped even without the program table
/// CONTRACT_SHAPE: bounded-change.
///
/// E14 (a)/(b), the R18 hazard body. Its healthy baseline reads no guard-table
/// presence: with the intercept program present and leg F listening, each R18
/// probe SYN — to the peer's address, to the bridge gateway, and to a real host
/// interface address, each at the wildcard host listener's port — receives a
/// SYN-ACK captured on the guest's TAP, and the wildcard listener accepts
/// nothing. Then it deletes `table ip overdrive-mtls` and asserts E14 (a) (zero
/// forwarded intercept-marked frames on the PEER's TAP, with the peer TAP up as
/// the live-capture witness) and E14 (b) (no SYN-ACK reaches the guest and the
/// wildcard listener accepts nothing), plus the per-run in-run ingress witness.
/// At 08-01 it runs once BEFORE the guard exists (the native RED that decides
/// R18) and again after (GREEN); its assertions encode the fail-closed (GREEN)
/// outcome.
#[tokio::test]
async fn marked_guest_tcp_is_neither_forwarded_nor_delivered_without_the_intercept_program() {
    let _teardown = TeardownBound::arm();
    let forwarding = std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .expect("read ip_forward")
        .trim()
        .to_owned();
    std::fs::write("/proc/sys/net/ipv4/ip_forward", b"1").expect("enable host forwarding");
    eprintln!("S-ND295-62: ip_forward recorded as {forwarding}, set to 1 as a precondition");

    let host_addr = host_interface_address();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-62-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let probe = build_syn_probe_guest(tmp.path(), host_addr);
    // Both allocations clone this one immutable master, containing the exact
    // authored peer and caller. A second single-binary staging call in this
    // directory would replace rootfs.ext4 and erase the peer executable.
    let rootfs = stage_rootfs_with_extra_binaries(
        tmp.path(),
        &fixture,
        &[(&peer, "gti-peer"), (&probe, "nd295-syn-probe")],
    );
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // A mesh Service (the peer) — polled to Running so the host learns the
    // peer's guest address and its production TAP for the E14 (a) capture.
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-62-peer.toml",
        &service_toml(Path::new("/sbin/gti-peer"), &fixture.kernel_path, &rootfs),
    );
    let peer_out = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh peer service");
    let peer_running = poll_until_running(&cfg, &peer_out.workload_id, RUNNING_BOUND).await;
    let peer_addr = peer_running
        .snapshot
        .rows
        .first()
        .expect("one Running peer row")
        .workload_addr
        .expect("the peer publishes its guest address");
    let peer_tap = tap_for(peer_addr);

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest_with_args(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        &[format!("{peer_addr}:{SERVICE_PORT}"), "r18".to_owned()],
    )
    .await;
    assert!(
        observe_shared_intercept_state().is_ok_and(|state| state.is_some()),
        "the intercept program is installed before the fault"
    );

    // Healthy baseline (reads no guard): each R18 probe SYN receives a SYN-ACK
    // at the guest's TAP, and the wildcard listener accepts nothing.
    let baseline = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let baseline_peer = SynCapture::open(&guest.tap, SERVICE_PORT);
    let host_local = [(GATEWAY, HOST_WILDCARD_PORT), (host_addr, HOST_WILDCARD_PORT)];
    let mut answered = BTreeSet::new();
    let every_host_local_dial_answered = wait_until(Duration::from_secs(8), || {
        answered.extend(baseline.syn_acks_seen(&host_local));
        answered.len() == host_local.len()
    })
    .await;
    assert!(
        every_host_local_dial_answered,
        "healthy baseline: a SYN-ACK for the bridge-gateway dial and for the real-host-address \
         dial reaches the guest's TAP; answered {answered:?} of {host_local:?}"
    );
    assert!(
        wait_until(Duration::from_secs(8), || baseline_peer.syn_ack_from(peer_addr, SERVICE_PORT))
            .await,
        "healthy baseline: a SYN-ACK for the peer-address dial reaches the guest's TAP"
    );
    assert_eq!(
        listener.accepted(),
        0,
        "healthy baseline: the wildcard host listener accepts nothing while the intercept answers"
    );
    listener.assert_healthy();
    drop(baseline);
    drop(baseline_peer);

    // Fault (a)/(b): delete the intercept program table.
    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let peer_ifindex = interface_index(&peer_tap);
    let guest_ifindex = interface_index(&guest.tap);
    let peer_capture = WireCapture::start_link_layer(peer_ifindex);
    // Positive witness for the peer-TAP zero-forwarded oracle: a capture on the
    // GUEST's own TAP counting its SERVICE_PORT (peer-dial) SYNs. If the guest
    // provably attempts the peer dial yet the peer's TAP sees zero forwarded
    // SYNs, the forwarding is blocked — the peer capture is not merely dead.
    let guest_capture = WireCapture::start_link_layer(guest_ifindex);
    let accepts_before = listener.accepted();
    let program_before_fault = observe_shared_intercept_state()
        .expect("observe the exact owned program before the table-loss experiment")
        .expect("the table-loss experiment begins with the owned program present");
    overdrive_netlink::nft::delete_table(INTERCEPT_TABLE)
        .expect("external actor deletes the intercept program table");
    let fault_applied_at = packet_clock_now();
    // Fault-point witnesses (not only at the end): the captures bind on TAPs
    // that read back up, so each is live when the fault lands (never a capture
    // on a quiesced TAP); host forwarding is on, so a forward COULD happen; and
    // the nft delete actually removed the intercept table.
    assert!(
        tap_is_up(&guest.tap),
        "the run is void, not GREEN: the guest's TAP {} was down at the fault point",
        guest.tap
    );
    assert!(
        tap_is_up(&peer_tap),
        "the run is void, not GREEN: the peer's TAP {peer_tap} was down at the fault point"
    );
    assert_eq!(
        std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
            .expect("read ip_forward at the fault point")
            .trim(),
        "1",
        "host forwarding is enabled at the fault point, so the peer-forward path is live"
    );
    assert!(
        !intercept_ip_tables().contains(INTERCEPT_TABLE),
        "the nft delete removed the intercept program table at the fault point"
    );

    // The probe keeps dialing across the window; no host listener accepts.
    tokio::time::sleep(PROBE_WINDOW).await;
    let peer_capture = peer_capture.stop_accounted().expect("lossless complete peer-TAP capture");
    let guest_capture =
        guest_capture.stop_accounted().expect("lossless complete sender-TAP capture");
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts a marked guest SYN without the program"
    );
    listener.assert_healthy();
    // The fault held for the whole window — no supervisor repaired the intercept
    // table mid-window (a repair would re-arm the door, and the fail-closed
    // assertions below would then test the wrong, re-armed state).
    assert!(
        !intercept_ip_tables().contains(INTERCEPT_TABLE),
        "the intercept program table stayed absent across the probe window (no repair spanned it)"
    );
    // E14 (a): zero forwarded intercept-marked frames on the peer's TAP, proven
    // non-vacuous three ways — the peer TAP is up (live-capture witness), the
    // guest provably attempted the peer dial on its own TAP (positive witness),
    // and the peer capture dropped no frames (so zero is a real observation, not
    // a silent queue overflow).
    assert!(tap_is_up(&peer_tap), "E14 (a) live-capture witness: the peer's TAP {peer_tap} is up");
    let fault_frames = |frame: &&super::guest_stack_mtls_egress::CapturedFrame| {
        let at = frame.kernel_event_at.expect("every captured frame has its kernel event time").0;
        assert!(
            !frame.truncated && !frame.control_truncated,
            "complete frame and timestamp evidence"
        );
        at >= fault_applied_at
    };
    let forwarded = peer_capture
        .frames
        .iter()
        .filter(fault_frames)
        .filter(|frame| {
            assert_eq!(
                frame.ifindex, peer_ifindex,
                "the peer capture keeps its exact bound ifindex"
            );
            is_syn_to_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(guest.addr)
        })
        .count();
    let peer_drops = peer_capture.statistics.drops;
    let guest_peer_syns = guest_capture
        .frames
        .iter()
        .filter(fault_frames)
        .filter(|frame| {
            assert_eq!(
                frame.ifindex, guest_ifindex,
                "the sender capture keeps its exact bound ifindex"
            );
            is_syn_to_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(guest.addr)
        })
        .count();
    let mut replied = BTreeSet::new();
    let mut host_local_syns = 0_usize;
    let mut requests = BTreeSet::new();
    for frame in guest_capture.frames.iter().filter(fault_frames) {
        assert_eq!(frame.ifindex, guest_ifindex, "exact sender TAP for the local-delivery oracle");
        if is_syn_to_port(&frame.bytes, HOST_WILDCARD_PORT)
            && frame_ipv4_src(&frame.bytes) == Some(guest.addr)
        {
            host_local_syns += 1;
            requests.insert(
                probe_request(&frame.bytes, guest.addr, HOST_WILDCARD_PORT)
                    .expect("the observed SYN carries its exact request nonce"),
            );
        }
    }
    for frame in guest_capture.frames.iter().filter(fault_frames) {
        if let Some(reply) = probe_reply(&frame.bytes, HOST_WILDCARD_PORT, &requests) {
            if host_local.contains(&reply) {
                replied.insert(reply);
            }
        }
    }
    eprintln!(
        "S-ND295-62 complete capture: peer_ifindex={peer_ifindex}, guest_ifindex={guest_ifindex}, \
        peer_frames={}, guest_frames={}, peer_drops={peer_drops}, guest_peer_syns={guest_peer_syns}, \
        host_local_syns={host_local_syns}, forwarded={forwarded}, replied={replied:?}",
        peer_capture.frames.len(),
        guest_capture.frames.len()
    );
    assert!(
        guest_peer_syns >= 1,
        "the guest attempts the peer dial on its own TAP (captured {guest_peer_syns} SERVICE_PORT \
         SYNs); the peer-TAP zero-forwarded oracle is not vacuous"
    );
    assert_eq!(
        peer_drops, 0,
        "the peer-TAP capture dropped no frames, so its zero-forwarded count is a real observation"
    );
    // Establish every ingress/control prerequisite before a fail-closed
    // assertion can report the native RED that decides R18.
    assert!(tap_is_up(&guest.tap), "the run is void, not GREEN: sender TAP was quiesced");
    assert!(host_local_syns >= 1, "the host-local probe SYNs enter the exact sender TAP");
    let intercepted = intercept_counter()
        .checked_sub(intercept_before)
        .expect("the Intercept counter does not decrease");
    assert!(
        intercepted >= (host_local_syns + guest_peer_syns) as u64,
        "the TCX Intercept counter rises by at least every observed probe SYN: {intercepted}"
    );
    assert_eq!(
        bridge_guard_drop_packets(),
        guard_before,
        "the bridge guard's default-drop counter is unchanged"
    );
    eprintln!(
        "S-ND295-62 ingress prerequisites passed: intercept_delta={intercepted}, bridge_default_drop_unchanged=true"
    );
    assert_eq!(
        forwarded, 0,
        "E14 (a): no marked guest SYN is forwarded to the peer's TAP without the program"
    );
    // E14 (b): no SYN-ACK reaches the guest for the gateway or host-address dial.
    assert!(
        replied.is_empty(),
        "E14 (b): no SYN-ACK reaches the guest for a host-local marked dial without the program; \
         replies from {replied:?}"
    );
    // The complete fault window and every E14 assertion ended above. Restore
    // only this experiment's deleted program, with its exact original targets
    // and members, before ordinary production stop. R10 deliberately refuses
    // element release while the observed program differs from the owner's
    // recorded identity; it does not authorize stop to create a missing table.
    overdrive_netlink::nft::replace_shared_ip_intercept_atomically(
        None,
        Some(program_before_fault.identity()),
    )
    .expect("restore only the deliberately deleted owned program at its original targets");
    assert_eq!(
        program_before_fault.managed_guest_ips(),
        program_before_fault.outbound_sources(),
        "the captured source membership is exactly paired"
    );
    for source in program_before_fault.outbound_sources() {
        overdrive_netlink::nft::insert_shared_ip_intercept_outbound_elements_atomically(
            program_before_fault.identity(),
            *source,
        )
        .expect("restore exactly the experiment's original source members");
    }
    for destination in program_before_fault.inbound_destinations() {
        overdrive_netlink::nft::insert_shared_ip_intercept_inbound_element_atomically(
            program_before_fault.identity(),
            *destination,
        )
        .expect("restore exactly the experiment's original inbound members");
    }
    assert_eq!(
        observe_shared_intercept_state().expect("read back the restored owned program"),
        Some(program_before_fault),
        "fixture cleanup restores exactly the pre-fault program and all of its members"
    );

    stop_and_await_terminal(&cfg, &guest.workload_id).await;
    stop_and_await_terminal(&cfg, &peer_out.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
    std::fs::write("/proc/sys/net/ipv4/ip_forward", forwarding.as_bytes())
        .expect("restore ip_forward");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-62 — Intercept-marked guest TCP is dropped even without the program table
/// CONTRACT_SHAPE: bounded-change.
///
/// The guard's own non-interference control (R18-conditional; this body is
/// removed at 08-01 if R18 is withdrawn). Before any fault it reads the guard
/// table back present (`observe_intercept_mark_guard()` returns `Ok(true)`) and
/// shows each R18 dial still gets its SYN-ACK at the guest's TAP — the guard
/// dropped nothing on the healthy path. Then it deletes ONLY the guard table
/// and shows the intercept program alone still keeps a marked guest SYN out of
/// every host wildcard listener.
#[tokio::test]
async fn the_intercept_program_still_catches_marked_tcp_without_the_guard_table() {
    let _teardown = TeardownBound::arm();
    let host_addr = host_interface_address();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-62g-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path(), host_addr);
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    // This control emits the same scripted SYNs as the R18 hazard body. A
    // completed application handshake would legitimately make the intercept
    // connect the wildcard backend and invalidate the zero-accept control.
    // There is no peer in this control: its first raw target is the real host.
    let guest = deploy_probe_guest_with_args(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &probe_rootfs,
        &[format!("{host_addr}:{HOST_WILDCARD_PORT}"), "r18".to_owned()],
    )
    .await;

    // Non-interference control: the guard table reads back present, and each
    // R18 dial still gets its SYN-ACK — the guard drops nothing on the healthy
    // path (its rule carries no counter; this presence read is the evidence).
    assert!(
        matches!(overdrive_netlink::nft::observe_intercept_mark_guard(), Ok(true)),
        "the intercept-mark guard table is present before the fault"
    );
    let baseline = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    assert!(
        wait_until(Duration::from_secs(8), || baseline.syn_ack_from(GATEWAY, HOST_WILDCARD_PORT))
            .await,
        "non-interference control: a SYN-ACK reaches the guest with the guard present"
    );
    drop(baseline);

    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let capture = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();
    // Delete ONLY the guard table; the intercept program stays installed.
    overdrive_netlink::nft::delete_table(GUARD_TABLE)
        .expect("external actor deletes only the guard table");
    // Fault-point witnesses: the capture binds on a TAP that reads back up
    // (never a quiesced-TAP capture), the guard table is actually gone, and the
    // intercept program stays installed — the control's whole premise.
    assert!(
        tap_is_up(&guest.tap),
        "the run is void, not GREEN: the guest's TAP {} was down at the fault point",
        guest.tap
    );
    assert!(
        !intercept_ip_tables().contains(GUARD_TABLE),
        "the nft delete removed the guard table at the fault point"
    );
    assert!(
        intercept_ip_tables().contains(INTERCEPT_TABLE),
        "the intercept program table stays installed after only the guard table is deleted"
    );

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "the intercept program alone still keeps a marked guest SYN out of any host listener"
    );
    listener.assert_healthy();
    // No supervisor re-added the guard table or removed the intercept program
    // across the window (the control would otherwise test a re-armed state).
    assert!(
        !intercept_ip_tables().contains(GUARD_TABLE),
        "the guard table stayed absent across the probe window (no repair spanned it)"
    );
    assert!(
        intercept_ip_tables().contains(INTERCEPT_TABLE),
        "the intercept program stayed installed across the probe window"
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
    assert!(
        output.status.success(),
        "nft list overdrive-mtls: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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

/// The inbound (leg-C) TPROXY loopback port of the installed intercept program:
/// the TPROXY-to-loopback port distinct from the leg-F port. Rules 1 (leg F,
/// outbound) and 3 (leg C, inbound) each TPROXY to their own loopback port
/// (FD § "Hazard 2"), so the program names at least two; leg C is the one that
/// is not leg F. Asserts the program is installed first (via [`leg_f_port`]).
fn leg_c_port() -> u16 {
    let leg_f = leg_f_port();
    let output = Command::new("nft")
        .args(["-j", "list", "table", "ip", INTERCEPT_TABLE])
        .output()
        .expect("run nft list for the intercept table");
    assert!(
        output.status.success(),
        "nft list overdrive-mtls: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let mut ports: BTreeSet<u16> = BTreeSet::new();
    for tail in text.split("\"tproxy\":").skip(1) {
        if let Some(key) = tail.find("\"port\":") {
            let start = key + "\"port\":".len();
            if let Some(port) = tail[start..]
                .trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|n| n.parse::<u16>().ok())
                .filter(|port| *port != 0)
            {
                ports.insert(port);
            }
        }
    }
    ports
        .into_iter()
        .find(|port| *port != leg_f)
        .expect("the intercept program's prerouting names a leg-C TPROXY port distinct from leg F")
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
async fn outbound_tcp_to_a_closed_listener_is_dropped_not_delivered_locally() {
    let _teardown = TeardownBound::arm();
    let host_addr = host_interface_address();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63c-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path(), host_addr);
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest_with_args(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &probe_rootfs,
        &["r19".to_owned()],
    )
    .await;

    assert_healthy_probe_synacks(
        &guest.tap,
        &[(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)],
    )
    .await;
    assert_eq!(listener.accepted(), 0, "healthy scripted SYNs complete no host wildcard handshake");
    listener.assert_healthy();
    let program_before = assert_r19_program_live(guest.addr).await;
    let leg_f = leg_f_port();
    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let ifindex = interface_index(&guest.tap);
    assert!(tap_is_up(&guest.tap), "the sender TAP is up before listener destruction");
    // This existing loss-accounted reader starts before the fault, continuously
    // drains through the owner's real link-down notification, seals the socket,
    // and requires every kernel-counted frame to have been read with zero loss.
    // No late ENETDOWN is converted into zero packets.
    let capture = WireCapture::start_link_layer(ifindex);
    let accepts_before = listener.accepted();

    // Destroy the exact leg-F LISTENING tuple from outside (sock-diag destroy of
    // the transparent listener on loopback), assert the destroy found and killed
    // it, then immediately re-occupy its port with a plain non-transparent
    // listener (port-theft), holding the listener-absent state.
    destroy_listening_tuple(Ipv4Addr::LOCALHOST, leg_f);
    let _thief = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, leg_f))
        .expect("re-occupy the leg-F port with a plain listener (port-theft)");
    let fault_applied_at = packet_clock_now();
    let mut tap_history = vec![tap_state_sample(&guest.tap, ifindex)];
    assert!(tap_history[0].1, "the sender TAP is still up when listener loss is established");
    let deadline = Instant::now() + PROBE_WINDOW;
    while Instant::now() < deadline {
        tap_history.push(tap_state_sample(&guest.tap, ifindex));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tap_history.push(tap_state_sample(&guest.tap, ifindex));
    let captured =
        capture.stop_accounted().expect("complete, lossless capture across owner quiescence");
    assert!(!captured.interface_removed, "the production TAP's ifindex survives the observation");
    let mut ingress = BTreeSet::new();
    let mut replied = BTreeSet::new();
    let targets = [(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)];
    let mut captured_syns = 0_usize;
    let mut requests = BTreeSet::new();
    for frame in &captured.frames {
        assert_eq!(frame.ifindex, ifindex, "every frame comes from the exact bound TAP");
        assert!(
            !frame.truncated && !frame.control_truncated,
            "capture retains complete frame and clock evidence"
        );
        let at =
            frame.kernel_event_at.expect("each frame carries its kernel packet-event timestamp").0;
        if at < fault_applied_at {
            continue;
        }
        let emitted_while_up = tap_history.windows(2).any(|samples| {
            samples[0].1 && samples[1].1 && samples[0].0 <= at && at <= samples[1].0
        });
        if emitted_while_up
            && is_syn_to_port(&frame.bytes, HOST_WILDCARD_PORT)
            && frame_ipv4_src(&frame.bytes) == Some(guest.addr)
        {
            captured_syns += 1;
            requests.insert(
                probe_request(&frame.bytes, guest.addr, HOST_WILDCARD_PORT)
                    .expect("the live SYN carries its request nonce"),
            );
            let dst =
                Ipv4Addr::new(frame.bytes[30], frame.bytes[31], frame.bytes[32], frame.bytes[33]);
            if targets.contains(&(dst, HOST_WILDCARD_PORT)) {
                ingress.insert((dst, HOST_WILDCARD_PORT));
            }
        }
    }
    for frame in &captured.frames {
        if frame.kernel_event_at.expect("each reply has its kernel event time").0
            >= fault_applied_at
        {
            if let Some(reply) = probe_reply(&frame.bytes, HOST_WILDCARD_PORT, &requests) {
                if targets.contains(&reply) {
                    replied.insert(reply);
                }
            }
        }
    }
    eprintln!(
        "S-ND295-63c complete capture: ifindex={ifindex}, frames={}, link_down_reports={:?}, \
         first_down={:?}, fault_at={fault_applied_at}, live_ingress={ingress:?}, replies={replied:?}",
        captured.frames.len(),
        captured.link_down_reports,
        tap_history.iter().find(|sample| !sample.1).map(|sample| sample.0),
    );
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts an outbound guest SYN once leg F is gone"
    );
    listener.assert_healthy();
    // No SYN-ACK reaches the guest for the gateway or external dial.
    assert!(
        replied.is_empty(),
        "no SYN-ACK reaches the guest for an outbound marked dial once leg F is gone; replies from {replied:?}"
    );
    assert!(
        captured_syns >= 1 && ingress.len() == targets.len(),
        "the run is void, not GREEN: each required gateway/external SYN must enter the exact TAP \
         while it is up and leg F is absent; observed {captured_syns} live SYNs, {ingress:?} of {targets:?}"
    );
    let intercepted = intercept_counter()
        .checked_sub(intercept_before)
        .expect("the Intercept counter does not decrease");
    assert!(
        intercepted >= captured_syns as u64,
        "the TCX Intercept counter rises by every live probe SYN"
    );
    assert_eq!(
        bridge_guard_drop_packets(),
        guard_before,
        "the bridge guard's default-drop counter is unchanged"
    );
    let program_after = assert_r19_program_live(guest.addr).await;
    assert_eq!(
        program_after, program_before,
        "R19's owned program and allocation members stay unchanged across listener loss"
    );
    eprintln!(
        "S-ND295-63c ingress prerequisites passed: intercept_delta={intercepted}, captured_live_syns={captured_syns}, bridge_default_drop_unchanged=true"
    );

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
async fn outbound_tcp_after_a_killed_server_is_dropped_while_the_vm_lives() {
    let _teardown = TeardownBound::arm();
    let host_addr = host_interface_address();
    // Installed before any host state is created, so its Drop reaps exactly the
    // killed server's residue (live VM, TAP, cgroup scope, bpffs pins, nft
    // tables) on every exit path, including the Running-precondition panic. No
    // second `serve` boot is relied on for cleanup.
    let mut residue = KilledServerResidueGuard::install("S-ND295-63d");
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63d-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let probe = build_syn_probe_guest(tmp.path(), host_addr);
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let listener = WildcardListener::bind(HOST_WILDCARD_PORT);
    let guest = deploy_probe_guest_with_args(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &probe_rootfs,
        &["r19".to_owned()],
    )
    .await;

    assert_healthy_probe_synacks(
        &guest.tap,
        &[(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)],
    )
    .await;
    assert_eq!(listener.accepted(), 0, "healthy scripted SYNs complete no host wildcard handshake");
    listener.assert_healthy();
    let program_before = assert_r19_program_live(guest.addr).await;
    residue.record_owned_program(
        &program_before,
        [guest.addr].into_iter().collect(),
        BTreeSet::new(),
    );
    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let ifindex = interface_index(&guest.tap);
    let capture = WireCapture::start_link_layer(ifindex);
    let accepts_before = listener.accepted();

    // Killed mode: abandon the serve owner; Cloud Hypervisor and the TAP stay
    // up, so the guest keeps dialing but leg F is gone.
    kill_serve_owner(handle).await.expect("killed-mode serve abandons its owner");
    let fault_applied_at = packet_clock_now();
    assert!(tap_is_up(&guest.tap), "the guest's TAP stays up after killed mode");
    let program_at_fault = assert_r19_program_live(guest.addr).await;
    assert_eq!(
        program_at_fault, program_before,
        "the killed-mode path retains R19's exact program and members"
    );

    tokio::time::sleep(PROBE_WINDOW).await;
    let capture = capture.stop_accounted().expect("complete lossless killed-mode observation");
    assert!(
        !capture.interface_removed && capture.link_down_reports.is_empty(),
        "the same TAP stays live across the killed-mode window"
    );
    let mut replied = BTreeSet::new();
    let mut ingress = BTreeSet::new();
    let targets = [(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)];
    let mut captured_syns = 0_usize;
    let mut requests = BTreeSet::new();
    for frame in &capture.frames {
        assert_eq!(frame.ifindex, ifindex, "same exact production TAP after killed mode");
        assert!(
            !frame.truncated && !frame.control_truncated,
            "complete frame and timestamp evidence"
        );
        if frame.kernel_event_at.expect("each packet has its kernel event time").0
            < fault_applied_at
        {
            continue;
        }
        if is_syn_to_port(&frame.bytes, HOST_WILDCARD_PORT)
            && frame_ipv4_src(&frame.bytes) == Some(guest.addr)
        {
            captured_syns += 1;
            requests.insert(
                probe_request(&frame.bytes, guest.addr, HOST_WILDCARD_PORT)
                    .expect("the post-kill SYN carries its request nonce"),
            );
            let dst =
                Ipv4Addr::new(frame.bytes[30], frame.bytes[31], frame.bytes[32], frame.bytes[33]);
            if targets.contains(&(dst, HOST_WILDCARD_PORT)) {
                ingress.insert((dst, HOST_WILDCARD_PORT));
            }
        }
    }
    for frame in &capture.frames {
        if frame.kernel_event_at.expect("each reply has its kernel event time").0
            >= fault_applied_at
        {
            if let Some(reply) = probe_reply(&frame.bytes, HOST_WILDCARD_PORT, &requests) {
                if targets.contains(&reply) {
                    replied.insert(reply);
                }
            }
        }
    }
    eprintln!(
        "S-ND295-63d complete capture: ifindex={ifindex}, frames={}, fault_at={fault_applied_at}, \
        captured_syns={captured_syns}, live_ingress={ingress:?}, replies={replied:?}",
        capture.frames.len()
    );
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts an outbound guest SYN after killed mode"
    );
    listener.assert_healthy();
    assert!(
        replied.is_empty(),
        "no SYN-ACK reaches the guest for an outbound marked dial after killed mode; replies from {replied:?}"
    );
    assert!(
        tap_is_up(&guest.tap),
        "the run is void, not GREEN: killed-mode sender TAP was quiesced"
    );
    assert!(
        captured_syns >= 1 && ingress.len() == targets.len(),
        "each exact R19 target receives guest-originated SYNs after killed mode; {ingress:?}"
    );
    let intercepted = intercept_counter()
        .checked_sub(intercept_before)
        .expect("the Intercept counter does not decrease");
    assert!(
        intercepted >= captured_syns as u64,
        "the TCX Intercept counter rises by every post-kill probe SYN"
    );
    assert_eq!(
        bridge_guard_drop_packets(),
        guard_before,
        "the bridge guard's default-drop counter is unchanged"
    );
    let program_after = assert_r19_program_live(guest.addr).await;
    assert_eq!(
        program_after, program_before,
        "R19's owned program and allocation members stay unchanged across killed mode"
    );
    eprintln!(
        "S-ND295-63d ingress prerequisites passed: intercept_delta={intercepted}, captured_live_syns={captured_syns}, bridge_default_drop_unchanged=true"
    );
    // The abandoned server left the VM live; `residue` reaps it (and every
    // other killed-server artifact) on drop — no `serve` reboot, no `let _ =`.
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-63 — Guest TCP to an absent listener fails closed
/// CONTRACT_SHAPE: bounded-change.
///
/// Inbound control (E14: "leg C closed; SYN to a registered destination is
/// dropped by rule 4 under both orders"). A host-originated SYN would traverse
/// OUTPUT, not PREROUTING. A managed guest source is in `outbound_sources` and
/// would select leg F first. This scenario sends from an unregistered source
/// over its test-owned veth into PREROUTING to the peer Service's exact
/// registered tuple. Leg C is made absent by a sock-diag destroy of its exact
/// loopback listening tuple (verified killed) whose port is immediately
/// occupied by a plain, non-transparent listener (the S-ND295-31B port-theft
/// shape). Oracle: no SYN-ACK from the peer's registered destination reaches
/// the inbound source, the peer's TAP carries no forwarded SYN, and the plain
/// listener accepts nothing. Runs under the rule order the program has at this
/// step: the retained mark → TPROXY → accept order. The R19 reorder is
/// withdrawn on native E14 evidence; rule 4 remains the inbound fail-closed
/// control.
#[tokio::test]
async fn inbound_tcp_to_a_closed_listener_is_dropped() {
    let _teardown = TeardownBound::arm();
    let owner_trace = InboundOwnerTrace::new();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(owner_trace.clone()),
    )
    .expect("the inbound owner trace owns this nextest process's subscriber");
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63i-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // The peer Service (the registered inbound destination).
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-63i-peer.toml",
        &service_toml(Path::new("/sbin/gti-peer"), &fixture.kernel_path, &rootfs),
    );
    let peer_out = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh peer service");
    let peer_running = poll_until_running(&cfg, &peer_out.workload_id, RUNNING_BOUND).await;
    let peer_addr = peer_running
        .snapshot
        .rows
        .first()
        .expect("one Running peer row")
        .workload_addr
        .expect("the peer publishes its guest address");
    let peer_tap = tap_for(peer_addr);
    let registered_destination = SocketAddrV4::new(peer_addr, SERVICE_PORT);
    let inbound_peer = InboundPeerTopology::provision(peer_addr);

    // S-ND295-63's inbound clause is a separate unregistered ingress source.
    // A production-managed source is in `outbound_sources` and would select
    // leg F before the registered-destination leg-C rule. The peer Service is
    // still the one live managed guest and owns the exact inbound destination.
    let state = observe_shared_intercept_state()
        .expect("read the existing shared intercept state")
        .expect("the shared program is present");
    assert!(
        state.inbound_destinations().contains(&registered_destination),
        "the running Service's declared tuple is registered for leg C"
    );
    assert!(!state.managed_guest_ips().contains(&inbound_peer.source_addr));
    assert!(!state.outbound_sources().contains(&inbound_peer.source_addr));

    // Healthy positive control: an unregistered source reaches the registered
    // tuple while leg C is listening. The exact correlated SYN-ACK and the
    // absence of that original source SYN on the peer TAP prove the packet
    // reached the local transparent leg-C path instead of being forwarded.
    let healthy_source_ifindex = interface_index(&inbound_peer.host_if);
    let peer_ifindex = interface_index(&peer_tap);
    let healthy_source_capture = WireCapture::start_link_layer(healthy_source_ifindex);
    let healthy_peer_capture = WireCapture::start_link_layer(peer_ifindex);
    let healthy_client_source = format!(
        r#"
use std::net::{{SocketAddr, TcpStream}};
use std::time::Duration;

fn main() {{
    let destination: SocketAddr = "{registered_destination}".parse().expect("destination");
    let stream = TcpStream::connect_timeout(&destination, Duration::from_secs(3))
        .expect("healthy inbound connection reaches the registered destination");
    std::thread::sleep(Duration::from_millis(100));
    drop(stream);
}}
"#
    );
    let healthy_client =
        build_static_binary(tmp.path(), "nd295-63-inbound-connect", &healthy_client_source);
    let healthy_status = Command::new("ip")
        .args(["netns", "exec", &inbound_peer.peer_ns])
        .arg(&healthy_client)
        .status();
    let healthy_connected = healthy_status.as_ref().is_ok_and(std::process::ExitStatus::success);
    let healthy_source_capture = healthy_source_capture
        .stop_accounted()
        .expect("lossless complete healthy inbound source capture");
    let healthy_peer_capture = healthy_peer_capture
        .stop_accounted()
        .expect("lossless complete healthy inbound peer capture");
    let mut healthy_requests = BTreeSet::new();
    for frame in &healthy_source_capture.frames {
        assert_eq!(frame.ifindex, healthy_source_ifindex, "exact healthy source veth");
        assert!(!frame.truncated && !frame.control_truncated, "complete healthy source evidence");
        if let Some(request) = probe_request(&frame.bytes, inbound_peer.source_addr, SERVICE_PORT)
            .filter(|request| request.0 == peer_addr)
        {
            healthy_requests.insert(request);
        }
    }
    let healthy_answered = healthy_source_capture
        .frames
        .iter()
        .filter(|frame| {
            is_syn_ack_from_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(peer_addr)
        })
        .filter(|frame| {
            probe_reply(&frame.bytes, SERVICE_PORT, &healthy_requests)
                == Some((peer_addr, SERVICE_PORT))
        })
        .count();
    let healthy_forwarded = healthy_peer_capture
        .frames
        .iter()
        .filter(|frame| {
            assert_eq!(frame.ifindex, peer_ifindex, "exact healthy peer Service TAP");
            is_syn_to_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(inbound_peer.source_addr)
        })
        .count();
    eprintln!(
        "S-ND295-63 healthy inbound leg-C control: connected={healthy_connected}, requests={}, correlated_synacks={healthy_answered}, original_source_forwarded={healthy_forwarded}",
        healthy_requests.len()
    );
    tokio::time::sleep(Duration::from_millis(250)).await;

    // Capture continuously before the fault. The source veth observes both
    // inbound requests and any correlated response; the exact peer TAP proves
    // whether the original source SYN was forwarded to the managed guest.
    let source_ifindex = interface_index(&inbound_peer.host_if);
    let peer_capture = WireCapture::start_link_layer(peer_ifindex);
    let source_capture = WireCapture::start_link_layer(source_ifindex);
    let guard_before = bridge_guard_drop_packets();

    // Make leg C absent: destroy its exact loopback listening tuple, then
    // occupy its port with a plain non-transparent listener (port-theft).
    let leg_c = leg_c_port();
    destroy_listening_tuple(Ipv4Addr::LOCALHOST, leg_c);
    let thief = WildcardListener::bind(leg_c);
    let fault_applied_at = packet_clock_now();
    let thief_accepts_before = thief.accepted();
    let peer_tap_up_at_fault = tap_is_up(&peer_tap);
    let sampled_tap = peer_tap.clone();
    let tap_samples = tokio::spawn(async move {
        let mut samples = Vec::new();
        for sample_number in 1..=4 {
            tokio::time::sleep(PROBE_WINDOW / 5).await;
            let flags = std::fs::read_to_string(format!("/sys/class/net/{sampled_tap}/flags"))
                .ok()
                .map(|value| value.trim().to_owned());
            let ifindex = std::fs::read_to_string(format!("/sys/class/net/{sampled_tap}/ifindex"))
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok());
            let up = flags
                .as_deref()
                .and_then(|text| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok())
                .is_some_and(|value| value & 0x1 == 0x1);
            samples.push((sample_number, flags, ifindex, up));
        }
        samples
    });

    // Repeated distinct raw SYNs keep the same inbound packet class present
    // throughout the original observation window. The namespace source is
    // unregistered, so no TCX mark or outbound-source rule selects leg F.
    let peer_ns = inbound_peer.peer_ns.clone();
    let source_addr = inbound_peer.source_addr;
    let raw_crafter = build_raw_syn_crafter(tmp.path());
    let raw_crafter = raw_crafter.to_string_lossy().into_owned();
    let peer_addr_arg = peer_addr.to_string();
    let probe_sender = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let deadline = Instant::now() + PROBE_WINDOW;
        let mut sent = 0_usize;
        while Instant::now() < deadline {
            let source_port = 62_000_u16
                .checked_add(u16::try_from(sent).map_err(|error| error.to_string())?)
                .ok_or_else(|| "inbound SYN source-port range overflowed".to_owned())?;
            let sequence = 0x295a_0000_u32.wrapping_add((sent as u32).wrapping_mul(4_000));
            let args = [
                source_addr.to_string(),
                source_port.to_string(),
                peer_addr_arg.clone(),
                SERVICE_PORT.to_string(),
                sequence.to_string(),
                "none".to_owned(),
            ];
            let status = Command::new("ip")
                .args(["netns", "exec", &peer_ns])
                .arg(&raw_crafter)
                .args(&args)
                .status()
                .map_err(|error| format!("run inbound raw SYN crafter: {error}"))?;
            if !status.success() {
                return Err(format!("inbound raw SYN exited with {status}"));
            }
            sent += 1;
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(sent)
    });

    tokio::time::sleep(PROBE_WINDOW).await;
    let sent_result = probe_sender.await.expect("join the inbound source's bounded raw-SYN loop");
    let tap_samples = tap_samples.await.expect("join the four bounded TAP state samples");
    let peer_capture =
        peer_capture.stop_accounted().expect("lossless complete inbound peer capture");
    let source_capture =
        source_capture.stop_accounted().expect("lossless complete inbound source capture");
    let fault_frames = |frame: &&super::guest_stack_mtls_egress::CapturedFrame| {
        assert!(!frame.truncated && !frame.control_truncated, "complete inbound frame evidence");
        frame.kernel_event_at.expect("every inbound frame has its kernel event time").0
            >= fault_applied_at
    };
    let forwarded = peer_capture
        .frames
        .iter()
        .filter(fault_frames)
        .filter(|frame| {
            assert_eq!(frame.ifindex, peer_ifindex, "the exact inbound peer TAP");
            is_syn_to_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(source_addr)
        })
        .count();
    let mut requests = BTreeSet::new();
    for frame in source_capture.frames.iter().filter(fault_frames) {
        assert_eq!(frame.ifindex, source_ifindex, "the exact unregistered inbound source veth");
        if let Some(request) = probe_request(&frame.bytes, source_addr, SERVICE_PORT)
            .filter(|request| request.0 == peer_addr)
        {
            requests.insert(request);
        }
    }
    let answered = source_capture
        .frames
        .iter()
        .filter(fault_frames)
        .filter(|frame| {
            is_syn_ack_from_port(&frame.bytes, SERVICE_PORT)
                && frame_ipv4_src(&frame.bytes) == Some(peer_addr)
        })
        .count();
    let fresh_answered = source_capture
        .frames
        .iter()
        .filter(fault_frames)
        .filter(|frame| {
            probe_reply(&frame.bytes, SERVICE_PORT, &requests) == Some((peer_addr, SERVICE_PORT))
        })
        .count();
    let peer_tap_up_after_window = tap_is_up(&peer_tap);
    let bridge_guard_after = bridge_guard_drop_packets();
    eprintln!(
        "S-ND295-63 inbound complete capture: requests={}, sent={sent_result:?}, forwarded={forwarded}, answered={answered}, fresh_answered={fresh_answered}, peer_drops={}, source_drops={}",
        requests.len(),
        peer_capture.statistics.drops,
        source_capture.statistics.drops
    );
    eprintln!("S-ND295-63 peer TAP samples (step, flags, ifindex, up): {tap_samples:?}");
    let owner_trace = owner_trace
        .snapshot()
        .into_iter()
        .map(|event| format!("+{:?} {} {:?}", event.elapsed, event.name, event.fields))
        .collect::<Vec<_>>()
        .join("; ");
    eprintln!("S-ND295-63 shared-owner trace: {owner_trace}");
    // Preserve the exact failed oracle while still completing this body's
    // ordinary production stop/shutdown. A failed observation must not leave
    // its owned members behind and turn later cases into startup refusals.
    let observation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert!(
            healthy_connected,
            "healthy unregistered inbound client reaches the registered tuple through leg C: {healthy_status:?}"
        );
        assert!(
            !healthy_requests.is_empty() && healthy_answered > 0,
            "healthy registered-destination control returns a correlated SYN-ACK: requests={}, replies={healthy_answered}",
            healthy_requests.len()
        );
        assert_eq!(
            healthy_forwarded, 0,
            "healthy inbound control is locally intercepted, not forwarded from the test source"
        );
        assert!(
            sent_result.as_ref().is_ok_and(|sent| *sent > 0),
            "the full-window raw-SYN sender completes with at least one inbound request: {sent_result:?}"
        );
        assert!(
            !requests.is_empty(),
            "the unregistered source sends SYNs to the exact registered destination"
        );
        assert_eq!(
            bridge_guard_after, guard_before,
            "the bridge guard's default-drop counter is unchanged during the inbound fault"
        );
        // The peer's workload never receives the forwarded SYN.
        assert!(peer_tap_up_at_fault, "the peer's TAP {peer_tap} is up at the listener-loss fault");
        assert!(
            peer_tap_up_after_window,
            "the peer's TAP {peer_tap} stays up through the complete inbound fault window"
        );
        assert_eq!(
            forwarded, 0,
            "no inbound source SYN is forwarded to the peer's registered destination with leg C closed"
        );
        // No SYN-ACK from the registered destination reaches the inbound source.
        assert_eq!(
            answered, 0,
            "no SYN-ACK from the registered destination reaches the inbound source with leg C closed"
        );
        // The plain port-theft listener on leg C accepts nothing.
        assert_eq!(
            thief.accepted(),
            thief_accepts_before,
            "the plain leg-C port-theft listener accepts no unregistered inbound SYN"
        );
        thief.assert_healthy();
    }));

    stop_and_await_terminal(&cfg, &peer_out.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
    if let Err(failure) = observation {
        std::panic::resume_unwind(failure);
    }
}

/// An unregistered inbound client connected to the host through a leased veth.
/// The host-side veth is an actual PREROUTING ingress path; the source address
/// is outside the production managed-guest and outbound-source sets.
struct InboundPeerTopology {
    _lease: TestCidrLease,
    peer_ns: String,
    host_if: String,
    source_addr: Ipv4Addr,
}

#[derive(Debug, Clone)]
struct InboundOwnerEvent {
    elapsed: Duration,
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Clone)]
struct InboundOwnerTrace {
    started: Instant,
    events: Arc<std::sync::Mutex<Vec<InboundOwnerEvent>>>,
}

impl InboundOwnerTrace {
    fn new() -> Self {
        Self { started: Instant::now(), events: Arc::new(std::sync::Mutex::new(Vec::new())) }
    }

    fn snapshot(&self) -> Vec<InboundOwnerEvent> {
        self.events.lock().expect("inbound owner trace lock").clone()
    }
}

#[derive(Default)]
struct InboundOwnerFields(BTreeMap<String, String>);

impl Visit for InboundOwnerFields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S: Subscriber> Layer<S> for InboundOwnerTrace {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let name = event.metadata().name();
        if !name.starts_with("guest_network.shared_owner_") {
            return;
        }
        let mut fields = InboundOwnerFields::default();
        event.record(&mut fields);
        self.events.lock().expect("inbound owner trace lock").push(InboundOwnerEvent {
            elapsed: self.started.elapsed(),
            name: name.to_owned(),
            fields: fields.0,
        });
    }
}

impl InboundPeerTopology {
    fn provision(destination: Ipv4Addr) -> Self {
        let lease = TestCidrLease::acquire("nd295-63i-inbound")
            .expect("acquire a CIDR for the unregistered inbound client");
        let peer_ns = format!("nd63-{}", std::process::id());
        let host_if = format!("{peer_ns}-h");
        let peer_if = format!("{peer_ns}-p");
        let source_addr = lease.workload_addr();
        let host_gateway = lease.host_gateway();
        // Construct the RAII owner before the first kernel mutation so a
        // partial namespace/veth setup is still confined to this exact pair.
        let topology = Self { _lease: lease, peer_ns, host_if, source_addr };

        run(["ip", "netns", "add", &topology.peer_ns]);
        run(["ip", "link", "add", &topology.host_if, "type", "veth", "peer", "name", &peer_if]);
        run(["ip", "link", "set", &peer_if, "netns", &topology.peer_ns]);
        let host_cidr = format!("{host_gateway}/24");
        run(["ip", "addr", "add", &host_cidr, "dev", &topology.host_if]);
        run(["ip", "link", "set", &topology.host_if, "up"]);
        let source_cidr = format!("{source_addr}/24");
        run(["ip", "-n", &topology.peer_ns, "addr", "add", &source_cidr, "dev", &peer_if]);
        run(["ip", "-n", &topology.peer_ns, "link", "set", &peer_if, "up"]);
        run(["ip", "-n", &topology.peer_ns, "link", "set", "lo", "up"]);
        let destination_route = format!("{destination}/32");
        let host_gateway = host_gateway.to_string();
        run([
            "ip",
            "-n",
            &topology.peer_ns,
            "route",
            "add",
            &destination_route,
            "via",
            &host_gateway,
            "dev",
            &peer_if,
        ]);

        topology
    }
}

impl Drop for InboundPeerTopology {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", &self.host_if]).status();
        let _ = Command::new("ip").args(["netns", "del", &self.peer_ns]).status();
    }
}

// ---------------------------------------------------------------------------
// S-ND295-64 — the TIME_WAIT side door, both controls first
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-64 — The TIME_WAIT side door is measured with both controls first
/// CONTRACT_SHAPE: bounded-change.
///
/// The controls run on a host-only path a test-owned veth peer namespace under
/// `TestCidrLease` provides — the TPROXY program does not handle it, so the
/// host holds a true `TIME_WAIT` entry. The negative control (stale ISN, no
/// timestamp) gets a bare ACK and no SYN-ACK, proving the substate and sequence
/// gates; the positive control (after `tcp_invalid_ratelimit`, a newer
/// sequence) reopens with a SYN-ACK. These two are door-independent (E14 (e):
/// "Two controls run first and do not depend on the door"): they prove the
/// kernel gates the guest door relies on, and they need no guest. The guest
/// door itself — a newer-sequence reconnect into a real intercepted guest's
/// leg-F `TIME_WAIT` entry — is
/// [`a_guest_reconnect_into_its_leg_f_time_wait_entry_is_recorded_and_a_reopen_goes_to_the_user`],
/// which needs a Running mesh guest (05-03) and so is marked separately.
///
/// ACTIVE (no marker): this body needs no guest and no production change — it
/// runs entirely on a test-owned veth peer namespace under `TestCidrLease`, so
/// it must pass today (it passed on metal ×3 in phase C).
#[tokio::test]
async fn both_time_wait_controls_prove_the_substate_and_sequence_gates() {
    prove_time_wait_controls().await;
}

/// The same original negative-then-positive controls also run before each
/// guest-door proof, so nextest's ordering cannot run the door first.
async fn prove_time_wait_controls() {
    let lease = TestCidrLease::acquire("nd295-64-time-wait")
        .expect("acquire a named CIDR for the TIME_WAIT controls");
    // The namespace name also prefixes the veth ends (`<ns>-h`, `<ns>-p`),
    // which must fit IFNAMSIZ (15 visible bytes): "nd64-" + a pid of at most
    // 7 digits (pid_max 4194304) + "-h" is at most 14.
    let peer_ns = format!("nd64-{}", std::process::id());
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
        negative,
        ReconnectReply::BareAck,
        "the stale-ISN probe gets a bare ACK and no SYN-ACK; the substate and sequence gates hold"
    );

    // Positive control (second): after tcp_invalid_ratelimit, a newer-sequence
    // SYN meets the sequence precondition and reopens with a SYN-ACK.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let positive = topology.probe_reconnect(tuple, ReconnectSeq::NewerSeq);
    assert_eq!(
        positive,
        ReconnectReply::SynAck,
        "the newer-sequence probe reopens the TIME_WAIT entry with a SYN-ACK"
    );
    eprintln!(
        "S-ND295-64 independent controls completed before the guest: negative={negative:?}, positive={positive:?}"
    );

    drop(topology);
    drop(lease);
}

/// Build the guest program that reproduces the E14 (e) guest side: it dials the
/// mesh peer by name (an intercepted leg-F flow), completes and cleanly closes
/// that connection from inside the guest — so the host's leg-F side enters the
/// true `TIME_WAIT` substate — recording its source port, then for the whole
/// probe window raw-crafts newer-sequence SYNs from that same source 4-tuple to
/// the original destination. A test-private event gate releases the reconnect
/// only after the host has observed TIME_WAIT and closed leg F; a reconnect that
/// lands while leg F is absent is what the door test records.
///
/// The guest is malicious by model (E14 (e)): it controls its source port, ISN,
/// and timestamps, so it reuses the closed connection's source port with a
/// sequence above the old `rcv_nxt` and a newer `TSval`. The raw-SYN crafting is
/// the same `AF_INET`/`SOCK_RAW`/`IP_HDRINCL` shape the host crafter uses,
/// declared inline because a bare-`rustc` static binary links no `libc` crate.
fn build_time_wait_guest(tmp: &Path) -> PathBuf {
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::{{Ipv4Addr, TcpStream, ToSocketAddrs}};
use std::time::{{Duration, Instant}};

const AF_INET: i32 = 2;
const SOCK_RAW: i32 = 3;
const IPPROTO_RAW: i32 = 255;
const IPPROTO_TCP: u8 = 6;

extern "C" {{
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
    fn close(fd: i32) -> i32;
}}

fn csum(bytes: &[u8]) -> u16 {{
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {{
        sum += u32::from(u16::from_be_bytes([bytes[i], bytes[i + 1]]));
        i += 2;
    }}
    if i < bytes.len() {{
        sum += u32::from(u16::from_be_bytes([bytes[i], 0]));
    }}
    while sum >> 16 != 0 {{
        sum = (sum & 0xffff) + (sum >> 16);
    }}
    !(sum as u16)
}}

/// Craft one SYN from (src, sport) to (dst, dport) with `seq` and a newer TS.
fn craft_syn(src: Ipv4Addr, sport: u16, dst: Ipv4Addr, dport: u16, seq: u32, tsval: u32) {{
    let mut tcp = vec![0u8; 32];
    tcp[0..2].copy_from_slice(&sport.to_be_bytes());
    tcp[2..4].copy_from_slice(&dport.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    tcp[12] = 0x80; // data offset 8 words (20 + 12 TS option)
    tcp[13] = 0x02; // SYN
    tcp[14..16].copy_from_slice(&0xffff_u16.to_be_bytes());
    // TS option: kind 8, len 10, TSval, TSecr 0, then NOP,NOP padding.
    tcp[20] = 8;
    tcp[21] = 10;
    tcp[22..26].copy_from_slice(&tsval.to_be_bytes());
    tcp[30] = 1;
    tcp[31] = 1;
    let mut pseudo = Vec::new();
    pseudo.extend_from_slice(&src.octets());
    pseudo.extend_from_slice(&dst.octets());
    pseudo.push(0);
    pseudo.push(IPPROTO_TCP);
    pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let tcp_csum = csum(&pseudo);
    tcp[16..18].copy_from_slice(&tcp_csum.to_be_bytes());
    let mut ip = vec![0u8; 20];
    ip[0] = 0x45;
    let total = (20 + tcp.len()) as u16;
    ip[2..4].copy_from_slice(&total.to_be_bytes());
    ip[8] = 64;
    ip[9] = IPPROTO_TCP;
    ip[12..16].copy_from_slice(&src.octets());
    ip[16..20].copy_from_slice(&dst.octets());
    let ip_csum = csum(&ip);
    ip[10..12].copy_from_slice(&ip_csum.to_be_bytes());
    let mut packet = ip;
    packet.extend_from_slice(&tcp);
    // SAFETY: a raw IP socket send of a self-built packet to a sockaddr_in.
    unsafe {{
        let fd = socket(AF_INET, SOCK_RAW, IPPROTO_RAW);
        // Fail loud: a raw socket the guest cannot open is not a silent no-op
        // (it would make the door probe vacuous). Exit non-zero.
        if fd < 0 {{ std::process::exit(21); }}
        let mut sa = [0u8; 16];
        sa[0] = AF_INET as u8;
        sa[2..4].copy_from_slice(&dport.to_be_bytes());
        sa[4..8].copy_from_slice(&dst.octets());
        let sent = sendto(fd, packet.as_ptr(), packet.len(), 0, sa.as_ptr(), 16);
        // Fail loud on any sendto error: a swallowed send is a vacuous probe.
        if sent < 0 {{ close(fd); std::process::exit(22); }}
        close(fd);
    }}
}}

fn main() {{
    // 1. Establish one intercepted connection to the peer by name, exchange a
    //    byte, and close it cleanly (guest FIN) so the host leg-F enters
    //    TIME_WAIT. The kernel picks the source port; read it back.
    // Fail loud on every establish failure — a silent return would leave no
    // TIME_WAIT entry and make the door probe vacuous with no signal.
    let args: Vec<String> = std::env::args().collect();
    let target = args.get(1).and_then(|target| target.to_socket_addrs().ok()).and_then(|mut addresses| addresses.next());
    let Some(nonce) = args.get(2) else {{ std::process::exit(23); }};
    // Binding a test-owned UDP receiver sends no frame. Its one host-to-guest
    // message later supplies values learned from the real FIN and releases
    // the already-prescribed post-fault reconnect, independently of leg F.
    let control = std::net::UdpSocket::bind(("0.0.0.0", {control_port})).unwrap();
    control.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let Some(target) = target else {{ std::process::exit(23); }};
    let Ok(mut stream) = TcpStream::connect_timeout(&target, Duration::from_secs(2)) else {{
        std::process::exit(24);
    }};
    // Fail loud if the source port cannot be read or is 0: a port-0 fallback
    // would craft reconnect SYNs that never match the leg-F TIME_WAIT 4-tuple,
    // so the door could not reopen regardless of the sequence gate and the
    // `!reopened` oracle would pass vacuously. The crafted SYNs must carry the
    // real TIME_WAIT source port.
    let sport = match stream.local_addr() {{
        Ok(local) if local.port() != 0 => local.port(),
        _ => std::process::exit(29),
    }};
    if stream.write_all(b"ND295-TW").is_err() {{ std::process::exit(27); }}
    let mut byte = [0u8; 1];
    // The peer closes without replying: EOF (Ok(0)) is the expected outcome. A
    // read error (a reset) means no clean close, so no TIME_WAIT entry forms;
    // fail loud rather than leave the door probe vacuous.
    if stream.read(&mut byte).is_err() {{ std::process::exit(28); }}
    drop(stream); // clean close from the guest side
    eprintln!("ND295-TW-CLOSED {{nonce}} {{sport}} {{target}}");
    let (dst, dport) = match target {{
        std::net::SocketAddr::V4(v4) => (*v4.ip(), v4.port()),
        std::net::SocketAddr::V6(_) => std::process::exit(25),
    }};
    // The guest's own address on its TAP.
    let Some(src) = local_ipv4() else {{ std::process::exit(26); }};
    // 2. The host first materializes the exact TIME_WAIT tuple, observes the
    //    actual guest FIN's sequence/TSval, and completes killed mode. Only
    //    then may the same guest make any crafted reconnect attempt.
    let mut command = [0_u8; 256];
    let (length, sender) = control.recv_from(&mut command).unwrap();
    if sender.ip().to_string() != "{gw}" {{ std::process::exit(23); }}
    let command = std::str::from_utf8(&command[..length]).unwrap();
    let fields: Vec<&str> = command.split_whitespace().collect();
    if fields.len() != 3 || fields[0] != nonce {{ std::process::exit(23); }}
    let seq_base: u32 = fields[1].parse().unwrap();
    let tsval_base: u32 = fields[2].parse().unwrap();
    eprintln!("ND295-TW-RECONNECT {{nonce}} {{sport}} {{seq_base}} {{tsval_base}}");
    let deadline = Instant::now() + Duration::from_secs({window});
    let mut n: u32 = 0;
    while Instant::now() < deadline {{
        craft_syn(src, sport, dst, dport, seq_base.wrapping_add(n * 4_000), tsval_base.wrapping_add(n));
        n += 1;
        std::thread::sleep(Duration::from_millis(200));
    }}
}}

fn local_ipv4() -> Option<Ipv4Addr> {{
    // The guest's single non-loopback IPv4, read from a UDP connect's local addr.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(("{gw}", 9)).ok()?;
    match sock.local_addr().ok()? {{
        std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
        std::net::SocketAddr::V6(_) => None,
    }}
}}
"#,
        gw = GATEWAY,
        window = PROBE_WINDOW.as_secs() + 20,
        control_port = TW_CONTROL_PORT,
    );
    build_static_binary(tmp, "nd295-tw-guest", &source)
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-64 — The TIME_WAIT side door is measured with both controls first
/// CONTRACT_SHAPE: bounded-change.
///
/// E14 (e), the guest door. A real mesh guest (deployed through serve + deploy)
/// opens an intercepted leg-F connection to a peer and closes it from the guest
/// side, so the host's leg-F socket holds the true `TIME_WAIT` substate. The
/// host then closes leg F (killed mode), and the guest reconnects from the same
/// source tuple with a newer sequence for the probe window. The host records
/// whether any reconnect is answered on the guest's TAP. **A reproduced SYN-ACK
/// is not absorbed: the body fails and surfaces the reopen to the user** (FD
/// § "[REF] Driven port — intercept element release, member convergence, boot
/// clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19
/// conditional on native RED)" (the `TIME_WAIT` side door's routing to the
/// user)). Its precondition — a Running mesh guest — needs the 05-03 fd handoff;
/// the door it records depends on R19 (08-01, conditional), which if withdrawn
/// makes every reconnect fall through to a drop, so it is marked 08-01.
#[tokio::test]
async fn a_guest_reconnect_into_its_leg_f_time_wait_entry_is_recorded_and_a_reopen_goes_to_the_user()
 {
    let _teardown = TeardownBound::arm();
    prove_time_wait_controls().await;
    // The killed-mode residue guard: installed before any host state, so its
    // Drop reaps both VMs and their TAPs/scopes/pins/tables on every exit path.
    let mut residue = KilledServerResidueGuard::install("S-ND295-64-door");
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-64e-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let tw_guest = build_time_wait_guest(tmp.path());
    let rootfs = stage_rootfs_with_extra_binaries(
        tmp.path(),
        &fixture,
        &[(&peer, "gti-peer"), (&tw_guest, "nd295-tw-guest")],
    );
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // The actual Service peer is polled to Running. Its allocation address
    // and the original named-Service destination are recorded separately.
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-64e-peer.toml",
        &service_toml(Path::new("/sbin/gti-peer"), &fixture.kernel_path, &rootfs),
    );
    let peer_out = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh peer service");
    let peer_running = poll_until_running(&cfg, &peer_out.workload_id, RUNNING_BOUND).await;
    let peer_addr = peer_running
        .snapshot
        .rows
        .first()
        .expect("one Running peer row")
        .workload_addr
        .expect("the peer publishes its registered destination address");

    // The E14 (e) `0.0.0.0:p` host wildcard listener: p is the original
    // destination port (SERVICE_PORT). If the door reproduces, the TCP_TW_SYN
    // handoff would deliver the reconnect to this listener; it must accept
    // nothing (fail-closed), and a SYN-ACK back to the guest is the reopen.
    let door_listener = WildcardListener::bind(SERVICE_PORT);
    let nonce = tmp.path().file_name().expect("test tempdir nonce").to_string_lossy().into_owned();
    // Preserve the named-Service journey. D is the address returned by the
    // real guest resolver, which can differ from the peer allocation address.
    let destination = format!("{MESH_NAME}:{SERVICE_PORT}");

    // The TIME_WAIT guest: establishing + reconnecting is its whole program.
    let guest_spec = write_toml(
        server_tmp.path(),
        "nd295-64e-guest.toml",
        &vm_job_toml(
            "nd295-tw",
            "/sbin/nd295-tw-guest",
            &[&destination, &nonce],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let guest_out = deploy(DeployArgs { spec: guest_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the TIME_WAIT guest");
    let running = poll_until_running(&cfg, &guest_out.workload_id, RUNNING_BOUND).await;
    let guest_addr = running
        .snapshot
        .rows
        .first()
        .expect("one Running guest row")
        .workload_addr
        .expect("the Running guest publishes its address");
    let guest_tap = tap_for(guest_addr);
    let guest_ifindex = interface_index(&guest_tap);
    let close_capture = WireCapture::start_link_layer(guest_ifindex);
    let alloc = AllocationId::new(&running.snapshot.rows.first().expect("guest row").alloc_id)
        .expect("server allocation ID");
    let console = VmRunDir::for_alloc(Path::new("/run/overdrive/vm"), &alloc).console_log();
    let marker = format!("ND295-TW-CLOSED {nonce} ");
    let mut source_port = None;
    let mut original_destination = None;
    assert!(
        wait_until(Duration::from_secs(30), || {
            let log = std::fs::read_to_string(&console).unwrap_or_default();
            for fields in log
                .lines()
                .filter_map(|line| line.strip_prefix(&marker))
                .map(|line| line.split_whitespace().collect::<Vec<_>>())
            {
                if fields.len() == 2 {
                    source_port = fields[0].parse::<u16>().ok().filter(|port| *port != 0);
                    original_destination = fields[1]
                        .parse::<SocketAddrV4>()
                        .ok()
                        .filter(|dest| dest.port() == SERVICE_PORT);
                }
            }
            source_port.is_some() && original_destination.is_some()
        })
        .await,
        "the actual guest reports its own completed close and nonzero source port; console={}",
        console.display()
    );
    let source_port = source_port.expect("the completed-close marker carries the true source port");
    let original_destination = original_destination
        .expect("the same actual guest reports its real DNS-resolved original destination");
    let original_addr = *original_destination.ip();

    // The guest establishes and closes its intercepted connection first; wait
    // for the host's leg-F socket to hold a TIME_WAIT entry keyed on the
    // ORIGINAL-destination 4-tuple. A TPROXY-accepted socket keeps the original
    // destination (original_addr:SERVICE_PORT) as its local address (IP_TRANSPARENT)
    // and the guest as its remote — NOT `127.0.0.1:leg_f` — so the entry is
    // matched on `sport = :SERVICE_PORT` toward the guest's address.
    let host_time_wait_present = wait_until(Duration::from_secs(30), || {
        Command::new("ss")
            .args([
                "-tan",
                "state",
                "time-wait",
                &format!("src {original_addr} and sport = :{SERVICE_PORT} and dst {guest_addr} and dport = :{source_port}"),
            ])
            .output()
            .is_ok_and(|out| {
                out.status.success()
                    && String::from_utf8_lossy(&out.stdout).contains(&guest_addr.to_string())
            })
    })
    .await;
    assert!(
        host_time_wait_present,
        "the guest's closed intercepted connection leaves the host in TIME_WAIT on the \
         original-destination 4-tuple ({original_destination} <- {guest_addr}:{source_port})"
    );
    let close_capture =
        close_capture.stop_accounted().expect("complete lossless close-sequence capture");
    let (fin_frame, fin) = close_capture
        .frames
        .iter()
        .find_map(|frame| {
            assert_eq!(
                frame.ifindex, guest_ifindex,
                "close evidence comes from the exact guest TAP"
            );
            assert!(
                !frame.truncated && !frame.control_truncated,
                "complete close and timestamp evidence"
            );
            if frame_ipv4_src(&frame.bytes) != Some(guest_addr) {
                return None;
            }
            let dst = frame.bytes.get(30..34)?;
            if dst != original_addr.octets() {
                return None;
            }
            tcp_segment_between(&frame.bytes, source_port, SERVICE_PORT)
                .filter(|segment| segment.flags & 1 != 0)
                .map(|segment| (frame, segment))
        })
        .expect("the guest's exact source tuple sends its FIN to the original destination");
    let ihl = usize::from(fin_frame.bytes[14] & 15) * 4;
    let tcp = &fin_frame.bytes[14 + ihl..];
    let tcp_header = usize::from(tcp[12] >> 4) * 4;
    let ip_total = usize::from(u16::from_be_bytes([fin_frame.bytes[16], fin_frame.bytes[17]]));
    let payload = ip_total.checked_sub(ihl + tcp_header).expect("valid FIN packet lengths");
    let rcv_nxt = fin.seq.wrapping_add(payload as u32).wrapping_add(1);
    let old_tsval =
        fin.tsval.expect("the actual FIN carries the established connection's timestamp");
    let seq_base = rcv_nxt.wrapping_add(100_000).max(TW_CRAFT_SEQ_BASE);
    let tsval_base = old_tsval.wrapping_add(1_000).max(TW_CRAFT_TSVAL_BASE);
    assert!(
        (seq_base.wrapping_sub(rcv_nxt) as i32) > 0,
        "the crafted sequence is newer under TCP's modulo comparison"
    );
    assert!(
        (tsval_base.wrapping_sub(old_tsval) as i32) > 0,
        "the crafted timestamp is newer than the actual FIN's TSval"
    );
    assert!(tap_is_up(&guest_tap), "the completed-close witness runs on the live production TAP");
    assert_eq!(
        door_listener.accepted(),
        0,
        "the healthy intercepted connection never reaches the wildcard door"
    );
    door_listener.assert_healthy();
    let materialized_at = packet_clock_now();
    eprintln!(
        "S-ND295-64 true TIME_WAIT materialized: tuple={original_destination}<-{guest_addr}:{source_port}, peer_allocation={peer_addr}, ifindex={guest_ifindex}, fin_seq={}, rcv_nxt={rcv_nxt}, old_tsval={old_tsval}, seq_base={seq_base}, tsval_base={tsval_base}, at={materialized_at}",
        fin.seq
    );

    // One complete capture retains both the exact crafted request nonces and
    // their replies, over the same original full exposure window.
    let capture = WireCapture::start_link_layer(guest_ifindex);
    let program_before = assert_r19_program_live(guest_addr).await;
    residue.record_owned_program(
        &program_before,
        [peer_addr, guest_addr].into_iter().collect(),
        [SocketAddrV4::new(peer_addr, SERVICE_PORT)].into_iter().collect(),
    );
    // Close leg F from the host (killed mode) so the door is open per flow.
    kill_serve_owner(handle).await.expect("killed-mode serve abandons its owner");
    let fault_applied_at = packet_clock_now();
    assert!(tap_is_up(&guest_tap), "the guest's TAP stays up after killed mode");
    let program_at_fault = assert_r19_program_live(guest_addr).await;
    assert_eq!(
        program_at_fault, program_before,
        "the actual program, members, and policy route survive the guest-door fault"
    );
    let tw_after_fault = Command::new("ss").args(["-tan", "state", "time-wait", &format!("src {original_addr} and sport = :{SERVICE_PORT} and dst {guest_addr} and dport = :{source_port}")]).output().expect("read the exact TIME_WAIT tuple after killed mode");
    assert!(
        tw_after_fault.status.success()
            && String::from_utf8_lossy(&tw_after_fault.stdout).contains(&guest_addr.to_string()),
        "the same true TIME_WAIT entry survives killed mode before any reconnect"
    );
    let control = UdpSocket::bind(SocketAddrV4::new(GATEWAY, 0))
        .expect("bind the test-owned host event gate");
    let command = format!("{nonce} {seq_base} {tsval_base}");
    let released_at = packet_clock_now();
    let sent =
        control.send_to(command.as_bytes(), SocketAddrV4::new(guest_addr, TW_CONTROL_PORT)).expect(
            "release the prescribed reconnect only after materialized TIME_WAIT and killed mode",
        );
    assert_eq!(sent, command.len(), "the complete test-private release reaches the guest");

    // The guest keeps reconnecting for the window.
    tokio::time::sleep(PROBE_WINDOW).await;
    let capture = capture.stop_accounted().expect("complete lossless guest-door exposure capture");
    assert!(
        !capture.interface_removed && capture.link_down_reports.is_empty(),
        "the same production TAP stays live over the full guest-door window"
    );

    // In-run witness: the guest's crafted newer-sequence SYNs (base seq
    // TW_CRAFT_SEQ_BASE, above the old rcv_nxt) reached its own TAP while it was
    // up — the door probe is not vacuous.
    assert!(tap_is_up(&guest_tap), "the run is void, not GREEN: the guest's TAP was quiesced");
    let mut requests = BTreeSet::new();
    let mut crafted = 0_usize;
    let mut pre_fault_crafted = 0_usize;
    for frame in &capture.frames {
        assert_eq!(
            frame.ifindex, guest_ifindex,
            "all reconnect evidence names the same TAP ifindex"
        );
        assert!(
            !frame.truncated && !frame.control_truncated,
            "complete reconnect frame/clock evidence"
        );
        let at = frame.kernel_event_at.expect("the reconnect frame has its kernel event time").0;
        if let Some(request) = probe_request(&frame.bytes, guest_addr, SERVICE_PORT) {
            if request.0 == original_addr
                && request.1 == source_port
                && request.2 >= TW_CRAFT_SEQ_BASE
            {
                if at < fault_applied_at {
                    pre_fault_crafted += 1;
                    continue;
                }
                assert!(
                    at >= released_at,
                    "the guest makes no reconnect before the post-fault release"
                );
                assert!(
                    (request.2.wrapping_sub(rcv_nxt) as i32) > 0,
                    "every observed reconnect is newer than the original rcv_nxt"
                );
                let segment = tcp_segment_between(&frame.bytes, source_port, SERVICE_PORT)
                    .expect("exact reconnect TCP tuple");
                assert!(
                    segment.tsval.is_some_and(|ts| (ts.wrapping_sub(old_tsval) as i32) > 0),
                    "each crafted reconnect carries a newer timestamp"
                );
                requests.insert(request);
                crafted += 1;
            }
        }
    }
    assert_eq!(
        pre_fault_crafted, 0,
        "zero crafted reconnects precede the required listener-loss fault"
    );
    assert!(
        crafted >= 1,
        "the guest's crafted newer-sequence reconnect SYNs reached its TAP: captured {crafted} >= 1"
    );

    // Record whether the reconnect is answered — a SYN-ACK from the original
    // destination back to the guest is the door reproduced.
    let mut reopened = false;
    for frame in &capture.frames {
        if frame.kernel_event_at.expect("the reply carries its kernel time").0 >= released_at {
            reopened |= probe_reply(&frame.bytes, SERVICE_PORT, &requests).is_some();
        }
    }
    eprintln!(
        "S-ND295-64 guest leg-F TIME_WAIT door: reopened={reopened} (SYN-ACK from \
         {original_destination} on {guest_tap}); door listener accepts={}",
        door_listener.accepted()
    );
    eprintln!(
        "S-ND295-64 ordered door evidence: materialized_at={materialized_at}, fault_at={fault_applied_at}, release_at={released_at}, crafted={crafted}, pre_fault_crafted={pre_fault_crafted}, frames={}, source_port={source_port}",
        capture.frames.len()
    );
    door_listener.assert_healthy();
    let program_after = assert_r19_program_live(guest_addr).await;
    assert_eq!(
        program_after, program_before,
        "the actual guest-door program and members stay unchanged through the full window"
    );
    assert_eq!(
        door_listener.accepted(),
        0,
        "the `0.0.0.0:{SERVICE_PORT}` host wildcard listener accepts no reopened reconnect"
    );
    assert!(
        !reopened,
        "the guest's newer-sequence reconnect into its leg-F TIME_WAIT entry was answered with a \
         SYN-ACK: the door reproduced and is surfaced to the user, not absorbed (E14 (e))"
    );

    // `_residue` reaps both abandoned VMs on drop — no `serve` reboot, no
    // `let _ = stop`.
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
/// dst_port, seq, tsval|none]`, sends exactly one crafted SYN through
/// `AF_INET`/`SOCK_RAW`/`IP_HDRINCL`, computing the IP and TCP checksums. Run
/// in the peer namespace via `ip netns exec`, it lets the controls choose the
/// sequence number (stale vs newer) and the timestamp option a normal stack
/// cannot.
fn build_raw_syn_crafter(tmp: &Path) -> PathBuf {
    // Built by bare `rustc` (`build_static_binary`), which links no crates:
    // the four libc symbols and the x86_64 Linux constants the crafter needs
    // are declared here, and `std` already links the C library.
    let source = r#"
use std::net::Ipv4Addr;
const AF_INET: i32 = 2;
const SOCK_RAW: i32 = 3;
const IPPROTO_IP: i32 = 0;
const IPPROTO_TCP: i32 = 6;
const IP_HDRINCL: i32 = 3;
#[repr(C)]
struct SockaddrIn { sin_family: u16, sin_port: u16, sin_addr: u32, sin_zero: [u8; 8] }
extern "C" {
    fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
    fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const SockaddrIn, addr_len: u32) -> isize;
    fn close(fd: i32) -> i32;
}
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
    let tsval: Option<u32> = if a[6] == "none" { None } else { Some(a[6].parse().unwrap()) };
    let tcp_len = if tsval.is_some() { 32 } else { 20 };
    // TCP header (+ optional timestamp option).
    let mut tcp = vec![0u8; tcp_len];
    tcp[0..2].copy_from_slice(&sport.to_be_bytes());
    tcp[2..4].copy_from_slice(&dport.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    tcp[12] = ((tcp_len / 4) as u8) << 4; // data offset
    tcp[13] = 0x02; // SYN
    tcp[14..16].copy_from_slice(&64240u16.to_be_bytes()); // window
    if let Some(tsval) = tsval {
        tcp[20] = 8; tcp[21] = 10; // TS option kind=8 len=10
        tcp[22..26].copy_from_slice(&tsval.to_be_bytes()); // TSval
        // TSecr (26..30) stays 0 on a SYN; two NOPs pad the header to 32.
        tcp[30] = 1; tcp[31] = 1;
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
        let fd = socket(AF_INET, SOCK_RAW, IPPROTO_TCP);
        assert!(fd >= 0, "raw socket");
        let one: i32 = 1;
        let set = setsockopt(fd, IPPROTO_IP, IP_HDRINCL,
            std::ptr::from_ref(&one).cast(), std::mem::size_of::<i32>() as u32);
        assert!(set == 0, "IP_HDRINCL");
        let addr = SockaddrIn {
            sin_family: AF_INET as u16,
            sin_port: dport.to_be(),
            sin_addr: u32::from_ne_bytes(dst.octets()),
            sin_zero: [0; 8],
        };
        let sent = sendto(fd, packet.as_ptr(), packet.len(), 0,
            std::ptr::from_ref(&addr), std::mem::size_of::<SockaddrIn>() as u32);
        assert!(sent >= 0, "sendto");
        close(fd);
    }
}
"#;
    build_static_binary(tmp, "nd295-raw-syn", source)
}

/// Capture the TCP flags of the first frame `from_port → to_port` on `iface`
/// within `window`; `None` when nothing replied. Used to classify the
/// TIME_WAIT reply (SYN-ACK vs bare ACK vs none).
fn capture_first_tcp_flags(
    iface: &str,
    from_port: u16,
    to_port: u16,
    window: Duration,
) -> Option<u8> {
    capture_first_tcp_segment(iface, from_port, to_port, window, |_| true)
        .map(|segment| segment.flags)
}

/// The fields of one captured TCP segment the TIME_WAIT controls read.
#[derive(Debug, Clone, Copy)]
struct TcpSegment {
    flags: u8,
    seq: u32,
    /// The TSval of the segment's timestamp option, when it carries one.
    tsval: Option<u32>,
}

/// Capture the first TCP segment `from_port → to_port` on `iface` that
/// `wanted` accepts, within `window`; `None` when none arrived.
fn capture_first_tcp_segment(
    iface: &str,
    from_port: u16,
    to_port: u16,
    window: Duration,
    wanted: impl Fn(&TcpSegment) -> bool,
) -> Option<TcpSegment> {
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
        libc::bind(
            fd,
            std::ptr::from_ref(&address).cast(),
            libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>()).unwrap(),
        )
    };
    assert_eq!(bound, 0, "bind reply capture: {}", std::io::Error::last_os_error());
    let deadline = Instant::now() + window;
    let mut result = None;
    while Instant::now() < deadline {
        let mut frame = [0_u8; 2048];
        // SAFETY: live buffer; owned fd.
        let read =
            unsafe { libc::recv(fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT) };
        if read <= 0 {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let length = usize::try_from(read).unwrap();
        if let Some(segment) = tcp_segment_between(&frame[..length], from_port, to_port)
            && wanted(&segment)
        {
            result = Some(segment);
            break;
        }
    }
    // SAFETY: owned fd.
    unsafe { libc::close(fd) };
    result
}

/// The flags, sequence number, and timestamp TSval of an Ethernet/IPv4/TCP
/// frame whose source port is `from_port` and destination port `to_port`,
/// else `None`.
fn tcp_segment_between(frame: &[u8], from_port: u16, to_port: u16) -> Option<TcpSegment> {
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
    if src != from_port || dst != to_port {
        return None;
    }
    let seq = u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]);
    let data_offset = usize::from(tcp[12] >> 4) * 4;
    let options = tcp.get(20..data_offset.min(tcp.len())).unwrap_or(&[]);
    let mut tsval = None;
    let mut at = 0;
    while at < options.len() {
        match options[at] {
            0 => break,
            1 => at += 1,
            kind => {
                let Some(&length) = options.get(at + 1) else { break };
                let length = usize::from(length);
                if length < 2 || at + length > options.len() {
                    break;
                }
                if kind == 8 && length == 10 {
                    tsval = Some(u32::from_be_bytes([
                        options[at + 2],
                        options[at + 3],
                        options[at + 4],
                        options[at + 5],
                    ]));
                }
                at += length;
            }
        }
    }
    Some(TcpSegment { flags: tcp[13], seq, tsval })
}

/// A peer client host binary: `argv = [src_ip, src_port, dst_ip, dst_port]`.
/// It binds the exact source 4-tuple end, connects, writes one byte, waits
/// 300 ms so the host listener closes its accepted socket first, then closes.
/// Built by bare `rustc`, so it declares the libc symbols it calls.
fn build_peer_client(tmp: &Path) -> PathBuf {
    let source = r#"
use std::net::Ipv4Addr;
const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const SOL_SOCKET: i32 = 1;
const SO_REUSEADDR: i32 = 2;
#[repr(C)]
struct SockaddrIn { sin_family: u16, sin_port: u16, sin_addr: u32, sin_zero: [u8; 8] }
extern "C" {
    fn socket(domain: i32, kind: i32, protocol: i32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
    fn bind(fd: i32, addr: *const SockaddrIn, len: u32) -> i32;
    fn connect(fd: i32, addr: *const SockaddrIn, len: u32) -> i32;
    fn write(fd: i32, buf: *const u8, len: usize) -> isize;
    fn close(fd: i32) -> i32;
}
fn addr(ip: Ipv4Addr, port: u16) -> SockaddrIn {
    SockaddrIn { sin_family: AF_INET as u16, sin_port: port.to_be(), sin_addr: u32::from_ne_bytes(ip.octets()), sin_zero: [0; 8] }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = addr(a[1].parse().unwrap(), a[2].parse().unwrap());
    let dst = addr(a[3].parse().unwrap(), a[4].parse().unwrap());
    let len = std::mem::size_of::<SockaddrIn>() as u32;
    // SAFETY: one owned TCP socket; every pointer is to a live local.
    unsafe {
        let fd = socket(AF_INET, SOCK_STREAM, 0);
        assert!(fd >= 0, "socket");
        let one: i32 = 1;
        assert!(setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, std::ptr::from_ref(&one).cast(), 4) == 0, "SO_REUSEADDR");
        assert!(bind(fd, &src, len) == 0, "bind the fixed source port");
        assert!(connect(fd, &dst, len) == 0, "connect");
        assert!(write(fd, b"x".as_ptr(), 1) == 1, "write");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(close(fd) == 0, "close");
    }
}
"#;
    build_static_binary(tmp, "nd64-peer-client", source)
}

/// The host's `TIME_WAIT` entry for the peer 4-tuple, as learned from the
/// peer's FIN on the wire: the entry's `rcv_nxt` is that FIN's sequence plus
/// one, and its `ts_recent` is that FIN's TSval.
#[derive(Debug, Clone, Copy)]
struct TimeWaitEntry {
    peer: SocketAddrV4,
    host: SocketAddrV4,
    rcv_nxt: u32,
    peer_tsval: Option<u32>,
}

/// A test-owned veth peer namespace + a host listener, on a `TestCidrLease`
/// CIDR the TPROXY program never handles, for the TIME_WAIT controls.
struct TimeWaitTopology {
    peer_ns: String,
    host_gateway: Ipv4Addr,
    workload_addr: Ipv4Addr,
    host_port: u16,
    crafter: PathBuf,
    peer_client: PathBuf,
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
        let peer_client = build_peer_client(tmp.path());
        // Build the veth pair + peer namespace + host route on the leased CIDR.
        run(["ip", "netns", "add", peer_ns]);
        let host_if = format!("{peer_ns}-h");
        let peer_if = format!("{peer_ns}-p");
        run(["ip", "link", "add", &host_if, "type", "veth", "peer", "name", &peer_if]);
        run(["ip", "link", "set", &peer_if, "netns", peer_ns]);
        run(["ip", "addr", "add", &format!("{}/24", lease.host_gateway()), "dev", &host_if]);
        run(["ip", "link", "set", &host_if, "up"]);
        run([
            "ip",
            "-n",
            peer_ns,
            "addr",
            "add",
            &format!("{}/24", lease.workload_addr()),
            "dev",
            &peer_if,
        ]);
        run(["ip", "-n", peer_ns, "link", "set", &peer_if, "up"]);
        run(["ip", "-n", peer_ns, "link", "set", "lo", "up"]);
        Self {
            peer_ns: peer_ns.to_owned(),
            host_gateway: lease.host_gateway(),
            workload_addr: lease.workload_addr(),
            host_port: HOST_WILDCARD_PORT + 1,
            crafter,
            peer_client,
            _tmp: tmp,
            _host_listener: WildcardListener::bind(HOST_WILDCARD_PORT + 1),
        }
    }

    /// Establish one connection from the peer namespace to the host listener,
    /// close the host's accepted socket first and the peer second, so the host
    /// holds the true `TIME_WAIT` substate. Returns that entry, with the
    /// `rcv_nxt` and `ts_recent` read from the peer's FIN on the host veth.
    fn establish_and_leave_host_in_time_wait(&self) -> TimeWaitEntry {
        let peer = SocketAddrV4::new(self.workload_addr, PEER_SRC_PORT);
        let host = SocketAddrV4::new(self.host_gateway, self.host_port);
        // The peer's FIN is the last segment it sends; capture it concurrently.
        let iface = format!("{}-h", self.peer_ns);
        let fin = std::thread::spawn(move || {
            capture_first_tcp_segment(
                &iface,
                PEER_SRC_PORT,
                host.port(),
                Duration::from_secs(3),
                |segment| segment.flags & 0x01 != 0,
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        // The peer client binds the fixed source port and dials the host
        // listener; the WildcardListener closes its accepted socket first, then
        // this client closes (peer close second), leaving the host in TIME_WAIT.
        let status = Command::new("ip")
            .args([
                "netns",
                "exec",
                &self.peer_ns,
                &self.peer_client.to_string_lossy(),
                &peer.ip().to_string(),
                &peer.port().to_string(),
                &host.ip().to_string(),
                &host.port().to_string(),
            ])
            .status()
            .expect("peer-namespace client dials the host listener");
        assert!(status.success(), "the peer namespace establishes and closes the connection");
        let fin = fin
            .join()
            .expect("peer FIN capture thread")
            .expect("the peer's FIN reaches the host veth");
        // Let the final ACK settle so the host holds TIME_WAIT.
        std::thread::sleep(Duration::from_millis(500));
        TimeWaitEntry { peer, host, rcv_nxt: fin.seq.wrapping_add(1), peer_tsval: fin.tsval }
    }

    /// Send one crafted reconnect SYN from the peer 4-tuple with the given
    /// sequence shape, and classify the host reply captured on the host veth.
    fn probe_reconnect(&self, entry: TimeWaitEntry, seq: ReconnectSeq) -> ReconnectReply {
        let (src, dst) = (entry.peer, entry.host);
        // Sequence comparisons are modulo 2^32, so "below" and "above" are
        // offsets from the entry's own rcv_nxt, well inside a half window.
        let (seq_num, tsval) = match seq {
            // Stale ISN below the old rcv_nxt, no timestamp option.
            ReconnectSeq::StaleIsn => (entry.rcv_nxt.wrapping_sub(100_000), None),
            // A sequence above the old rcv_nxt, with a timestamp newer than
            // the entry's ts_recent so PAWS accepts it.
            ReconnectSeq::NewerSeq => (
                entry.rcv_nxt.wrapping_add(100_000),
                entry.peer_tsval.map(|tsval| tsval.wrapping_add(1_000)),
            ),
        };
        let seq_num = seq_num.to_string();
        let tsval = tsval.map_or_else(|| "none".to_owned(), |tsval| tsval.to_string());
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
                "netns",
                "exec",
                &self.peer_ns,
                &self.crafter.to_string_lossy(),
                &src.ip().to_string(),
                &src.port().to_string(),
                &dst.ip().to_string(),
                &dst.port().to_string(),
                &seq_num,
                &tsval,
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
