//! netns-density-295 native-metal fault evidence (S-ND295-30B / 66 / 67 / 69).
//!
//! Real Cloud Hypervisor guests driven through the production entry points
//! (`serve::run_with_kek` + `deploy`/`stop`), on a non-virtualized x86_64 host
//! under `cargo xtask metal run --`. Every fault enters through a real kernel
//! mutation (`ip link del`, `nft delete table`, `detach_pinned_link`, a pin
//! `rm`, `delete_member`, an out-of-VM `SIOCSIFHWADDR` on a `pidfd_getfd`-copied
//! TAP queue) or a resource the production path created; no test installs a
//! production effect the production path omits.
//!
//! These bodies observe the R14 per-VM kill scope, R5 teardown-converges-on-
//! absence, R21 host-side-MAC egress control, and R15 in-place program repair —
//! none of which is on the production path yet — so every body is
//! `#[ignore = "pending DELIVER step 10-02 (S-ND295-xx)"]` and fails today for
//! the right reason. Phase C runs them natively.
//!
//! The dialer is plaintext by design (CLAUDE.md § "East-west mTLS tests"): the
//! S-ND295-69 post-repair proof is a fresh natural mesh Job dial that reaches
//! `Terminated`/exit 0 on its own, never a test-side TLS client.
//!
//! # `#[serial(cgroup)]`
//!
//! Every body boots a real `overdrive serve` against the machine-global
//! `/sys/fs/cgroup/overdrive.slice` tree; the whole `overdrive-cli` integration
//! binary is already in the cross-process `host-kernel-shared` group, and the
//! in-process serialization matches `vm_walking_skeleton.rs`.

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
use std::net::Ipv4Addr;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::id::AllocationId;
use overdrive_core::vm::config::{OVERDRIVE_VMM_UID, VmRunDir};
use overdrive_dataplane::guest_tcx::{
    GuestTcxCounter, detach_pinned_link, endpoint_present, read_counter,
};
use overdrive_netlink::Client;
use overdrive_netlink::nft::bridge::{
    BridgeGuardObservation, BridgeGuardSpec, delete_member, observe as observe_bridge_guard,
};
use serial_test::serial;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use overdrive_testing::vm_fixture::VmFixture;

use super::guest_stack_mtls_egress::{
    build_mesh_guest, build_mesh_peer, observe_shared_intercept_state,
    poll_until_natural_job_completion, service_toml,
};
use super::vm_walking_skeleton::{
    build_spin_binary, config_path, poll_until_running, poll_until_terminal, shared_staging_root,
    spawn_vm_server_mtls_composed, stage_rootfs_with_extra_binaries,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

// ---------------------------------------------------------------------------
// Shared constants and small pure helpers
// ---------------------------------------------------------------------------

const SHARED_IPV4_INTERCEPT_TABLE: &str = "overdrive-mtls";
const PIN_ROOT: &str = "/sys/fs/bpf/overdrive/mtls-endpoints";
const HOST_ROUTE_DEVICE_BASE: &str = "/sys/class/net";
const VM_RUN_ROOT: &str = "/run/overdrive/vm";
/// One-second audit period plus its detection/quiescence budget headroom.
const ONE_AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// Bound for a real production boot to reach Running.
const RUNNING_BOUND: Duration = Duration::from_secs(90);
/// Bound for a stop or teardown to converge on an empty complement.
const TERMINAL_BOUND: Duration = Duration::from_secs(30);
/// The per-VM kill event (D-295-R14, FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (detection and the kill-scope table)) — pinned name/fields.
const VM_KILLED_EVENT: &str = "guest_network.shared_owner_vm_killed";
/// The node-level unhealthy detection event (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (detection), lib.rs:1538).
const UNHEALTHY_EVENT: &str = "guest_network.shared_owner_unhealthy";
/// The lease-released event (D-295-R7, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events)).
const LEASE_RELEASED_EVENT: &str = "guest_network.lease_released";
/// A distinct wire marker so a captured frame is provably one we injected.
const HIJACK_NEEDLE: &[u8] = b"ND295-67-HIJACK-IDENTIFIABLE";

/// The IP-derived production TAP name for a guest address (`ovd-tp-<4hex>`),
/// exactly as `vm_walking_skeleton` derives it from the last two octets.
fn tap_for(addr: Ipv4Addr) -> String {
    let octets = addr.octets();
    format!("ovd-tp-{:04x}", u16::from_be_bytes([octets[2], octets[3]]))
}

/// The deterministic guest virtio-net MAC for a guest address:
/// `[0x02, 0x00, a, b, c, d]` (`guest_network.rs:481`; increment-z).
fn guest_mac(addr: Ipv4Addr) -> [u8; 6] {
    let o = addr.octets();
    [0x02, 0x00, o[0], o[1], o[2], o[3]]
}

/// The live kernel ifindex of a real interface, or a panic if it is absent.
fn ifindex_of(iface: &str) -> u32 {
    let name = std::ffi::CString::new(iface).expect("interface name has no NUL");
    // SAFETY: `name` is a live NUL-terminated string for this call.
    let ifindex = unsafe { libc::if_nametoindex(name.as_ptr()) };
    assert_ne!(ifindex, 0, "interface {iface} is live");
    ifindex
}

fn tap_device_dir(tap: &str) -> PathBuf {
    Path::new(HOST_ROUTE_DEVICE_BASE).join(tap)
}

fn tap_exists(tap: &str) -> bool {
    tap_device_dir(tap).exists()
}

fn ingress_link_pin(tap: &str) -> PathBuf {
    Path::new(PIN_ROOT).join("links").join(format!("{tap}-ingress"))
}

fn egress_link_pin(tap: &str) -> PathBuf {
    Path::new(PIN_ROOT).join("links").join(format!("{tap}-egress"))
}

fn endpoint_map_pin() -> PathBuf {
    Path::new(PIN_ROOT).join("maps/endpoints")
}

fn counter_map_pin() -> PathBuf {
    Path::new(PIN_ROOT).join("maps/counters")
}

/// The canonical shared-bridge guard specification (identical to the one
/// `vm_walking_skeleton` and `serve_lifetime_fail_stop` observe).
fn canonical_guard() -> BridgeGuardSpec {
    BridgeGuardSpec::new(
        SHARED_IPV4_INTERCEPT_TABLE.to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical shared bridge guard specification")
}

/// The live bridge (master) a managed TAP is attached to.
fn bridge_of(tap: &str) -> String {
    let master = std::fs::read_link(tap_device_dir(tap).join("master"))
        .unwrap_or_else(|error| panic!("the managed TAP {tap} has a bridge master: {error}"));
    master
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .expect("bridge master has a UTF-8 interface name")
        .to_owned()
}

/// The live `cloud-hypervisor` pid whose argv references this allocation's run
/// directory, or `None` when no such process is present.
fn cloud_hypervisor_pid_for_alloc(alloc: &AllocationId) -> Option<u32> {
    let run_dir = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc);
    let needle = run_dir.path().to_string_lossy().into_owned();
    for entry_result in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc entry while locating {alloc}: {error}"),
        };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline = match std::fs::read(entry.path().join("cmdline")) {
            Ok(cmdline) => cmdline,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read /proc/{pid}/cmdline while locating {alloc}: {error}"),
        };
        let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
        if Path::new(&String::from_utf8_lossy(argv0).into_owned()).file_name()
            != Some(std::ffi::OsStr::new("cloud-hypervisor"))
        {
            continue;
        }
        if String::from_utf8_lossy(&cmdline).contains(&needle) {
            return Some(pid);
        }
    }
    None
}

/// Descriptor 3 of a Cloud Hypervisor process is its own TAP queue: the
/// `iff:` line of `/proc/<pid>/fdinfo/3` names `tap` (D-295-R3, FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (`CloudHypervisorVmm::create`: `VMM_TAP_QUEUE_FD`)). Any
/// read failure returns `false` so a vanished process reads as "no holder".
fn descriptor_three_holds_tap_queue(pid: u32, tap: &str) -> bool {
    let Ok(fdinfo) = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/3")) else {
        return false;
    };
    fdinfo.lines().find_map(|line| line.strip_prefix("iff:")).map(str::trim) == Some(tap)
}

// ---------------------------------------------------------------------------
// Supervisor tracing-event capture (thread-local; the composition and its
// supervisor task run on this `#[tokio::test]` current-thread runtime, so a
// thread-local default subscriber sees their events — the shape
// `serve_lifetime_fail_stop.rs` relies on).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CapturedEvent {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct EventFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for EventFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct SupervisorEvents {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl SupervisorEvents {
    fn snapshot(&self) -> Vec<CapturedEvent> {
        self.events.lock().expect("supervisor event lock").clone()
    }

    /// The set of allocation ids named by `shared_owner_vm_killed`, optionally
    /// filtered to one pinned `cause`.
    fn killed(&self, cause: Option<&str>) -> BTreeSet<String> {
        self.snapshot()
            .into_iter()
            .filter(|event| event.name == VM_KILLED_EVENT)
            .filter(|event| {
                cause.is_none_or(|c| event.fields.get("cause").map(String::as_str) == Some(c))
            })
            .filter_map(|event| event.fields.get("alloc").cloned())
            .collect()
    }

    /// The set of allocation ids named by `lease_released`.
    fn released(&self) -> BTreeSet<String> {
        self.snapshot()
            .into_iter()
            .filter(|event| event.name == LEASE_RELEASED_EVENT)
            .filter_map(|event| event.fields.get("alloc").cloned())
            .collect()
    }

    /// The ordered `component` values of every `shared_owner_unhealthy`.
    fn unhealthy_components(&self) -> Vec<String> {
        self.snapshot()
            .into_iter()
            .filter(|event| event.name == UNHEALTHY_EVENT)
            .filter_map(|event| event.fields.get("component").cloned())
            .collect()
    }

    fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for event in self.snapshot() {
            let _ = writeln!(out, "  {} {:?}", event.name, event.fields);
        }
        out
    }
}

impl<S: Subscriber> Layer<S> for SupervisorEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let name = event.metadata().name();
        if name != VM_KILLED_EVENT && name != UNHEALTHY_EVENT && name != LEASE_RELEASED_EVENT {
            return;
        }
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("supervisor event lock")
            .push(CapturedEvent { name: name.to_owned(), fields: visitor.fields });
    }
}

/// Install the capture on the current thread for the body's lifetime. The
/// returned guard restores the previous subscriber on drop.
fn capture_supervisor_events() -> (tracing::subscriber::DefaultGuard, SupervisorEvents) {
    let events = SupervisorEvents::default();
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::registry()
            .with(events.clone().with_filter(tracing_subscriber::filter::LevelFilter::INFO)),
    );
    (guard, events)
}

/// Poll a real-wall-clock predicate up to `bound`, returning whether it held.
async fn wait_until(bound: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Await the observed administrative state of a real link reaching `want`
/// (`Some(true)` up, `Some(false)` down, `None` absent) within `bound`.
async fn wait_link_state(client: &Client, tap: &str, want: Option<bool>, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        let observed = client.observe_link(tap).await.ok().flatten();
        if observed == want {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Deploy helpers
// ---------------------------------------------------------------------------

/// A Running VM allocation and the kernel identities its guest owns.
struct DeployedVm {
    workload_id: String,
    alloc: AllocationId,
    addr: Ipv4Addr,
    tap: String,
    ifindex: u32,
}

/// Deploy one long-lived spin VM and drive it to Running, returning its guest
/// identities. The caller owns cleanup.
async fn deploy_spin_vm(
    cfg: &Path,
    server_dir: &Path,
    kernel: &Path,
    rootfs: &Path,
    id: &str,
) -> DeployedVm {
    let spec = write_toml(
        server_dir,
        &format!("{id}.toml"),
        &vm_job_toml(id, "/sbin/spin", &[], kernel, rootfs),
    );
    let output = deploy(DeployArgs { spec, config_path: cfg.to_path_buf() })
        .await
        .expect("deploy spin VM through the production handler");
    let running = poll_until_running(cfg, &output.workload_id, RUNNING_BOUND).await;
    let row = running.snapshot.rows.first().expect("one Running allocation row");
    let alloc = AllocationId::new(&row.alloc_id).expect("allocation id parses");
    let addr = row.workload_addr.expect("a Running VM publishes its guest address");
    let tap = tap_for(addr);
    let ifindex = ifindex_of(&tap);
    DeployedVm { workload_id: output.workload_id, alloc, addr, tap, ifindex }
}

/// Stop a workload through the production verb and assert it reaches
/// `Terminated`. Used by RAII-shaped cleanup at the tail of every body.
async fn stop_and_await_terminal(cfg: &Path, workload_id: &str) {
    stop(StopArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
        .await
        .expect("stop the exact workload through the production verb");
    let terminal = poll_until_terminal(cfg, workload_id, TERMINAL_BOUND).await;
    assert_eq!(
        terminal.snapshot.rows.first().expect("a terminal allocation row").state,
        AllocStateWire::Terminated,
        "the stopped workload reaches Terminated",
    );
}

/// The empty-complement assertion R5 teardown must reach (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (teardown converges on absence)): no
/// TAP, no endpoint entry, no ingress or egress pin.
fn assert_empty_complement(vm: &DeployedVm) {
    assert!(!tap_exists(&vm.tap), "teardown removes the TAP {}", vm.tap);
    assert!(
        !endpoint_present(endpoint_map_pin(), vm.ifindex)
            .expect("observe endpoint map after teardown"),
        "teardown removes the endpoint entry for {}",
        vm.tap
    );
    assert!(!ingress_link_pin(&vm.tap).exists(), "teardown removes the ingress pin for {}", vm.tap);
    assert!(!egress_link_pin(&vm.tap).exists(), "teardown removes the egress pin for {}", vm.tap);
}

// ---------------------------------------------------------------------------
// Raw AF_PACKET wire injection and capture (host-transmit path + per-TAP read)
// ---------------------------------------------------------------------------

/// An AF_PACKET capture bound to one interface, counting frames that carry a
/// caller-chosen needle. Its Drop closes the socket.
struct NeedleCapture {
    fd: RawFd,
    needle: Vec<u8>,
}

impl NeedleCapture {
    fn open(interface: &str, needle: &[u8]) -> Self {
        const ETH_P_ALL: u16 = 0x0003;
        let ifindex = ifindex_of(interface);
        // SAFETY: AF_PACKET raw socket; the fd is owned by this NeedleCapture.
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                i32::from(ETH_P_ALL.to_be()),
            )
        };
        assert!(
            fd >= 0,
            "open AF_PACKET capture on {interface}: {}",
            std::io::Error::last_os_error()
        );
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
            // SAFETY: fd was returned by socket and is closed exactly here on bind failure.
            unsafe { libc::close(fd) };
            panic!("bind AF_PACKET capture on {interface}: {error}");
        }
        Self { fd, needle: needle.to_vec() }
    }

    /// Drain every queued frame, counting those carrying the needle.
    fn drain(&self) -> usize {
        let mut matched = 0;
        loop {
            let mut frame = [0_u8; 2048];
            // SAFETY: `frame` is a live writable buffer and `self.fd` is owned.
            let read = unsafe {
                libc::recv(self.fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                matched += usize::from(
                    frame[..length].windows(self.needle.len()).any(|w| w == self.needle),
                );
                continue;
            }
            if read == 0 {
                return matched;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return matched;
            }
            panic!("capture recv failed: {error}");
        }
    }
}

impl Drop for NeedleCapture {
    fn drop(&mut self) {
        // SAFETY: NeedleCapture exclusively owns this socket fd.
        unsafe { libc::close(self.fd) };
    }
}

/// Build a well-formed host->guest IPv4/UDP frame to `dst_mac`, carrying the
/// needle so a capture can attribute it to this test.
fn host_unicast_frame(dst_mac: [u8; 6], needle: &[u8]) -> Vec<u8> {
    // Arbitrary non-guest host source MAC (locally administered, not 0x02:0x00).
    let src_mac = [0x02, 0x0a, 0x00, 0x00, 0x00, 0x01];
    let payload_len = 8 + needle.len();
    let ip_total = 20 + payload_len;
    let mut frame = Vec::with_capacity(14 + ip_total);
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&src_mac);
    frame.extend_from_slice(&0x0800_u16.to_be_bytes());
    frame.extend_from_slice(&[0x45, 0]);
    frame.extend_from_slice(&(ip_total as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0, 0, 0, 64, 17, 0, 0]);
    frame.extend_from_slice(&[10, 0, 0, 1]); // arbitrary host source IP
    frame.extend_from_slice(&[10, 0, 0, 2]); // arbitrary host dest IP
    frame.extend_from_slice(&40_037_u16.to_be_bytes());
    frame.extend_from_slice(&19_037_u16.to_be_bytes());
    frame.extend_from_slice(&(payload_len as u16).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(needle);
    frame
}

/// Send `repeat` host-originated unicast frames to `dst_mac` out of the bridge
/// device (the host-transmit path, `br_dev_xmit`; increment-z substrate note).
fn inject_host_unicast(bridge_ifindex: u32, dst_mac: [u8; 6], needle: &[u8], repeat: usize) {
    const ETH_P_ALL: u16 = 0x0003;
    // SAFETY: AF_PACKET raw socket; closed at the end of this call.
    let fd = unsafe {
        libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            i32::from(ETH_P_ALL.to_be()),
        )
    };
    assert!(fd >= 0, "open AF_PACKET injector: {}", std::io::Error::last_os_error());
    // SAFETY: zero-initialised sockaddr_ll before its fields are set.
    let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
    address.sll_protocol = ETH_P_ALL.to_be();
    address.sll_ifindex = i32::try_from(bridge_ifindex).expect("bridge ifindex fits i32");
    address.sll_halen = 6;
    address.sll_addr[..6].copy_from_slice(&dst_mac);
    let frame = host_unicast_frame(dst_mac, needle);
    for _ in 0..repeat {
        // SAFETY: `frame` and `address` are live for this send; fd is owned.
        let sent = unsafe {
            libc::sendto(
                fd,
                frame.as_ptr().cast(),
                frame.len(),
                0,
                std::ptr::from_ref(&address).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll length fits socklen_t"),
            )
        };
        assert_eq!(
            sent,
            frame.len() as isize,
            "host-transmit sendto: {}",
            std::io::Error::last_os_error()
        );
    }
    // SAFETY: injector fd owned locally.
    unsafe { libc::close(fd) };
}

/// The `bridge fdb show` entries as `(mac, dev)` pairs.
fn bridge_fdb_entries() -> Vec<(String, String)> {
    let output =
        Command::new("bridge").args(["fdb", "show"]).output().expect("run bridge fdb show");
    assert!(
        output.status.success(),
        "bridge fdb show: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("bridge fdb output is UTF-8");
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let mac = fields.next()?.to_owned();
            let mut dev = None;
            let mut permanent = false;
            while let Some(token) = fields.next() {
                match token {
                    "dev" => dev = fields.next().map(str::to_owned),
                    "permanent" | "static" => permanent = true,
                    _ => {}
                }
            }
            // Retain the learned/dynamic entries plus a permanent-flag sentinel
            // in the dev column so a caller can tell them apart.
            dev.map(|dev| (mac, if permanent { format!("{dev} permanent") } else { dev }))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The out-of-VM MAC hijack: `pidfd_getfd` a copy of the attacker's TAP queue
// as root, drop to uid 4200 with no capabilities, then `SIOCSIFHWADDR` on the
// held queue (increment-z STEPs 4-6; the process runs outside the D-295-R22
// launch filter, modelling a change the filter does not see).
// ---------------------------------------------------------------------------

/// The result the forked hijack child reports back over its pipe.
struct HijackOutcome {
    /// `SIOCSIFHWADDR` return code observed with `CapEff=0` at uid 4200.
    ioctl_rc: i32,
    /// Needle frames the attacker read out of the held queue during the window.
    stolen: u32,
}

/// Matches the kernel `struct ifreq` byte-for-byte (name + a 24-byte union),
/// so `SIOCSIFHWADDR`'s fixed-size `copy_from_user(sizeof(struct ifreq))`
/// stays in bounds. For `ifr_hwaddr` the union's first 2 bytes are the
/// `sa_family` and the next 14 the `sa_data`; the tail is padding.
#[repr(C)]
struct HwAddrIfreq {
    name: [libc::c_char; libc::IFNAMSIZ],
    sa_family: libc::sa_family_t,
    sa_data: [u8; 22],
}

/// Fork a child that duplicates `ch_pid`'s descriptor 3 (its TAP queue) via
/// `pidfd_getfd` while still root, drops to uid 4200 with `CapEff=0`, sets the
/// held TAP's host-side MAC to `victim_mac` with `SIOCSIFHWADDR`, then reads
/// the queue for `read_window` counting needle frames. The parent injects and
/// captures concurrently; both must see zero if the egress control holds.
fn run_mac_hijack_child(ch_pid: u32, victim_mac: [u8; 6], read_window: Duration) -> HijackOutcome {
    let mut pipe_fds = [0_i32; 2];
    // SAFETY: `pipe_fds` is a live 2-element array.
    assert_eq!(unsafe { libc::pipe(pipe_fds.as_mut_ptr()) }, 0, "create result pipe");
    let [read_end, write_end] = pipe_fds;

    // SAFETY: fork in a #[tokio::test] current-thread runtime; the child only
    // performs async-signal-safe libc calls and `_exit`, never returning to the
    // async runtime.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork the hijack child: {}", std::io::Error::last_os_error());

    if pid == 0 {
        // ---- child ----
        // SAFETY: single-threaded child; each syscall operates on its own args.
        unsafe {
            libc::close(read_end);
            let target_pid = libc::pid_t::try_from(ch_pid).unwrap_or(-1);
            let pidfd = libc::syscall(libc::SYS_pidfd_open, target_pid, 0);
            if pidfd < 0 {
                libc::_exit(11);
            }
            let held = libc::syscall(libc::SYS_pidfd_getfd, pidfd, 3, 0);
            if held < 0 {
                libc::_exit(12);
            }
            // Drop every privilege before touching the queue (increment-z crux).
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                || libc::setgroups(0, std::ptr::null()) != 0
                || libc::setgid(OVERDRIVE_VMM_UID) != 0
                || libc::setuid(OVERDRIVE_VMM_UID) != 0
            {
                libc::_exit(13);
            }
            let mut ifreq: HwAddrIfreq = std::mem::zeroed();
            ifreq.sa_family = libc::ARPHRD_ETHER;
            ifreq.sa_data[..6].copy_from_slice(&victim_mac);
            let ioctl_rc = libc::ioctl(held as libc::c_int, libc::SIOCSIFHWADDR, &ifreq) as i32;

            // Read the held queue for the window, counting needle frames.
            let deadline = std::time::Instant::now() + read_window;
            let mut stolen: u32 = 0;
            let mut buffer = [0_u8; 2048];
            while std::time::Instant::now() < deadline {
                let read =
                    libc::read(held as libc::c_int, buffer.as_mut_ptr().cast(), buffer.len());
                if read > 0 {
                    let length = read as usize;
                    if buffer[..length].windows(HIJACK_NEEDLE.len()).any(|w| w == HIJACK_NEEDLE) {
                        stolen = stolen.saturating_add(1);
                    }
                }
                // The held queue is non-blocking (`O_NONBLOCK`, E2/F15); a short
                // sleep keeps the read loop from spinning the CPU.
                libc::usleep(2_000);
            }
            // Stack-only payload: [ioctl_rc: i32 LE][stolen: u32 LE]. No heap
            // allocation after fork (malloc is not async-signal-safe).
            let mut payload = [0_u8; 8];
            payload[..4].copy_from_slice(&ioctl_rc.to_le_bytes());
            payload[4..].copy_from_slice(&stolen.to_le_bytes());
            libc::write(write_end, payload.as_ptr().cast(), payload.len());
            libc::close(write_end);
            libc::_exit(0);
        }
    }

    // ---- parent ----
    // SAFETY: parent closes its write end; the child owns the other.
    unsafe { libc::close(write_end) };
    let mut payload = [0_u8; 8];
    let mut filled = 0;
    loop {
        // SAFETY: `payload` is live; `read_end` is owned by the parent.
        let read = unsafe {
            libc::read(read_end, payload.as_mut_ptr().add(filled).cast(), payload.len() - filled)
        };
        if read > 0 {
            filled += read as usize;
            if filled == payload.len() {
                break;
            }
            continue;
        }
        break;
    }
    let mut status = 0;
    // SAFETY: reap the child; `status` is a live out-param.
    unsafe { libc::waitpid(pid, &raw mut status, 0) };
    // SAFETY: parent owns the read end.
    unsafe { libc::close(read_end) };
    assert_eq!(filled, payload.len(), "the hijack child reports a complete result");
    let ioctl_rc = i32::from_le_bytes(payload[..4].try_into().expect("4 bytes"));
    let stolen = u32::from_le_bytes(payload[4..].try_into().expect("4 bytes"));
    HijackOutcome { ioctl_rc, stolen }
}

// ---------------------------------------------------------------------------
// S-ND295-30B — a damaged or unconfirmable VM is stopped alone
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-30B — A TAP lost during quiescence stops only its VM and the node recovers
/// CONTRACT_SHAPE: bounded-change.
///
/// Case (e): an `IpRules` table loss opens recovery; deleting one Active TAP
/// right after means quiescence meets the missing TAP, so the owner reports
/// `unconfirmed = {A}` and only A is killed (cause `quiescence_unconfirmed`).
/// Recovery reopens for the survivor, A's teardown converges on absence, and
/// A's lease is released.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-30B)"]
async fn a_tap_lost_during_quiescence_stops_only_its_vm_and_the_node_recovers() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-30b-e-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();

    let survivor = deploy_spin_vm(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30b-survivor",
    )
    .await;
    let victim =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-30b-victim")
            .await;

    // Fault: IpRules loss, then delete the victim's Active TAP so its set-down
    // read-back fails and quiescence returns `unconfirmed = {victim}`.
    overdrive_netlink::nft::delete_table(SHARED_IPV4_INTERCEPT_TABLE)
        .expect("external actor deletes the shared IPv4 intercept table");
    let client = Client::new().expect("open typed host-netlink client");
    client.del_link(&victim.tap).await.expect("external actor deletes the victim's Active TAP");

    let killed = wait_until(Duration::from_secs(6), || {
        events.killed(Some("quiescence_unconfirmed")).contains(victim.alloc.as_str())
    })
    .await;
    assert!(killed, "only the victim is killed for unconfirmed quiescence\n{}", events.render());
    assert_eq!(
        events.killed(None),
        BTreeSet::from([victim.alloc.as_str().to_owned()]),
        "no allocation other than the victim is killed",
    );
    assert!(
        wait_until(TERMINAL_BOUND, || cloud_hypervisor_pid_for_alloc(&victim.alloc).is_none())
            .await,
        "the victim's Cloud Hypervisor process ends",
    );

    // The survivor keeps serving: a byte-distinct fresh deploy reaches Running,
    // proving recovery reopened admission.
    assert!(
        cloud_hypervisor_pid_for_alloc(&survivor.alloc).is_some(),
        "the survivor VM keeps running through the victim's kill",
    );
    let reopened = deploy_spin_vm(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30b-reopened",
    )
    .await;

    assert!(
        wait_until(TERMINAL_BOUND, || !tap_exists(&victim.tap)).await,
        "the killed victim's teardown converges on TAP absence",
    );
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(victim.alloc.as_str())).await,
        "the killed victim's lease is released\n{}",
        events.render(),
    );

    for id in [&survivor.workload_id, &reopened.workload_id] {
        stop_and_await_terminal(&cfg, id).await;
    }
    handle.shutdown().await.expect("clean shutdown");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-30B — A still-booting VM's deleted TAP stops only that VM
/// CONTRACT_SHAPE: bounded-change.
///
/// Case (f): the TAP of an allocation still `ProvisionedDown` (its guest has
/// not reached READY, so its row is Pending and its TAP exists administratively
/// down) is deleted from outside. The audit reports it damaged while EXEC stays
/// Open, only that VM is killed, its start is rejected through the VMM-exit
/// path, teardown converges on absence, and its lease is released.
///
/// The ProvisionedDown window is the natural pre-Running boot span
/// [TAP created … `activate` raises it]; the body deletes the TAP on first
/// sight of it while the row is still Pending. See the return notes: a
/// deterministically-widened window needs a delayed-READY guest image the
/// `overdrive-testing` fixture does not yet expose.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-30B)"]
async fn a_booting_vms_deleted_tap_stops_only_that_vm() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-30b-f-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();

    let survivor = deploy_spin_vm(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30bf-survivor",
    )
    .await;

    // Deploy the target and delete its TAP while it is still Pending
    // (ProvisionedDown). The address is not yet published on a Pending row, so
    // enumerate the new `ovd-tp-*` TAP that appeared for this allocation.
    let taps_before: BTreeSet<String> = list_managed_taps();
    let spec = write_toml(
        server_tmp.path(),
        "nd295-30bf-target.toml",
        &vm_job_toml("nd295-30bf-target", "/sbin/spin", &[], &fixture.kernel_path, &rootfs),
    );
    let target =
        deploy(DeployArgs { spec, config_path: cfg.clone() }).await.expect("deploy the target VM");
    let client = Client::new().expect("open typed host-netlink client");
    // The per-allocation TAP is created administratively DOWN at provision,
    // before `activate` raises it post-intercept: its first appearance is the
    // ProvisionedDown signal. Delete it the instant it appears.
    let mut booting_tap = None;
    let caught = wait_until(RUNNING_BOUND, || {
        if let Some(tap) = list_managed_taps().difference(&taps_before).next() {
            booting_tap = Some(tap.clone());
            return true;
        }
        false
    })
    .await;
    let booting_tap = booting_tap
        .expect("the target's ProvisionedDown TAP appeared before this allocation was torn down");
    assert!(caught, "the ProvisionedDown TAP appeared within the boot bound");
    client.del_link(&booting_tap).await.expect("external actor deletes the still-booting TAP");

    let target_alloc = wait_for_alloc_id(&target.workload_id, &cfg).await;
    let killed = wait_until(Duration::from_secs(6), || {
        events.killed(Some("attachment_damaged")).contains(target_alloc.as_str())
    })
    .await;
    assert!(killed, "only the booting target is killed for attachment damage\n{}", events.render());
    assert_eq!(
        events.killed(None),
        BTreeSet::from([target_alloc.as_str().to_owned()]),
        "no allocation other than the booting target is killed",
    );

    // EXEC stayed Open (audit damage while Open never fail-stops): the survivor
    // keeps running and a fresh deploy still reaches Running.
    assert!(
        cloud_hypervisor_pid_for_alloc(&survivor.alloc).is_some(),
        "EXEC stays Open: the survivor keeps running",
    );
    let admitted = deploy_spin_vm(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30bf-admitted",
    )
    .await;

    assert!(
        wait_until(TERMINAL_BOUND, || !tap_exists(&booting_tap)).await,
        "the killed target's teardown converges on TAP absence",
    );
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(target_alloc.as_str())).await,
        "the killed target's lease is released\n{}",
        events.render(),
    );

    for id in [&survivor.workload_id, &admitted.workload_id] {
        stop_and_await_terminal(&cfg, id).await;
    }
    handle.shutdown().await.expect("clean shutdown");
}

/// Every `ovd-tp-*` TAP currently present on the host.
fn list_managed_taps() -> BTreeSet<String> {
    let mut taps = BTreeSet::new();
    let dir = match std::fs::read_dir(HOST_ROUTE_DEVICE_BASE) {
        Ok(dir) => dir,
        Err(error) => panic!("read {HOST_ROUTE_DEVICE_BASE}: {error}"),
    };
    for entry in dir {
        let entry = entry.expect("read a net device entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("ovd-tp-") {
            taps.insert(name);
        }
    }
    taps
}

/// Resolve a workload's first allocation id once it exists.
async fn wait_for_alloc_id(workload_id: &str, cfg: &Path) -> AllocationId {
    use overdrive_cli::commands::workload::{DescribeArgs, describe};
    let deadline = Instant::now() + RUNNING_BOUND;
    loop {
        let described =
            describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
                .await
                .expect("describe while resolving the allocation id");
        if let Some(row) = described.snapshot.rows.first() {
            return AllocationId::new(&row.alloc_id).expect("allocation id parses");
        }
        assert!(Instant::now() < deadline, "the workload {workload_id} produced an allocation row");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-30B — A removed ingress link, egress link, or guard member stops only that VM
/// CONTRACT_SHAPE: bounded-change.
///
/// Case (g): out-of-band removal of one TAP's TCX ingress link, separately its
/// TCX egress link, and separately its bridge-guard member is per-allocation
/// damage the audit reports while EXEC stays Open. Each removal kills only that
/// VM, leaves EXEC Open, converges teardown on absence, and releases the lease;
/// a bystander keeps serving throughout.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-30B)"]
async fn a_removed_ingress_link_egress_link_or_guard_member_stops_only_that_vm() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-30b-g-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let guard_spec = canonical_guard();

    let bystander = deploy_spin_vm(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30bg-bystander",
    )
    .await;

    // Three per-allocation damage classes, each on its own fresh target.
    let mut index = 0;
    for damage in ["ingress-link", "egress-link", "guard-member"] {
        index += 1;
        let target = deploy_spin_vm(
            &cfg,
            server_tmp.path(),
            &fixture.kernel_path,
            &rootfs,
            &format!("nd295-30bg-{index}"),
        )
        .await;
        let killed_before = events.killed(None);

        match damage {
            "ingress-link" => {
                detach_pinned_link(ingress_link_pin(&target.tap))
                    .expect("external actor detaches the target's TCX ingress link");
            }
            "egress-link" => {
                detach_pinned_link(egress_link_pin(&target.tap))
                    .expect("external actor detaches the target's TCX egress link");
            }
            "guard-member" => {
                delete_member(&guard_spec, &target.tap)
                    .expect("external actor removes the target's bridge-guard member");
            }
            other => panic!("unreachable damage class {other}"),
        }

        let killed = wait_until(Duration::from_secs(6), || {
            events.killed(Some("attachment_damaged")).contains(target.alloc.as_str())
        })
        .await;
        assert!(killed, "{damage}: only the target VM is killed\n{}", events.render());
        let newly_killed: BTreeSet<String> =
            events.killed(None).difference(&killed_before).cloned().collect();
        assert_eq!(
            newly_killed,
            BTreeSet::from([target.alloc.as_str().to_owned()]),
            "{damage}: exactly the damaged target is newly killed",
        );
        assert!(
            cloud_hypervisor_pid_for_alloc(&bystander.alloc).is_some(),
            "{damage}: EXEC stays Open and the bystander keeps running",
        );
        assert!(
            wait_until(TERMINAL_BOUND, || !tap_exists(&target.tap)).await,
            "{damage}: the killed target's teardown converges on absence",
        );
        assert!(
            wait_until(TERMINAL_BOUND, || events.released().contains(target.alloc.as_str())).await,
            "{damage}: the killed target's lease is released\n{}",
            events.render(),
        );
    }

    stop_and_await_terminal(&cfg, &bystander.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-66 — a stop succeeds even when parts of the attachment are gone
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-66 — A stop converges when attachment parts are already gone
/// CONTRACT_SHAPE: bounded-change.
///
/// A Running VM's TAP, separately its ingress link pin, and separately its
/// guard member are removed from outside before the operator stops it. R5
/// teardown converges on absence: the stop reaches an empty complement and
/// releases the lease.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-66)"]
async fn a_stop_converges_when_attachment_parts_are_already_gone() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-66-parts-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let guard_spec = canonical_guard();

    let vm =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-66").await;

    // Remove three attachment parts out of band before the stop.
    let client = Client::new().expect("open typed host-netlink client");
    client.del_link(&vm.tap).await.expect("external actor deletes the TAP");
    // The TAP deletion detaches its TCX links; remove any residual ingress pin.
    if ingress_link_pin(&vm.tap).exists() {
        std::fs::remove_file(ingress_link_pin(&vm.tap))
            .expect("remove the residual ingress link pin");
    }
    delete_member(&guard_spec, &vm.tap).expect("external actor removes the guard member");

    // The stop still reaches an empty complement and releases the lease.
    stop_and_await_terminal(&cfg, &vm.workload_id).await;
    assert_empty_complement(&vm);
    match observe_bridge_guard(&guard_spec, &BTreeSet::new()).expect("observe emptied guard") {
        BridgeGuardObservation::Absent { .. } | BridgeGuardObservation::Exact { .. } => {}
        BridgeGuardObservation::Conflict { inventory } => {
            panic!("stop over already-gone parts leaves a conflicting guard: {inventory:?}")
        }
    }
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(vm.alloc.as_str())).await,
        "the stop releases the lease even over already-gone parts\n{}",
        events.render(),
    );

    handle.shutdown().await.expect("clean shutdown");
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH
/// S-ND295-66 — A launch that fails leaves no TAP and no queue holder
/// CONTRACT_SHAPE: bounded-change.
///
/// A `[vm]` spec naming a kernel image absent at start (present at serve boot,
/// deleted before this allocation starts — the only reachable producer of an
/// allocation-level Failed row) reaches Failed. The preflight fails before any
/// TAP is created, so the launch leaves no `ovd-tp-*` TAP and no Cloud
/// Hypervisor queue holder for the allocation.
#[tokio::test]
#[serial(cgroup)]
async fn a_launch_that_fails_leaves_no_tap_and_no_queue_holder() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-66-launch-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");

    // A per-test kernel copy that is valid at serve boot, deleted before start.
    let kernel_copy = tmp.path().join("kernel-to-delete");
    std::fs::copy(&fixture.kernel_path, &kernel_copy)
        .expect("copy the fixture kernel for this test");

    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());

    let taps_before = list_managed_taps();
    let spec = write_toml(
        server_tmp.path(),
        "nd295-66-missing-kernel.toml",
        &vm_job_toml("nd295-66-missing-kernel", "/sbin/spin", &[], &kernel_copy, &rootfs),
    );
    std::fs::remove_file(&kernel_copy).expect("delete the configured kernel before start");

    let output = deploy(DeployArgs { spec, config_path: cfg.clone() })
        .await
        .expect("deploy accepts the workload whose kernel was just deleted");
    let terminal = poll_until_terminal(&cfg, &output.workload_id, TERMINAL_BOUND).await;
    let row = terminal.snapshot.rows.first().expect("one failed allocation row");
    assert_eq!(row.state, AllocStateWire::Failed, "the missing-kernel launch reaches Failed");
    let alloc = AllocationId::new(&row.alloc_id).expect("allocation id parses");

    assert_eq!(list_managed_taps(), taps_before, "the failed launch leaves no new ovd-tp-* TAP");
    assert!(
        cloud_hypervisor_pid_for_alloc(&alloc).is_none(),
        "the failed launch leaves no Cloud Hypervisor queue holder for {alloc}",
    );

    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-67 — a MAC hijack from outside the VM steals nothing and the victim recovers
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-67 — A MAC hijack from outside the VM steals nothing and the victim recovers
/// CONTRACT_SHAPE: bounded-change.
///
/// A process outside the launch filter (uid 4200, `CapEff=0`), holding a
/// `pidfd_getfd` copy of attacker A's TAP queue, sets A's host-side MAC to
/// victim V's guest MAC with `SIOCSIFHWADDR`. The R21 TCX egress classifier
/// keeps A's TAP from delivering any frame addressed to V, while A's own
/// unicast and every broadcast still arrive; the audit reports A's host-side
/// MAC as `TapHostMac` damage and kills only A; after A's teardown the poisoned
/// FDB entry is gone and V answers the host again with its MAC re-learned on
/// its own port. Pre-control RED oracle: increment-z STEPs 4-6.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-67)"]
async fn a_mac_hijack_from_outside_the_vm_steals_nothing_and_the_victim_recovers() {
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-67-hijack-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let rootfs = stage_rootfs_with_extra_binary(tmp.path(), &fixture, &spin, "spin");
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();

    let attacker =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-67-attacker")
            .await;
    let victim =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-67-victim")
            .await;
    let bridge_ifindex = ifindex_of(&bridge_of(&attacker.tap));
    let victim_mac = guest_mac(victim.addr);
    let attacker_mac = guest_mac(attacker.addr);

    let attacker_pid = cloud_hypervisor_pid_for_alloc(&attacker.alloc)
        .expect("the attacker VM's Cloud Hypervisor process is live");
    assert!(
        descriptor_three_holds_tap_queue(attacker_pid, &attacker.tap),
        "the attacker VMM holds its own TAP queue at descriptor 3",
    );

    // Oracle (1): the hijack sets A's host-side MAC to V's MAC; while injecting
    // host unicast to V's MAC, A's TAP transmits zero such frames and the
    // attacker reads zero from its held queue.
    let attacker_leak = NeedleCapture::open(&attacker.tap, HIJACK_NEEDLE);
    let counter_before = read_counter(counter_map_pin(), GuestTcxCounter::EgressDestinationDrop)
        .expect("read the node-wide egress-destination-drop counter baseline");

    // Inject continuously in a background OS thread for the whole window.
    let injector_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let injector_flag = injector_stop.clone();
    let injector = std::thread::spawn(move || {
        while !injector_flag.load(std::sync::atomic::Ordering::Relaxed) {
            inject_host_unicast(bridge_ifindex, victim_mac, HIJACK_NEEDLE, 8);
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    // The child sets the MAC, then reads its queue for the window.
    let outcome = run_mac_hijack_child(attacker_pid, victim_mac, Duration::from_secs(2));
    injector_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    injector.join().expect("host-unicast injector thread joins");

    assert_eq!(
        outcome.ioctl_rc, 0,
        "SIOCSIFHWADDR succeeds at uid 4200 with CapEff=0 (the hazard)"
    );
    assert_eq!(
        outcome.stolen, 0,
        "the attacker reads zero frames addressed to the victim from its queue"
    );
    assert_eq!(
        attacker_leak.drain(),
        0,
        "the attacker's TAP transmits zero frames addressed to the victim"
    );
    let counter_after = read_counter(counter_map_pin(), GuestTcxCounter::EgressDestinationDrop)
        .expect("read the node-wide egress-destination-drop counter after the steal attempt");
    assert!(
        counter_after > counter_before,
        "the shared EgressDestinationDrop slot rises for the dropped foreign-destination frames",
    );

    // Oracle (2): positive controls — host unicast to A's own MAC arrives, and
    // a host broadcast reaches every guest.
    let attacker_own = NeedleCapture::open(&attacker.tap, HIJACK_NEEDLE);
    let victim_own = NeedleCapture::open(&victim.tap, HIJACK_NEEDLE);
    inject_host_unicast(bridge_ifindex, attacker_mac, HIJACK_NEEDLE, 8);
    inject_host_unicast(bridge_ifindex, [0xff; 6], HIJACK_NEEDLE, 8);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        attacker_own.drain() > 0,
        "host unicast to the attacker's own MAC still reaches its TAP"
    );
    assert!(victim_own.drain() > 0, "a host broadcast still reaches the victim guest");

    // Oracle (3): while poisoned, the victim receives no host unicast.
    let victim_unicast = NeedleCapture::open(&victim.tap, HIJACK_NEEDLE);
    inject_host_unicast(bridge_ifindex, victim_mac, HIJACK_NEEDLE, 8);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        victim_unicast.drain(),
        0,
        "the poisoned entry delivers no host unicast to the victim"
    );

    // Oracle (4): the audit reports A's host-side MAC damage and kills only A.
    let killed = wait_until(Duration::from_secs(3), || {
        events.killed(Some("attachment_damaged")).contains(attacker.alloc.as_str())
    })
    .await;
    assert!(
        killed,
        "the audit kills only the attacker for its host-side MAC damage\n{}",
        events.render()
    );
    assert_eq!(
        events.killed(None),
        BTreeSet::from([attacker.alloc.as_str().to_owned()]),
        "the victim and every other allocation are untouched by the kill",
    );
    assert!(
        cloud_hypervisor_pid_for_alloc(&victim.alloc).is_some(),
        "EXEC stays Open and the victim keeps running",
    );

    // Oracle (5): after A's teardown the poisoned entry is gone, the victim
    // answers the host again, and its MAC is re-learned on its own port.
    assert!(
        wait_until(TERMINAL_BOUND, || cloud_hypervisor_pid_for_alloc(&attacker.alloc).is_none())
            .await,
        "the attacker VM's process ends and its teardown proceeds",
    );
    assert!(
        wait_until(TERMINAL_BOUND, || !tap_exists(&attacker.tap)).await,
        "the attacker's teardown converges on TAP absence",
    );
    let victim_mac_string = format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        victim_mac[0], victim_mac[1], victim_mac[2], victim_mac[3], victim_mac[4], victim_mac[5]
    );
    let relearned = wait_until(ONE_AUDIT_PERIOD + Duration::from_secs(2), || {
        // Re-inject to prompt the victim; the entry must be on the victim's port only.
        inject_host_unicast(bridge_ifindex, victim_mac, HIJACK_NEEDLE, 4);
        bridge_fdb_entries().iter().any(|(mac, dev)| {
            mac == &victim_mac_string
                && dev.split_whitespace().next() == Some(victim.tap.as_str())
                && !dev.contains("permanent")
        }) && !bridge_fdb_entries().iter().any(|(mac, dev)| {
            mac == &victim_mac_string && dev.split_whitespace().next() != Some(victim.tap.as_str())
        })
    })
    .await;
    assert!(relearned, "the victim's MAC is re-learned only on the victim's port after teardown");

    stop_and_await_terminal(&cfg, &victim.workload_id).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-69 — losing the whole program table with live mesh VMs is repaired in place
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-69 — A deleted program table is repaired in place with live mesh VMs
/// CONTRACT_SHAPE: bounded-change.
///
/// With a mesh Service and its client Running, deleting `table ip
/// overdrive-mtls` closes new commands, quiesces the TAPs, restores the program
/// and its `2 + P` members, raises the TAPs again, and reopens admission — with
/// no restart. A fresh natural mesh client Job then reaches its own
/// `Terminated`/exit 0, proving the Service answers again without a restart.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-69)"]
async fn a_deleted_program_table_is_repaired_with_live_mesh_vms() {
    use overdrive_cli::commands::workload::{DescribeArgs, describe};

    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-69-repair-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let mesh_guest = build_mesh_guest(tmp.path());
    let rootfs = stage_rootfs_with_extra_binaries(
        tmp.path(),
        &fixture,
        &[(&peer, "gti-peer"), (&mesh_guest, "gti-mesh-guest")],
    );
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let client = Client::new().expect("open typed host-netlink client");

    // Deploy the mesh Service (callee) and one mesh client Job (caller).
    let service_spec = write_toml(
        server_tmp.path(),
        "nd295-69-service.toml",
        &service_toml(Path::new("/sbin/gti-peer"), &fixture.kernel_path, &rootfs),
    );
    let service = deploy(DeployArgs { spec: service_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh Service");
    let service_running = poll_until_running(&cfg, &service.workload_id, RUNNING_BOUND).await;
    let service_row = service_running.snapshot.rows.first().expect("one Service allocation row");
    let service_alloc = AllocationId::new(&service_row.alloc_id).expect("service alloc id parses");
    let service_addr = service_row.workload_addr.expect("the Service publishes its guest address");
    let service_tap = tap_for(service_addr);
    let restart_before = service_row.restart_count;

    let client_spec = write_toml(
        server_tmp.path(),
        "nd295-69-client.toml",
        &vm_job_toml("nd295-69-client", "/sbin/gti-mesh-guest", &[], &fixture.kernel_path, &rootfs),
    );
    let mesh_client = deploy(DeployArgs { spec: client_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the mesh client Job");
    let client_running = poll_until_running(&cfg, &mesh_client.workload_id, RUNNING_BOUND).await;
    let client_addr = client_running
        .snapshot
        .rows
        .first()
        .expect("one client row")
        .workload_addr
        .expect("client address");
    let client_tap = tap_for(client_addr);

    // Fault: delete the whole program table.
    overdrive_netlink::nft::delete_table(SHARED_IPV4_INTERCEPT_TABLE)
        .expect("external actor deletes the shared IPv4 intercept program table");

    // Detection reports IpRules; TAPs quiesce within one audit period.
    let detected = wait_until(Duration::from_secs(3), || {
        events.unhealthy_components().iter().any(|component| component == "IpRules")
    })
    .await;
    assert!(detected, "the table deletion is detected as IpRules\n{}", events.render());
    for tap in [&service_tap, &client_tap] {
        assert!(
            wait_link_state(&client, tap, Some(false), ONE_AUDIT_PERIOD + Duration::from_secs(1))
                .await,
            "the managed TAP {tap} reads down within one audit period of the deletion",
        );
    }

    // Repair restores the program and its 2 + P members and raises the TAPs.
    let repaired = wait_until(Duration::from_secs(6), || {
        matches!(observe_shared_intercept_state(), Ok(Some(_)))
    })
    .await;
    assert!(repaired, "the shared IPv4 intercept program is restored in place");
    let state = observe_shared_intercept_state()
        .expect("observe restored intercept state")
        .expect("the restored program is present");
    assert!(
        state.managed_guest_ips().contains(&service_addr)
            && state.managed_guest_ips().contains(&client_addr),
        "the restored program readmits both live guest source addresses (2 members)",
    );
    for tap in [&service_tap, &client_tap] {
        assert!(
            wait_link_state(&client, tap, Some(true), Duration::from_secs(3)).await,
            "the managed TAP {tap} reads back up after repair",
        );
    }

    // Admission reopened and no restart occurred: a fresh natural mesh client
    // Job dials the Service by name and reaches Terminated/exit 0 on its own.
    let reopened_spec = write_toml(
        server_tmp.path(),
        "nd295-69-reopened.toml",
        &vm_job_toml(
            "nd295-69-reopened",
            "/sbin/gti-mesh-guest",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    );
    let reopened = deploy(DeployArgs { spec: reopened_spec, config_path: cfg.clone() })
        .await
        .expect("deploy a fresh mesh client after repair");
    let reopened_running = poll_until_running(&cfg, &reopened.workload_id, RUNNING_BOUND).await;
    let reopened_alloc =
        reopened_running.snapshot.rows.first().expect("one reopened client row").alloc_id.clone();
    poll_until_natural_job_completion(&cfg, &reopened.workload_id, &reopened_alloc, RUNNING_BOUND)
        .await
        .expect("the fresh mesh client reaches Terminated/exit 0 without a restart");

    // The Service's restart count is unchanged across the repair.
    let after =
        describe(DescribeArgs { id: service.workload_id.clone(), config_path: cfg.clone() })
            .await
            .expect("describe the Service after repair");
    let service_row_after = after
        .snapshot
        .rows
        .iter()
        .find(|row| row.alloc_id == service_alloc.as_str())
        .expect("the Service allocation row survives the repair");
    assert_eq!(
        service_row_after.restart_count, restart_before,
        "the Service is repaired in place, not restarted",
    );

    for id in [&service.workload_id, &mesh_client.workload_id, &reopened.workload_id] {
        stop_and_await_terminal(&cfg, id).await;
    }
    handle.shutdown().await.expect("clean shutdown");
}
