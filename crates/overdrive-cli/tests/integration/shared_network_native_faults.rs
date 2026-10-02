//! netns-density-295 native-metal fault evidence (S-ND295-30B / 66 / 67 / 69).
//!
//! Real Cloud Hypervisor guests driven through the production entry points
//! (`serve::run_with_kek` + `deploy`/`stop`), on a non-virtualized x86_64 host
//! under `cargo xtask metal run --`. Every fault enters through a real kernel
//! mutation (`ip link del`, `nft delete table`, `detach_pinned_link`,
//! `delete_member`, an out-of-VM `SIOCSIFHWADDR` on a `pidfd_getfd`-copied TAP
//! queue), a guest image the test stages (the holding init of
//! [`super::delayed_ready_guest`]), or a resource the production path created;
//! no test installs a production effect the production path omits.
//!
//! These bodies observe the R14 per-VM kill scope, R5 teardown-converges-on-
//! absence, the fd handoff, R21 host-side-MAC egress control, and R15 in-place
//! program repair — none of which is on the production path yet — so every
//! body is `#[ignore = "pending DELIVER step 10-02 (S-ND295-xx)"]`. Today each
//! fails on a preceding-step gap before its own oracle: no guest reaches
//! Running until the 05-03 fd handoff, because Cloud Hypervisor, launched as
//! the confined uid, cannot raise a TAP it opens by name (`Tap::enable`,
//! `SIOCSIFFLAGS` → `EPERM`). The body's own oracle is verified at 10-02's RED
//! phase (`red-classification.md` Phase G).
//!
//! Every mesh dialer here is a guest that speaks plaintext by design
//! (CLAUDE.md § "East-west mTLS tests"); the proof that a Service still serves
//! is a byte-distinct REQUEST/RESPONSE exchange the guest completes on its own
//! (its natural exit 0), or the peer-authored RESPONSE observed on the
//! dialer's own TAP, never a test-side TLS client.
//!
//! # Clocks
//!
//! Interval oracles (S-ND295-67's audit and re-learn bounds) compare
//! `CLOCK_MONOTONIC` readings taken at the kernel-facing event: the hijack
//! child reads it immediately after its `ioctl`, and the tracing capture reads
//! it when the supervisor emits its kill event.
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
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::{AllocStateWire, AllocStatusRowBody};
use overdrive_core::TransitionReason;
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

use super::delayed_ready_guest::{HOLD_MARKER, HoldingInit, RELEASE_MARKER, stage_holding_rootfs};
use super::guest_stack_mtls_egress::{
    MESH_NAME, REQUEST, RESPONSE, SERVICE_PORT, build_mesh_guest_with_timing, build_mesh_peer,
    build_static_binary, observe_shared_intercept_state, service_toml,
};
use super::vm_walking_skeleton::{
    TeardownBound, build_spin_binary, config_path, poll_until_running, poll_until_terminal,
    shared_staging_root, spawn_vm_server_mtls_composed, stage_rootfs_with_extra_binaries,
    stage_rootfs_with_extra_binary, vm_job_toml, write_toml,
};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

const SHARED_IPV4_INTERCEPT_TABLE: &str = "overdrive-mtls";
const PIN_ROOT: &str = "/sys/fs/bpf/overdrive/mtls-endpoints";
const HOST_ROUTE_DEVICE_BASE: &str = "/sys/class/net";
const VM_RUN_ROOT: &str = "/run/overdrive/vm";
/// The supervisor's audit period (ADR-0124; FD § "[REF] Runtime shared-network
/// supervisor (D-295-R13, R14, R15, R16)": `SHARED_NETWORK_AUDIT_PERIOD = 1 s`).
const ONE_AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// The *Full audit* rule's ceiling on one full audit's latency: the design
/// keeps L at or below 1 s, and a longer full audit is surfaced to the user,
/// not absorbed (FD § "[REF] Runtime shared-network supervisor", *Full
/// audit*). E18 measures L; it never raises this ceiling silently.
const FULL_AUDIT_LATENCY_CEILING: Duration = Duration::from_secs(1);
/// E12 (h) bound (4), "within one audit period (1 s, subject to E18) of the
/// change": the next audit starts within one audit period of the change and
/// completes within the full-audit latency ceiling.
const NEXT_AUDIT_BOUND: Duration =
    Duration::from_secs(ONE_AUDIT_PERIOD.as_secs() + FULL_AUDIT_LATENCY_CEILING.as_secs());
/// The per-VM kill write's bound floor. The supervisor's private
/// `SHARED_NETWORK_VM_KILL_CALL_BOUND` is `max(1 s, 4 × W)` and holds this 1 s
/// floor until E18 measures W; this native lane cannot name the private
/// constant, so it states the write's share as that floor (user decision 1 of
/// 2026-09-30). DELIVER step 09-01, which sets the bound from E18, re-checks
/// this horizon.
const KILL_WRITE_BOUND_FLOOR: Duration = Duration::from_secs(1);
/// E12 (h) bound (4) horizon, change → kill: the next audit detects the damage
/// within [`NEXT_AUDIT_BOUND`], then the one per-VM kill write lands within its
/// own bound (the event follows the write). The kill while Open is a one-write
/// loop, so the horizon gains exactly one kill-write bound.
const KILL_AFTER_CHANGE_BOUND: Duration =
    Duration::from_secs(NEXT_AUDIT_BOUND.as_secs() + KILL_WRITE_BOUND_FLOOR.as_secs());
/// E12 (h) bound (5): the echo answered and the re-learned entry observed
/// within 1 s of teardown's complement read-back.
const RELEARN_BOUND: Duration = Duration::from_secs(1);
/// Wait for a per-allocation damage kill (E12 (e)-(g) name no tighter bound).
const PER_ALLOCATION_KILL_WAIT: Duration = Duration::from_secs(6);
/// Bound for a real production boot to reach Running.
const RUNNING_BOUND: Duration = Duration::from_secs(90);
/// Bound for activation (after the Running write) to raise a TAP.
const ACTIVATION_BOUND: Duration = Duration::from_secs(10);
/// Bound for a stop or teardown to converge on an empty complement.
const TERMINAL_BOUND: Duration = Duration::from_secs(30);
/// Bound for a body's final stop to quiesce every workload it deployed. It
/// also covers a restarted successor stopped while its guest is still inside
/// a holding init's hold (`delayed_ready_guest::READY_HOLD`).
const QUIESCENCE_BOUND: Duration = Duration::from_secs(60);
/// Bound for a mesh dialer Job to complete its byte-distinct exchange.
const DIAL_BOUND: Duration = Duration::from_secs(90);
/// Real-wall-clock poll interval for kernel and describe observations.
const POLL: Duration = Duration::from_millis(25);
/// Time for synchronously injected frames to reach every capture queue.
const SETTLE: Duration = Duration::from_millis(100);
/// Descriptor the VMM adapter hands the TAP queue to Cloud Hypervisor at
/// (D-295-R3; `overdrive-host` `VMM_TAP_QUEUE_FD`).
const VMM_TAP_QUEUE_FD: i32 = 3;

/// The per-VM kill event (D-295-R14; FD § "[REF] Runtime shared-network
/// supervisor" (detection and the kill-scope table)) — pinned name/fields.
const VM_KILLED_EVENT: &str = "guest_network.shared_owner_vm_killed";
/// The node-level unhealthy detection event (FD § "[REF] Runtime shared-network
/// supervisor" (detection)).
const UNHEALTHY_EVENT: &str = "guest_network.shared_owner_unhealthy";
/// The fail-stop event (FD § "[REF] Runtime shared-network supervisor").
const FAIL_STOP_EVENT: &str = "guest_network.shared_owner_fail_stop";
/// The lease-released event (D-295-R7; FD § "[REF] Component — node-wide
/// guest-attachment admission (D-295-R6, R7, R8)" (the lease events)).
const LEASE_RELEASED_EVENT: &str = "guest_network.lease_released";

/// Frames injected per stimulus class in S-ND295-67.
const INJECTED_FRAMES: usize = 16;
/// Host unicast to V's guest MAC before the change (the pre-change control).
const PRE_CHANGE_NEEDLE: &[u8] = b"ND295-67-PRE-CHANGE-TO-VICTIM";
/// Host unicast to V's guest MAC after the change (the stolen class).
const HIJACK_NEEDLE: &[u8] = b"ND295-67-HIJACK-IDENTIFIABLE";
/// Host unicast to A's own guest MAC after the change (positive control).
const ATTACKER_OWN_NEEDLE: &[u8] = b"ND295-67-TO-ATTACKER-OWN-MAC";
/// Host broadcast after the change (positive control).
const BROADCAST_NEEDLE: &[u8] = b"ND295-67-HOST-BROADCAST";
/// Payload of the host→V ICMP echo of oracle (5).
const ECHO_PAYLOAD: &[u8] = b"ND295-67-HOST-TO-VICTIM-ECHO";
/// Peer-authored RESPONSE frames the S-ND295-69 client must receive before
/// the fault, and again after the repair.
const EXCHANGES_WITNESSED: usize = 3;
/// Bound for [`EXCHANGES_WITNESSED`] exchanges to be observed.
const EXCHANGE_BOUND: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Small pure helpers and host reads
// ---------------------------------------------------------------------------

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

fn mac_text(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// The live kernel ifindex of a real interface, or a panic if it is absent.
fn ifindex_of(iface: &str) -> u32 {
    interface_ifindex(iface).unwrap_or_else(|| panic!("interface {iface} is live"))
}

/// The interface's ifindex; `None` when it is absent (`NotFound`, or `ENODEV`
/// from a device removed while its attribute was read).
fn interface_ifindex(iface: &str) -> Option<u32> {
    let path = tap_device_dir(iface).join("ifindex");
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(
            text.trim()
                .parse()
                .unwrap_or_else(|error| panic!("parse {} ({text:?}): {error}", path.display())),
        ),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ENODEV) =>
        {
            None
        }
        Err(error) => panic!("read {}: {error}", path.display()),
    }
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

/// The bpffs pin's inode; `None` when the pin is absent. A re-pinned object
/// of the same name has a new inode.
fn pin_inode(path: &Path) -> Option<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.ino()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("stat {}: {error}", path.display()),
    }
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

/// Every `ovd-tp-*` TAP currently present on the host.
fn list_managed_taps() -> BTreeSet<String> {
    let dir = std::fs::read_dir(HOST_ROUTE_DEVICE_BASE)
        .unwrap_or_else(|error| panic!("read {HOST_ROUTE_DEVICE_BASE}: {error}"));
    let mut taps = BTreeSet::new();
    for entry in dir {
        let name =
            entry.expect("read a net device entry").file_name().to_string_lossy().into_owned();
        if name.starts_with("ovd-tp-") {
            taps.insert(name);
        }
    }
    taps
}

/// Every body here drives real kernel state (loop mounts, raw sockets,
/// `pidfd_getfd`, cgroups): it must run as root, as `cargo xtask metal run --`
/// runs it.
fn assert_root() {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    assert_eq!(
        euid, 0,
        "this native body runs as root under `cargo xtask metal run --` (euid {euid})"
    );
}

/// `CLOCK_MONOTONIC` now. Every interval oracle in this file compares readings
/// of this one clock.
fn monotonic_now() -> Duration {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a live out-parameter for this call.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now) };
    assert_eq!(rc, 0, "clock_gettime(CLOCK_MONOTONIC): {}", std::io::Error::last_os_error());
    Duration::new(
        u64::try_from(now.tv_sec).expect("monotonic seconds are non-negative"),
        u32::try_from(now.tv_nsec).expect("monotonic nanoseconds fit u32"),
    )
}

// ---------------------------------------------------------------------------
// Process and TUN-queue inventory (`/proc`)
// ---------------------------------------------------------------------------

/// Whether `error` means the process or descriptor vanished while read.
fn vanished(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}

fn proc_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").expect("read /proc") {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if vanished(&error) => continue,
            Err(error) => panic!("read a /proc entry: {error}"),
        };
        if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
            pids.push(pid);
        }
    }
    pids
}

/// The process's NUL-separated argv, or `None` once it has vanished.
fn process_cmdline(pid: u32) -> Option<Vec<u8>> {
    match std::fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(cmdline) => Some(cmdline),
        Err(error) if vanished(&error) => None,
        Err(error) => panic!("read /proc/{pid}/cmdline: {error}"),
    }
}

/// Matches on `argv[0]`, never the `TASK_COMM_LEN`-truncated `comm`.
fn is_cloud_hypervisor(cmdline: &[u8]) -> bool {
    let argv0 = cmdline.split(|&byte| byte == 0).next().unwrap_or(&[]);
    Path::new(&String::from_utf8_lossy(argv0).into_owned()).file_name()
        == Some(std::ffi::OsStr::new("cloud-hypervisor"))
}

/// A process has ended when `/proc/<pid>/stat` is gone or reads zombie/dead.
fn process_has_ended(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_none_or(|state| state == 'Z' || state == 'X'),
        Err(error) if vanished(&error) => true,
        Err(error) => panic!("read /proc/{pid}/stat: {error}"),
    }
}

/// The live `cloud-hypervisor` pid whose argv references this allocation's run
/// directory, or `None` when no such process is present.
fn cloud_hypervisor_pid_for_alloc(alloc: &AllocationId) -> Option<u32> {
    let run_dir = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc);
    let needle = run_dir.path().to_string_lossy().into_owned();
    proc_pids().into_iter().find(|pid| {
        process_cmdline(*pid).is_some_and(|cmdline| {
            is_cloud_hypervisor(&cmdline) && String::from_utf8_lossy(&cmdline).contains(&needle)
        }) && !process_has_ended(*pid)
    })
}

/// The allocation whose run directory a Cloud Hypervisor process's argv names
/// (`/run/overdrive/vm/<alloc>/…`).
fn alloc_of_cloud_hypervisor(pid: u32) -> Option<AllocationId> {
    let cmdline = process_cmdline(pid)?;
    let prefix = format!("{VM_RUN_ROOT}/");
    cmdline.split(|&byte| byte == 0).find_map(|argument| {
        let argument = String::from_utf8_lossy(argument).into_owned();
        let (_, rest) = argument.split_once(&prefix)?;
        let name = rest.split('/').next()?;
        Some(AllocationId::new(name).unwrap_or_else(|error| {
            panic!(
                "the run directory {name:?} of Cloud Hypervisor {pid} is an allocation id: {error}"
            )
        }))
    })
}

/// One `/dev/net/tun` descriptor and the TAP its fdinfo names (`iff:`); an
/// empty `iff` is a queue attached to no live TAP.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TunDescriptor {
    pid: u32,
    fd: u32,
    iff: String,
}

fn tun_descriptors_of(pid: u32) -> Vec<TunDescriptor> {
    let fd_dir = format!("/proc/{pid}/fd");
    let entries = match std::fs::read_dir(&fd_dir) {
        Ok(entries) => entries,
        Err(error) if vanished(&error) => return Vec::new(),
        Err(error) => panic!("read {fd_dir}: {error}"),
    };
    let mut descriptors = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if vanished(&error) => continue,
            Err(error) => panic!("read an entry of {fd_dir}: {error}"),
        };
        let Ok(fd) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let target = match std::fs::read_link(entry.path()) {
            Ok(target) => target,
            Err(error) if vanished(&error) => continue,
            Err(error) => panic!("readlink {}: {error}", entry.path().display()),
        };
        if target != Path::new("/dev/net/tun") {
            continue;
        }
        let fdinfo_path = format!("/proc/{pid}/fdinfo/{fd}");
        let fdinfo = match std::fs::read_to_string(&fdinfo_path) {
            Ok(fdinfo) => fdinfo,
            Err(error) if vanished(&error) => continue,
            Err(error) => panic!("read {fdinfo_path}: {error}"),
        };
        let iff = fdinfo
            .lines()
            .find_map(|line| line.strip_prefix("iff:"))
            .unwrap_or_else(|| panic!("{fdinfo_path} of a /dev/net/tun descriptor has an iff line"))
            .trim()
            .to_owned();
        descriptors.push(TunDescriptor { pid, fd, iff });
    }
    descriptors
}

/// Every `/dev/net/tun` descriptor held by any process on the host.
fn all_tun_descriptors() -> BTreeSet<TunDescriptor> {
    proc_pids().into_iter().flat_map(tun_descriptors_of).collect()
}

/// The live Cloud Hypervisor process holding a queue of `tap`, if any.
fn cloud_hypervisor_queue_holder(tap: &str) -> Option<u32> {
    proc_pids().into_iter().find(|pid| {
        process_cmdline(*pid).is_some_and(|cmdline| is_cloud_hypervisor(&cmdline))
            && !process_has_ended(*pid)
            && tun_descriptors_of(*pid).iter().any(|descriptor| descriptor.iff == tap)
    })
}

/// Descriptor 3 of a Cloud Hypervisor process is its own TAP queue: the
/// `iff:` line of `/proc/<pid>/fdinfo/3` names `tap` (D-295-R3, FD § "[REF]
/// Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4)"
/// (`CloudHypervisorVmm::create`: `VMM_TAP_QUEUE_FD`)).
fn descriptor_three_holds_tap_queue(pid: u32, tap: &str) -> bool {
    tun_descriptors_of(pid)
        .iter()
        .any(|descriptor| descriptor.fd == VMM_TAP_QUEUE_FD as u32 && descriptor.iff == tap)
}

/// The allocation's serial console capture (`VmRunDir::console_log`), or an
/// empty string before Cloud Hypervisor has created it.
fn console_log(alloc: &AllocationId) -> String {
    let path = VmRunDir::for_alloc(Path::new(VM_RUN_ROOT), alloc).console_log();
    match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

// ---------------------------------------------------------------------------
// Supervisor tracing-event capture (thread-local; the composition and its
// supervisor task run on this `#[tokio::test]` current-thread runtime, so a
// thread-local default subscriber sees their events — the shape
// `serve_lifetime_fail_stop.rs` relies on). Nothing in a body blocks this
// thread for longer than a short synchronous read.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct CapturedEvent {
    /// `CLOCK_MONOTONIC` when the event was emitted.
    at: Duration,
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

    /// When the first `shared_owner_vm_killed { alloc, cause }` was emitted.
    fn first_kill_at(&self, alloc: &AllocationId, cause: &str) -> Option<Duration> {
        self.snapshot()
            .into_iter()
            .filter(|event| event.name == VM_KILLED_EVENT)
            .filter(|event| event.fields.get("alloc").map(String::as_str) == Some(alloc.as_str()))
            .filter(|event| event.fields.get("cause").map(String::as_str) == Some(cause))
            .map(|event| event.at)
            .min()
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

    /// How many `shared_owner_fail_stop` events were emitted.
    fn fail_stops(&self) -> usize {
        self.snapshot().into_iter().filter(|event| event.name == FAIL_STOP_EVENT).count()
    }

    /// EXEC stayed Open: no node-level detection and no fail-stop.
    fn exec_stayed_open(&self) -> bool {
        self.unhealthy_components().is_empty() && self.fail_stops() == 0
    }

    fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for event in self.snapshot() {
            writeln!(out, "  at={:?} {} {:?}", event.at, event.name, event.fields)
                .expect("write to a String");
        }
        out
    }
}

impl<S: Subscriber> Layer<S> for SupervisorEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let name = event.metadata().name();
        if ![VM_KILLED_EVENT, UNHEALTHY_EVENT, FAIL_STOP_EVENT, LEASE_RELEASED_EVENT]
            .contains(&name)
        {
            return;
        }
        let at = monotonic_now();
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().expect("supervisor event lock").push(CapturedEvent {
            at,
            name: name.to_owned(),
            fields: visitor.fields,
        });
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

// ---------------------------------------------------------------------------
// Polling helpers
// ---------------------------------------------------------------------------

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
        tokio::time::sleep(POLL).await;
    }
}

/// Await the observed administrative state of a real link reaching `want`
/// (`Some(true)` up, `Some(false)` down, `None` absent) within `bound`. A
/// failed read is a failed observation, never "not yet".
async fn wait_link_state(client: &Client, tap: &str, want: Option<bool>, bound: Duration) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        let observed = client
            .observe_link(tap)
            .await
            .unwrap_or_else(|error| panic!("observe the administrative state of {tap}: {error}"));
        if observed == want {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Await activation: the action shim raises a TAP only after the Running
/// write (FD § "[REF] Driven port — TAP activation gate (D-295-R5)", the
/// action-shim order), so a Running row is not yet an `Active` attachment.
async fn await_active(client: &Client, tap: &str) {
    assert!(
        wait_link_state(client, tap, Some(true), ACTIVATION_BOUND).await,
        "the managed TAP {tap} reads administratively up (Active) within {ACTIVATION_BOUND:?} \
         of its Running row",
    );
}

async fn describe_rows(cfg: &Path, workload_id: &str) -> Vec<AllocStatusRowBody> {
    describe(DescribeArgs { id: workload_id.to_owned(), config_path: cfg.to_path_buf() })
        .await
        .unwrap_or_else(|error| panic!("describe {workload_id}: {error}"))
        .snapshot
        .rows
}

/// The workload's row for `alloc`, which must exist.
async fn row_of(cfg: &Path, workload_id: &str, alloc: &AllocationId) -> AllocStatusRowBody {
    describe_rows(cfg, workload_id)
        .await
        .into_iter()
        .find(|row| row.alloc_id == alloc.as_str())
        .unwrap_or_else(|| panic!("workload {workload_id} has a row for {alloc}"))
}

/// Poll until `alloc`'s row reaches a terminal state, recording every state
/// observed on the way (`None` while no row exists yet).
async fn await_terminal_row(
    cfg: &Path,
    workload_id: &str,
    alloc: &AllocationId,
    bound: Duration,
) -> (AllocStatusRowBody, Vec<Option<AllocStateWire>>) {
    let deadline = Instant::now() + bound;
    let mut observed = Vec::new();
    loop {
        let row = describe_rows(cfg, workload_id)
            .await
            .into_iter()
            .find(|row| row.alloc_id == alloc.as_str());
        let state = row.as_ref().map(|row| row.state);
        if observed.last() != Some(&state) {
            observed.push(state);
        }
        if let Some(row) = row
            && matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
        {
            return (row, observed);
        }
        assert!(
            Instant::now() < deadline,
            "{alloc} of {workload_id} reached no terminal row within {bound:?}; states: {observed:?}",
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Await a Job's natural completion with exit 0 without first needing to see
/// it Running (a quick dialer can finish between two polls). The Job must keep
/// exactly one allocation: a restart or a failure is the dialer's
/// byte-distinct exchange failing.
async fn await_job_exit_zero(cfg: &Path, workload_id: &str, bound: Duration) -> AllocStatusRowBody {
    let deadline = Instant::now() + bound;
    loop {
        let rows = describe_rows(cfg, workload_id).await;
        match rows.as_slice() {
            [row] if row.state == AllocStateWire::Terminated && row.exit_code == Some(0) => {
                return row.clone();
            }
            [row] if matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed) => {
                panic!("the dialer Job {workload_id} ended without its exchange: {row:#?}")
            }
            [] | [_] => {}
            rows => panic!("the dialer Job {workload_id} was restarted: {rows:#?}"),
        }
        assert!(
            Instant::now() < deadline,
            "the dialer Job {workload_id} did not complete within {bound:?}; rows={rows:#?}",
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---------------------------------------------------------------------------
// Deploy and stop helpers
// ---------------------------------------------------------------------------

/// A Running VM allocation and the kernel identities its guest owns.
struct DeployedVm {
    workload_id: String,
    alloc: AllocationId,
    addr: Ipv4Addr,
    tap: String,
    ifindex: u32,
}

/// Deploy one workload spec and drive it to Running, returning its guest
/// identities. The caller owns cleanup.
async fn deploy_running(cfg: &Path, server_dir: &Path, file: &str, spec: &str) -> DeployedVm {
    let spec = write_toml(server_dir, file, spec);
    let output = deploy(DeployArgs { spec, config_path: cfg.to_path_buf() })
        .await
        .unwrap_or_else(|error| panic!("deploy {file} through the production handler: {error}"));
    let running = poll_until_running(cfg, &output.workload_id, RUNNING_BOUND).await;
    let row = running.snapshot.rows.first().expect("one Running allocation row");
    let alloc = AllocationId::new(&row.alloc_id).expect("allocation id parses");
    let addr = row.workload_addr.expect("a Running VM publishes its guest address");
    let tap = tap_for(addr);
    let ifindex = ifindex_of(&tap);
    DeployedVm { workload_id: output.workload_id, alloc, addr, tap, ifindex }
}

/// Deploy one long-lived spin VM and drive it to Running.
async fn deploy_spin_vm(
    cfg: &Path,
    server_dir: &Path,
    kernel: &Path,
    rootfs: &Path,
    id: &str,
) -> DeployedVm {
    deploy_running(
        cfg,
        server_dir,
        &format!("{id}.toml"),
        &vm_job_toml(id, "/sbin/spin", &[], kernel, rootfs),
    )
    .await
}

/// Deploy the mesh Service (`gti-peer`, workload `server`, reachable by
/// `MESH_NAME`) and drive it to Running.
async fn deploy_mesh_service(
    cfg: &Path,
    server_dir: &Path,
    kernel: &Path,
    rootfs: &Path,
    file: &str,
) -> DeployedVm {
    deploy_running(
        cfg,
        server_dir,
        file,
        &service_toml(Path::new("/sbin/gti-peer"), kernel, rootfs),
    )
    .await
}

/// Stop a workload through the production verb and assert it reaches
/// `Terminated`.
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

/// Cleanup at the tail of a body: stop every workload the body deployed —
/// including one whose killed allocation was replaced by a restarted
/// successor — and wait until none has a non-terminal row or a live Cloud
/// Hypervisor process, so no VM outlives the body.
async fn stop_and_await_quiescence(cfg: &Path, workload_ids: &[&str]) {
    for id in workload_ids {
        stop(StopArgs { id: (*id).to_owned(), config_path: cfg.to_path_buf() })
            .await
            .unwrap_or_else(|error| panic!("stop {id} through the production verb: {error}"));
    }
    for id in workload_ids {
        let deadline = Instant::now() + QUIESCENCE_BOUND;
        loop {
            let rows = describe_rows(cfg, id).await;
            let live: Vec<(String, AllocStateWire)> = rows
                .iter()
                .filter(|row| {
                    !matches!(row.state, AllocStateWire::Terminated | AllocStateWire::Failed)
                })
                .map(|row| (row.alloc_id.clone(), row.state))
                .collect();
            let hypervisors: Vec<u32> = rows
                .iter()
                .filter_map(|row| {
                    cloud_hypervisor_pid_for_alloc(
                        &AllocationId::new(&row.alloc_id).expect("allocation id parses"),
                    )
                })
                .collect();
            if live.is_empty() && hypervisors.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{id} did not quiesce within {QUIESCENCE_BOUND:?}: live rows {live:?}, \
                 Cloud Hypervisor pids {hypervisors:?}",
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// The empty-complement assertion R5 teardown must reach (FD § "[REF] Driven
/// port — TAP activation gate (D-295-R5)" (teardown converges on absence)): no
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
// Attachment identity: kernel objects by identity, not name (a restarted
// successor may reuse the released address, and so the TAP and pin names)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AttachmentIdentity {
    tap: String,
    ifindex: u32,
    ingress_pin_inode: u64,
    egress_pin_inode: u64,
}

impl AttachmentIdentity {
    /// Record a provisioned attachment's TAP ifindex and both link-pin inodes.
    fn capture(tap: &str) -> Self {
        let ingress_pin_inode = pin_inode(&ingress_link_pin(tap))
            .unwrap_or_else(|| panic!("the provisioned TAP {tap} has its ingress link pin"));
        let egress_pin_inode = pin_inode(&egress_link_pin(tap))
            .unwrap_or_else(|| panic!("the provisioned TAP {tap} has its egress link pin"));
        Self { tap: tap.to_owned(), ifindex: ifindex_of(tap), ingress_pin_inode, egress_pin_inode }
    }

    /// Whether this attachment's complement is empty: no link with its
    /// ifindex, no endpoint entry for its ifindex, and neither recorded link
    /// pin (by inode). A failed endpoint read is a failed observation.
    fn complement_absent(&self) -> bool {
        let tap_absent = interface_ifindex(&self.tap) != Some(self.ifindex);
        let endpoint_absent =
            !endpoint_present(endpoint_map_pin(), self.ifindex).unwrap_or_else(|error| {
                panic!("observe the endpoint entry for ifindex {}: {error}", self.ifindex)
            });
        let ingress_absent =
            pin_inode(&ingress_link_pin(&self.tap)) != Some(self.ingress_pin_inode);
        let egress_absent = pin_inode(&egress_link_pin(&self.tap)) != Some(self.egress_pin_inode);
        tap_absent && endpoint_absent && ingress_absent && egress_absent
    }
}

/// A booting allocation held in its pre-`READY` window: a TAP provisioned
/// administratively down, a Cloud Hypervisor process holding its queue, and a
/// guest whose holding init has not released.
struct BootingAttachment {
    alloc: AllocationId,
    hypervisor_pid: u32,
    identity: AttachmentIdentity,
}

/// Witness the booting window of the one allocation `workload_id` creates:
/// all of (1) exactly one new `ovd-tp-*` TAP, (2) no row of the workload has
/// left `Pending` (the start arm writes the allocation's first row only when
/// the start returns, so "no row or a Pending row" is the observable
/// `ProvisionedDown` phase), (3) the TAP reads back administratively down,
/// (4) a Cloud Hypervisor process holds its queue (fdinfo `iff:`), and (5)
/// that guest's console shows the holding init's hold marker and not its
/// release marker, so the guest has not reached `READY`.
///
/// A row that leaves `Pending` first, or a TAP that disappears, fails the
/// witness: the launch ended before its queue was held.
async fn witness_booting_attachment(
    cfg: &Path,
    client: &Client,
    workload_id: &str,
    taps_before: &BTreeSet<String>,
    bound: Duration,
) -> BootingAttachment {
    let deadline = Instant::now() + bound;
    let mut seen_taps: BTreeSet<String> = BTreeSet::new();
    let mut last = String::from("no new managed TAP yet");
    loop {
        let rows = describe_rows(cfg, workload_id).await;
        assert!(
            rows.iter().all(|row| row.state == AllocStateWire::Pending),
            "{workload_id} left Pending before its booting attachment was witnessed \
             (last observation: {last}); rows: {rows:#?}",
        );
        let new_taps: Vec<String> = list_managed_taps().difference(taps_before).cloned().collect();
        for tap in &seen_taps {
            assert!(
                new_taps.contains(tap),
                "{workload_id}'s TAP {tap} disappeared before its booting attachment was \
                 witnessed (last observation: {last})",
            );
        }
        seen_taps.extend(new_taps.iter().cloned());
        if let [tap] = new_taps.as_slice() {
            let administratively_up = client
                .observe_link(tap)
                .await
                .unwrap_or_else(|error| panic!("observe the booting TAP {tap}: {error}"));
            let holder = cloud_hypervisor_queue_holder(tap);
            let holder_alloc = holder.and_then(alloc_of_cloud_hypervisor);
            let holding = holder_alloc.as_ref().is_some_and(|alloc| {
                let console = console_log(alloc);
                console.contains(HOLD_MARKER) && !console.contains(RELEASE_MARKER)
            });
            if administratively_up == Some(false)
                && let (Some(hypervisor_pid), Some(alloc)) = (holder, holder_alloc.clone())
                && holding
            {
                return BootingAttachment {
                    alloc,
                    hypervisor_pid,
                    identity: AttachmentIdentity::capture(tap),
                };
            }
            last = format!(
                "TAP {tap} administratively_up={administratively_up:?} queue_holder={holder:?} \
                 holder_alloc={holder_alloc:?} console_holding={holding}"
            );
        } else if new_taps.len() > 1 {
            panic!("{workload_id} created more than one managed TAP: {new_taps:?}");
        }
        assert!(
            Instant::now() < deadline,
            "{workload_id}'s booting attachment was not witnessed within {bound:?}; \
             last observation: {last}",
        );
        tokio::time::sleep(POLL).await;
    }
}

// ---------------------------------------------------------------------------
// rtnetlink link monitor (RTMGRP_LINK): every RTM_NEWLINK / RTM_DELLINK,
// in kernel order, with overrun detection
// ---------------------------------------------------------------------------

const NLMSG_HEADER_LEN: usize = 16;
const IFINFOMSG_LEN: usize = 16;
const NLMSG_NOOP: u16 = 1;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLMSG_OVERRUN: u16 = 4;
const IFLA_IFNAME: u16 = 3;
const LINK_MONITOR_RECEIVE_BUFFER: libc::c_int = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkNoticeKind {
    New,
    Deleted,
}

#[derive(Debug, Clone)]
struct LinkNotice {
    kind: LinkNoticeKind,
    ifindex: u32,
    flags: u32,
    ifname: Option<String>,
}

impl LinkNotice {
    fn is_up(&self) -> bool {
        self.flags & libc::IFF_UP as u32 != 0
    }
}

/// Started before the stimulus; `finish` returns every notice or the reason
/// the stream cannot be trusted (an overrun is a lost notification).
struct LinkMonitor {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<Vec<LinkNotice>, String>>>,
}

impl LinkMonitor {
    fn start() -> Self {
        let fd = open_link_monitor_socket()
            .unwrap_or_else(|error| panic!("open and bind the rtnetlink link monitor: {error}"));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::spawn(move || monitor_links(&fd, &stop_thread));
        Self { stop, handle: Some(handle) }
    }

    fn finish(mut self) -> Result<Vec<LinkNotice>, String> {
        self.stop.store(true, Ordering::SeqCst);
        let handle = self.handle.take().ok_or("the link monitor thread was already joined")?;
        handle.join().map_err(|_| "the link monitor thread panicked".to_owned())?
    }
}

impl Drop for LinkMonitor {
    fn drop(&mut self) {
        // Panic-path cleanup only: the thread's result is already moot once a
        // body has failed, and `finish` is the path that reads it.
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            eprintln!("[link-monitor] the monitor thread panicked during cleanup");
        }
    }
}

fn open_link_monitor_socket() -> std::io::Result<OwnedFd> {
    // SAFETY: plain socket creation; the descriptor moves into `OwnedFd`.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::NETLINK_ROUTE,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let size = LINK_MONITOR_RECEIVE_BUFFER;
    // SAFETY: `fd` is live and the option value points to one integer.
    if unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVBUFFORCE,
            std::ptr::from_ref(&size).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&size))
                .expect("option length fits socklen_t"),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: an all-zero `sockaddr_nl` is valid; its public fields are set.
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    address.nl_groups = libc::RTMGRP_LINK as u32;
    // SAFETY: the live sockaddr has exactly the supplied size.
    if unsafe {
        libc::bind(
            fd.as_raw_fd(),
            std::ptr::from_ref(&address).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&address))
                .expect("sockaddr_nl length fits socklen_t"),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

fn monitor_links(fd: &OwnedFd, stop: &AtomicBool) -> Result<Vec<LinkNotice>, String> {
    let mut notices = Vec::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        // Read the flag before receiving, so a final empty receive after it
        // was set proves the queue is drained.
        let stopping = stop.load(Ordering::SeqCst);
        // SAFETY: `buffer` is live writable storage of the stated length.
        let received = unsafe {
            libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), libc::MSG_TRUNC)
        };
        if received < 0 {
            let error = std::io::Error::last_os_error();
            match error.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => {
                    if stopping {
                        return Ok(notices);
                    }
                    std::thread::sleep(Duration::from_micros(500));
                    continue;
                }
                _ if error.raw_os_error() == Some(libc::ENOBUFS) => {
                    return Err("the rtnetlink link monitor overran (ENOBUFS): link \
                                notifications were lost"
                        .to_owned());
                }
                _ => return Err(format!("the rtnetlink link monitor receive failed: {error}")),
            }
        }
        let length = usize::try_from(received).expect("recv length is non-negative");
        if length > buffer.len() {
            return Err(format!("an rtnetlink datagram of {length} bytes exceeded the buffer"));
        }
        parse_link_notices(&buffer[..length], &mut notices)?;
    }
}

fn read_ne_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
        .map(u16::from_ne_bytes)
        .ok_or_else(|| format!("truncated u16 at offset {offset}"))
}

fn read_ne_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map(u32::from_ne_bytes)
        .ok_or_else(|| format!("truncated u32 at offset {offset}"))
}

const fn netlink_align(length: usize) -> usize {
    (length + 3) & !3
}

fn parse_link_notices(datagram: &[u8], notices: &mut Vec<LinkNotice>) -> Result<(), String> {
    let mut offset = 0;
    while offset < datagram.len() {
        let length = read_ne_u32(datagram, offset)? as usize;
        let kind = read_ne_u16(datagram, offset + 4)?;
        let end = offset.checked_add(length).ok_or("rtnetlink message length overflows")?;
        if length < NLMSG_HEADER_LEN || end > datagram.len() {
            return Err(format!("malformed rtnetlink message length {length} at {offset}"));
        }
        let body = &datagram[offset + NLMSG_HEADER_LEN..end];
        match kind {
            libc::RTM_NEWLINK => notices.push(parse_link_notice(LinkNoticeKind::New, body)?),
            libc::RTM_DELLINK => notices.push(parse_link_notice(LinkNoticeKind::Deleted, body)?),
            NLMSG_ERROR | NLMSG_OVERRUN => {
                return Err(format!("rtnetlink reported message type {kind} to the link monitor"));
            }
            NLMSG_NOOP | NLMSG_DONE => {}
            other => return Err(format!("unexpected rtnetlink message type {other}")),
        }
        offset = netlink_align(end);
    }
    Ok(())
}

fn parse_link_notice(kind: LinkNoticeKind, body: &[u8]) -> Result<LinkNotice, String> {
    if body.len() < IFINFOMSG_LEN {
        return Err(format!("truncated ifinfomsg of {} bytes", body.len()));
    }
    let ifindex = read_ne_u32(body, 4)?;
    let flags = read_ne_u32(body, 8)?;
    let mut ifname = None;
    let mut offset = IFINFOMSG_LEN;
    while offset + 4 <= body.len() {
        let attribute_length = usize::from(read_ne_u16(body, offset)?);
        let attribute_type = read_ne_u16(body, offset + 2)? & 0x3fff;
        if attribute_length < 4 || offset + attribute_length > body.len() {
            return Err(format!("malformed rtattr length {attribute_length} at {offset}"));
        }
        if attribute_type == IFLA_IFNAME {
            let value = &body[offset + 4..offset + attribute_length];
            let value = value.split(|&byte| byte == 0).next().unwrap_or(&[]);
            ifname = Some(String::from_utf8_lossy(value).into_owned());
        }
        offset += netlink_align(attribute_length);
    }
    Ok(LinkNotice { kind, ifindex, flags, ifname })
}

// ---------------------------------------------------------------------------
// Raw AF_PACKET wire capture and injection (host-transmit path + per-TAP read)
// ---------------------------------------------------------------------------

const ETH_P_ALL: u16 = 0x0003;
const PACKET_STATISTICS: libc::c_int = 6;

/// `struct tpacket_stats` (`include/uapi/linux/if_packet.h`).
#[repr(C)]
#[derive(Default)]
struct PacketStatistics {
    packets: u32,
    drops: u32,
}

/// An AF_PACKET capture bound to one interface's exact ifindex, tallying the
/// frames that carry each caller-chosen needle. It sees what the interface
/// actually transmits and receives: a frame the TAP's TCX egress program drops
/// never reaches the packet tap. Dropping it closes the socket.
struct NeedleCapture {
    fd: OwnedFd,
    interface: String,
    needles: Vec<&'static [u8]>,
    counts: Vec<usize>,
    frame: Vec<u8>,
}

impl NeedleCapture {
    /// Open on an administratively up interface (a down one would queue a
    /// pending `ENETDOWN`).
    fn open(interface: &str, needles: &[&'static [u8]]) -> Self {
        let ifindex = ifindex_of(interface);
        // Protocol 0 hooks the socket into no receive path; the bind below
        // names `ETH_P_ALL` and the interface together. A socket created with
        // `ETH_P_ALL` is hooked on every interface until its bind, so it could
        // queue another interface's frames first.
        // SAFETY: AF_PACKET raw socket; the descriptor moves into `OwnedFd`.
        let raw = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        };
        assert!(
            raw >= 0,
            "open AF_PACKET capture on {interface}: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: `raw` is a fresh descriptor nothing else owns.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let size: libc::c_int = 4 * 1024 * 1024;
        // SAFETY: `fd` is live and the option value points to one integer.
        let buffered = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUFFORCE,
                std::ptr::from_ref(&size).cast(),
                libc::socklen_t::try_from(std::mem::size_of_val(&size))
                    .expect("option length fits socklen_t"),
            )
        };
        assert_eq!(
            buffered,
            0,
            "size the capture buffer on {interface}: {}",
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
                fd.as_raw_fd(),
                std::ptr::from_ref(&address).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::sockaddr_ll>())
                    .expect("sockaddr_ll length fits socklen_t"),
            )
        };
        assert_eq!(
            bound,
            0,
            "bind AF_PACKET capture on {interface}: {}",
            std::io::Error::last_os_error()
        );
        Self {
            fd,
            interface: interface.to_owned(),
            needles: needles.to_vec(),
            counts: vec![0; needles.len()],
            frame: vec![0; 65_536],
        }
    }

    /// Read every queued frame into the tally.
    fn drain(&mut self) {
        loop {
            // SAFETY: `self.frame` is a live writable buffer; `self.fd` is owned.
            let read = unsafe {
                libc::recv(
                    self.fd.as_raw_fd(),
                    self.frame.as_mut_ptr().cast(),
                    self.frame.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if read > 0 {
                let length = usize::try_from(read).expect("positive recv length");
                let frame = &self.frame[..length];
                for (needle, count) in self.needles.iter().zip(self.counts.iter_mut()) {
                    if frame.windows(needle.len()).any(|window| window == *needle) {
                        *count += 1;
                    }
                }
                continue;
            }
            let error = std::io::Error::last_os_error();
            match error.kind() {
                std::io::ErrorKind::WouldBlock => return,
                std::io::ErrorKind::Interrupted => {}
                _ if error.raw_os_error() == Some(libc::ENETDOWN) => panic!(
                    "the capture on {} reported ENETDOWN: the interface went down inside the \
                     evidence window",
                    self.interface
                ),
                _ => panic!("capture recv on {} failed: {error}", self.interface),
            }
        }
    }

    /// Drain until `needle` has been seen at least `wanted` times or `bound`
    /// elapses; returns whether it was.
    async fn await_count(&mut self, needle: &[u8], wanted: usize, bound: Duration) -> bool {
        let deadline = Instant::now() + bound;
        loop {
            self.drain();
            if self.count(needle) >= wanted {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn count(&self, needle: &[u8]) -> usize {
        let index = self
            .needles
            .iter()
            .position(|known| *known == needle)
            .unwrap_or_else(|| panic!("the capture on {} tallies this needle", self.interface));
        self.counts[index]
    }

    /// Fail on any frame the capture socket dropped: a lossy capture cannot
    /// support a zero-frame oracle.
    fn assert_lossless(&self) {
        let mut statistics = PacketStatistics::default();
        let mut length = libc::socklen_t::try_from(std::mem::size_of::<PacketStatistics>())
            .expect("tpacket_stats length fits socklen_t");
        // SAFETY: `statistics` is live writable storage of `length` bytes.
        let rc = unsafe {
            libc::getsockopt(
                self.fd.as_raw_fd(),
                libc::SOL_PACKET,
                PACKET_STATISTICS,
                std::ptr::from_mut(&mut statistics).cast(),
                &raw mut length,
            )
        };
        assert_eq!(
            rc,
            0,
            "read PACKET_STATISTICS of the capture on {}: {}",
            self.interface,
            std::io::Error::last_os_error()
        );
        assert_eq!(
            statistics.drops, 0,
            "the capture on {} dropped {} of {} frames; its tallies cannot support the oracle",
            self.interface, statistics.drops, statistics.packets,
        );
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

/// Send `repeat` host-originated frames to `dst_mac` out of the bridge device
/// (the host-transmit path, `br_dev_xmit`; increment-z substrate note). Each
/// send completes the bridge's forwarding decision synchronously.
fn inject_host_unicast(bridge_ifindex: u32, dst_mac: [u8; 6], needle: &[u8], repeat: usize) {
    // Protocol 0: a send-only socket needs no receive hook, so it queues no
    // frame of any interface. `sendto` names the device and the protocol.
    // SAFETY: AF_PACKET raw socket; the descriptor moves into `OwnedFd`.
    let raw = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) };
    assert!(raw >= 0, "open AF_PACKET injector: {}", std::io::Error::last_os_error());
    // SAFETY: `raw` is a fresh descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: zero-initialised sockaddr_ll before its fields are set.
    let mut address: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    address.sll_family = u16::try_from(libc::AF_PACKET).expect("AF_PACKET fits u16");
    address.sll_protocol = ETH_P_ALL.to_be();
    address.sll_ifindex = i32::try_from(bridge_ifindex).expect("bridge ifindex fits i32");
    address.sll_halen = 6;
    address.sll_addr[..6].copy_from_slice(&dst_mac);
    let frame = host_unicast_frame(dst_mac, needle);
    for _ in 0..repeat {
        // SAFETY: `frame` and `address` are live for this send; `fd` is owned.
        let sent = unsafe {
            libc::sendto(
                fd.as_raw_fd(),
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
}

// ---------------------------------------------------------------------------
// Bridge FDB and host→guest ICMP echo
// ---------------------------------------------------------------------------

/// One `bridge fdb show br <bridge>` entry.
#[derive(Debug, Clone)]
struct FdbEntry {
    mac: String,
    dev: Option<String>,
    /// `permanent` or `static`: a local or pinned entry, never a learned one.
    permanent: bool,
    /// A `self` entry of the device's own address list, not the bridge FDB.
    self_entry: bool,
}

fn bridge_fdb_entries(bridge: &str) -> Vec<FdbEntry> {
    let output = Command::new("bridge")
        .args(["fdb", "show", "br", bridge])
        .output()
        .expect("run bridge fdb show");
    assert!(
        output.status.success(),
        "bridge fdb show br {bridge}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("bridge fdb output is UTF-8");
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let mac = fields.next()?.to_owned();
            let mut entry = FdbEntry { mac, dev: None, permanent: false, self_entry: false };
            while let Some(token) = fields.next() {
                match token {
                    "dev" => entry.dev = fields.next().map(str::to_owned),
                    "permanent" | "static" => entry.permanent = true,
                    "self" => entry.self_entry = true,
                    _ => {}
                }
            }
            Some(entry)
        })
        .collect()
}

/// The standard Internet checksum of `bytes` (RFC 1071).
fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in bytes.chunks(2) {
        let word = match chunk {
            [high, low] => u16::from_be_bytes([*high, *low]),
            [high] => u16::from_be_bytes([*high, 0]),
            _ => unreachable!("chunks(2) yields one or two bytes"),
        };
        sum += u32::from(word);
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Send one ICMP echo request from the host to `destination` and wait up to
/// `bound` for its matching reply. Blocking: run it off the runtime thread.
fn icmp_echo(
    destination: Ipv4Addr,
    identifier: u16,
    payload: &[u8],
    bound: Duration,
) -> Result<Duration, String> {
    const SEQUENCE: u16 = 1;
    let started = Instant::now();
    // SAFETY: raw ICMP socket; the descriptor moves into `OwnedFd`.
    let raw = unsafe {
        libc::socket(libc::AF_INET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, libc::IPPROTO_ICMP)
    };
    if raw < 0 {
        return Err(format!("open a raw ICMP socket: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: `raw` is a fresh descriptor nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut request = Vec::with_capacity(8 + payload.len());
    request.extend_from_slice(&[8, 0, 0, 0]);
    request.extend_from_slice(&identifier.to_be_bytes());
    request.extend_from_slice(&SEQUENCE.to_be_bytes());
    request.extend_from_slice(payload);
    let checksum = internet_checksum(&request);
    request[2..4].copy_from_slice(&checksum.to_be_bytes());
    // SAFETY: an all-zero sockaddr_in is valid; its fields are set below.
    let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    address.sin_family = libc::AF_INET as libc::sa_family_t;
    address.sin_addr = libc::in_addr { s_addr: u32::from(destination).to_be() };
    // SAFETY: `request` and `address` are live for this call.
    let sent = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            std::ptr::from_ref(&address).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&address))
                .expect("sockaddr_in length fits socklen_t"),
        )
    };
    if sent != request.len() as isize {
        return Err(format!("send the echo request: {}", std::io::Error::last_os_error()));
    }
    let mut buffer = [0_u8; 2048];
    loop {
        let remaining = bound.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(format!("no echo reply from {destination} within {bound:?}"));
        }
        let mut poll = libc::pollfd { fd: fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let timeout =
            libc::c_int::try_from(remaining.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
        // SAFETY: `poll` is one live pollfd.
        let ready = unsafe { libc::poll(&raw mut poll, 1, timeout) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("poll the ICMP socket: {error}"));
        }
        if ready == 0 {
            continue;
        }
        // SAFETY: `buffer` is live writable storage of its length.
        let received =
            unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if received < 0 {
            return Err(format!("receive on the ICMP socket: {}", std::io::Error::last_os_error()));
        }
        let packet = &buffer[..usize::try_from(received).expect("recv length is non-negative")];
        let Some(&version_and_length) = packet.first() else { continue };
        let header_length = usize::from(version_and_length & 0x0f) * 4;
        let Some(icmp) = packet.get(header_length..) else { continue };
        let source = packet
            .get(12..16)
            .map(|octets| Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
        if source == Some(destination)
            && icmp.len() == 8 + payload.len()
            && icmp[0] == 0
            && icmp[1] == 0
            && icmp[4..6] == identifier.to_be_bytes()
            && icmp[6..8] == SEQUENCE.to_be_bytes()
            && &icmp[8..] == payload
        {
            return Ok(started.elapsed());
        }
    }
}

// ---------------------------------------------------------------------------
// The out-of-VM MAC hijack: `pidfd_getfd` a copy of the attacker's TAP queue
// as root, drop to uid 4200 with no capabilities, then `SIOCSIFHWADDR` on the
// held queue (increment-z STEPs 4-6; the process runs outside the D-295-R22
// launch filter, modelling a change the filter does not see).
//
// The forked child reports twice over a non-blocking pipe: once right after
// the ioctl (its result, the effective capability set, and the
// `CLOCK_MONOTONIC` instant of the change), and once when the parent closes
// its control pipe (how many victim-bound frames it read from the held
// queue). The parent awaits both with async sleeps, so the current-thread
// runtime that hosts the supervisor keeps running throughout.
// ---------------------------------------------------------------------------

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

/// `struct __user_cap_header_struct`.
#[repr(C)]
struct CapUserHeader {
    version: u32,
    pid: libc::c_int,
}

/// `struct __user_cap_data_struct`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapUserData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
const CHANGE_REPORT_LEN: usize = 28;
const READS_REPORT_LEN: usize = 8;
/// The child's own ceiling on its read window (200 polls of 50 ms), after
/// which it reports even if the parent never closes its control pipe.
const HIJACK_MAX_POLLS: u32 = 200;

/// What the child reports right after its `SIOCSIFHWADDR`.
#[derive(Debug)]
struct HijackChange {
    ioctl_rc: i32,
    ioctl_errno: i32,
    /// The child's effective capability set (two 32-bit words).
    cap_effective: [u32; 2],
    /// Whether the held queue descriptor is `O_NONBLOCK` (E2/F15).
    queue_nonblocking: bool,
    /// `CLOCK_MONOTONIC` immediately after the ioctl returned.
    changed_at: Duration,
}

/// What the child reports when its read window ends.
#[derive(Debug)]
struct HijackReads {
    /// Frames carrying [`HIJACK_NEEDLE`] read from the held queue.
    stolen: u32,
    /// Every frame read from the held queue.
    total: u32,
}

/// The forked hijack child and the parent's ends of its two pipes.
struct MacHijack {
    pid: libc::pid_t,
    report: OwnedFd,
    control: Option<OwnedFd>,
    reaped: bool,
}

/// Exit codes the child uses before it can report.
fn hijack_exit_meaning(code: i32) -> &'static str {
    match code {
        11 => "pidfd_open of the attacker's Cloud Hypervisor failed",
        12 => "pidfd_getfd of the attacker's queue (descriptor 3) failed",
        13 => "dropping to uid 4200 with no groups and no_new_privs failed",
        14 => "capget failed",
        15 => "writing a report to the parent failed",
        16 => "poll on the held queue failed",
        _ => "unexpected hijack child exit",
    }
}

fn nonblocking_pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: `fds` is a live two-element array.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    assert_eq!(rc, 0, "create a hijack pipe: {}", std::io::Error::last_os_error());
    // SAFETY: both descriptors are fresh and owned by nothing else.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

impl MacHijack {
    fn spawn(ch_pid: u32, victim_mac: [u8; 6]) -> Self {
        let (report_read, report_write) = nonblocking_pipe();
        let (control_read, control_write) = nonblocking_pipe();
        let target = libc::pid_t::try_from(ch_pid).expect("Cloud Hypervisor pid fits pid_t");
        // SAFETY: the child only performs async-signal-safe raw syscalls and
        // `_exit`; it never returns into Rust code of the forked runtime.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork the hijack child: {}", std::io::Error::last_os_error());
        if pid == 0 {
            // SAFETY: called exactly once, in the freshly forked child.
            unsafe {
                hijack_child(
                    target,
                    victim_mac,
                    report_write.as_raw_fd(),
                    control_read.as_raw_fd(),
                    [report_read.as_raw_fd(), control_write.as_raw_fd()],
                )
            }
        }
        drop(report_write);
        drop(control_read);
        Self { pid, report: report_read, control: Some(control_write), reaped: false }
    }

    /// Await the change report (bounded). A child that exits before it
    /// reports fails the body with the step it could not perform.
    async fn await_change(&mut self) -> HijackChange {
        let report: [u8; CHANGE_REPORT_LEN] = self.read_report("change").await;
        let word = |at: usize| u32::from_le_bytes(report[at..at + 4].try_into().expect("4 bytes"));
        let nanos = u64::from_le_bytes(report[20..28].try_into().expect("8 bytes"));
        HijackChange {
            ioctl_rc: word(0) as i32,
            ioctl_errno: word(4) as i32,
            cap_effective: [word(8), word(12)],
            queue_nonblocking: word(16) != 0,
            changed_at: Duration::from_nanos(nanos),
        }
    }

    /// Close the control pipe, which ends the child's read window, then await
    /// its read tallies and reap it.
    async fn finish(mut self) -> HijackReads {
        drop(self.control.take());
        let report: [u8; READS_REPORT_LEN] = self.read_report("reads").await;
        let reads = HijackReads {
            stolen: u32::from_le_bytes(report[0..4].try_into().expect("4 bytes")),
            total: u32::from_le_bytes(report[4..8].try_into().expect("4 bytes")),
        };
        let status = self.reap(Duration::from_secs(5)).await;
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "the hijack child exits 0 after reporting (status {status:#x})",
        );
        reads
    }

    async fn read_report<const N: usize>(&mut self, what: &str) -> [u8; N] {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut report = [0_u8; N];
        let mut filled = 0;
        while filled < N {
            // SAFETY: `report[filled..]` is live writable storage.
            let read = unsafe {
                libc::read(
                    self.report.as_raw_fd(),
                    report[filled..].as_mut_ptr().cast(),
                    N - filled,
                )
            };
            if read > 0 {
                filled += usize::try_from(read).expect("positive read length");
                continue;
            }
            if read == 0 {
                let status = self.reap(Duration::from_secs(5)).await;
                let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 };
                panic!(
                    "the hijack child ended before its {what} report (status {status:#x}): {}",
                    hijack_exit_meaning(code)
                );
            }
            let error = std::io::Error::last_os_error();
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ),
                "read the hijack child's {what} report: {error}",
            );
            assert!(
                Instant::now() < deadline,
                "the hijack child sent no {what} report within 10s (a blocking read on a queue \
                 that is not O_NONBLOCK can do this)",
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        report
    }

    async fn reap(&mut self, bound: Duration) -> libc::c_int {
        let deadline = Instant::now() + bound;
        loop {
            let mut status = 0;
            // SAFETY: `status` is a live out-parameter; `self.pid` is our child.
            let reaped = unsafe { libc::waitpid(self.pid, &raw mut status, libc::WNOHANG) };
            if reaped == self.pid {
                self.reaped = true;
                return status;
            }
            assert!(
                reaped == 0,
                "waitpid on the hijack child: {}",
                std::io::Error::last_os_error()
            );
            assert!(Instant::now() < deadline, "the hijack child did not exit within {bound:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl Drop for MacHijack {
    fn drop(&mut self) {
        if !self.reaped {
            // Panic-path cleanup: never leave a process holding the attacker's
            // queue behind a failed body.
            // SAFETY: `self.pid` is our unreaped child.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
                let mut status = 0;
                libc::waitpid(self.pid, &raw mut status, 0);
            }
        }
    }
}

/// Write all of `bytes` to `fd` with raw `write` (async-signal-safe).
///
/// # Safety
/// `fd` must be a descriptor the caller owns.
unsafe fn write_all_raw(fd: RawFd, bytes: &[u8]) -> bool {
    let mut written = 0;
    while written < bytes.len() {
        // SAFETY: `bytes[written..]` is live readable storage.
        let n = unsafe { libc::write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if n <= 0 {
            return false;
        }
        written += n as usize;
    }
    true
}

/// The hijack child's whole life. Only raw async-signal-safe syscalls, no
/// heap allocation, and `_exit` on every path.
///
/// # Safety
/// Must be called exactly once, in the freshly forked child.
unsafe fn hijack_child(
    target: libc::pid_t,
    victim_mac: [u8; 6],
    report: RawFd,
    control: RawFd,
    parent_ends: [RawFd; 2],
) -> ! {
    // SAFETY: single-threaded forked child; every call operates on its own
    // arguments and owned descriptors.
    unsafe {
        for fd in parent_ends {
            libc::close(fd);
        }
        let pidfd = libc::syscall(libc::SYS_pidfd_open, target, 0);
        if pidfd < 0 {
            libc::_exit(11);
        }
        let held = libc::syscall(libc::SYS_pidfd_getfd, pidfd, VMM_TAP_QUEUE_FD, 0);
        if held < 0 {
            libc::_exit(12);
        }
        let held = held as libc::c_int;
        // Drop every privilege before touching the queue (increment-z crux).
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::setgroups(0, std::ptr::null()) != 0
            || libc::setgid(OVERDRIVE_VMM_UID) != 0
            || libc::setuid(OVERDRIVE_VMM_UID) != 0
        {
            libc::_exit(13);
        }
        let mut header = CapUserHeader { version: LINUX_CAPABILITY_VERSION_3, pid: 0 };
        let mut data = [CapUserData::default(); 2];
        if libc::syscall(libc::SYS_capget, &raw mut header, data.as_mut_ptr()) != 0 {
            libc::_exit(14);
        }
        let nonblocking = libc::fcntl(held, libc::F_GETFL) & libc::O_NONBLOCK != 0;

        let mut ifreq: HwAddrIfreq = std::mem::zeroed();
        ifreq.sa_family = libc::ARPHRD_ETHER;
        ifreq.sa_data[..6].copy_from_slice(&victim_mac);
        let rc = libc::ioctl(held, libc::SIOCSIFHWADDR, &raw const ifreq);
        let errno = if rc == 0 { 0 } else { *libc::__errno_location() };
        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now);
        let changed_nanos = (now.tv_sec as u64) * 1_000_000_000 + now.tv_nsec as u64;

        let mut change = [0_u8; CHANGE_REPORT_LEN];
        change[0..4].copy_from_slice(&(rc as u32).to_le_bytes());
        change[4..8].copy_from_slice(&(errno as u32).to_le_bytes());
        change[8..12].copy_from_slice(&data[0].effective.to_le_bytes());
        change[12..16].copy_from_slice(&data[1].effective.to_le_bytes());
        change[16..20].copy_from_slice(&u32::from(nonblocking).to_le_bytes());
        change[20..28].copy_from_slice(&changed_nanos.to_le_bytes());
        if !write_all_raw(report, &change) {
            libc::_exit(15);
        }

        // Read the held queue until the parent closes the control pipe.
        let mut stolen: u32 = 0;
        let mut total: u32 = 0;
        let mut buffer = [0_u8; 16_384];
        for _ in 0..HIJACK_MAX_POLLS {
            let mut fds = [
                libc::pollfd { fd: held, events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: control, events: libc::POLLIN, revents: 0 },
            ];
            let ready = libc::poll(fds.as_mut_ptr(), 2, 50);
            if ready < 0 {
                if *libc::__errno_location() == libc::EINTR {
                    continue;
                }
                libc::_exit(16);
            }
            if fds[0].revents & libc::POLLIN != 0 {
                loop {
                    let read = libc::read(held, buffer.as_mut_ptr().cast(), buffer.len());
                    if read <= 0 {
                        break;
                    }
                    total = total.saturating_add(1);
                    let frame = &buffer[..read as usize];
                    if frame.windows(HIJACK_NEEDLE.len()).any(|window| window == HIJACK_NEEDLE) {
                        stolen = stolen.saturating_add(1);
                    }
                    if !nonblocking {
                        // A blocking queue: one read per readiness keeps a
                        // drained queue from parking this child; a frame the
                        // VMM reads first still can, and the parent's bounded
                        // report wait then fails the body closed.
                        break;
                    }
                }
            }
            if fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
                || fds[1].revents != 0
            {
                break;
            }
        }
        let mut reads = [0_u8; READS_REPORT_LEN];
        reads[0..4].copy_from_slice(&stolen.to_le_bytes());
        reads[4..8].copy_from_slice(&total.to_le_bytes());
        if !write_all_raw(report, &reads) {
            libc::_exit(15);
        }
        libc::_exit(0);
    }
}

// ---------------------------------------------------------------------------
// Guest programs
// ---------------------------------------------------------------------------

/// A standing mesh client: one byte-distinct REQUEST/RESPONSE exchange with
/// the mesh Service by name every 250 ms, for as long as it runs. A failed
/// exchange (for example while its TAP is quiesced) is retried on the next
/// round, and the process never exits on its own, so its allocation stays the
/// same across a repair.
fn build_looping_mesh_client(tmp: &Path) -> PathBuf {
    let reply_len = RESPONSE.len();
    let source = format!(
        r#"
use std::io::{{Read, Write}};
use std::net::{{TcpStream, ToSocketAddrs}};
use std::time::Duration;

fn exchange() -> bool {{
    let Ok(addrs) = ("{MESH_NAME}", {SERVICE_PORT}).to_socket_addrs() else {{
        return false;
    }};
    for addr in addrs {{
        let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {{
            continue;
        }};
        if stream.set_read_timeout(Some(Duration::from_secs(2))).is_err() {{
            continue;
        }}
        if stream.write_all(&{REQUEST:?}).is_err() || stream.flush().is_err() {{
            continue;
        }}
        let mut got = vec![0_u8; {reply_len}];
        if stream.read_exact(&mut got).is_ok() && got == {RESPONSE:?} {{
            return true;
        }}
    }}
    false
}}

fn main() {{
    loop {{
        exchange();
        std::thread::sleep(Duration::from_millis(250));
    }}
}}
"#
    );
    build_static_binary(tmp, "nd295-mesh-loop", &source)
}

/// A quick mesh dialer: one byte-distinct exchange by name, then exit 0; it
/// exits non-zero if no exchange succeeds within its own 30 s budget.
fn build_quick_dialer(tmp: &Path) -> PathBuf {
    build_mesh_guest_with_timing(tmp, "nd295-dial", 0, 0)
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
    let _teardown = TeardownBound::arm();
    assert_root();
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

    let killed = wait_until(PER_ALLOCATION_KILL_WAIT, || {
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
        wait_until(TERMINAL_BOUND, || interface_ifindex(&victim.tap) != Some(victim.ifindex)).await,
        "the killed victim's teardown converges on TAP absence",
    );
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(victim.alloc.as_str())).await,
        "the killed victim's lease is released\n{}",
        events.render(),
    );

    stop_and_await_quiescence(
        &cfg,
        &[&survivor.workload_id, &victim.workload_id, &reopened.workload_id],
    )
    .await;
    handle.shutdown().await.expect("clean shutdown");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-30B — A still-booting VM's deleted TAP stops only that VM
/// CONTRACT_SHAPE: bounded-change.
///
/// Case (f): the target boots a guest image whose holding init delays `READY`
/// by [`super::delayed_ready_guest::READY_HOLD`], so its allocation stays
/// `ProvisionedDown` deterministically. The TAP is deleted from outside only
/// once the body has witnessed that window: the workload has no row beyond
/// `Pending`, the TAP reads back administratively down, a Cloud Hypervisor
/// process holds its queue, and the guest's console shows it still holding.
/// The audit reports the damage while EXEC stays Open, only that VM is killed
/// (`attachment_damaged`), its hypervisor ends, its start is rejected through
/// the VMM-exit path (a Failed row, never Running), teardown converges on
/// absence, and its lease is released. The mesh Service survivor keeps serving:
/// a fresh dialer Job completes a byte-distinct exchange with it by name.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-30B)"]
async fn a_booting_vms_deleted_tap_stops_only_that_vm() {
    let _teardown = TeardownBound::arm();
    assert_root();
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-30b-f-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let dialer = build_quick_dialer(tmp.path());
    let rootfs = stage_rootfs_with_extra_binaries(
        tmp.path(),
        &fixture,
        &[(&peer, "gti-peer"), (&dialer, "nd295-dial")],
    );
    let spin = build_spin_binary(tmp.path());
    let delayed_rootfs =
        stage_holding_rootfs(tmp.path(), &fixture, HoldingInit::DelayedReady, &[(&spin, "spin")]);
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let client = Client::new().expect("open typed host-netlink client");

    // The survivor: the mesh Service, Active.
    let survivor = deploy_mesh_service(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-30bf-service.toml",
    )
    .await;
    await_active(&client, &survivor.tap).await;
    let survivor_pid = cloud_hypervisor_pid_for_alloc(&survivor.alloc)
        .expect("the survivor's Cloud Hypervisor process is live");
    let survivor_restarts =
        row_of(&cfg, &survivor.workload_id, &survivor.alloc).await.restart_count;

    // The target: its guest holds READY, so its start waits in the boot race.
    let taps_before = list_managed_taps();
    let target_spec = write_toml(
        server_tmp.path(),
        "nd295-30bf-target.toml",
        &vm_job_toml("nd295-30bf-target", "/sbin/spin", &[], &fixture.kernel_path, &delayed_rootfs),
    );
    let target = deploy(DeployArgs { spec: target_spec, config_path: cfg.clone() })
        .await
        .expect("deploy the delayed-READY target through the production handler");
    let booting =
        witness_booting_attachment(&cfg, &client, &target.workload_id, &taps_before, RUNNING_BOUND)
            .await;

    // Fault: delete the ProvisionedDown TAP from outside.
    client
        .del_link(&booting.identity.tap)
        .await
        .expect("external actor deletes the still-booting TAP");

    // Only the target is killed, for attachment damage, while EXEC stays Open.
    let killed = wait_until(PER_ALLOCATION_KILL_WAIT, || {
        events.killed(Some("attachment_damaged")).contains(booting.alloc.as_str())
    })
    .await;
    assert!(killed, "only the booting target is killed for attachment damage\n{}", events.render());
    assert_eq!(
        events.killed(None),
        BTreeSet::from([booting.alloc.as_str().to_owned()]),
        "no allocation other than the booting target is killed",
    );
    // The kill while Open is a one-write loop (user decision 1 of 2026-09-30):
    // only the booting target's scope is written. No workloads-slice kill and
    // no fail-stop request occur — a missed kill-write bound would slice-kill
    // the survivor instead. The one-write loop's time budget is one audit
    // period plus the full-audit ceiling plus one per-VM kill-write bound, well
    // within the generous `PER_ALLOCATION_KILL_WAIT` this native lane waits.
    assert_eq!(
        events.fail_stops(),
        0,
        "the kill while Open never slice-kills or fail-stops\n{}",
        events.render(),
    );
    assert!(
        wait_until(TERMINAL_BOUND, || process_has_ended(booting.hypervisor_pid)).await,
        "the target's Cloud Hypervisor process {} ends",
        booting.hypervisor_pid,
    );

    // Its start is rejected through the VMM-exit path: a Failed row, never Running.
    let (row, states) =
        await_terminal_row(&cfg, &target.workload_id, &booting.alloc, TERMINAL_BOUND).await;
    assert_eq!(
        row.state,
        AllocStateWire::Failed,
        "the killed target's start is rejected: {row:#?}"
    );
    assert!(
        matches!(row.reason, Some(TransitionReason::VmGuestExitUnreported { .. })),
        "the rejection is the VMM-exit start rejection: {row:#?}",
    );
    assert!(
        !states.contains(&Some(AllocStateWire::Running)),
        "the killed target never reached Running: {states:?}",
    );

    // Teardown converges on absence and the lease is released.
    assert!(
        wait_until(TERMINAL_BOUND, || booting.identity.complement_absent()).await,
        "the killed target's teardown converges on an empty complement: {:?}",
        booting.identity,
    );
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(booting.alloc.as_str())).await,
        "the killed target's lease is released\n{}",
        events.render(),
    );

    // EXEC stayed Open: no node-level detection, no fail-stop, and the survivor
    // is the same untouched allocation.
    assert!(
        events.exec_stayed_open(),
        "per-allocation damage never closes EXEC\n{}",
        events.render()
    );
    assert!(
        !process_has_ended(survivor_pid),
        "the survivor's Cloud Hypervisor process keeps running"
    );
    let survivor_row = row_of(&cfg, &survivor.workload_id, &survivor.alloc).await;
    assert_eq!(survivor_row.state, AllocStateWire::Running, "the survivor stays Running");
    assert_eq!(survivor_row.restart_count, survivor_restarts, "the survivor was never restarted");

    // The survivor keeps serving, and admission is open: a fresh dialer Job
    // completes a byte-distinct exchange with it by name.
    let dial_spec = write_toml(
        server_tmp.path(),
        "nd295-30bf-dial.toml",
        &vm_job_toml("nd295-30bf-dial", "/sbin/nd295-dial", &[], &fixture.kernel_path, &rootfs),
    );
    let dial = deploy(DeployArgs { spec: dial_spec, config_path: cfg.clone() })
        .await
        .expect("deploy a fresh dialer after the kill");
    await_job_exit_zero(&cfg, &dial.workload_id, DIAL_BOUND).await;

    stop_and_await_quiescence(
        &cfg,
        &[&target.workload_id, &dial.workload_id, &survivor.workload_id],
    )
    .await;
    handle.shutdown().await.expect("clean shutdown");
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
    let _teardown = TeardownBound::arm();
    assert_root();
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
    let mut targets = Vec::new();
    for (index, damage) in ["ingress-link", "egress-link", "guard-member"].into_iter().enumerate() {
        let target = deploy_spin_vm(
            &cfg,
            server_tmp.path(),
            &fixture.kernel_path,
            &rootfs,
            &format!("nd295-30bg-{}", index + 1),
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

        let killed = wait_until(PER_ALLOCATION_KILL_WAIT, || {
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
            wait_until(TERMINAL_BOUND, || interface_ifindex(&target.tap) != Some(target.ifindex))
                .await,
            "{damage}: the killed target's teardown converges on absence",
        );
        assert!(
            wait_until(TERMINAL_BOUND, || events.released().contains(target.alloc.as_str())).await,
            "{damage}: the killed target's lease is released\n{}",
            events.render(),
        );
        targets.push(target.workload_id);
    }

    let mut workloads: Vec<&str> = targets.iter().map(String::as_str).collect();
    workloads.push(&bystander.workload_id);
    stop_and_await_quiescence(&cfg, &workloads).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-66 — a stop succeeds even when parts of the attachment are gone,
// and a failed launch leaves no TAP and no queue holder
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
    let _teardown = TeardownBound::arm();
    assert_root();
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
    // The TAP deletion detaches its TCX links, but the ingress link's bpffs pin
    // outlives the TAP; the external actor removes it too. Unconditional, so
    // the stimulus is always applied: a missing pin fails here rather than
    // silently skipping one of the three removed parts.
    std::fs::remove_file(ingress_link_pin(&vm.tap))
        .expect("external actor removes the ingress link pin that outlived its TAP");
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
/// The launch fails after provision and after the queue attach: the guest
/// image's holding init powers the guest off before `READY`
/// (`HoldingInit::PowerOffBeforeReady`, after
/// [`super::delayed_ready_guest::POWER_OFF_HOLD`]), so Cloud Hypervisor exits
/// while the start waits in the boot race, and the start is rejected through
/// the existing VMM-exit path, exactly the path a hypervisor that rejects its
/// kernel after spawning takes, with a window long enough to observe. A
/// missing kernel is not used: its preflight fails before the hypervisor ever
/// holds a queue, so it cannot show the holder was released.
///
/// Oracles: (1) positive witness that the launch got past provision and the
/// queue attach: the TAP read down while a Cloud Hypervisor process held its
/// queue (fdinfo `iff:`), before `READY`; (2) an rtnetlink monitor started
/// before deploy shows the TAP never administratively up and its
/// `RTM_DELLINK` observed; (3) the witnessed holder ended, and no new detached
/// `/dev/net/tun` descriptor appeared anywhere on the host (a descriptor still
/// attached when its TAP is deleted reads back with an empty `iff:`, so a
/// holder that outlived the TAP shows up exactly as that); (4) the TAP and its
/// whole attachment are gone; (5) the lease is released
/// (`guest_network.lease_released`). (4) and (5) are judged before the
/// workload is stopped, so they are the failed start's own cleanup; (2) and
/// (3) are judged after the stop has quiesced any restarted attempt, so no
/// attempt is mid-provision while the host is inventoried.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-66)"]
async fn a_launch_that_fails_leaves_no_tap_and_no_queue_holder() {
    let _teardown = TeardownBound::arm();
    assert_root();
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-66-launch-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let spin = build_spin_binary(tmp.path());
    let power_off_rootfs = stage_holding_rootfs(
        tmp.path(),
        &fixture,
        HoldingInit::PowerOffBeforeReady,
        &[(&spin, "spin")],
    );
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let client = Client::new().expect("open typed host-netlink client");

    let taps_before = list_managed_taps();
    let detached_before: BTreeSet<TunDescriptor> =
        all_tun_descriptors().into_iter().filter(|descriptor| descriptor.iff.is_empty()).collect();
    let monitor = LinkMonitor::start();

    let spec = write_toml(
        server_tmp.path(),
        "nd295-66-power-off.toml",
        &vm_job_toml(
            "nd295-66-power-off",
            "/sbin/spin",
            &[],
            &fixture.kernel_path,
            &power_off_rootfs,
        ),
    );
    let output = deploy(DeployArgs { spec, config_path: cfg.clone() })
        .await
        .expect("deploy the power-off-before-READY workload");

    // (1) The launch got past provision and the queue attach.
    let launched =
        witness_booting_attachment(&cfg, &client, &output.workload_id, &taps_before, RUNNING_BOUND)
            .await;
    assert!(
        descriptor_three_holds_tap_queue(launched.hypervisor_pid, &launched.identity.tap),
        "the hypervisor holds the TAP's queue at descriptor 3",
    );

    // The launch fails through the VMM-exit start rejection.
    let (row, states) = await_terminal_row(
        &cfg,
        &output.workload_id,
        &launched.alloc,
        HoldingInit::PowerOffBeforeReady.hold() + TERMINAL_BOUND,
    )
    .await;
    assert_eq!(row.state, AllocStateWire::Failed, "the launch fails: {row:#?}");
    assert!(
        matches!(row.reason, Some(TransitionReason::VmGuestExitUnreported { .. })),
        "the launch fails through the VMM-exit start rejection: {row:#?}",
    );
    assert!(
        !states.contains(&Some(AllocStateWire::Running)),
        "the failed launch never reached Running: {states:?}",
    );

    // (4) The TAP and its whole attachment are gone (the start-failure
    // teardown runs before the Failed row is written).
    assert!(
        wait_until(TERMINAL_BOUND, || launched.identity.complement_absent()).await,
        "the failed launch leaves no TAP, endpoint entry, or link pin: {:?}",
        launched.identity,
    );
    // (5) The lease is released.
    assert!(
        wait_until(TERMINAL_BOUND, || events.released().contains(launched.alloc.as_str())).await,
        "the failed launch releases its lease\n{}",
        events.render(),
    );

    // Stop the workload so no restarted attempt is in flight while the host
    // is inventoried and the monitor is read.
    stop_and_await_quiescence(&cfg, &[&output.workload_id]).await;

    // (3) No queue holder remains.
    assert!(
        process_has_ended(launched.hypervisor_pid),
        "the witnessed queue holder {} has ended",
        launched.hypervisor_pid,
    );
    let new_detached: Vec<TunDescriptor> = all_tun_descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.iff.is_empty() && !detached_before.contains(descriptor))
        .collect();
    assert!(
        new_detached.is_empty(),
        "no process kept a queue of the deleted TAP (new detached /dev/net/tun descriptors: \
         {new_detached:?})",
    );

    // (2) The TAP was never up, and its deletion was observed.
    let notices = monitor.finish().unwrap_or_else(|reason| {
        panic!("the link monitor's stream cannot support the oracle: {reason}")
    });
    let ifindex = launched.identity.ifindex;
    let of_tap: Vec<&LinkNotice> =
        notices.iter().filter(|notice| notice.ifindex == ifindex).collect();
    assert!(
        of_tap.iter().any(|notice| {
            notice.kind == LinkNoticeKind::New
                && notice.ifname.as_deref() == Some(launched.identity.tap.as_str())
        }),
        "the monitor saw {} (ifindex {ifindex}) created: {of_tap:#?}",
        launched.identity.tap,
    );
    let up: Vec<&&LinkNotice> = of_tap.iter().filter(|notice| notice.is_up()).collect();
    assert!(up.is_empty(), "the failed launch's TAP was never administratively up: {up:#?}");
    let deletions = of_tap.iter().filter(|notice| notice.kind == LinkNoticeKind::Deleted).count();
    assert_eq!(deletions, 1, "the TAP's RTM_DELLINK was observed exactly once: {of_tap:#?}");
    assert_eq!(
        of_tap.last().map(|notice| notice.kind),
        Some(LinkNoticeKind::Deleted),
        "nothing follows the TAP's deletion: {of_tap:#?}",
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
/// victim V's guest MAC with `SIOCSIFHWADDR`. Pre-control RED oracle:
/// increment-z STEPs 4-6.
///
/// Every capture is opened before the change, on both Active TAPs, and every
/// while-poisoned stimulus is injected and read within a few hundred
/// milliseconds of it, while the hijack child still holds A's queue: A's kill
/// can land at the next audit, but A's TAP and the poisoned entry stand until
/// A's teardown, which follows the kill by at least the one-second restart
/// backoff. Oracles, in the E12 (h) numbering:
/// (1) A's TAP transmits zero frames addressed to V, the held queue reads
/// zero, and the shared `EgressDestinationDrop` slot rises by at least the
/// frames sent; (2) host unicast to A's own guest MAC reaches A's TAP, and a
/// host broadcast reaches both; (3) V's TAP receives no host unicast while the
/// entry is poisoned (and did before the change); (4) within one audit period
/// of the change (plus the full-audit latency ceiling plus one per-VM kill
/// write — the kill while Open is a one-write loop and its event follows the
/// write, user decision 1 of 2026-09-30) only A is killed for attachment
/// damage, EXEC stays Open, V is untouched, and A's teardown by its ordinary
/// lifecycle is timed from the kill; (5) within 1 s of A's complement
/// read-back, V's MAC is on no port but V's, a host→V ICMP echo is answered,
/// and V's reply re-learns V's MAC as a learned, non-permanent entry on V's
/// port.
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-67)"]
async fn a_mac_hijack_from_outside_the_vm_steals_nothing_and_the_victim_recovers() {
    let _teardown = TeardownBound::arm();
    assert_root();
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
    let client = Client::new().expect("open typed host-netlink client");

    let attacker =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-67-attacker")
            .await;
    let victim =
        deploy_spin_vm(&cfg, server_tmp.path(), &fixture.kernel_path, &rootfs, "nd295-67-victim")
            .await;
    await_active(&client, &attacker.tap).await;
    await_active(&client, &victim.tap).await;
    let bridge = bridge_of(&attacker.tap);
    assert_eq!(bridge_of(&victim.tap), bridge, "both TAPs are ports of the one shared bridge");
    let bridge_ifindex = ifindex_of(&bridge);
    let victim_mac = guest_mac(victim.addr);
    let attacker_mac = guest_mac(attacker.addr);
    let attacker_identity = AttachmentIdentity::capture(&attacker.tap);

    let attacker_pid = cloud_hypervisor_pid_for_alloc(&attacker.alloc)
        .expect("the attacker VM's Cloud Hypervisor process is live");
    assert!(
        descriptor_three_holds_tap_queue(attacker_pid, &attacker.tap),
        "the attacker VMM holds its own TAP queue at descriptor 3",
    );
    let victim_pid = cloud_hypervisor_pid_for_alloc(&victim.alloc)
        .expect("the victim VM's Cloud Hypervisor process is live");
    let victim_restarts = row_of(&cfg, &victim.workload_id, &victim.alloc).await.restart_count;

    let needles = [PRE_CHANGE_NEEDLE, HIJACK_NEEDLE, ATTACKER_OWN_NEEDLE, BROADCAST_NEEDLE];
    let mut attacker_capture = NeedleCapture::open(&attacker.tap, &needles);
    let mut victim_capture = NeedleCapture::open(&victim.tap, &needles);

    // Pre-change control: V's MAC is unknown to the FDB, so host unicast to it
    // floods, and only V's egress classifier admits it.
    inject_host_unicast(bridge_ifindex, victim_mac, PRE_CHANGE_NEEDLE, INJECTED_FRAMES);
    tokio::time::sleep(SETTLE).await;
    attacker_capture.drain();
    victim_capture.drain();
    assert_eq!(
        victim_capture.count(PRE_CHANGE_NEEDLE),
        INJECTED_FRAMES,
        "before the change, host unicast to V's guest MAC reaches V's TAP",
    );
    assert_eq!(
        attacker_capture.count(PRE_CHANGE_NEEDLE),
        0,
        "before the change, A's TAP transmits no frame addressed to V",
    );

    // The change: A's host-side MAC becomes V's guest MAC.
    let mut hijack = MacHijack::spawn(attacker_pid, victim_mac);
    let change = hijack.await_change().await;
    assert_eq!(
        change.ioctl_rc, 0,
        "SIOCSIFHWADDR succeeds at uid 4200 with CapEff=0 (the hazard); errno {}, held queue \
         O_NONBLOCK {}",
        change.ioctl_errno, change.queue_nonblocking,
    );
    assert_eq!(change.cap_effective, [0, 0], "the hijacker holds no capabilities: {change:?}");

    // Oracle (1): host unicast to V's MAC now follows the poisoned entry to A's
    // port, where A's egress classifier drops every frame.
    let drops_before = read_counter(counter_map_pin(), GuestTcxCounter::EgressDestinationDrop)
        .expect("read the node-wide egress-destination-drop counter before the steal attempt");
    inject_host_unicast(bridge_ifindex, victim_mac, HIJACK_NEEDLE, INJECTED_FRAMES);
    tokio::time::sleep(SETTLE).await;
    let drops_after = read_counter(counter_map_pin(), GuestTcxCounter::EgressDestinationDrop)
        .expect("read the node-wide egress-destination-drop counter after the steal attempt");
    // Oracle (2) stimuli.
    inject_host_unicast(bridge_ifindex, attacker_mac, ATTACKER_OWN_NEEDLE, INJECTED_FRAMES);
    inject_host_unicast(bridge_ifindex, [0xff; 6], BROADCAST_NEEDLE, INJECTED_FRAMES);
    tokio::time::sleep(SETTLE).await;
    attacker_capture.drain();
    victim_capture.drain();
    let evidence_closed_at = monotonic_now();
    let reads = hijack.finish().await;
    attacker_capture.assert_lossless();
    victim_capture.assert_lossless();
    assert_eq!(
        interface_ifindex(&attacker.tap),
        Some(attacker.ifindex),
        "harness: A's TAP stood through the whole evidence window ({:?} after the change)",
        evidence_closed_at.saturating_sub(change.changed_at),
    );

    // Oracle (1): A's TAP transmitted nothing addressed to V (the primary
    // oracle), the held queue read nothing addressed to V, and the shared
    // drop slot counted at least every frame sent.
    assert_eq!(
        attacker_capture.count(HIJACK_NEEDLE),
        0,
        "(1) A's TAP transmits zero frames addressed to V",
    );
    assert_eq!(
        reads.stolen, 0,
        "(1) the attacker reads zero frames addressed to V from its held queue ({} of the {} \
         frames it read carried V's needle)",
        reads.stolen, reads.total,
    );
    let dropped = drops_after.saturating_sub(drops_before);
    assert!(
        dropped >= INJECTED_FRAMES as u64,
        "(1) EgressDestinationDrop rises by at least the {INJECTED_FRAMES} frames sent \
         (rose by {dropped})",
    );
    // Oracle (2): positive controls on the same capture sockets.
    assert_eq!(
        attacker_capture.count(ATTACKER_OWN_NEEDLE),
        INJECTED_FRAMES,
        "(2) host unicast to A's own guest MAC still reaches A's TAP",
    );
    assert_eq!(
        attacker_capture.count(BROADCAST_NEEDLE),
        INJECTED_FRAMES,
        "(2) a host broadcast reaches A's TAP",
    );
    assert_eq!(
        victim_capture.count(BROADCAST_NEEDLE),
        INJECTED_FRAMES,
        "(2) a host broadcast reaches V's TAP",
    );
    assert_eq!(
        victim_capture.count(ATTACKER_OWN_NEEDLE),
        0,
        "(2) only A's egress classifier admits unicast to A's guest MAC",
    );
    // Oracle (3): nothing addressed to V reached V while the entry was poisoned.
    assert_eq!(
        victim_capture.count(HIJACK_NEEDLE),
        0,
        "(3) while the entry is poisoned, V's TAP receives no host unicast",
    );
    drop(attacker_capture);
    drop(victim_capture);

    // Oracle (4): the next audit kills only A, within one audit period of the
    // change plus the full-audit latency ceiling plus one per-VM kill write
    // (the kill while Open is a one-write loop and the event follows the write,
    // user decision 1 of 2026-09-30); EXEC stays Open.
    assert!(
        wait_until(KILL_AFTER_CHANGE_BOUND, || {
            events.killed(Some("attachment_damaged")).contains(attacker.alloc.as_str())
        })
        .await,
        "(4) the audit kills the attacker for its host-side MAC damage\n{}",
        events.render(),
    );
    let killed_at = events
        .first_kill_at(&attacker.alloc, "attachment_damaged")
        .expect("the attacker's kill event was captured");
    assert!(
        killed_at
            .checked_sub(change.changed_at)
            .is_some_and(|interval| interval <= KILL_AFTER_CHANGE_BOUND),
        "(4) the kill lands within {KILL_AFTER_CHANGE_BOUND:?} of the change (one audit period, the \
         full-audit ceiling, and one per-VM kill write) (change at {:?}, kill at {killed_at:?})",
        change.changed_at,
    );
    assert_eq!(
        events.killed(None),
        BTreeSet::from([attacker.alloc.as_str().to_owned()]),
        "(4) the victim and every other allocation are untouched by the kill",
    );
    assert!(events.exec_stayed_open(), "(4) EXEC stays Open\n{}", events.render());
    assert!(!process_has_ended(victim_pid), "(4) the victim's Cloud Hypervisor keeps running");
    let victim_row = row_of(&cfg, &victim.workload_id, &victim.alloc).await;
    assert_eq!(victim_row.state, AllocStateWire::Running, "(4) the victim stays Running");
    assert_eq!(victim_row.restart_count, victim_restarts, "(4) the victim is never restarted");

    // A's teardown by its ordinary lifecycle; the kill→teardown interval is
    // recorded.
    assert!(
        wait_until(TERMINAL_BOUND, || attacker_identity.complement_absent()).await,
        "(4) the attacker's teardown returns its empty complement: {attacker_identity:?}",
    );
    let complement_at = monotonic_now();
    let kill_to_teardown = complement_at.saturating_sub(killed_at);
    eprintln!("[S-ND295-67] kill→teardown interval (complement read back): {kill_to_teardown:?}");

    // Oracle (5): within 1 s of the complement read-back, V's MAC is on no
    // port but V's, a host→V echo is answered, and V's reply re-learns V's
    // MAC as a learned, non-permanent entry on V's port.
    let victim_mac_text = mac_text(victim_mac);
    let foreign: Vec<FdbEntry> = bridge_fdb_entries(&bridge)
        .into_iter()
        .filter(|entry| entry.mac == victim_mac_text && entry.dev.as_deref() != Some(&victim.tap))
        .collect();
    assert!(
        foreign.is_empty(),
        "(5) after A's teardown V's MAC is on no port but V's: {foreign:#?}"
    );
    let remaining = RELEARN_BOUND.saturating_sub(monotonic_now().saturating_sub(complement_at));
    let victim_addr = victim.addr;
    let identifier = u16::try_from(std::process::id() & 0xffff).expect("masked to 16 bits");
    let echo = tokio::task::spawn_blocking(move || {
        icmp_echo(victim_addr, identifier, ECHO_PAYLOAD, remaining)
    })
    .await
    .expect("the echo task joins");
    let rtt = echo.unwrap_or_else(|reason| panic!("(5) the host→V echo is answered: {reason}"));
    let relearned = wait_until(
        RELEARN_BOUND.saturating_sub(monotonic_now().saturating_sub(complement_at)),
        || {
            let entries = bridge_fdb_entries(&bridge);
            let of_victim: Vec<&FdbEntry> =
                entries.iter().filter(|entry| entry.mac == victim_mac_text).collect();
            !of_victim.is_empty()
                && of_victim.iter().all(|entry| entry.dev.as_deref() == Some(victim.tap.as_str()))
                && of_victim.iter().any(|entry| !entry.permanent && !entry.self_entry)
        },
    )
    .await;
    let within = monotonic_now().saturating_sub(complement_at);
    assert!(
        relearned && within <= RELEARN_BOUND,
        "(5) V's MAC is re-learned as a learned, non-permanent entry only on V's port within \
         {RELEARN_BOUND:?} of A's complement (echo rtt {rtt:?}, observed after {within:?}): {:#?}",
        bridge_fdb_entries(&bridge)
            .into_iter()
            .filter(|entry| entry.mac == victim_mac_text)
            .collect::<Vec<_>>(),
    );

    stop_and_await_quiescence(&cfg, &[&attacker.workload_id, &victim.workload_id]).await;
    handle.shutdown().await.expect("clean shutdown");
}

// ---------------------------------------------------------------------------
// S-ND295-69 — losing the whole program table with live mesh VMs is repaired in place
// ---------------------------------------------------------------------------

/// Outcome anchor: OUT-ND295-BORN-CAPTURED
/// S-ND295-69 — A deleted program table is repaired in place with live mesh VMs
/// CONTRACT_SHAPE: bounded-change.
///
/// A mesh Service and a standing mesh client (one byte-distinct exchange every
/// 250 ms, never exiting) are Running and Active, and the peer-authored
/// RESPONSE is arriving on the client's TAP. Deleting `table ip
/// overdrive-mtls` is detected as `IpRules`, quiesces both TAPs, and is
/// repaired in place: the program read back after the repair is the exact
/// program read back before the fault — the same identity and the same member
/// set, not merely the same size — both TAPs read up again, the same client
/// allocation's exchanges resume, no VM is killed or restarted, and admission
/// reopens (a fresh dialer Job completes its own exchange).
#[tokio::test]
#[serial(cgroup)]
#[ignore = "pending DELIVER step 10-02 (S-ND295-69)"]
async fn a_deleted_program_table_is_repaired_with_live_mesh_vms() {
    let _teardown = TeardownBound::arm();
    assert_root();
    let fixture =
        VmFixture::provision(&shared_staging_root()).expect("provision native VM fixture");
    let tmp = tempfile::Builder::new()
        .prefix("nd295-69-repair-")
        .tempdir_in(shared_staging_root())
        .expect("native tempdir");
    let peer = build_mesh_peer(tmp.path());
    let looping_client = build_looping_mesh_client(tmp.path());
    let dialer = build_quick_dialer(tmp.path());
    let rootfs = stage_rootfs_with_extra_binaries(
        tmp.path(),
        &fixture,
        &[(&peer, "gti-peer"), (&looping_client, "nd295-mesh-loop"), (&dialer, "nd295-dial")],
    );
    let (handle, server_tmp) = spawn_vm_server_mtls_composed().await;
    let cfg = config_path(server_tmp.path());
    let (_guard, events) = capture_supervisor_events();
    let client = Client::new().expect("open typed host-netlink client");
    let response_needle: &'static [u8] = &RESPONSE[..24];

    let service = deploy_mesh_service(
        &cfg,
        server_tmp.path(),
        &fixture.kernel_path,
        &rootfs,
        "nd295-69-service.toml",
    )
    .await;
    let mesh_client = deploy_running(
        &cfg,
        server_tmp.path(),
        "nd295-69-client.toml",
        &vm_job_toml(
            "nd295-69-client",
            "/sbin/nd295-mesh-loop",
            &[],
            &fixture.kernel_path,
            &rootfs,
        ),
    )
    .await;
    await_active(&client, &service.tap).await;
    await_active(&client, &mesh_client.tap).await;
    let service_restarts = row_of(&cfg, &service.workload_id, &service.alloc).await.restart_count;
    let client_restarts =
        row_of(&cfg, &mesh_client.workload_id, &mesh_client.alloc).await.restart_count;

    // Before the fault: the client's exchanges complete, and the program holds
    // exactly the 2 + P members of the two live guests.
    let mut before = NeedleCapture::open(&mesh_client.tap, &[response_needle]);
    assert!(
        before.await_count(response_needle, EXCHANGES_WITNESSED, EXCHANGE_BOUND).await,
        "before the fault the client receives {EXCHANGES_WITNESSED} peer-authored RESPONSEs \
         (saw {})",
        before.count(response_needle),
    );
    before.assert_lossless();
    drop(before);
    let state_before = observe_shared_intercept_state()
        .expect("observe the intercept program before the fault")
        .expect("the intercept program is present before the fault");
    assert_eq!(
        state_before.managed_guest_ips(),
        &BTreeSet::from([service.addr, mesh_client.addr]),
        "before the fault both live guests are managed sources",
    );
    assert_eq!(
        state_before.outbound_sources(),
        &BTreeSet::from([service.addr, mesh_client.addr]),
        "before the fault both live guests are outbound sources",
    );
    assert_eq!(
        state_before.inbound_destinations(),
        &BTreeSet::from([std::net::SocketAddrV4::new(service.addr, SERVICE_PORT)]),
        "before the fault the Service's one listener is the only inbound destination",
    );

    // Fault: delete the whole program table.
    overdrive_netlink::nft::delete_table(SHARED_IPV4_INTERCEPT_TABLE)
        .expect("external actor deletes the shared IPv4 intercept program table");

    // Detection reports IpRules; TAPs quiesce within one audit period.
    let detected = wait_until(Duration::from_secs(3), || {
        events.unhealthy_components().iter().any(|component| component == "IpRules")
    })
    .await;
    assert!(detected, "the table deletion is detected as IpRules\n{}", events.render());
    for tap in [&service.tap, &mesh_client.tap] {
        assert!(
            wait_link_state(&client, tap, Some(false), ONE_AUDIT_PERIOD + Duration::from_secs(1))
                .await,
            "the managed TAP {tap} reads down within one audit period of the deletion",
        );
    }

    // Repair restores the exact program: the same identity and member set.
    let restored = wait_until(
        Duration::from_secs(6),
        || matches!(observe_shared_intercept_state(), Ok(Some(state)) if state == state_before),
    )
    .await;
    assert!(
        restored,
        "the intercept program is restored in place with the identity and member set it had \
         before the fault; before={state_before:?} now={:?}",
        observe_shared_intercept_state(),
    );
    for tap in [&service.tap, &mesh_client.tap] {
        assert!(
            wait_link_state(&client, tap, Some(true), Duration::from_secs(3)).await,
            "the managed TAP {tap} reads back up after repair",
        );
    }

    // The same client allocation's exchanges resume after the repair.
    let mut after = NeedleCapture::open(&mesh_client.tap, &[response_needle]);
    assert!(
        after.await_count(response_needle, EXCHANGES_WITNESSED, EXCHANGE_BOUND).await,
        "after the repair the same client receives {EXCHANGES_WITNESSED} peer-authored \
         RESPONSEs again (saw {})",
        after.count(response_needle),
    );
    after.assert_lossless();
    drop(after);

    // Nothing was killed or restarted.
    assert!(events.killed(None).is_empty(), "no VM is killed by the repair\n{}", events.render());
    for (vm, restarts) in [(&service, service_restarts), (&mesh_client, client_restarts)] {
        let row = row_of(&cfg, &vm.workload_id, &vm.alloc).await;
        assert_eq!(row.state, AllocStateWire::Running, "{} stays Running", vm.workload_id);
        assert_eq!(
            row.restart_count, restarts,
            "{} is repaired in place, not restarted",
            vm.workload_id
        );
    }

    // Admission reopened: a fresh dialer Job completes its own exchange.
    let dial_spec = write_toml(
        server_tmp.path(),
        "nd295-69-dial.toml",
        &vm_job_toml("nd295-69-dial", "/sbin/nd295-dial", &[], &fixture.kernel_path, &rootfs),
    );
    let dial = deploy(DeployArgs { spec: dial_spec, config_path: cfg.clone() })
        .await
        .expect("deploy a fresh dialer after the repair");
    await_job_exit_zero(&cfg, &dial.workload_id, DIAL_BOUND).await;

    stop_and_await_quiescence(
        &cfg,
        &[&service.workload_id, &mesh_client.workload_id, &dial.workload_id],
    )
    .await;
    handle.shutdown().await.expect("clean shutdown");
}
