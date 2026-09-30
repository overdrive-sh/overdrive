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
//! Most bodies observe R18/R19 fail-closure, which is not on the production
//! path until DELIVER step 08-01 decides R18/R19 from this native RED, so each
//! of those is `#[ignore = "pending DELIVER step 08-01 (S-ND295-xx)"]` and
//! fails today for the right reason (its Running precondition, gap 7, the 05-03
//! fd handoff). The one exception is
//! [`both_time_wait_controls_prove_the_substate_and_sequence_gates`]: it is a
//! door-independent kernel pin on a test-owned veth peer namespace under
//! `TestCidrLease`, needs no guest and no production change beyond what exists,
//! and is therefore ACTIVE (no marker) — it must pass today.
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

use std::collections::BTreeSet;
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
use overdrive_netlink::nft::bridge::{BridgeGuardSpec, observe as observe_bridge_guard};
use overdrive_testing::cidr_lease::TestCidrLease;
use overdrive_testing::vm_fixture::VmFixture;

use super::guest_stack_mtls_egress::{
    MESH_NAME, SERVICE_PORT, build_mesh_peer, build_static_binary, observe_shared_intercept_state,
    service_toml,
};
use super::serve_lifetime_support::kill_serve_owner;
use super::vm_walking_skeleton::{
    TeardownBound, config_path, guard_default_drop_packets, poll_until_running,
    poll_until_terminal, shared_staging_root, spawn_vm_server_mtls_composed,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
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
/// above any plausible old `rcv_nxt`, so the door's in-run witness can require
/// a captured SYN whose sequence is at or above it (E14 (e): "a sequence above
/// the old `rcv_nxt`"). Shared between the guest source and the witness.
const TW_CRAFT_SEQ_BASE: u32 = 0x5000_0000;
/// The base TSval the TIME_WAIT guest stamps its crafted reconnect SYNs with —
/// newer than the entry's `ts_recent`, so PAWS accepts the reconnect.
const TW_CRAFT_TSVAL_BASE: u32 = 0x0100_0000;
/// The cgroup slice every VM allocation's scope lives under; the killed-server
/// residue guard reaps the scopes that appeared under it.
const WORKLOADS_SLICE: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";
/// The bpffs directory the production TCX loader pins its per-endpoint links
/// under; the residue guard removes the pins that appeared in it.
const TCX_LINK_PIN_DIR: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/links";

// ---------------------------------------------------------------------------
// Probe guest and mesh service
// ---------------------------------------------------------------------------

/// Build a guest binary that dials, in a loop for the whole probe window, the
/// mesh peer by name and the gateway, the real host interface address, and an
/// external address at `HOST_WILDCARD_PORT`, so the host can observe whether
/// any SYN is answered or forwarded while a fault holds. Every dial is a
/// plaintext `TcpStream` with a short connect timeout (the workload is
/// identity-unaware). `host` is the real host interface address the R18 (b)
/// "another host interface address" case dials; it is read from the kernel at
/// test time (never a literal).
fn build_syn_probe_guest(tmp: &Path, host: Ipv4Addr) -> PathBuf {
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
        dial("{host}:{port}");
        dial("{ext}:{port}");
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

    /// Drain every queued frame, counting IPv4 TCP SYNs to `self.dst_port`
    /// whose IPv4 source address is `source` — a forwarded guest frame on the
    /// interface this capture is bound to (E14 (a): zero of these on the peer's
    /// TAP once the intercept program is gone).
    fn drain_forwarded_syns(&self, source: Ipv4Addr) -> usize {
        let mut matched = 0;
        loop {
            let mut frame = [0_u8; 2048];
            // SAFETY: `frame` is a live writable buffer; `self.fd` is owned.
            let read = unsafe {
                libc::recv(self.fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                let slice = &frame[..length];
                if is_syn_to_port(slice, self.dst_port) && frame_ipv4_src(slice) == Some(source) {
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

    /// Drain every queued frame, counting IPv4 TCP SYNs to `self.dst_port`
    /// whose TCP sequence number is at or above `seq_floor` — the guest door's
    /// in-run witness that the crafted newer-sequence reconnect SYNs (base
    /// [`TW_CRAFT_SEQ_BASE`], above the old `rcv_nxt`) reached the guest's TAP.
    fn drain_newer_seq_syns(&self, seq_floor: u32) -> usize {
        let mut matched = 0;
        loop {
            let mut frame = [0_u8; 2048];
            // SAFETY: `frame` is a live writable buffer; `self.fd` is owned.
            let read = unsafe {
                libc::recv(self.fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                let slice = &frame[..length];
                if is_syn_to_port(slice, self.dst_port)
                    && syn_seq_to_port(slice, self.dst_port).is_some_and(|seq| seq >= seq_floor)
                {
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

/// The TCP sequence number of an Ethernet/IPv4/TCP SYN to `dst_port`, else
/// `None`.
fn syn_seq_to_port(frame: &[u8], dst_port: u16) -> Option<u32> {
    if frame.len() < 14 + 20 + 20 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
        return None;
    }
    let ihl = usize::from(frame[14] & 0x0f) * 4;
    if frame[23] != 0x06 || frame.len() < 14 + ihl + 20 {
        return None;
    }
    let tcp = &frame[14 + ihl..];
    if u16::from_be_bytes([tcp[2], tcp[3]]) != dst_port {
        return None;
    }
    Some(u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]))
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
    let port = u16::from_be_bytes([tcp[2], tcp[3]]);
    let flags = tcp[13];
    port == src_port && (flags & 0x02) != 0 && (flags & 0x10) != 0
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
}

impl KilledServerResidueGuard {
    fn install(label: &str) -> Self {
        Self { baseline: HostResidue::capture(), label: label.to_owned() }
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
#[ignore = "pending DELIVER step 08-01 (S-ND295-62)"]
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
    let peer_rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // A mesh Service (the peer) — polled to Running so the host learns the
    // peer's guest address and its production TAP for the E14 (a) capture.
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-62-peer.toml",
        &service_toml(&peer, &fixture.kernel_path, &peer_rootfs),
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
    let guest =
        deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;
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
    let peer_capture = SynCapture::open(&peer_tap, SERVICE_PORT);
    let witness = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let synack = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
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
    listener.assert_healthy();
    // E14 (a): zero forwarded intercept-marked frames on the peer's TAP; the
    // peer TAP being up is the live-capture witness (the guest-side ingress
    // witness below proves the guest is sending).
    assert!(tap_is_up(&peer_tap), "E14 (a) live-capture witness: the peer's TAP {peer_tap} is up");
    assert_eq!(
        peer_capture.drain_forwarded_syns(guest.addr),
        0,
        "E14 (a): no marked guest SYN is forwarded to the peer's TAP without the program"
    );
    // E14 (b): no SYN-ACK reaches the guest for the gateway or host-address dial.
    let replied =
        synack.syn_acks_seen(&[(GATEWAY, HOST_WILDCARD_PORT), (host_addr, HOST_WILDCARD_PORT)]);
    assert!(
        replied.is_empty(),
        "E14 (b): no SYN-ACK reaches the guest for a host-local marked dial without the program; \
         replies from {replied:?}"
    );
    // Per-run SYN-entered-host ingress witness on the guest's own TAP.
    assert_syn_entered_host(&guest.tap, &witness, intercept_before, guard_before, 1);

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
#[ignore = "pending DELIVER step 08-01 (S-ND295-62)"]
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
    let guest =
        deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

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

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "the intercept program alone still keeps a marked guest SYN out of any host listener"
    );
    listener.assert_healthy();
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
#[ignore = "pending DELIVER step 08-01 (S-ND295-63)"]
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
    let guest =
        deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    let leg_f = leg_f_port();
    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let witness = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let synack = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let accepts_before = listener.accepted();

    // Destroy the exact leg-F LISTENING tuple from outside (sock-diag destroy of
    // the transparent listener on loopback), assert the destroy found and killed
    // it, then immediately re-occupy its port with a plain non-transparent
    // listener (port-theft), holding the listener-absent state.
    destroy_listening_tuple(Ipv4Addr::LOCALHOST, leg_f);
    let _thief = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, leg_f))
        .expect("re-occupy the leg-F port with a plain listener (port-theft)");

    tokio::time::sleep(PROBE_WINDOW).await;
    assert_eq!(
        listener.accepted(),
        accepts_before,
        "no host wildcard listener accepts an outbound guest SYN once leg F is gone"
    );
    listener.assert_healthy();
    // No SYN-ACK reaches the guest for the gateway or external dial.
    let replied =
        synack.syn_acks_seen(&[(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)]);
    assert!(
        replied.is_empty(),
        "no SYN-ACK reaches the guest for an outbound marked dial once leg F is gone; replies from {replied:?}"
    );
    assert_syn_entered_host(&guest.tap, &witness, intercept_before, guard_before, 1);

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
    let _teardown = TeardownBound::arm();
    let host_addr = host_interface_address();
    // Installed before any host state is created, so its Drop reaps exactly the
    // killed server's residue (live VM, TAP, cgroup scope, bpffs pins, nft
    // tables) on every exit path, including the Running-precondition panic. No
    // second `serve` boot is relied on for cleanup.
    let _residue = KilledServerResidueGuard::install("S-ND295-63d");
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
    let guest =
        deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    let intercept_before = intercept_counter();
    let guard_before = bridge_guard_drop_packets();
    let witness = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
    let synack = SynCapture::open(&guest.tap, HOST_WILDCARD_PORT);
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
    listener.assert_healthy();
    let replied =
        synack.syn_acks_seen(&[(GATEWAY, HOST_WILDCARD_PORT), (EXTERNAL, HOST_WILDCARD_PORT)]);
    assert!(
        replied.is_empty(),
        "no SYN-ACK reaches the guest for an outbound marked dial after killed mode; replies from {replied:?}"
    );
    assert_syn_entered_host(&guest.tap, &witness, intercept_before, guard_before, 1);
    // The abandoned server left the VM live; `_residue` reaps it (and every
    // other killed-server artifact) on drop — no `serve` reboot, no `let _ =`.
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-63 — Guest TCP to an absent listener fails closed
/// CONTRACT_SHAPE: bounded-change.
///
/// Inbound control (E14: "leg C closed; SYN to a registered destination is
/// dropped by rule 4 under both orders"). A host-originated SYN would traverse
/// OUTPUT, not PREROUTING, so it cannot test rule 4; this is a GUEST-originated
/// SYN from the probe VM, through its TAP, to a PEER VM's registered inbound
/// destination (the peer Service's declared TCP port on its guest address, via
/// the mesh name). Leg C is made absent by a sock-diag destroy of its exact
/// loopback listening tuple (verified killed) whose port is immediately
/// occupied by a plain, non-transparent listener (the S-ND295-31B port-theft
/// shape). Oracle: no SYN-ACK from the peer's registered destination reaches
/// the probe's TAP, the peer's TAP carries no forwarded SYN, and the plain
/// listener accepts nothing. Runs under the rule order the program has at this
/// step; the R19 order (rule 3 → rule 4) is exercised once 08-01 lands R19.
#[tokio::test]
#[ignore = "pending DELIVER step 08-01 (S-ND295-63)"]
async fn inbound_tcp_to_a_closed_listener_is_dropped() {
    let _teardown = TeardownBound::arm();
    let host_addr = host_interface_address();
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-63i-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let probe = build_syn_probe_guest(tmp.path(), host_addr);
    let peer_rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let probe_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &probe, "nd295-syn-probe");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // The peer Service (the registered inbound destination).
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-63i-peer.toml",
        &service_toml(&peer, &fixture.kernel_path, &peer_rootfs),
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

    // The probe VM dials the peer's registered destination by mesh name.
    let guest =
        deploy_probe_guest(&cfg, server_tmp.path(), &fixture.kernel_path, &probe_rootfs).await;

    // Make leg C absent: destroy its exact loopback listening tuple, then
    // occupy its port with a plain non-transparent listener (port-theft).
    let leg_c = leg_c_port();
    destroy_listening_tuple(Ipv4Addr::LOCALHOST, leg_c);
    let thief = WildcardListener::bind(leg_c);

    let peer_capture = SynCapture::open(&peer_tap, SERVICE_PORT);
    let probe_synack = SynCapture::open(&guest.tap, SERVICE_PORT);
    let thief_accepts_before = thief.accepted();

    tokio::time::sleep(PROBE_WINDOW).await;
    // The peer's workload never receives the forwarded SYN.
    assert!(tap_is_up(&peer_tap), "the peer's TAP {peer_tap} is up (live-capture witness)");
    assert_eq!(
        peer_capture.drain_forwarded_syns(guest.addr),
        0,
        "no guest SYN is forwarded to the peer's registered destination with leg C closed"
    );
    // No SYN-ACK from the peer's registered destination reaches the probe.
    assert!(
        !probe_synack.syn_ack_from(peer_addr, SERVICE_PORT),
        "no SYN-ACK from the registered destination reaches the probe with leg C closed"
    );
    // The plain port-theft listener on leg C accepts nothing.
    assert_eq!(
        thief.accepted(),
        thief_accepts_before,
        "the plain leg-C port-theft listener accepts no inbound guest SYN"
    );
    thief.assert_healthy();

    stop_and_await_terminal(&cfg, &guest.workload_id).await;
    stop_and_await_terminal(&cfg, &peer_out.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
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

    drop(topology);
    drop(lease);
}

/// Build the guest program that reproduces the E14 (e) guest side: it dials the
/// mesh peer by name (an intercepted leg-F flow), completes and cleanly closes
/// that connection from inside the guest — so the host's leg-F side enters the
/// true `TIME_WAIT` substate — recording its source port, then for the whole
/// probe window raw-crafts newer-sequence SYNs from that same source 4-tuple to
/// the original destination. The host closes leg F meanwhile; a reconnect that
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
    let target = ("{mesh}", {svc}).to_socket_addrs().ok().and_then(|mut a| a.next());
    let Some(target) = target else {{ std::process::exit(23); }};
    let Ok(mut stream) = TcpStream::connect_timeout(&target, Duration::from_secs(2)) else {{
        std::process::exit(24);
    }};
    let sport = stream.local_addr().map(|a| a.port()).unwrap_or(0);
    if stream.write_all(b"ND295-TW").is_err() {{ std::process::exit(27); }}
    let mut byte = [0u8; 1];
    // The peer closes without replying: EOF (Ok(0)) is the expected outcome. A
    // read error (a reset) means no clean close, so no TIME_WAIT entry forms;
    // fail loud rather than leave the door probe vacuous.
    if stream.read(&mut byte).is_err() {{ std::process::exit(28); }}
    drop(stream); // clean close from the guest side
    let (dst, dport) = match target {{
        std::net::SocketAddr::V4(v4) => (*v4.ip(), v4.port()),
        std::net::SocketAddr::V6(_) => std::process::exit(25),
    }};
    // The guest's own address on its TAP.
    let Some(src) = local_ipv4() else {{ std::process::exit(26); }};
    // 2. For the whole window, reconnect from the same source tuple with a
    //    sequence above the old rcv_nxt and a newer TSval. A base seq/TSval is
    //    fine: the host holds one TIME_WAIT entry and PAWS accepts a newer TS.
    let deadline = Instant::now() + Duration::from_secs({window});
    let mut n: u32 = 0;
    while Instant::now() < deadline {{
        craft_syn(src, sport, dst, dport, {seq_base}u32.wrapping_add(n * 4_000), {tsval_base}u32 + n);
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
        mesh = MESH_NAME,
        svc = SERVICE_PORT,
        gw = GATEWAY,
        window = PROBE_WINDOW.as_secs() + 20,
        seq_base = TW_CRAFT_SEQ_BASE,
        tsval_base = TW_CRAFT_TSVAL_BASE,
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
#[ignore = "pending DELIVER step 08-01 (S-ND295-64)"]
async fn a_guest_reconnect_into_its_leg_f_time_wait_entry_is_recorded_and_a_reopen_goes_to_the_user()
 {
    let _teardown = TeardownBound::arm();
    // The killed-mode residue guard: installed before any host state, so its
    // Drop reaps both VMs and their TAPs/scopes/pins/tables on every exit path.
    let _residue = KilledServerResidueGuard::install("S-ND295-64-door");
    let fixture = VmFixture::provision(&shared_staging_root()).expect("native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-64e-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let peer_rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &peer, "gti-peer");
    let tw_guest = build_time_wait_guest(tmp.path());
    let guest_rootfs =
        stage_rootfs_with_extra_binary(tmp.path(), &fixture, &tw_guest, "nd295-tw-guest");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    // The mesh peer the guest dials by name — polled to Running so the host
    // learns the peer's registered destination address (the crafted SYN's
    // destination and the wildcard listener's expected source).
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-64e-peer.toml",
        &service_toml(&peer, &fixture.kernel_path, &peer_rootfs),
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

    // The TIME_WAIT guest: establishing + reconnecting is its whole program.
    let guest_spec = write_toml(
        server_tmp.path(),
        "nd295-64e-guest.toml",
        &vm_job_toml("nd295-tw", "/sbin/nd295-tw-guest", &[], &fixture.kernel_path, &guest_rootfs),
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

    // The guest establishes and closes its intercepted connection first; wait
    // for the host's leg-F socket to hold a TIME_WAIT entry keyed on the
    // ORIGINAL-destination 4-tuple. A TPROXY-accepted socket keeps the original
    // destination (peer_addr:SERVICE_PORT) as its local address (IP_TRANSPARENT)
    // and the guest as its remote — NOT `127.0.0.1:leg_f` — so the entry is
    // matched on `sport = :SERVICE_PORT` toward the guest's address.
    let host_time_wait_present = wait_until(Duration::from_secs(30), || {
        Command::new("ss")
            .args([
                "-tan",
                "state",
                "time-wait",
                &format!("src {peer_addr} and sport = :{SERVICE_PORT} and dst {guest_addr}"),
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
         original-destination 4-tuple ({peer_addr}:{SERVICE_PORT} <- {guest_addr})"
    );

    // Two independent captures on the guest's TAP: one witnesses the crafted
    // newer-sequence SYNs entering the host, the other records a reopen SYN-ACK.
    let witness = SynCapture::open(&guest_tap, SERVICE_PORT);
    let reopen = SynCapture::open(&guest_tap, SERVICE_PORT);
    // Close leg F from the host (killed mode) so the door is open per flow.
    kill_serve_owner(handle).await.expect("killed-mode serve abandons its owner");
    assert!(tap_is_up(&guest_tap), "the guest's TAP stays up after killed mode");

    // The guest keeps reconnecting for the window.
    tokio::time::sleep(PROBE_WINDOW).await;

    // In-run witness: the guest's crafted newer-sequence SYNs (base seq
    // TW_CRAFT_SEQ_BASE, above the old rcv_nxt) reached its own TAP while it was
    // up — the door probe is not vacuous.
    assert!(tap_is_up(&guest_tap), "the run is void, not GREEN: the guest's TAP was quiesced");
    let crafted = witness.drain_newer_seq_syns(TW_CRAFT_SEQ_BASE);
    assert!(
        crafted >= 1,
        "the guest's crafted newer-sequence reconnect SYNs reached its TAP: captured {crafted} >= 1"
    );

    // Record whether the reconnect is answered — a SYN-ACK from the original
    // destination back to the guest is the door reproduced.
    let reopened = reopen.syn_ack_from(peer_addr, SERVICE_PORT);
    eprintln!(
        "S-ND295-64 guest leg-F TIME_WAIT door: reopened={reopened} (SYN-ACK from \
         {peer_addr}:{SERVICE_PORT} on {guest_tap}); door listener accepts={}",
        door_listener.accepted()
    );
    door_listener.assert_healthy();
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
