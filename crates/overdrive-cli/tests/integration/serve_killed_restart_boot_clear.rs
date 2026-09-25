//! netns-density-295 correctness-recovery proof §3.5 (S-ND295-13C): a killed
//! server's replacement clears stale members and admits work (correctness
//! gap 6; D-295-R12, D-295-R17 killed mode).
//!
//! # Contract under proof
//!
//! `docs/feature/netns-density-295/feature-delta.md` § *Boot ordering
//! (D-295-R12)*: a fresh process adopts no VMM, allocation, lease, listener,
//! token, or member of the process it replaces. After the serve owner is lost,
//! the next boot through the production composition root runs, in this order:
//!
//! 1. VM reclamation (R12 step 3). Every Cloud Hypervisor process the dead
//!    owner left is killed and its allocation scope removed.
//! 2. The stale shared-attachment sweep (step 4). The dead allocation's TAP,
//!    TCX link pins, endpoint-map entry, and bridge-guard member are removed
//!    and read back absent.
//! 3. Member convergence to empty (step 6.2). The intercept owner converges
//!    the three dynamic sets of `table ip overdrive-mtls` to empty in one
//!    atomic batch that mutates no program object.
//! 4. Program convergence and read-back (steps 6.5 and 6.6). The constant
//!    program, retargeted to the fresh listeners in the D-295-R19 rule order,
//!    the policy route, and, while D-295-R18 stands, the intercept-mark guard
//!    table are converged and read back with zero members.
//! 5. Admission (step 9). Only then may an allocation insert members. The
//!    fresh per-server pool (D-295-R6) leases the smallest free address,
//!    `100.95.0.2`.
//!
//! The boot must recover rather than refuse, and no stale member may survive
//! into the admitted state.
//!
//! # How the owner is lost: the serve lifetime port's killed mode
//!
//! Boot one runs the production CLI composition (`serve::run_with_kek` →
//! `run_server`) in this test process. The test deploys a mesh Service VM
//! through the public `deploy` handler, proves the three canonical members,
//! the TAP, its TCX link pins, its endpoint-map entry, and its bridge-guard
//! member on the kernel, then abandons the owner through the lifetime port's
//! killed mode: `ServeSignal::Kill` through `ServeLifetime`
//! (`serve_lifetime_support::kill_serve_owner` → `ServeHandle::kill_for_test`
//! → `ServerHandle::kill_for_test`, D-295-R17). Killed mode performs no
//! graceful shutdown or workload cleanup, and it drops the worker on a thread
//! that has given up every Linux capability, so its guard destructors run and
//! the kernel refuses their effects. The residue is read back from the kernel
//! before boot two, which runs the same composition root in this process
//! against the unchanged data and config roots. The `overdrive` binary is
//! never spawned, and no test code installs, clears, or mutates a production
//! object: every stale object boot two meets is the real residue of the killed
//! first server.
//!
//! Killed-mode fidelity deviation (FD 3951-3953): `EbpfDataplane`'s destructor
//! still runs, so its SERVICE_MAP pin unlink and XDP detach happen. No oracle
//! below relies on either surviving.
//!
//! # Oracles (Tier 3, observable kernel side effects only)
//!
//! * `nft monitor`, the kernel's ordered commit stream. Each batch ends with
//!   `# new generation N by process TID (comm)`, so member deletions, program
//!   and guard-table mutations, sweep deletions, and admissions are ordered and
//!   attributed to this process without trusting any product log. Only the
//!   owned tables are replayed: `ip overdrive-mtls`, `ip overdrive-mtls-guard`,
//!   and `bridge overdrive-mtls`.
//! * The typed `overdrive_netlink::nft::observe_shared_ip_intercept_state`
//!   read-back, sampled independently every 10 ms and once at the end.
//! * `/proc` liveness of the dead owner's Cloud Hypervisor PIDs, the
//!   allocation cgroup scope, the dead TAP by ifindex, its TCX link pins by
//!   inode, and its endpoint-map entry by ifindex, sampled every 2-10 ms.
//!   Identity, not name, marks the old objects gone: the replacement reuses
//!   the same address, TAP name, pin names, and member keys.
//! * The final kernel listing of `table ip overdrive-mtls` (rule order),
//!   `ip rule` and routing table 100 (policy route), and, while R18 stands,
//!   `observe_intercept_mark_guard`.
//! * The production boot-phase tracing events, as corroboration only.
//!
//! Every verdict is evaluated and reported; the test fails listing each RED
//! verdict. Evidence is appended, never rewritten, under
//! `/var/tmp/overdrive-killed-serve-boot-clear-proof/<run>/`.
//!
//! # Placement
//!
//! `overdrive-cli` owns the `serve` + `deploy` production entry points and the
//! lifetime port. Every real-guest restart scenario of this feature lives in
//! this integration binary behind `integration-tests` + `kvm-tests` (native
//! x86_64 metal only). The binary is a member of the catch-all
//! `host-kernel-shared` nextest group, and the §3.5 slow-timeout override
//! names this test.
//!
//! Hypothesis: the fresh-process branch of the intercept owner has no member
//! convergence, so boot two meets the killed server's three stale members and
//! refuses after VM reclamation has already run.
//! Prediction (today's code): `run_with_kek` returns `Err` with a
//! `health.startup.refused` event, reason `mtls.shared_owner` (V0 RED); the
//! boot-one Cloud Hypervisor PIDs are dead (V1 GREEN); no boot-two batch
//! deletes the stale members (V2, V3, V6 RED); no program convergence and no
//! admission follow (V4, V5, A1 RED). If the tree's deferred TAP activation
//! keeps boot one's mesh VM from reaching Running (proof findings §3.5,
//! gap 7), the body stops at boot one's `poll_until_running` before any
//! residue exists; every step 08-02 depends on lands before it.
//! Falsification: one boot-two batch deletes exactly the three stale members
//! before any program mutation, the program converges in the R19 order, boot
//! two returns `Ok`, and a replacement is admitted at `100.95.0.2` with every
//! predecessor object already absent.

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    reason = "Tier-3 proof fixture: fail-fast diagnostics, exact CONTRACT_SHAPE token, libc pid casts"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::Ipv4Addr;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::{ServeArgs, ServeHandle};
use overdrive_cli::commands::serve_lifetime::{ServeExit, ServeLifetime, ServeSignal};
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_dataplane::guest_tcx::endpoint_present;
use overdrive_netlink::nft::bridge::{
    BridgeGuardMemberIdentity, BridgeGuardObservation, BridgeGuardSpec,
    observe as observe_bridge_guard,
};
use overdrive_netlink::nft::{self, SharedIpInterceptState};
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::serve_lifetime_support::{
    RecordingClock, SignalDriver, driven_signals, kill_serve_owner,
};
use super::vm_walking_skeleton::{
    config_path, poll_until_running, shared_staging_root, stage_rootfs_with_extra_binary,
    write_toml,
};

const WORKLOAD_ID: &str = "boot-clear-residue";
const SERVICE_PORT: u16 = 18_961;
const GUEST_BINARY: &str = "boot-clear-listener";
/// `table ip overdrive-mtls`: the constant intercept program and its three
/// dynamic sets.
const INTERCEPT_TABLE: &str = "overdrive-mtls";
/// `table ip overdrive-mtls-guard`: the D-295-R18 intercept-mark guard
/// (conditional on DELIVER step 08-01).
const INTERCEPT_MARK_GUARD_TABLE: &str = "overdrive-mtls-guard";
/// `table bridge overdrive-mtls`: the shared bridge guard.
const BRIDGE_GUARD_TABLE: &str = "overdrive-mtls";
const BRIDGE_GUARD_SET: &str = "managed_taps";
const WORKLOADS_SLICE: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";
const TCX_LINK_PINS: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/links";
const ENDPOINT_MAP_PIN: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/maps/endpoints";
const EVIDENCE_ROOT: &str = "/var/tmp/overdrive-killed-serve-boot-clear-proof";
const MONITOR_PROBE_TABLE: &str = "ovd_boot_clear_monitor_probe";
/// The smallest free address of a fresh per-server pool (D-295-R6); the bridge
/// gateway holds `.1`.
const FIRST_FREE_LEASE: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 2);

// ---------------------------------------------------------------------------
// Append-only evidence
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Evidence {
    start: Instant,
    dir: PathBuf,
    timeline: Arc<Mutex<std::fs::File>>,
}

impl Evidence {
    fn create(start: Instant) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("wall clock after epoch")
            .as_secs();
        let dir = PathBuf::from(EVIDENCE_ROOT).join(format!("{now}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create proof evidence directory");
        let timeline = std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join("timeline.log"))
            .expect("create append-only timeline");
        Self { start, dir, timeline: Arc::new(Mutex::new(timeline)) }
    }

    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    fn record(&self, phase: &str, observation: &str) {
        let line = format!(
            "elapsed_ms={:>8} phase={phase} observation={observation}",
            self.elapsed().as_millis()
        );
        eprintln!("[boot-clear-proof] {line}");
        let mut file = self.timeline.lock().expect("timeline lock");
        writeln!(file, "{line}").expect("append timeline");
        file.flush().expect("flush timeline");
    }

    fn append_file(&self, name: &str, body: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(name))
            .unwrap_or_else(|error| panic!("open evidence file {name}: {error}"));
        file.write_all(body.as_bytes()).expect("append evidence file");
        file.flush().expect("flush evidence file");
    }
}

fn command_text(program: &str, args: &[&str]) -> String {
    match Command::new(program).args(args).output() {
        Ok(output) => format!(
            "$ {program} {}\nstatus={}\nstdout:\n{}stderr:\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => format!("$ {program} {}\nspawn error: {error}\n", args.join(" ")),
    }
}

/// Stdout of a diagnostic command that must succeed; a failure keeps its
/// exit status and stderr.
fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("spawn {program} {}: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {} exited {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_owned())
}

// ---------------------------------------------------------------------------
// Host inventory (read-only)
// ---------------------------------------------------------------------------

fn directory_entries(path: &Path) -> BTreeSet<String> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => panic!("read {}: {error}", path.display()),
    }
}

fn nft_tables() -> BTreeSet<String> {
    let output =
        Command::new("nft").args(["list", "tables"]).output().expect("run nft list tables");
    assert!(output.status.success(), "nft list tables failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).lines().map(|line| line.trim().to_owned()).collect()
}

fn cloud_hypervisor_pids() -> BTreeSet<u32> {
    directory_entries(Path::new("/proc"))
        .into_iter()
        .filter_map(|name| name.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|bytes| {
                bytes.split(|byte| *byte == 0).next().is_some_and(|argv0| {
                    String::from_utf8_lossy(argv0).ends_with("cloud-hypervisor")
                })
            })
        })
        .filter(|pid| process_is_alive(*pid))
        .collect()
}

/// A PID is alive while `/proc/<pid>/stat` exists and is not a zombie.
fn process_is_alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .and_then(|(_, rest)| rest.chars().next())
            .is_some_and(|state| state != 'Z' && state != 'X')
    })
}

fn cgroup_procs(scope: &Path) -> BTreeSet<u32> {
    std::fs::read_to_string(scope.join("cgroup.procs"))
        .map(|body| body.lines().filter_map(|line| line.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// The interface's ifindex; `Ok(None)` when it is absent (`NotFound`, or
/// `ENODEV` from a device removed while its attribute was read).
fn interface_ifindex(name: &str) -> Result<Option<u32>, String> {
    match std::fs::read_to_string(Path::new("/sys/class/net").join(name).join("ifindex")) {
        Ok(text) => text
            .trim()
            .parse()
            .map(Some)
            .map_err(|error| format!("parse ifindex of {name}: {error}")),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ENODEV) =>
        {
            Ok(None)
        }
        Err(error) => Err(format!("read ifindex of {name}: {error}")),
    }
}

/// The bpffs pin's inode; `Ok(None)` when the pin is absent. A re-pinned
/// object of the same name has a new inode.
fn pin_inode(path: &Path) -> Result<Option<u64>, String> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata.ino())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("stat {}: {error}", path.display())),
    }
}

/// Whether the pinned endpoint map holds an entry for `ifindex`. No pinned map
/// means no entry.
fn endpoint_entry_present(ifindex: u32) -> Result<bool, String> {
    if pin_inode(Path::new(ENDPOINT_MAP_PIN))?.is_none() {
        return Ok(false);
    }
    endpoint_present(ENDPOINT_MAP_PIN, ifindex).map_err(|error| format!("{error}"))
}

fn bridge_guard_spec() -> BridgeGuardSpec {
    BridgeGuardSpec::new(
        BRIDGE_GUARD_TABLE.to_owned(),
        "prerouting".to_owned(),
        BRIDGE_GUARD_SET.to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical bridge guard spec")
}

/// The ifname members of the bridge guard's `managed_taps` set; an absent
/// guard table has none.
fn bridge_guard_members() -> Result<BTreeSet<String>, String> {
    let observation = observe_bridge_guard(&bridge_guard_spec(), &BTreeSet::new())
        .map_err(|error| format!("{error}"))?;
    let inventory = match observation {
        BridgeGuardObservation::Absent { .. } => return Ok(BTreeSet::new()),
        BridgeGuardObservation::Exact { inventory }
        | BridgeGuardObservation::Conflict { inventory } => inventory,
    };
    Ok(inventory
        .members
        .into_iter()
        .filter(|member| member.set == BRIDGE_GUARD_SET)
        .map(|member| match member.identity {
            BridgeGuardMemberIdentity::Ifname(name) => name,
            BridgeGuardMemberIdentity::ForeignEncoding { encoded_len } => {
                format!("<foreign-encoding:{encoded_len}>")
            }
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostInventory {
    nft_tables: BTreeSet<String>,
    ovd_links: BTreeSet<String>,
    alloc_scopes: BTreeSet<String>,
    vm_run_dirs: BTreeSet<String>,
    tcx_link_pins: BTreeSet<String>,
    ch_pids: BTreeSet<u32>,
    intercept_members: BTreeSet<String>,
    ip_rules: String,
    route_table_100: String,
}

impl HostInventory {
    fn capture() -> Self {
        Self {
            nft_tables: nft_tables(),
            ovd_links: directory_entries(Path::new("/sys/class/net"))
                .into_iter()
                .filter(|name| name.starts_with("ovd-"))
                .collect(),
            alloc_scopes: directory_entries(Path::new(WORKLOADS_SLICE))
                .into_iter()
                .filter(|name| name.starts_with("alloc-"))
                .collect(),
            vm_run_dirs: directory_entries(Path::new("/run/overdrive/vm")),
            tcx_link_pins: directory_entries(Path::new(TCX_LINK_PINS)),
            ch_pids: cloud_hypervisor_pids(),
            intercept_members: match observe_state() {
                Ok(Some(state)) => state_members(&state),
                Ok(None) | Err(_) => BTreeSet::new(),
            },
            ip_rules: command_text("ip", &["rule", "show"]),
            route_table_100: command_text("ip", &["route", "show", "table", "100"]),
        }
    }
}

// ---------------------------------------------------------------------------
// Typed shared-IP intercept state projection
// ---------------------------------------------------------------------------

/// Canonical nft-monitor spelling of one dynamic-set member.
fn member_key(set: &str, member: &str) -> String {
    format!("{set}:{member}")
}

fn state_members(state: &SharedIpInterceptState) -> BTreeSet<String> {
    let mut members = BTreeSet::new();
    for ip in state.managed_guest_ips() {
        members.insert(member_key("managed_guest_ips", &ip.to_string()));
    }
    for ip in state.outbound_sources() {
        members.insert(member_key("outbound_sources", &ip.to_string()));
    }
    for destination in state.inbound_destinations() {
        members.insert(member_key(
            "inbound_destinations",
            &format!("{} . {}", destination.ip(), destination.port()),
        ));
    }
    members
}

fn identity_fingerprint(state: &SharedIpInterceptState) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{:?}", state.identity()).hash(&mut hasher);
    hasher.finish()
}

fn summarize_state(observed: &Result<Option<SharedIpInterceptState>, String>) -> String {
    match observed {
        Ok(None) => "table-absent".to_owned(),
        Ok(Some(state)) => format!(
            "identity={:016x} members={:?}",
            identity_fingerprint(state),
            state_members(state)
        ),
        Err(error) => format!("observe-error={error}"),
    }
}

fn observe_state() -> Result<Option<SharedIpInterceptState>, String> {
    nft::observe_shared_ip_intercept_state().map_err(|error| format!("{error}"))
}

/// The D-295-R18 guard observation. The surface is provisional
/// (FD 3462-3475): DELIVER step 08-01 implements it if R18 stands, or deletes
/// it (and this proof's guard assertion with it) if R18 is withdrawn. Until
/// 08-01 it is a RED scaffold, so its panic is reported as a RED observation
/// instead of aborting the proof's other verdicts.
fn observe_intercept_mark_guard() -> Result<bool, String> {
    std::panic::catch_unwind(nft::observe_intercept_mark_guard)
        .map_err(|panic| {
            format!("R18 observation surface panicked: {}", panic_text(panic.as_ref()))
        })?
        .map_err(|error| format!("{error}"))
}

/// Each rule of a kernel listing of `table ip overdrive-mtls` that TPROXYs,
/// with whether its tail runs `tproxy`, then `meta mark set`, then `accept`
/// (the D-295-R19 order, FD 3349-3356).
fn tproxy_rule_orders(listing: &str) -> Vec<(String, bool)> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("tproxy"))
        .map(|line| {
            let r19_order = line.find("tproxy").is_some_and(|tproxy| {
                line[tproxy..].find("mark set").is_some_and(|mark| {
                    let mark = tproxy + mark;
                    line[mark..].contains("accept")
                })
            });
            (line.to_owned(), r19_order)
        })
        .collect()
}

/// The policy route: the `fwmark 0x1 lookup 100` rule plus routing table
/// 100's `local 0.0.0.0/0 dev lo` route.
fn policy_route_present(ip_rules: &str, route_table_100: &str) -> bool {
    ip_rules.lines().any(|line| line.contains("fwmark 0x1 lookup 100"))
        && route_table_100.lines().map(str::trim).any(|line| {
            line.starts_with("local")
                && (line.contains("default") || line.contains("0.0.0.0/0"))
                && line.contains("dev lo")
        })
}

// ---------------------------------------------------------------------------
// In-process tracing capture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TracedEvent {
    at: Duration,
    name: String,
    fields: BTreeMap<String, String>,
}

impl TracedEvent {
    fn is(&self, name: &str) -> bool {
        self.name == name
            || self.fields.get("name").map(String::as_str) == Some(name)
            || self.fields.get("event").map(String::as_str) == Some(name)
    }
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone)]
struct TraceCapture {
    evidence: Evidence,
    events: Arc<Mutex<Vec<TracedEvent>>>,
}

impl<S> Layer<S> for TraceCapture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let traced = TracedEvent {
            at: self.evidence.elapsed(),
            name: event.metadata().name().to_owned(),
            fields: visitor.fields,
        };
        self.evidence.append_file(
            "parent.trace.log",
            &format!(
                "elapsed_ms={:>8} level={} target={} name={} fields={:?}\n",
                traced.at.as_millis(),
                event.metadata().level(),
                event.metadata().target(),
                traced.name,
                traced.fields
            ),
        );
        self.events.lock().expect("trace lock").push(traced);
    }
}

// ---------------------------------------------------------------------------
// Kernel commit-stream oracle: `nft monitor`
// ---------------------------------------------------------------------------

struct NftMonitor {
    child: Child,
    lines: Arc<Mutex<Vec<MonitorLine>>>,
    reader: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct MonitorLine {
    at: Duration,
    text: String,
    /// Thread-group id resolved from `/proc/<tid>/status` at receipt time for
    /// `# new generation ... by process <tid>` lines.
    tgid: Option<u32>,
}

impl NftMonitor {
    fn start(evidence: &Evidence) -> Self {
        let mut child = Command::new("stdbuf")
            .args(["-oL", "-eL", "nft", "monitor"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn nft monitor");
        let stdout = child.stdout.take().expect("nft monitor stdout");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let evidence_for_reader = evidence.clone();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(text) = line else { break };
                let at = evidence_for_reader.elapsed();
                let tgid = generation_tid(&text).and_then(resolve_tgid);
                evidence_for_reader.append_file(
                    "nft-monitor.log",
                    &format!("elapsed_ms={:>8} tgid={tgid:?} {text}\n", at.as_millis()),
                );
                sink.lock().expect("monitor lock").push(MonitorLine { at, text, tgid });
            }
        });
        let monitor = Self { child, lines, reader: Some(reader) };
        // Readiness is proven, not assumed: a test-owned probe table must be
        // reported by the subscription before the observed window begins.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut announced = false;
        while !announced && Instant::now() < deadline {
            let added = Command::new("nft")
                .args(["add", "table", "inet", MONITOR_PROBE_TABLE])
                .status()
                .expect("run nft add probe table");
            assert!(added.success(), "the test-owned monitor probe table is added");
            let probe_until = Instant::now() + Duration::from_millis(500);
            while Instant::now() < probe_until {
                if monitor.contains(&format!("add table inet {MONITOR_PROBE_TABLE}")) {
                    announced = true;
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let status = Command::new("nft")
                .args(["delete", "table", "inet", MONITOR_PROBE_TABLE])
                .status()
                .expect("run nft delete probe table");
            assert!(status.success(), "the test-owned monitor probe table is removed");
        }
        assert!(announced, "nft monitor reported the readiness probe within 10s");
        let deleted_until = Instant::now() + Duration::from_secs(5);
        while !monitor.contains(&format!("delete table inet {MONITOR_PROBE_TABLE}")) {
            assert!(Instant::now() < deleted_until, "nft monitor reports the probe removal");
            thread::sleep(Duration::from_millis(10));
        }
        monitor
    }

    fn contains(&self, needle: &str) -> bool {
        self.lines.lock().expect("monitor lock").iter().any(|line| line.text.contains(needle))
    }

    fn stop(mut self) -> Vec<MonitorLine> {
        // Stopping the diagnostic subscriber: its kill/wait outcome carries no
        // evidence, and the reader thread ends with the pipe.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.lines.lock().expect("monitor lock").clone()
    }
}

impl Drop for NftMonitor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn generation_tid(text: &str) -> Option<u32> {
    let rest = text.strip_prefix("# new generation ")?;
    let (_, after) = rest.split_once(" by process ")?;
    after.split_whitespace().next()?.parse().ok()
}

fn resolve_tgid(tid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{tid}/status")).ok()?;
    status.lines().find_map(|line| line.strip_prefix("Tgid:")?.trim().parse().ok())
}

/// The three node-shared tables whose commits this proof replays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedTable {
    /// `ip overdrive-mtls`: the constant program and the three dynamic sets.
    Intercept,
    /// `ip overdrive-mtls-guard`: the D-295-R18 guard table (conditional).
    InterceptMarkGuard,
    /// `bridge overdrive-mtls`: the shared bridge guard (`managed_taps`).
    BridgeGuard,
}

impl OwnedTable {
    fn classify(family: &str, table: &str) -> Option<Self> {
        match family {
            "ip" if table == INTERCEPT_TABLE => Some(Self::Intercept),
            "ip" if table == INTERCEPT_MARK_GUARD_TABLE => Some(Self::InterceptMarkGuard),
            "bridge" if table == BRIDGE_GUARD_TABLE => Some(Self::BridgeGuard),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NftEvent {
    AddElement {
        table: OwnedTable,
        set: String,
        member: String,
    },
    DeleteElement {
        table: OwnedTable,
        set: String,
        member: String,
    },
    FlushSet {
        table: OwnedTable,
        set: String,
    },
    /// Any table/chain/set/rule mutation. `discards_members_of` names the set
    /// whose members the mutation drops (`"*"` for the whole table).
    Object {
        table: OwnedTable,
        verb: String,
        kind: String,
        discards_members_of: Option<String>,
    },
}

impl NftEvent {
    const fn table(&self) -> OwnedTable {
        match self {
            Self::AddElement { table, .. }
            | Self::DeleteElement { table, .. }
            | Self::FlushSet { table, .. }
            | Self::Object { table, .. } => *table,
        }
    }

    /// A mutation of the constant intercept program or of the R18 guard table.
    fn is_program_mutation(&self) -> bool {
        match self {
            Self::Object { table: OwnedTable::Intercept, .. } => true,
            event => event.table() == OwnedTable::InterceptMarkGuard,
        }
    }

    /// Whether this event removes the bridge-guard member naming `tap`.
    fn removes_guard_member(&self, tap: &str) -> bool {
        match self {
            Self::DeleteElement { table: OwnedTable::BridgeGuard, set, member } => {
                set == BRIDGE_GUARD_SET && member == tap
            }
            Self::FlushSet { table: OwnedTable::BridgeGuard, set }
            | Self::Object {
                table: OwnedTable::BridgeGuard, discards_members_of: Some(set), ..
            } => set == BRIDGE_GUARD_SET || set == "*",
            _ => false,
        }
    }
}

fn parse_events(line: &str) -> Vec<NftEvent> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    // `<verb> <kind> <family> <table> ...`; the bare table line has exactly
    // four tokens. Foreign tables are not replayed.
    if tokens.len() < 4 {
        return Vec::new();
    }
    let Some(table) = OwnedTable::classify(tokens[2], tokens[3]) else {
        return Vec::new();
    };
    let verb = tokens[0];
    let kind = tokens[1];
    let brace = |line: &str| {
        line.split_once('{')
            .and_then(|(_, rest)| rest.rsplit_once('}'))
            .map(|(inner, _)| inner.to_owned())
            .unwrap_or_default()
    };
    match (verb, kind) {
        ("add" | "delete" | "destroy", "element") if tokens.len() >= 5 => {
            let set = tokens[4].to_owned();
            brace(line)
                .split(',')
                .map(|member| member.trim().trim_matches('"'))
                .filter(|member| !member.is_empty())
                .map(|member| {
                    let member = member.to_owned();
                    if verb == "add" {
                        NftEvent::AddElement { table, set: set.clone(), member }
                    } else {
                        NftEvent::DeleteElement { table, set: set.clone(), member }
                    }
                })
                .collect()
        }
        ("flush", "set") if tokens.len() >= 5 => {
            vec![NftEvent::FlushSet { table, set: tokens[4].to_owned() }]
        }
        (_, "table" | "chain" | "set" | "rule" | "map" | "flowtable" | "counter" | "quota") => {
            let removal = matches!(verb, "delete" | "destroy");
            let discards_members_of = if (removal || verb == "flush") && kind == "table" {
                Some("*".to_owned())
            } else if removal && kind == "set" {
                tokens.get(4).map(|name| (*name).to_owned())
            } else {
                None
            };
            vec![NftEvent::Object {
                table,
                verb: verb.to_owned(),
                kind: kind.to_owned(),
                discards_members_of,
            }]
        }
        ("flush", _) => vec![NftEvent::Object {
            table,
            verb: verb.to_owned(),
            kind: kind.to_owned(),
            discards_members_of: None,
        }],
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone)]
struct Batch {
    closed_at: Duration,
    tid: Option<u32>,
    tgid: Option<u32>,
    comm: String,
    lines: Vec<String>,
    events: Vec<NftEvent>,
}

impl Batch {
    const fn touches_owned_table(&self) -> bool {
        !self.events.is_empty()
    }

    fn touches(&self, table: OwnedTable) -> bool {
        self.events.iter().any(|event| event.table() == table)
    }

    fn mutates_program(&self) -> bool {
        self.events.iter().any(NftEvent::is_program_mutation)
    }

    /// The intercept-set member keys this batch adds.
    fn intercept_additions(&self) -> BTreeSet<String> {
        self.events
            .iter()
            .filter_map(|event| match event {
                NftEvent::AddElement { table: OwnedTable::Intercept, set, member } => {
                    Some(member_key(set, member))
                }
                _ => None,
            })
            .collect()
    }

    /// True when every intercept-table event of this batch is an element
    /// removal, and there is at least one.
    fn only_intercept_element_removals(&self) -> bool {
        let mut intercept =
            self.events.iter().filter(|event| event.table() == OwnedTable::Intercept).peekable();
        intercept.peek().is_some()
            && intercept.all(|event| {
                matches!(event, NftEvent::DeleteElement { .. } | NftEvent::FlushSet { .. })
            })
    }

    fn adds_guard_member(&self) -> bool {
        self.events.iter().any(|event| {
            matches!(
                event,
                NftEvent::AddElement { table: OwnedTable::BridgeGuard, set, .. }
                    if set == BRIDGE_GUARD_SET
            )
        })
    }

    fn removes_guard_member(&self, tap: &str) -> bool {
        self.events.iter().any(|event| event.removes_guard_member(tap))
    }

    fn describe(&self) -> String {
        format!(
            "closed_at_ms={} tid={:?} tgid={:?} comm={} lines={:?}",
            self.closed_at.as_millis(),
            self.tid,
            self.tgid,
            self.comm,
            self.lines
        )
    }
}

fn batches(lines: &[MonitorLine]) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for line in lines {
        if let Some(rest) = line.text.strip_prefix("# new generation ") {
            let comm = rest
                .rsplit_once('(')
                .and_then(|(_, comm)| comm.strip_suffix(')'))
                .unwrap_or_default()
                .to_owned();
            let lines = std::mem::take(&mut pending);
            let events = lines.iter().flat_map(|line| parse_events(line)).collect();
            out.push(Batch {
                closed_at: line.at,
                tid: generation_tid(&line.text),
                tgid: line.tgid,
                comm,
                lines,
                events,
            });
        } else {
            pending.push(line.text.clone());
        }
    }
    out
}

/// Replay one batch against the tracked membership of the three intercept
/// sets.
fn apply(membership: &mut BTreeSet<String>, batch: &Batch) {
    for event in &batch.events {
        match event {
            NftEvent::AddElement { table: OwnedTable::Intercept, set, member } => {
                membership.insert(member_key(set, member));
            }
            NftEvent::DeleteElement { table: OwnedTable::Intercept, set, member } => {
                membership.remove(&member_key(set, member));
            }
            NftEvent::FlushSet { table: OwnedTable::Intercept, set }
            | NftEvent::Object {
                table: OwnedTable::Intercept,
                discards_members_of: Some(set),
                ..
            } => {
                if set == "*" {
                    membership.clear();
                } else {
                    membership.retain(|key| !key.starts_with(&format!("{set}:")));
                }
            }
            _ => {}
        }
    }
}

/// Retire tracked stale members by removal events only; a later add of the
/// same key is a fresh boot-two member, never the surviving stale one.
fn retire(stale: &mut BTreeSet<String>, batch: &Batch) {
    for event in &batch.events {
        match event {
            NftEvent::DeleteElement { table: OwnedTable::Intercept, set, member } => {
                stale.remove(&member_key(set, member));
            }
            NftEvent::FlushSet { table: OwnedTable::Intercept, set }
            | NftEvent::Object {
                table: OwnedTable::Intercept,
                discards_members_of: Some(set),
                ..
            } => {
                if set == "*" {
                    stale.clear();
                } else {
                    stale.retain(|key| !key.starts_with(&format!("{set}:")));
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Residue and replacement poller (2 ms; heavier reads every 10 ms)
// ---------------------------------------------------------------------------

/// One typed read-back of the intercept members, stamped with the time the
/// read STARTED (so a sample that saw a state was taken before any later
/// commit that changed it).
#[derive(Debug, Clone)]
struct MemberSample {
    at: Duration,
    members: Result<Option<BTreeSet<String>>, String>,
}

#[derive(Debug, Default, Clone)]
struct PollHistory {
    pid_dead_at: BTreeMap<u32, Duration>,
    scope_gone_at: Option<Duration>,
    /// First sample on which the dead TAP's ifindex was absent.
    tap_gone_at: Option<Duration>,
    /// First sample on which an interface of the dead TAP's name carried a new
    /// ifindex: the replacement's TAP.
    replacement_tap_seen_at: Option<Duration>,
    /// Per dead link pin, the first sample on which it was absent or re-pinned.
    pin_gone_at: BTreeMap<String, Duration>,
    /// First sample on which the endpoint map held no entry for the dead
    /// TAP's ifindex.
    endpoint_gone_at: Option<Duration>,
    /// Typed member read-backs, recorded on change.
    member_samples: Vec<MemberSample>,
}

struct Poller {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<PollHistory>>,
}

impl Poller {
    fn start(evidence: &Evidence, residue: &Residue) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let evidence = evidence.clone();
        let residue = residue.clone();
        let handle = thread::spawn(move || {
            let mut history = PollHistory::default();
            let mut last_members: Option<String> = None;
            let mut last_errors: BTreeMap<&'static str, String> = BTreeMap::new();
            let mut error = |kind: &'static str, text: String| {
                if last_errors.get(kind) != Some(&text) {
                    evidence.record(kind, &text);
                    last_errors.insert(kind, text);
                }
            };
            let mut tick = 0_u64;
            while !flag.load(Ordering::SeqCst) {
                let at = evidence.elapsed();
                for pid in &residue.ch_pids {
                    if !history.pid_dead_at.contains_key(pid) && !process_is_alive(*pid) {
                        history.pid_dead_at.insert(*pid, at);
                        evidence.record("poll_pid_dead", &format!("pid={pid}"));
                    }
                }
                if history.scope_gone_at.is_none() && !residue.scope.exists() {
                    history.scope_gone_at = Some(at);
                    evidence.record("poll_scope_gone", &residue.scope.display().to_string());
                }
                if history.replacement_tap_seen_at.is_none() {
                    match interface_ifindex(&residue.tap) {
                        Ok(Some(index)) if index == residue.tap_ifindex => {}
                        Ok(found) => {
                            if history.tap_gone_at.is_none() {
                                history.tap_gone_at = Some(at);
                                evidence.record(
                                    "poll_dead_tap_gone",
                                    &format!("{} ifindex={}", residue.tap, residue.tap_ifindex),
                                );
                            }
                            if let Some(index) = found {
                                history.replacement_tap_seen_at = Some(at);
                                evidence.record(
                                    "poll_replacement_tap_seen",
                                    &format!("{} ifindex={index}", residue.tap),
                                );
                            }
                        }
                        Err(text) => error("poll_tap_error", text),
                    }
                }
                for (pin, inode) in &residue.link_pins {
                    if history.pin_gone_at.contains_key(pin) {
                        continue;
                    }
                    match pin_inode(&Path::new(TCX_LINK_PINS).join(pin)) {
                        Ok(Some(current)) if current == *inode => {}
                        Ok(_) => {
                            history.pin_gone_at.insert(pin.clone(), at);
                            evidence.record("poll_dead_link_pin_gone", pin);
                        }
                        Err(text) => error("poll_pin_error", text),
                    }
                }
                if tick.is_multiple_of(5) {
                    if history.endpoint_gone_at.is_none() {
                        match endpoint_entry_present(residue.tap_ifindex) {
                            Ok(true) => {}
                            Ok(false) => {
                                history.endpoint_gone_at = Some(at);
                                evidence.record(
                                    "poll_dead_endpoint_entry_gone",
                                    &format!("ifindex={}", residue.tap_ifindex),
                                );
                            }
                            Err(text) => error("poll_endpoint_error", text),
                        }
                    }
                    let members = observe_state().map(|state| state.as_ref().map(state_members));
                    let summary = format!("{members:?}");
                    if last_members.as_ref() != Some(&summary) {
                        evidence.record("poll_intercept_members", &summary);
                        history.member_samples.push(MemberSample { at, members });
                        last_members = Some(summary);
                    }
                }
                tick += 1;
                thread::sleep(Duration::from_millis(2));
            }
            history
        });
        Self { stop, handle: Some(handle) }
    }

    fn finish(mut self) -> PollHistory {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take().map(|handle| handle.join().expect("poller joins")).unwrap_or_default()
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // A panicked poller already failed the proof through `finish`.
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic link/udev/route oracles (append-only files; never asserted on)
// ---------------------------------------------------------------------------

/// Background `ip monitor` and `udevadm monitor` writing straight to evidence
/// files, started before boot one so a boot-one failure still carries its own
/// kernel history.
struct DiagnosticMonitors {
    children: Vec<Child>,
}

impl DiagnosticMonitors {
    fn start(evidence: &Evidence) -> Self {
        let mut children = Vec::new();
        for (name, program, args) in [
            ("link-monitor.log", "ip", &["-ts", "-d", "monitor", "link"][..]),
            ("route-monitor.log", "ip", &["-ts", "monitor", "rule", "route"][..]),
            (
                "udev-monitor.log",
                "udevadm",
                &["monitor", "--udev", "--property", "--subsystem-match=net"][..],
            ),
        ] {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(evidence.dir.join(name))
                .expect("open diagnostic monitor log");
            let stderr = file.try_clone().expect("clone diagnostic monitor log");
            match Command::new(program).args(args).stdout(file).stderr(stderr).spawn() {
                Ok(child) => children.push(child),
                Err(error) => evidence
                    .record("diagnostic_monitor_unavailable", &format!("{program}: {error}")),
            }
        }
        Self { children }
    }
}

impl Drop for DiagnosticMonitors {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn build_listener_guest(tmp: &Path) -> PathBuf {
    let src = tmp.join(format!("{GUEST_BINARY}.rs"));
    std::fs::write(
        &src,
        format!(
            "use std::net::TcpListener;\nfn main() {{\n    let listener = TcpListener::bind((\"0.0.0.0\", {SERVICE_PORT})).unwrap();\n    for stream in listener.incoming() {{\n        let stream = stream;\n        std::thread::spawn(move || {{ let _held = stream; std::thread::sleep(std::time::Duration::from_secs(3600)); }});\n    }}\n}}\n"
        ),
    )
    .expect("write listener guest source");
    let out = tmp.join(GUEST_BINARY);
    let status = Command::new("rustc")
        .args(["--edition", "2021", "-C", "opt-level=0", "-C", "target-feature=+crt-static"])
        .args(["--target", "x86_64-unknown-linux-musl", "-o"])
        .arg(&out)
        .arg(&src)
        .status()
        .expect("spawn rustc for the listener guest");
    assert!(status.success(), "rustc builds the static listener guest");
    out
}

fn service_toml(kernel: &Path, rootfs: &Path) -> String {
    format!(
        "[service]\nid = \"{WORKLOAD_ID}\"\nreplicas = 1\n\n[[listener]]\nport = {SERVICE_PORT}\nprotocol = \"tcp\"\n\n\
         [vm]\ncommand = \"/sbin/{GUEST_BINARY}\"\nargs = []\nkernel = \"{}\"\nrootfs = \"{}\"\n\n\
         [resources]\ncpu_milli = 100\nmemory_bytes = 134217728\n",
        kernel.display(),
        rootfs.display()
    )
}

// ---------------------------------------------------------------------------
// Residue proof
// ---------------------------------------------------------------------------

/// The killed server's allocation, identified by kernel identity (ifindex,
/// pin inode), not by name: the replacement reuses every name.
#[derive(Debug, Clone)]
struct Residue {
    alloc: AllocationId,
    guest: Ipv4Addr,
    tap: String,
    tap_ifindex: u32,
    /// Pin file name → inode, for every TCX link pin of the dead TAP.
    link_pins: BTreeMap<String, u64>,
    scope: PathBuf,
    ch_pids: BTreeSet<u32>,
    members: BTreeSet<String>,
    identity: u64,
}

fn residue_report(residue: &Residue) -> String {
    let mut report = String::new();
    let _ = writeln!(
        report,
        "alloc={} guest={} tap={} tap_ifindex={}",
        residue.alloc, residue.guest, residue.tap, residue.tap_ifindex
    );
    let _ = writeln!(report, "intercept_state={}", summarize_state(&observe_state()));
    let _ = writeln!(
        report,
        "ch_pids={:?} alive={:?}",
        residue.ch_pids,
        residue.ch_pids.iter().map(|pid| (*pid, process_is_alive(*pid))).collect::<Vec<_>>()
    );
    let _ = writeln!(
        report,
        "scope={} exists={} procs={:?}",
        residue.scope.display(),
        residue.scope.exists(),
        cgroup_procs(&residue.scope)
    );
    let _ = writeln!(report, "tap_ifindex_now={:?}", interface_ifindex(&residue.tap));
    let _ = writeln!(
        report,
        "link_pins={:?} now={:?}",
        residue.link_pins,
        residue
            .link_pins
            .keys()
            .map(|pin| (pin.clone(), pin_inode(&Path::new(TCX_LINK_PINS).join(pin))))
            .collect::<Vec<_>>()
    );
    let _ = writeln!(
        report,
        "endpoint_entry_present={:?}",
        endpoint_entry_present(residue.tap_ifindex)
    );
    let _ = writeln!(report, "bridge_guard_members={:?}", bridge_guard_members());
    let _ = writeln!(
        report,
        "vm_run_dir_present={}",
        Path::new("/run/overdrive/vm").join(residue.alloc.as_str()).exists()
    );
    report.push_str(&command_text("nft", &["list", "table", "ip", INTERCEPT_TABLE]));
    report.push_str(&command_text("nft", &["list", "table", "bridge", BRIDGE_GUARD_TABLE]));
    report
}

// ---------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------

struct Verdicts {
    rows: Vec<(String, bool, String)>,
}

impl Verdicts {
    fn push(&mut self, id: &str, green: bool, detail: String) {
        self.rows.push((id.to_owned(), green, detail));
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for (id, green, detail) in &self.rows {
            let _ = writeln!(out, "{} {id}: {detail}", if *green { "GREEN" } else { "RED  " });
        }
        out
    }
}

/// The program, policy route, and guard read back at the end of the
/// evaluated window.
struct FinalProgram {
    listing: Result<String, String>,
    ip_rules: Result<String, String>,
    route_table_100: Result<String, String>,
    intercept_mark_guard: Result<bool, String>,
}

impl FinalProgram {
    fn capture() -> Self {
        Self {
            listing: command_stdout("nft", &["list", "table", "ip", INTERCEPT_TABLE]),
            ip_rules: command_stdout("ip", &["rule", "show"]),
            route_table_100: command_stdout("ip", &["route", "show", "table", "100"]),
            intercept_mark_guard: observe_intercept_mark_guard(),
        }
    }
}

struct BootTwoObservation {
    started_at: Duration,
    returned_at: Duration,
    result: Result<(), String>,
    /// The replacement allocation reported Running, with its lease.
    replacement: Option<(String, Option<Ipv4Addr>)>,
    final_state: Result<Option<SharedIpInterceptState>, String>,
    final_program: FinalProgram,
    /// End of the evaluated window: taken before the proof's own cleanup
    /// `stop`, so stop-driven deletions can never be mistaken for boot work.
    final_state_at: Duration,
}

/// Positions in the boot-two commit stream the verdicts are judged against.
struct CommitOrder {
    /// The one batch that removes exactly the stale members (R12 step 6.2).
    clear: Option<usize>,
    /// Every batch that removes intercept members before the first admission.
    removals_before_admission: Vec<usize>,
    /// Every constant-program or guard-table mutation.
    program: Vec<usize>,
    /// The first batch that adds intercept members (the replacement's install).
    first_admission: Option<usize>,
    /// The first batch that adds a bridge-guard member (the replacement's TAP).
    first_guard_add: Option<usize>,
    /// The first batch that removes the dead TAP's bridge-guard member.
    guard_member_removed: Option<usize>,
    /// Replayed membership at the end of the window.
    membership: BTreeSet<String>,
}

fn commit_order(residue: &Residue, boot_two_batches: &[Batch]) -> CommitOrder {
    let first_admission =
        boot_two_batches.iter().position(|batch| !batch.intercept_additions().is_empty());
    let mut order = CommitOrder {
        clear: None,
        removals_before_admission: Vec::new(),
        program: Vec::new(),
        first_admission,
        first_guard_add: boot_two_batches.iter().position(Batch::adds_guard_member),
        guard_member_removed: boot_two_batches
            .iter()
            .position(|batch| batch.removes_guard_member(&residue.tap)),
        membership: residue.members.clone(),
    };
    for (index, batch) in boot_two_batches.iter().enumerate() {
        if batch.mutates_program() {
            order.program.push(index);
        }
        if !batch.touches(OwnedTable::Intercept) {
            continue;
        }
        let before = order.membership.clone();
        apply(&mut order.membership, batch);
        let removed: BTreeSet<String> = before.difference(&order.membership).cloned().collect();
        if !removed.is_empty() && first_admission.is_none_or(|admission| index < admission) {
            order.removals_before_admission.push(index);
        }
        if order.clear.is_none()
            && removed == residue.members
            && batch.only_intercept_element_removals()
            && !batch.mutates_program()
        {
            order.clear = Some(index);
        }
    }
    order
}

fn evaluate(
    residue: &Residue,
    boot_two: &BootTwoObservation,
    events: &[TracedEvent],
    boot_two_batches: &[Batch],
    poll: &PollHistory,
) -> Verdicts {
    let mut verdicts = Verdicts { rows: Vec::new() };
    let order = commit_order(residue, boot_two_batches);
    let closed_at = |index: usize| boot_two_batches[index].closed_at;
    let describe_batch = |index: Option<usize>| {
        index.map_or_else(|| "none".to_owned(), |index| boot_two_batches[index].describe())
    };
    let clear_at = order.clear.map(closed_at);
    let first_admission_at = order.first_admission.map(closed_at);
    // The first effect of any new allocation: its intercept install, its
    // bridge-guard member, or its TAP (the replacement reuses the dead TAP's
    // name, so a new ifindex marks it).
    let admission_at = [first_admission_at, order.first_guard_add.map(closed_at)]
        .into_iter()
        .chain([poll.replacement_tap_seen_at])
        .flatten()
        .min();

    // V0 — the restart recovers through the real composition root.
    let refused: Vec<&TracedEvent> =
        events.iter().filter(|event| event.is("health.startup.refused")).collect();
    verdicts.push(
        "V0 restart_recovers_through_composition_root",
        boot_two.result.is_ok() && refused.is_empty(),
        format!(
            "run_with_kek={:?}; startup_refused_events={:?}",
            boot_two.result,
            refused.iter().map(|event| &event.fields).collect::<Vec<_>>()
        ),
    );

    // V1 — the dead owner's VMMs are dead and its scope gone before boot two
    // commits anything to an owned shared-network table. R12 step 2
    // (`probe_startup`, isolated scratch) legitimately precedes reclamation,
    // so its scratch tables are outside this bound.
    let first_owned_at = boot_two_batches
        .iter()
        .find(|batch| batch.touches_owned_table())
        .map(|batch| batch.closed_at);
    let all_dead = residue.ch_pids.iter().all(|pid| poll.pid_dead_at.contains_key(pid));
    let reclaimed_at = if all_dead {
        residue
            .ch_pids
            .iter()
            .filter_map(|pid| poll.pid_dead_at.get(pid).copied())
            .chain(poll.scope_gone_at)
            .max()
    } else {
        None
    };
    let reclamation_completed_event = events.iter().find(|event| {
        event.is("guest_network.shared_owner_boot_phase")
            && event.fields.get("phase").map(String::as_str) == Some("vm_reclamation")
            && event.fields.get("transition").map(String::as_str) == Some("completed")
    });
    let v1 = all_dead
        && poll.scope_gone_at.is_some()
        && reclaimed_at
            .is_some_and(|reclaimed| first_owned_at.is_none_or(|mutation| reclaimed < mutation));
    verdicts.push(
        "V1 vmm_reclamation_precedes_every_owned_table_commit",
        v1,
        format!(
            "ch_pid_deaths_ms={:?}; scope_gone_ms={:?}; reclamation_completed_event_ms={:?}; \
             first_boot_two_owned_table_batch_ms={:?}",
            residue
                .ch_pids
                .iter()
                .map(|pid| (*pid, poll.pid_dead_at.get(pid).map(Duration::as_millis)))
                .collect::<Vec<_>>(),
            poll.scope_gone_at.map(|at| at.as_millis()),
            reclamation_completed_event.map(|event| event.at.as_millis()),
            first_owned_at.map(|at| at.as_millis())
        ),
    );

    // V2 — exactly one boot-two batch deletes exactly the three stale members,
    // and no program or guard-table mutation precedes it.
    let no_program_before_clear =
        order.clear.is_some_and(|clear| order.program.iter().all(|program| *program > clear));
    let v2 = order.clear.is_some()
        && order.removals_before_admission == order.clear.into_iter().collect::<Vec<_>>()
        && no_program_before_clear;
    verdicts.push(
        "V2 one_batch_deletes_exactly_the_stale_members_before_any_program_mutation",
        v2,
        format!(
            "stale_members={:?}; clear_batch={}; member_removing_batches_before_admission={:?}; \
             program_or_guard_batches={:?}; boot_two_owned_table_batches={}",
            residue.members,
            describe_batch(order.clear),
            order.removals_before_admission,
            order.program,
            boot_two_batches.iter().filter(|batch| batch.touches_owned_table()).count()
        ),
    );

    // V3 — the typed read-back shows zero members after the clear, before
    // anything is admitted.
    let zero_sample = poll.member_samples.iter().find(|sample| {
        sample.at >= boot_two.started_at
            && matches!(&sample.members, Ok(Some(members)) if members.is_empty())
    });
    let v3 = order.clear.is_some()
        && zero_sample
            .is_some_and(|sample| first_admission_at.is_none_or(|admission| sample.at < admission));
    verdicts.push(
        "V3 typed_read_back_shows_zero_members_after_the_clear",
        v3,
        format!(
            "clear_ms={:?}; first_zero_member_sample_ms={:?}; first_admission_ms={:?}; \
             boot_two_samples={:?}",
            clear_at.map(|at| at.as_millis()),
            zero_sample.map(|sample| sample.at.as_millis()),
            first_admission_at.map(|at| at.as_millis()),
            poll.member_samples
                .iter()
                .filter(|sample| sample.at >= boot_two.started_at)
                .map(|sample| (sample.at.as_millis(), &sample.members))
                .collect::<Vec<_>>()
        ),
    );

    // V4 — after the clear, the constant program converges in the R19 order,
    // with the policy route and (while R18 stands) the guard table.
    let final_program = &boot_two.final_program;
    // The typed observer accepts only the canonical owned program shape.
    let canonical = matches!(&boot_two.final_state, Ok(Some(_)));
    let tproxy_rules = final_program
        .listing
        .as_ref()
        .map(|listing| tproxy_rule_orders(listing))
        .unwrap_or_default();
    let r19_order = tproxy_rules.len() == 2 && tproxy_rules.iter().all(|(_, r19)| *r19);
    let policy_route = match (&final_program.ip_rules, &final_program.route_table_100) {
        (Ok(rules), Ok(table)) => policy_route_present(rules, table),
        _ => false,
    };
    // R18 (conditional, FD 3478-3488): asserted only through its provisional
    // observation surface; if 08-01 withdraws R18 it deletes that surface and
    // this clause.
    let intercept_mark_guard = matches!(final_program.intercept_mark_guard, Ok(true));
    let v4 = boot_two.result.is_ok()
        && no_program_before_clear
        && canonical
        && r19_order
        && policy_route
        && intercept_mark_guard;
    let final_identity = match &boot_two.final_state {
        Ok(Some(state)) => Some(identity_fingerprint(state)),
        _ => None,
    };
    verdicts.push(
        "V4 program_converges_after_the_clear_in_r19_order_with_policy_route_and_r18_guard",
        v4,
        format!(
            "typed_final_state={}; boot_one_identity={:016x}; final_identity={final_identity:x?} \
             (fresh listeners: expected to differ); tproxy_rules={tproxy_rules:?}; \
             r19_order={r19_order}; policy_route={policy_route}; ip_rules={:?}; \
             route_table_100={:?}; r18_intercept_mark_guard={:?}; program_or_guard_batches={}",
            summarize_state(&boot_two.final_state),
            residue.identity,
            final_program.ip_rules,
            final_program.route_table_100,
            final_program.intercept_mark_guard,
            order
                .program
                .iter()
                .map(|index| boot_two_batches[*index].describe())
                .collect::<Vec<_>>()
                .join(" || ")
        ),
    );

    // V5 — admission opens only after the clear and the program convergence.
    let v5 = boot_two.replacement.is_some()
        && order.first_admission.is_some_and(|admission| {
            order.clear.is_some_and(|clear| admission > clear)
                && order.program.iter().all(|program| *program < admission)
        })
        && admission_at.is_some_and(|admitted| clear_at.is_some_and(|clear| admitted > clear));
    verdicts.push(
        "V5 admission_opens_after_the_clear_and_program_convergence",
        v5,
        format!(
            "replacement_running={:?}; first_new_allocation_effect_ms={:?}; \
             first_admission_batch={}",
            boot_two.replacement,
            admission_at.map(|at| at.as_millis()),
            describe_batch(order.first_admission)
        ),
    );

    // V6 — no stale member survives: each is retired by a removal event.
    let mut stale = residue.members.clone();
    for batch in boot_two_batches {
        retire(&mut stale, batch);
    }
    let final_members = match &boot_two.final_state {
        Ok(Some(state)) => state_members(state),
        _ => BTreeSet::new(),
    };
    verdicts.push(
        "V6 no_stale_member_survives_restart",
        stale.is_empty(),
        format!(
            "never_removed_stale_members={stale:?}; final_kernel_members={final_members:?}; \
             replayed_membership={:?}; oracles_agree={}; boot_two_returned_ms={}",
            order.membership,
            order.membership == final_members,
            boot_two.returned_at.as_millis()
        ),
    );

    // A1 — the first new lease is the smallest free address of a fresh pool:
    // nothing of the dead server's lease was adopted.
    // The kernel corroborates it: every member the first admission batch adds
    // names that address (`<addr>` or `<addr> . <port>`), whichever install
    // lands first.
    let replacement_lease = boot_two.replacement.as_ref().and_then(|(_, lease)| *lease);
    let first_admission_members = order
        .first_admission
        .map(|index| boot_two_batches[index].intercept_additions())
        .unwrap_or_default();
    let lease = FIRST_FREE_LEASE.to_string();
    let a1 = replacement_lease == Some(FIRST_FREE_LEASE)
        && !first_admission_members.is_empty()
        && first_admission_members.iter().all(|key| {
            key.split_once(':').is_some_and(|(_, member)| {
                member == lease || member.starts_with(&format!("{lease} . "))
            })
        });
    verdicts.push(
        "A1 first_new_lease_is_the_smallest_free_address",
        a1,
        format!(
            "expected={FIRST_FREE_LEASE}; replacement_lease={replacement_lease:?}; \
             dead_server_lease={}; first_admission_members={first_admission_members:?}",
            residue.guest
        ),
    );

    // A2 — the dead allocation's TAP, TCX link pins, endpoint entry, and
    // bridge-guard member are gone before any new allocation takes effect.
    let bound = admission_at.unwrap_or(boot_two.final_state_at);
    let tap_gone = poll.tap_gone_at.is_some_and(|gone| gone < bound);
    let pins_gone = !residue.link_pins.is_empty()
        && residue
            .link_pins
            .keys()
            .all(|pin| poll.pin_gone_at.get(pin).is_some_and(|gone| *gone < bound));
    let endpoint_gone = poll.endpoint_gone_at.is_some_and(|gone| gone < bound);
    let guard_member_gone =
        order.guard_member_removed.is_some_and(|removed| closed_at(removed) < bound);
    verdicts.push(
        "A2 predecessor_attachment_absent_before_admission",
        tap_gone && pins_gone && endpoint_gone && guard_member_gone,
        format!(
            "bound_ms={} (admission observed: {}); tap_gone_ms={:?}; link_pins_gone_ms={:?}; \
             endpoint_entry_gone_ms={:?}; guard_member_removed_batch={}; clear_ms={:?} \
             (R12 sweeps before the clear)",
            bound.as_millis(),
            admission_at.is_some(),
            poll.tap_gone_at.map(|at| at.as_millis()),
            residue
                .link_pins
                .keys()
                .map(|pin| (pin.clone(), poll.pin_gone_at.get(pin).map(Duration::as_millis)))
                .collect::<Vec<_>>(),
            poll.endpoint_gone_at.map(|at| at.as_millis()),
            describe_batch(order.guard_member_removed),
            clear_at.map(|at| at.as_millis())
        ),
    );
    verdicts
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

/// Owns the proof runtime; dropping it (on every path, including a panic)
/// stops boot two's tasks with a bound before the exact-complement cleanup.
struct RuntimeGuard(Option<tokio::runtime::Runtime>);

impl RuntimeGuard {
    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.0.as_ref().expect("live proof runtime").block_on(future)
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(10));
        }
    }
}

/// Restores the exact pre-test host complement on every exit path (GREEN,
/// RED, or panic). Declared before the runtime guard, so the runtime and boot
/// two's tasks stop first.
struct HostComplementGuard {
    evidence: Evidence,
    pre: HostInventory,
}

impl Drop for HostComplementGuard {
    fn drop(&mut self) {
        let restored = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cleanup(&self.evidence, &self.pre);
            self.evidence.record("evidence_dir", &self.evidence.dir.display().to_string());
        }));
        if let Err(panic) = restored {
            eprintln!(
                "[boot-clear-proof] exact-complement cleanup panicked: {} (evidence: {})",
                panic_text(panic.as_ref()),
                self.evidence.dir.display()
            );
        }
    }
}

struct ProofOutcome {
    rendered: String,
    red: usize,
    total: usize,
    phase_events: Vec<String>,
}

/// S-ND295-13C — A killed server's replacement clears stale members and admits work
///
/// Proof §3.5 in killed mode. Boot one admits a mesh Service VM; the lifetime
/// port's killed mode (`ServeSignal::Kill` through `ServeLifetime`) abandons
/// it, leaving the VM, its TAP, TCX link pins, endpoint entry, bridge-guard
/// member, and the three stale intercept members behind. Boot two, on the same
/// data and config roots, must reclaim the VM before any owned-table commit
/// (V1), sweep the attachment and delete exactly the three stale members in
/// one batch before any program mutation (V2), read back zero members (V3),
/// converge the program in the D-295-R19 order with the policy route and the
/// guard table (V4), boot (V0), and only then admit (V5) a replacement at
/// `100.95.0.2` (A1) with every predecessor object already gone (A2); no stale
/// member survives (V6).
///
/// D-295-R18 is conditional on DELIVER step 08-01's native RED. V4's guard
/// clause reads the guard only through its provisional observation surface,
/// `overdrive_netlink::nft::observe_intercept_mark_guard` (FD 3462-3475): if
/// 08-01 withdraws R18 it deletes that surface and this clause with it
/// (FD 3478-3485, "Shape without R18"). D-295-R19 is conditional in the same
/// way (FD 3486-3488); the order clause expects the R19 tail the scenario
/// names.
///
/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 08-02 (S-ND295-13C)"]
#[serial(cgroup)]
fn a_killed_serve_reboot_reclaims_clears_stale_intercept_members_then_admits() {
    let start = Instant::now();
    let root = tempfile::Builder::new()
        .prefix("killed-serve-boot-clear-")
        .tempdir_in(shared_staging_root())
        .expect("proof tempdir on the reflink-capable metal staging root");
    std::fs::create_dir_all(root.path().join("data")).expect("create data dir");
    std::fs::create_dir_all(root.path().join("conf")).expect("create config dir");
    let evidence = Evidence::create(start);
    let _diagnostics = DiagnosticMonitors::start(&evidence);

    // One process-global capture: boot one, killed mode, and boot two all run
    // in this process on the proof runtime's worker threads.
    let trace =
        TraceCapture { evidence: evidence.clone(), events: Arc::new(Mutex::new(Vec::new())) };
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry()
            .with(trace.clone().with_filter(tracing_subscriber::filter::LevelFilter::INFO)),
    )
    .expect("install the proof's tracing capture");

    let pre_inventory = HostInventory::capture();
    evidence.record(
        "precondition",
        &format!(
            "kernel={} inventory={pre_inventory:?}",
            std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim()
        ),
    );
    // Node-shared objects (the constant program with EMPTY sets, the bridge
    // guard, `ovd-gbr0`) legitimately survive a graceful shutdown. Only
    // dynamic allocation residue would contaminate the measured baseline. A
    // program the typed observer rejects (for example a pre-R19 rule order
    // left by an earlier #295 build) is stale node-global state the operator
    // clears first (FD 5203-5207).
    let pre_state = observe_state();
    evidence.record("precondition_intercept_state", &summarize_state(&pre_state));
    let dynamic_members_present = matches!(&pre_state, Ok(Some(state)) if !state_members(state).is_empty())
        || pre_state.is_err();
    if dynamic_members_present
        || pre_inventory.ovd_links.iter().any(|link| link.starts_with("ovd-tp-"))
        || !pre_inventory.alloc_scopes.is_empty()
        || !pre_inventory.ch_pids.is_empty()
    {
        panic!(
            "precondition: the metal host must carry no earlier overdrive guest-network residue; \
             clean it before running this proof (nothing was touched): {pre_inventory:#?}"
        );
    }

    let _complement = HostComplementGuard { evidence: evidence.clone(), pre: pre_inventory };
    let runtime = RuntimeGuard(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("proof runtime"),
    ));
    let outcome = run_proof(&runtime, &evidence, &trace, root.path());
    // Stop boot two's tasks before the complement guard restores the host.
    drop(runtime);
    assert!(
        outcome.red == 0,
        "boot-clear-after-killed-serve contract violated ({} RED of {}):\n{}\n\
         boot-two phase events: {:#?}\nevidence: {}",
        outcome.red,
        outcome.total,
        outcome.rendered,
        outcome.phase_events,
        evidence.dir.display()
    );
}

/// Start the production CLI composition over the proof roots.
async fn start_serve(root: &Path) -> Result<ServeHandle, String> {
    overdrive_cli::commands::serve::run_with_kek(
        ServeArgs {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir: root.join("data"),
            config_dir: root.join("conf"),
        },
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .map_err(|error| format!("{error}"))
}

/// Start the production CLI composition and hand it to the library lifetime
/// owner on a spawned task, so the proof's cleanup can stop it gracefully.
async fn boot_owned_serve(
    root: &Path,
) -> Result<(SignalDriver, tokio::task::JoinHandle<Result<ServeExit, String>>), String> {
    let handle = start_serve(root).await?;
    let (signals, driver) = driven_signals();
    let lifetime = ServeLifetime::new(signals, Arc::new(RecordingClock::new()));
    let task = tokio::spawn(async move { lifetime.run(handle).await.map_err(|e| e.to_string()) });
    Ok((driver, task))
}

fn run_proof(
    runtime: &RuntimeGuard,
    evidence: &Evidence,
    trace: &TraceCapture,
    root: &Path,
) -> ProofOutcome {
    let fixture = VmFixture::provision(&shared_staging_root()).expect("provision VM fixture");
    let guest = build_listener_guest(root);
    let rootfs = stage_rootfs_with_extra_binary(root, &fixture, &guest, GUEST_BINARY);
    let cfg = config_path(root);
    let spec =
        write_toml(root, "boot-clear-residue.toml", &service_toml(&fixture.kernel_path, &rootfs));

    // --- Boot one: admit one mesh Service VM and read its attachment back.
    let (boot_one, residue) = runtime.block_on(async {
        let handle = start_serve(root).await.expect("boot one starts through the composition root");
        evidence.record("boot_one_ready", "serve::run_with_kek");
        let deployed = deploy(DeployArgs { spec, config_path: cfg.clone() })
            .await
            .expect("deploy the residue workload through boot one");
        let running =
            poll_until_running(&cfg, &deployed.workload_id, Duration::from_secs(60)).await;
        let row = running.snapshot.rows.first().expect("one Running boot-one allocation");
        let alloc = AllocationId::new(&row.alloc_id).expect("allocation id parses");
        let guest_ip = row.workload_addr.expect("Running VM publishes its guest address");
        let tap = format!(
            "ovd-tp-{:04x}",
            u16::from_be_bytes([guest_ip.octets()[2], guest_ip.octets()[3]])
        );
        let expected: BTreeSet<String> = [
            member_key("managed_guest_ips", &guest_ip.to_string()),
            member_key("outbound_sources", &guest_ip.to_string()),
            member_key("inbound_destinations", &format!("{guest_ip} . {SERVICE_PORT}")),
        ]
        .into_iter()
        .collect();
        let members_deadline = Instant::now() + Duration::from_secs(30);
        let identity = loop {
            if let Ok(Some(state)) = observe_state()
                && state_members(&state) == expected
            {
                break identity_fingerprint(&state);
            }
            assert!(
                Instant::now() < members_deadline,
                "boot one publishes the exact 2 + P canonical members {expected:?}; last={}",
                summarize_state(&observe_state())
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        let tap_ifindex = interface_ifindex(&tap)
            .expect("read the boot-one TAP ifindex")
            .unwrap_or_else(|| panic!("the Running VM's TAP {tap} exists"));
        let link_pins: BTreeMap<String, u64> = directory_entries(Path::new(TCX_LINK_PINS))
            .into_iter()
            .filter(|name| name.starts_with(&format!("{tap}-")))
            .map(|name| {
                let inode = pin_inode(&Path::new(TCX_LINK_PINS).join(&name))
                    .expect("stat the boot-one TCX link pin")
                    .unwrap_or_else(|| panic!("the boot-one TCX link pin {name} exists"));
                (name, inode)
            })
            .collect();
        let scope = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));
        let ch_pids: BTreeSet<u32> =
            cgroup_procs(&scope).into_iter().filter(|pid| process_is_alive(*pid)).collect();
        (
            handle,
            Residue {
                alloc,
                guest: guest_ip,
                tap,
                tap_ifindex,
                link_pins,
                scope,
                ch_pids,
                members: expected,
                identity,
            },
        )
    });
    evidence.append_file("residue-before-kill.txt", &residue_report(&residue));
    evidence.record("residue_before_kill", &format!("{residue:?}"));

    // --- Killed mode: the lifetime port abandons the owner in-process.
    let kill_started = evidence.elapsed();
    let killed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), kill_serve_owner(boot_one)).await
    });
    let kill_returned = evidence.elapsed();
    evidence.record(
        "boot_one_killed",
        &format!(
            "killed mode outcome={killed:?} took_ms={}",
            kill_returned.saturating_sub(kill_started).as_millis()
        ),
    );
    let kill_window_events: Vec<String> = trace
        .events
        .lock()
        .expect("trace lock")
        .iter()
        .filter(|event| event.at >= kill_started && event.at <= kill_returned)
        .map(|event| {
            format!("at_ms={} name={} fields={:?}", event.at.as_millis(), event.name, event.fields)
        })
        .collect();
    evidence.append_file("kill-window-trace.txt", &kill_window_events.join("\n"));
    let after_kill = observe_state();
    let residue_after_kill = residue_report(&residue);
    evidence.append_file("residue-after-kill.txt", &residue_after_kill);
    evidence.record("residue_after_kill", &residue_after_kill.replace('\n', " | "));

    // The fixture is valid only if killed mode left the real residue the
    // oracles need: the stale members, the VM, the scope, the TAP (same
    // ifindex), its link pins (same inodes), its endpoint entry, and its
    // bridge-guard member.
    let killed_ok = matches!(&killed, Ok(Ok(())));
    let members_survive =
        matches!(&after_kill, Ok(Some(state)) if state_members(state) == residue.members);
    let vmm_survives = residue.ch_pids.iter().any(|pid| process_is_alive(*pid));
    let tap_survives = interface_ifindex(&residue.tap) == Ok(Some(residue.tap_ifindex));
    let pins_survive = !residue.link_pins.is_empty()
        && residue
            .link_pins
            .iter()
            .all(|(pin, inode)| pin_inode(&Path::new(TCX_LINK_PINS).join(pin)) == Ok(Some(*inode)));
    let endpoint_survives = endpoint_entry_present(residue.tap_ifindex) == Ok(true);
    let guard_member_survives =
        bridge_guard_members().is_ok_and(|members| members.contains(&residue.tap));
    assert!(
        killed_ok
            && members_survive
            && vmm_survives
            && residue.scope.exists()
            && tap_survives
            && pins_survive
            && endpoint_survives
            && guard_member_survives,
        "fixture invalid — killed mode did not leave the canonical residue \
         (killed={killed:?}, members_survive={members_survive}, vmm_survives={vmm_survives}, \
         scope={}, tap_survives={tap_survives}, pins_survive={pins_survive}, \
         endpoint_survives={endpoint_survives}, guard_member_survives={guard_member_survives}):\n\
         {residue_after_kill}\nkill-window trace:\n{}",
        residue.scope.exists(),
        kill_window_events.join("\n")
    );

    // --- Oracles armed before boot two.
    let monitor = NftMonitor::start(evidence);
    let poller = Poller::start(evidence, &residue);

    // --- Boot two: the same production composition root over the same roots.
    let proof_pid = std::process::id();
    let boot_two = runtime.block_on(async {
        let started_at = evidence.elapsed();
        evidence.record("boot_two_start", "serve::run_with_kek over the retained roots");
        let outcome = tokio::time::timeout(Duration::from_secs(60), boot_owned_serve(root)).await;
        let returned_at = evidence.elapsed();
        let (result, owner) = match outcome {
            Ok(Ok(owner)) => (Ok(()), Some(owner)),
            Ok(Err(error)) => (Err(error), None),
            Err(_) => (Err("boot two did not return within 60s".to_owned()), None),
        };
        evidence.record("boot_two_returned", &format!("{result:?}"));
        let mut replacement = None;
        if owner.is_some() {
            let deadline = Instant::now() + Duration::from_secs(60);
            while replacement.is_none() && Instant::now() < deadline {
                if let Ok(out) =
                    describe(DescribeArgs { id: WORKLOAD_ID.to_owned(), config_path: cfg.clone() })
                        .await
                    && let Some(row) = out.snapshot.rows.iter().find(|row| {
                        row.state == AllocStateWire::Running
                            && row.alloc_id != residue.alloc.as_str()
                    })
                {
                    replacement = Some((row.alloc_id.clone(), row.workload_addr));
                } else {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
            evidence.record("boot_two_replacement", &format!("{replacement:?}"));
        }
        // Let a replacement's element publication reach the kernel, then close
        // the evaluated window before the proof's own cleanup stop.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let final_state = observe_state();
        let final_program = FinalProgram::capture();
        let final_state_at = evidence.elapsed();
        evidence.record("boot_two_final_state", &summarize_state(&final_state));
        evidence.append_file(
            "boot-two-final-program.txt",
            &format!(
                "listing={:?}\nip_rules={:?}\nroute_table_100={:?}\nintercept_mark_guard={:?}\n",
                final_program.listing,
                final_program.ip_rules,
                final_program.route_table_100,
                final_program.intercept_mark_guard
            ),
        );
        if let Some((signals, lifetime)) = owner {
            let stopped =
                stop(StopArgs { id: WORKLOAD_ID.to_owned(), config_path: cfg.clone() }).await;
            evidence.record("cleanup_stop_replacement", &format!("{stopped:?}"));
            let stop_deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < stop_deadline {
                let settled =
                    describe(DescribeArgs { id: WORKLOAD_ID.to_owned(), config_path: cfg.clone() })
                        .await
                        .is_ok_and(|out| {
                            out.snapshot.rows.iter().all(|row| row.state != AllocStateWire::Running)
                        });
                if settled {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            signals.deliver(ServeSignal::Interrupt);
            let shutdown = tokio::time::timeout(Duration::from_secs(30), lifetime).await;
            evidence.record("cleanup_boot_two_interrupt", &format!("{shutdown:?}"));
        }
        BootTwoObservation {
            started_at,
            returned_at,
            result,
            replacement,
            final_state,
            final_program,
            final_state_at,
        }
    });

    let poll = poller.finish();
    let lines = monitor.stop();
    let events = trace.events.lock().expect("trace lock").clone();

    let boot_two_batches: Vec<Batch> = batches(&lines)
        .into_iter()
        .filter(|batch| {
            batch.closed_at >= boot_two.started_at
                && batch.closed_at <= boot_two.final_state_at
                && (batch.tgid == Some(proof_pid) || (batch.tgid.is_none() && batch.comm != "nft"))
        })
        .collect();
    let mut batch_report = String::new();
    for batch in &boot_two_batches {
        let _ = writeln!(batch_report, "{} events={:?}", batch.describe(), batch.events);
    }
    evidence.append_file("boot-two-batches.txt", &batch_report);

    let phase_events: Vec<String> = events
        .iter()
        .filter(|event| {
            event.at >= boot_two.started_at
                && (event.is("guest_network.shared_owner_boot_phase")
                    || event.is("health.startup.refused")
                    || event.is("guest_network.shared_owner_boot_recovered"))
        })
        .map(|event| {
            format!("at_ms={} name={} fields={:?}", event.at.as_millis(), event.name, event.fields)
        })
        .collect();
    evidence.record("boot_two_phase_events", &format!("{phase_events:?}"));

    let boot_two_events: Vec<TracedEvent> =
        events.into_iter().filter(|event| event.at >= boot_two.started_at).collect();
    let verdicts = evaluate(&residue, &boot_two, &boot_two_events, &boot_two_batches, &poll);
    let rendered = verdicts.render();
    evidence.append_file("verdicts.txt", &rendered);
    evidence.record("verdicts", &rendered.replace('\n', " | "));
    let red = verdicts.rows.iter().filter(|row| !row.1).count();
    ProofOutcome { rendered, red, total: verdicts.rows.len(), phase_events }
}

// ---------------------------------------------------------------------------
// Exact-complement fixture cleanup (runs after all evidence is captured)
// ---------------------------------------------------------------------------

fn cleanup(evidence: &Evidence, pre: &HostInventory) {
    let now = HostInventory::capture();
    evidence.record("cleanup_inventory_before", &format!("{now:?}"));
    for scope in now.alloc_scopes.difference(&pre.alloc_scopes) {
        let path = Path::new(WORKLOADS_SLICE).join(scope);
        let kill = std::fs::write(path.join("cgroup.kill"), "1");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cgroup_procs(&path).is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let rmdir = std::fs::remove_dir(&path);
        evidence.record("cleanup_scope", &format!("{scope}: kill={kill:?} rmdir={rmdir:?}"));
    }
    for pid in now.ch_pids.difference(&pre.ch_pids) {
        // SAFETY: SIGKILL to a Cloud Hypervisor process this proof started.
        let rc = unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
        evidence.record("cleanup_ch_pid", &format!("pid={pid} kill_rc={rc}"));
    }
    // Dynamic set members this proof created (a pre-existing constant
    // program keeps its table, so table removal alone cannot restore it).
    for key in now.intercept_members.difference(&pre.intercept_members) {
        if let Some((set, member)) = key.split_once(':') {
            let element = format!("{{ {member} }}");
            evidence.record(
                "cleanup_intercept_member",
                &command_text("nft", &["delete", "element", "ip", INTERCEPT_TABLE, set, &element])
                    .replace('\n', " | "),
            );
        }
    }
    for table in now.nft_tables.difference(&pre.nft_tables) {
        if let Some(spec) = table.strip_prefix("table ")
            && (spec.ends_with(INTERCEPT_TABLE) || spec.ends_with(INTERCEPT_MARK_GUARD_TABLE))
        {
            let args: Vec<&str> =
                ["delete", "table"].into_iter().chain(spec.split_whitespace()).collect();
            evidence.record("cleanup_nft_table", &command_text("nft", &args).replace('\n', " | "));
        }
    }
    for link in now.ovd_links.difference(&pre.ovd_links) {
        evidence.record(
            "cleanup_link",
            &command_text("ip", &["link", "del", link]).replace('\n', " | "),
        );
    }
    for pin in now.tcx_link_pins.difference(&pre.tcx_link_pins) {
        let removed = std::fs::remove_file(Path::new(TCX_LINK_PINS).join(pin));
        evidence.record("cleanup_tcx_pin", &format!("{pin}: {removed:?}"));
    }
    for dir in now.vm_run_dirs.difference(&pre.vm_run_dirs) {
        let removed = std::fs::remove_dir_all(Path::new("/run/overdrive/vm").join(dir));
        evidence.record("cleanup_vm_run_dir", &format!("{dir}: {removed:?}"));
    }
    if !pre.ip_rules.contains("fwmark 0x1 lookup 100")
        && now.ip_rules.contains("fwmark 0x1 lookup 100")
    {
        evidence.record(
            "cleanup_ip_rule",
            &command_text("ip", &["rule", "del", "fwmark", "0x1", "lookup", "100"])
                .replace('\n', " | "),
        );
    }
    if pre.route_table_100.lines().filter(|line| line.contains("local")).count() == 0
        && now.route_table_100.contains("local")
    {
        evidence.record(
            "cleanup_route_table_100",
            &command_text("ip", &["route", "flush", "table", "100"]).replace('\n', " | "),
        );
    }
    let after = HostInventory::capture();
    evidence.record("cleanup_inventory_after", &format!("{after:?}"));
}
