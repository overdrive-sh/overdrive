//! netns-density-295 correctness-recovery proof §3.5 — boot clear after the
//! serve owner is killed (correctness gap 6).
//!
//! # Contract under proof
//!
//! `docs/feature/netns-density-295/feature-delta.md` § *DESIGN-02-03
//! correction* and § RUN-295 allocation registration, plus ADR-0124: no
//! allocation-element token survives a process restart. After the serve
//! owner is lost the next boot through the production composition root must
//! run, in this order:
//!
//! 1. VMM reclamation (every Cloud Hypervisor process the dead owner left is
//!    killed and its allocation scope removed);
//! 2. boot clear — one atomic nft batch that deletes every stale member of the
//!    three `ip overdrive-mtls` dynamic sets while retaining the constant
//!    program (it never deletes a set, rule, chain, or table);
//! 3. empty read-back — the three sets are empty before anything else runs;
//! 4. constant-program adoption / listener-target convergence (D15);
//! 5. admission — only then may an allocation insert new members.
//!
//! The boot must RECOVER (not refuse) and no stale capability member may
//! survive into the admitted state.
//!
//! # How the owner is lost: the serve lifetime port's killed mode
//!
//! Boot one runs the production CLI composition (`serve::run_with_kek` →
//! `run_server`) in this test process and is owned by the library lifetime
//! owner `ServeLifetime` — the same owner `main.rs` runs. The test deploys a
//! mesh Service VM through the public `deploy` handler, proves the canonical
//! non-empty dynamic sets on the kernel, then delivers the port's killed-mode
//! signal (`ServeSignal::Kill`). Killed mode abandons the owner without
//! graceful shutdown or workload cleanup; see
//! `ServerHandle::kill_for_test` for exactly which in-process cleanup still
//! runs. The residue is read back from the kernel BEFORE boot two, which runs
//! the same composition root in this process against the unchanged data and
//! config roots. The `overdrive` binary is never spawned.
//!
//! No test code installs, clears, or mutates any intercept element. The lower
//! adapter `clear_shared_ip_intercept_elements_atomically` is never called by
//! this test: whether a production caller reaches it is exactly what the
//! proof observes.
//!
//! # Oracles (Tier-3, observable kernel side effects only)
//!
//! * `nft monitor` — the kernel's ordered commit stream. Each batch ends with
//!   `# new generation N by process TID (comm)`, so element deletions,
//!   program mutations, and admissions are ordered and attributed to this
//!   process without trusting any product log;
//! * the typed `overdrive_netlink::nft::observe_shared_ip_intercept_state`
//!   read-back, sampled independently;
//! * `/proc` liveness of the dead owner's Cloud Hypervisor PIDs and the
//!   allocation cgroup scope, sampled every 2 ms;
//! * the bridge guard, TAP, and pinned TCX-link inventory as residue evidence;
//! * the production boot-phase tracing events as program-order corroboration.
//!
//! Every verdict is evaluated and reported; the test fails listing each RED
//! verdict. Evidence is appended (never rewritten) under
//! `/var/tmp/overdrive-killed-serve-boot-clear-proof/<run>/`.
//!
//! # Placement
//!
//! `overdrive-cli` owns the `serve` + `deploy` production entry points and the
//! lifetime port; every real-guest restart scenario of this feature lives in
//! this integration binary behind `integration-tests` + `kvm-tests` (native
//! x86_64 metal only). The binary is a member of the catch-all
//! `host-kernel-shared` nextest group.
//!
//! Hypothesis: boot clear has no production caller, so boot two's D15 shared
//! observation meets the non-empty stale sets and refuses startup after VMM
//! reclamation has already run.
//! Prediction: `run_with_kek` returns `Err`; a `health.startup.refused` event
//! with `reason = "mtls.shared_owner"`; boot-one CH PIDs dead; no boot-two
//! element-deletion batch; the three stale members still present.
//! Falsification: a boot-two batch deleting every stale member before any
//! program mutation, followed by a successful boot and admission.
//!
//! CONTRACT_SHAPE: bounded-change.

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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use overdrive_cli::commands::deploy::{DeployArgs, StopArgs, deploy, stop};
use overdrive_cli::commands::serve::ServeArgs;
use overdrive_cli::commands::serve_lifetime::{ServeExit, ServeLifetime, ServeSignal};
use overdrive_cli::commands::workload::{DescribeArgs, describe};
use overdrive_control_plane::api::AllocStateWire;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::AllocationId;
use overdrive_netlink::nft::bridge::{BridgeGuardSpec, observe as observe_bridge_guard};
use overdrive_netlink::nft::{self, SharedIpInterceptState};
use overdrive_testing::vm_fixture::VmFixture;
use serial_test::serial;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::serve_lifetime_support::{RecordingClock, SignalDriver, driven_signals};
use super::vm_walking_skeleton::{
    config_path, poll_until_running, shared_staging_root, stage_rootfs_with_extra_binary,
    write_toml,
};

const WORKLOAD_ID: &str = "boot-clear-residue";
const SERVICE_PORT: u16 = 18_961;
const GUEST_BINARY: &str = "boot-clear-listener";
const INTERCEPT_TABLE: &str = "overdrive-mtls";
const WORKLOADS_SLICE: &str = "/sys/fs/cgroup/overdrive.slice/workloads.slice";
const TCX_LINK_PINS: &str = "/sys/fs/bpf/overdrive/mtls-endpoints/links";
const EVIDENCE_ROOT: &str = "/var/tmp/overdrive-killed-serve-boot-clear-proof";
const MONITOR_PROBE_TABLE: &str = "ovd_boot_clear_monitor_probe";

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

// ---------------------------------------------------------------------------
// In-process tracing capture for boot two
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
            let _ = Command::new("nft")
                .args(["add", "table", "inet", MONITOR_PROBE_TABLE])
                .status()
                .expect("run nft add probe table");
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum IpEvent {
    AddElement {
        set: String,
        member: String,
    },
    DeleteElement {
        set: String,
        member: String,
    },
    FlushSet {
        set: String,
    },
    /// Any table/chain/set/rule mutation of the constant program.
    Program {
        verb: String,
        kind: String,
        set_deleted: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct Batch {
    closed_at: Duration,
    tid: Option<u32>,
    tgid: Option<u32>,
    comm: String,
    lines: Vec<String>,
}

impl Batch {
    fn ip_events(&self) -> Vec<IpEvent> {
        self.lines.iter().flat_map(|line| parse_ip_events(line)).collect()
    }

    fn touches_intercept_table(&self) -> bool {
        !self.ip_events().is_empty()
    }

    fn mutates_program(&self) -> bool {
        self.ip_events().iter().any(|event| matches!(event, IpEvent::Program { .. }))
    }

    fn adds_elements(&self) -> bool {
        self.ip_events().iter().any(|event| matches!(event, IpEvent::AddElement { .. }))
    }

    fn only_element_removals(&self) -> bool {
        let events = self.ip_events();
        !events.is_empty()
            && events.iter().all(|event| {
                matches!(event, IpEvent::DeleteElement { .. } | IpEvent::FlushSet { .. })
            })
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

fn parse_ip_events(line: &str) -> Vec<IpEvent> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    // `<verb> <kind> ip overdrive-mtls ...`; the bare table line has exactly
    // four tokens. Bridge-family and foreign tables are not in this replay.
    if tokens.len() < 4 || tokens[2] != "ip" || tokens[3] != INTERCEPT_TABLE {
        return Vec::new();
    }
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
                .map(str::trim)
                .filter(|member| !member.is_empty())
                .map(|member| {
                    let member = member.to_owned();
                    if verb == "add" {
                        IpEvent::AddElement { set: set.clone(), member }
                    } else {
                        IpEvent::DeleteElement { set: set.clone(), member }
                    }
                })
                .collect()
        }
        ("flush", "set") if tokens.len() >= 5 => {
            vec![IpEvent::FlushSet { set: tokens[4].to_owned() }]
        }
        (_, "table" | "chain" | "set" | "rule" | "map" | "flowtable" | "counter" | "quota") => {
            let set_deleted = (matches!(verb, "delete" | "destroy") && kind == "set")
                .then(|| tokens.get(4).map(|name| (*name).to_owned()))
                .flatten();
            let table_deleted = matches!(verb, "delete" | "destroy") && kind == "table";
            vec![IpEvent::Program {
                verb: verb.to_owned(),
                kind: kind.to_owned(),
                set_deleted: if table_deleted { Some("*".to_owned()) } else { set_deleted },
            }]
        }
        ("flush", _) => vec![IpEvent::Program {
            verb: verb.to_owned(),
            kind: kind.to_owned(),
            set_deleted: None,
        }],
        _ => Vec::new(),
    }
}

fn batches(lines: &[MonitorLine]) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut pending = Vec::new();
    for line in lines {
        if let Some(rest) = line.text.strip_prefix("# new generation ") {
            let comm = rest
                .rsplit_once('(')
                .and_then(|(_, comm)| comm.strip_suffix(')'))
                .unwrap_or_default()
                .to_owned();
            out.push(Batch {
                closed_at: line.at,
                tid: generation_tid(&line.text),
                tgid: line.tgid,
                comm,
                lines: std::mem::take(&mut pending),
            });
        } else {
            pending.push(line.text.clone());
        }
    }
    out
}

/// Retire tracked stale members by deletion events only; a later add of the
/// same address is a fresh boot-two member, never the surviving stale one.
fn retire(stale: &mut BTreeSet<String>, batch: &Batch) {
    for event in batch.ip_events() {
        match event {
            IpEvent::DeleteElement { set, member } => {
                stale.remove(&member_key(&set, &member));
            }
            IpEvent::FlushSet { set } | IpEvent::Program { set_deleted: Some(set), .. } => {
                if set == "*" {
                    stale.clear();
                } else {
                    stale.retain(|key| !key.starts_with(&format!("{set}:")));
                }
            }
            IpEvent::AddElement { .. } | IpEvent::Program { .. } => {}
        }
    }
}

/// Replay one batch against the tracked membership of the three sets.
fn apply(membership: &mut BTreeSet<String>, batch: &Batch) {
    for event in batch.ip_events() {
        match event {
            IpEvent::AddElement { set, member } => {
                membership.insert(member_key(&set, &member));
            }
            IpEvent::DeleteElement { set, member } => {
                membership.remove(&member_key(&set, &member));
            }
            IpEvent::FlushSet { set } => {
                membership.retain(|key| !key.starts_with(&format!("{set}:")));
            }
            IpEvent::Program { set_deleted: Some(set), .. } => {
                if set == "*" {
                    membership.clear();
                } else {
                    membership.retain(|key| !key.starts_with(&format!("{set}:")));
                }
            }
            IpEvent::Program { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 2 ms liveness / scope / typed-state poller
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct PollHistory {
    pid_dead_at: BTreeMap<u32, Duration>,
    scope_gone_at: Option<Duration>,
    state_samples: Vec<(Duration, String)>,
}

struct Poller {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<PollHistory>>,
}

impl Poller {
    fn start(evidence: &Evidence, pids: BTreeSet<u32>, scope: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let evidence = evidence.clone();
        let handle = thread::spawn(move || {
            let mut history = PollHistory::default();
            let mut last_state = String::new();
            let mut tick = 0_u64;
            while !flag.load(Ordering::SeqCst) {
                let now = evidence.elapsed();
                for pid in &pids {
                    if !history.pid_dead_at.contains_key(pid) && !process_is_alive(*pid) {
                        history.pid_dead_at.insert(*pid, now);
                        evidence.record("poll_pid_dead", &format!("pid={pid}"));
                    }
                }
                if history.scope_gone_at.is_none() && !scope.exists() {
                    history.scope_gone_at = Some(now);
                    evidence.record("poll_scope_gone", &scope.display().to_string());
                }
                if tick.is_multiple_of(10) {
                    let summary = summarize_state(&observe_state());
                    if summary != last_state {
                        evidence.record("poll_intercept_state", &summary);
                        history.state_samples.push((evidence.elapsed(), summary.clone()));
                        last_state = summary;
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
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Diagnostic link/udev oracles (append-only files; never asserted on)
// ---------------------------------------------------------------------------

/// Background `ip monitor link` and `udevadm monitor` writing straight to
/// evidence files, started before boot one so a
/// boot-one refusal still carries its own kernel link history.
struct DiagnosticMonitors {
    children: Vec<Child>,
}

impl DiagnosticMonitors {
    fn start(evidence: &Evidence) -> Self {
        let mut children = Vec::new();
        for (name, program, args) in [
            ("link-monitor.log", "ip", &["-ts", "-d", "monitor", "link"][..]),
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

#[derive(Debug, Clone)]
struct Residue {
    alloc: AllocationId,
    guest: Ipv4Addr,
    tap: String,
    scope: PathBuf,
    ch_pids: BTreeSet<u32>,
    members: BTreeSet<String>,
    identity: u64,
}

fn residue_report(residue: &Residue) -> String {
    let guard = BridgeGuardSpec::new(
        INTERCEPT_TABLE.to_owned(),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("canonical bridge guard spec");
    let mut report = String::new();
    let _ = writeln!(report, "alloc={} guest={} tap={}", residue.alloc, residue.guest, residue.tap);
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
    let _ =
        writeln!(report, "tap_present={}", Path::new("/sys/class/net").join(&residue.tap).exists());
    let _ = writeln!(report, "tcx_link_pins={:?}", directory_entries(Path::new(TCX_LINK_PINS)));
    let _ = writeln!(
        report,
        "bridge_guard={:?}",
        observe_bridge_guard(&guard, &BTreeSet::new()).map_err(|error| error.to_string())
    );
    let _ = writeln!(
        report,
        "vm_run_dir_present={}",
        Path::new("/run/overdrive/vm").join(residue.alloc.as_str()).exists()
    );
    report.push_str(&command_text("nft", &["list", "table", "ip", INTERCEPT_TABLE]));
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

struct BootTwoObservation {
    started_at: Duration,
    returned_at: Duration,
    result: Result<(), String>,
    replacement: Option<(String, Option<Ipv4Addr>)>,
    final_state: Result<Option<SharedIpInterceptState>, String>,
    /// End of the evaluated window: taken before the proof's own cleanup
    /// `stop`, so stop-driven deletions can never be mistaken for boot clear.
    final_state_at: Duration,
}

fn evaluate(
    residue: &Residue,
    boot_two: &BootTwoObservation,
    events: &[TracedEvent],
    boot_two_batches: &[Batch],
    poll: &PollHistory,
) -> Verdicts {
    let mut verdicts = Verdicts { rows: Vec::new() };
    let refused: Vec<&TracedEvent> =
        events.iter().filter(|event| event.is("health.startup.refused")).collect();

    // V0 — the restart recovers through the real composition root.
    verdicts.push(
        "V0 restart_recovers_through_composition_root",
        boot_two.result.is_ok() && refused.is_empty(),
        format!(
            "run_with_kek={:?}; startup_refused_events={:?}",
            boot_two.result,
            refused.iter().map(|event| &event.fields).collect::<Vec<_>>()
        ),
    );

    // Replay the kernel commit stream from the proven residue baseline.
    let mut membership = residue.members.clone();
    let mut clear_index = None;
    let mut first_program_index = None;
    let mut first_admission_index = None;
    let mut membership_before_program = None;
    for (index, batch) in boot_two_batches.iter().enumerate() {
        if !batch.touches_intercept_table() {
            continue;
        }
        let before = membership.clone();
        apply(&mut membership, batch);
        if clear_index.is_none()
            && first_program_index.is_none()
            && batch.only_element_removals()
            && before == residue.members
            && membership.is_empty()
        {
            clear_index = Some(index);
        }
        if first_program_index.is_none() && batch.mutates_program() {
            first_program_index = Some(index);
            membership_before_program = Some(before.clone());
        }
        if first_admission_index.is_none() && batch.adds_elements() {
            first_admission_index = Some(index);
        }
    }
    let first_intercept_mutation =
        boot_two_batches.iter().find(|batch| batch.touches_intercept_table()).map(|b| b.closed_at);

    // V1 — VMM reclamation completes before any boot-two intercept mutation.
    let pid_deaths: Vec<Option<Duration>> =
        residue.ch_pids.iter().map(|pid| poll.pid_dead_at.get(pid).copied()).collect();
    let all_dead = pid_deaths.iter().all(Option::is_some);
    let reclaimed_at = if all_dead {
        pid_deaths.iter().flatten().copied().chain(poll.scope_gone_at).max()
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
        && match (reclaimed_at, first_intercept_mutation) {
            (Some(reclaimed), Some(mutation)) => reclaimed < mutation,
            (Some(_), None) => true,
            _ => false,
        };
    verdicts.push(
        "V1 vmm_reclamation_precedes_intercept_recovery",
        v1,
        format!(
            "ch_pid_deaths_ms={:?}; scope_gone_ms={:?}; reclamation_completed_event_ms={:?}; \
             first_boot_two_intercept_batch_ms={:?}",
            residue
                .ch_pids
                .iter()
                .map(|pid| (*pid, poll.pid_dead_at.get(pid).map(Duration::as_millis)))
                .collect::<Vec<_>>(),
            poll.scope_gone_at.map(|at| at.as_millis()),
            reclamation_completed_event.map(|event| event.at.as_millis()),
            first_intercept_mutation.map(|at| at.as_millis())
        ),
    );

    // V2 — one atomic boot-two batch deletes every stale member, program retained.
    verdicts.push(
        "V2 boot_clear_deletes_every_stale_member_in_one_batch",
        clear_index.is_some(),
        clear_index.map_or_else(
            || {
                format!(
                    "no boot-two batch removed the {} stale member(s) {:?} before a program \
                     mutation; boot-two intercept batches={}",
                    residue.members.len(),
                    residue.members,
                    boot_two_batches.iter().filter(|batch| batch.touches_intercept_table()).count()
                )
            },
            |index| boot_two_batches[index].describe(),
        ),
    );

    // V3 — the sets are empty from the clear until program convergence.
    let v3 = match (clear_index, first_program_index, first_admission_index) {
        (Some(clear), Some(program), _) => {
            clear < program && membership_before_program.as_ref().is_some_and(BTreeSet::is_empty)
        }
        (Some(clear), None, admission) => admission.is_none_or(|admission| admission > clear),
        (None, _, _) => false,
    };
    verdicts.push(
        "V3 sets_empty_between_boot_clear_and_program_convergence",
        v3,
        format!(
            "clear_batch={clear_index:?}; first_program_batch={first_program_index:?}; \
             membership_before_program={membership_before_program:?}"
        ),
    );

    // V4 — the constant program converges after the clear and is canonical.
    let program_present = matches!(boot_two.final_state, Ok(Some(_)));
    let v4 = boot_two.result.is_ok()
        && program_present
        && clear_index.is_some()
        && first_program_index.is_none_or(|program| Some(program) > clear_index);
    let final_identity = match &boot_two.final_state {
        Ok(Some(state)) => Some(identity_fingerprint(state)),
        _ => None,
    };
    verdicts.push(
        "V4 constant_program_converges_after_clear",
        v4,
        format!(
            "boot_one_identity={:016x}; final_identity={final_identity:x?} (equal = adopted, \
             different = converged to new listener targets); final_state={}; first_program_batch={}",
            residue.identity,
            summarize_state(&boot_two.final_state),
            first_program_index.map_or_else(
                || "none".to_owned(),
                |index| boot_two_batches[index].describe()
            )
        ),
    );

    // V5 — admission only after clear and convergence.
    let v5 = boot_two.replacement.is_some()
        && match (first_admission_index, clear_index) {
            (Some(admission), Some(clear)) => {
                admission > clear && first_program_index.is_none_or(|program| admission > program)
            }
            _ => false,
        };
    verdicts.push(
        "V5 admission_after_clear_and_convergence",
        v5,
        format!(
            "replacement_running={:?}; first_admission_batch={}",
            boot_two.replacement,
            first_admission_index
                .map_or_else(|| "none".to_owned(), |index| boot_two_batches[index].describe())
        ),
    );

    // V6 — no stale boot-one member survives in the kernel.
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
            "never_deleted_stale_members={stale:?}; final_kernel_members={final_members:?}; \
             replayed_membership={membership:?}; oracles_agree={}; boot_two_returned_ms={}",
            membership == final_members,
            boot_two.returned_at.as_millis()
        ),
    );
    verdicts
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

/// Owns the parent runtime; a panic path still shuts it down with a bound so
/// boot two's tasks are stopped before the exact-complement cleanup runs.
struct RuntimeGuard(Option<tokio::runtime::Runtime>);

impl RuntimeGuard {
    fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.0.as_ref().expect("live parent runtime").block_on(future)
    }
}

impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_timeout(Duration::from_secs(10));
        }
    }
}

struct ProofOutcome {
    rendered: String,
    red: usize,
    total: usize,
    phase_events: Vec<String>,
}

/// Proof §3.5 — a killed serve owner leaves non-empty dynamic sets; the next
/// boot through the production composition root must reclaim VMMs, clear the
/// three sets atomically, read back empty, converge the constant program, and
/// only then admit.
/// CONTRACT_SHAPE: bounded-change.
#[test]
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
    let diagnostics = DiagnosticMonitors::start(&evidence);

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
    // dynamic allocation residue would contaminate the measured baseline.
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

    let runtime = RuntimeGuard(Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("proof runtime"),
    ));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_proof(&runtime, &evidence, &trace, root.path())
    }));
    // Every path — GREEN, RED, or panic — stops the runtime first, then
    // restores the exact pre-test host complement.
    drop(runtime);
    cleanup(&evidence, &pre_inventory);
    drop(diagnostics);
    evidence.record("evidence_dir", &evidence.dir.display().to_string());
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(panic) => std::panic::resume_unwind(panic),
    };
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

/// Boot the production CLI composition over the proof roots and hand it to
/// the library lifetime owner on a spawned task.
async fn boot_owned_serve(
    root: &Path,
) -> Result<(SignalDriver, tokio::task::JoinHandle<Result<ServeExit, String>>), String> {
    let handle = overdrive_cli::commands::serve::run_with_kek(
        ServeArgs {
            bind: "127.0.0.1:0".parse().expect("loopback bind"),
            data_dir: root.join("data"),
            config_dir: root.join("conf"),
        },
        Arc::new(overdrive_sim::adapters::SimKek::for_boot()),
    )
    .await
    .map_err(|error| format!("{error}"))?;
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

    // --- Boot one, owned by the lifetime port; admit one mesh Service VM.
    let (boot_one_signals, boot_one_lifetime, residue) = runtime.block_on(async {
        let (signals, lifetime) =
            boot_owned_serve(root).await.expect("boot one starts through the composition root");
        evidence.record("boot_one_ready", "serve::run_with_kek owned by ServeLifetime");
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
        let scope = CgroupPath::for_alloc(&alloc).resolve(Path::new("/sys/fs/cgroup"));
        let ch_pids: BTreeSet<u32> =
            cgroup_procs(&scope).into_iter().filter(|pid| process_is_alive(*pid)).collect();
        (
            signals,
            lifetime,
            Residue { alloc, guest: guest_ip, tap, scope, ch_pids, members: expected, identity },
        )
    });
    evidence.append_file("residue-before-kill.txt", &residue_report(&residue));
    evidence.record("residue_before_kill", &format!("{residue:?}"));

    // --- Killed mode: the lifetime port abandons the owner in-process.
    let kill_started = evidence.elapsed();
    boot_one_signals.deliver(ServeSignal::Kill);
    let killed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(60), boot_one_lifetime)
            .await
            .map(|joined| joined.expect("boot-one lifetime task joins"))
    });
    let kill_returned = evidence.elapsed();
    evidence.record(
        "boot_one_killed",
        &format!(
            "lifetime outcome={killed:?} took_ms={}",
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
    let killed_ok = matches!(&killed, Ok(Ok(ServeExit::Killed)));
    let residue_members_survive =
        matches!(&after_kill, Ok(Some(state)) if state_members(state) == residue.members);
    let vmm_survives = residue.ch_pids.iter().any(|pid| process_is_alive(*pid));
    assert!(
        killed_ok && residue_members_survive && vmm_survives && residue.scope.exists(),
        "fixture invalid — killed mode did not leave the canonical residue \
         (killed={killed:?}, members_survive={residue_members_survive}, \
         vmm_survives={vmm_survives}, scope={}):\n{residue_after_kill}\n\
         kill-window trace:\n{}",
        residue.scope.exists(),
        kill_window_events.join("\n")
    );

    // --- Oracles armed before boot two.
    let monitor = NftMonitor::start(evidence);
    let poller = Poller::start(evidence, residue.ch_pids.clone(), residue.scope.clone());

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
        let final_state_at = evidence.elapsed();
        evidence.record("boot_two_final_state", &summarize_state(&final_state));
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
        let _ = writeln!(batch_report, "{}", batch.describe());
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
            && spec.ends_with(INTERCEPT_TABLE)
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
