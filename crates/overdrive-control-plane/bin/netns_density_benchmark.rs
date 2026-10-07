//! DISTILL-owned E18 support: real production owners, raw native measurements.
//! The two profiles have fixed accepted populations. This binary neither derives
//! recovery bounds nor records their values; that work belongs to step 09-01.
#![allow(clippy::print_stderr, clippy::too_many_lines, clippy::result_large_err)]

use clap::{Parser, ValueEnum};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Profile {
    #[value(name = "e18-t1-base")]
    E18T1Base,
    #[value(name = "e18-t1-port4")]
    E18T1Port4,
}

impl Profile {
    const fn name(self) -> &'static str {
        match self {
            Self::E18T1Base => "e18-t1-base",
            Self::E18T1Port4 => "e18-t1-port4",
        }
    }

    const fn ports(self) -> &'static [u16] {
        match self {
            Self::E18T1Base => &[],
            Self::E18T1Port4 => &[8080, 8081, 8443, 9000],
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Non-EDD M-ND295-E18 raw measurements on the canonical native metal lease")]
struct Args {
    #[arg(long, value_enum)]
    profile: Profile,
    #[arg(long)]
    out: PathBuf,
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
    match native::run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("E18 measurement refused or failed: {error:?}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let args = Args::parse();
    eprintln!("{} requires native Linux metal; output {}", args.profile.name(), args.out.display());
    let _ports = args.profile.ports();
    ExitCode::FAILURE
}

#[cfg(target_os = "linux")]
mod native {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::net::Ipv4Addr;
    use std::num::NonZeroU16;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use ipnet::Ipv4Net;
    use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
    use overdrive_control_plane::dns_responder::responder::DnsResponderError;
    use overdrive_control_plane::dns_responder::{
        GuestDns, GuestDnsDeps, GuestDnsFactory, HostGuestDnsFactory,
    };
    use overdrive_control_plane::guest_network::{
        GuestAddressPool, GuestNetworkError, GuestNetworkPlan, SharedGuestNetworkAuditError,
        SharedGuestNetworkOwner, TapActivation, e18_test_support,
    };
    use overdrive_control_plane::identity_mgr::IdentityMgr;
    use overdrive_control_plane::mtls_resolve_adapter::ServiceBackendsResolve;
    use overdrive_core::traits::IdentityRead;
    use overdrive_core::traits::clock::Clock;
    use overdrive_core::traits::driver::{AllocationSpec, DriverPayload, Resources, VmPayload};
    use overdrive_core::traits::mtls_enforcement::{MtlsEnforcement, MtlsLimits};
    use overdrive_core::traits::mtls_resolve::{MtlsResolve, MtlsResolveError};
    use overdrive_core::traits::observation_store::{ObservationStore, ObservationStoreError};
    use overdrive_core::{AllocationId, SpiffeId};
    use overdrive_dataplane::mtls::HostMtlsEnforcement;
    use overdrive_host::{RealCgroupFs, SystemClock};
    use overdrive_netlink::nft::bridge::{BridgeGuardDeleteOutcome, BridgeGuardSpec};
    use overdrive_netlink::{
        Client, ObservedLinkIdentity, ObservedLinkKind, block_on_host_netlink, nft,
    };
    use overdrive_store_local::LocalObservationStore;
    use overdrive_worker::cgroup_manager::{CgroupManager, CgroupPath};
    use overdrive_worker::mtls_intercept_port::{HostMtlsIntercept, MtlsIntercept};
    use overdrive_worker::mtls_intercept_worker::{
        MtlsInterceptInstallError, MtlsInterceptOwnerShutdownError, MtlsInterceptStopError,
        MtlsInterceptWorker, MtlsSharedOwnerError,
    };
    use serde_json::{Value, json};
    use tokio::process::Child;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    use super::{Args, Profile};

    const POPULATION: usize = 16_384;
    const SAMPLES: usize = 5;
    const BRIDGE: &str = "ovd-gbr0";
    const PIN_ROOT: &str = "/sys/fs/bpf/overdrive/mtls-endpoints";
    const CGROUP_ROOT: &str = "/sys/fs/cgroup";
    const SLICE: &str = "overdrive.slice/workloads.slice";
    const DIAGNOSTIC_TARGET: &str = "overdrive::netns_density_benchmark";
    const HOLD_EVENT: &str = "e18.member_audit_mutex_hold";
    const DOWN_EVENT: &str = "e18.quiescence_last_tap_down";

    #[derive(Debug)]
    pub enum Precondition {
        NativePhysicalHost,
        CanonicalLease,
        SourceMarker,
        ExistingOwnedKernelState,
        ForeignWorkload,
        UnexpectedProcess,
        ExistingReceipt,
        LiveOwner,
    }

    #[derive(Debug)]
    pub enum TraceProblem {
        Missing,
        Duplicate,
        MissingField,
        WrongPopulation,
        InvalidDuration,
        WrongLastAttachment,
    }

    /// Test-support taxonomy. Production errors retain their exact original
    /// typed sources; no missing production distinction is encoded in strings.
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("support I/O failed ({operation}) at {path}: {source}")]
        Io {
            operation: &'static str,
            path: PathBuf,
            #[source]
            source: std::io::Error,
        },
        #[error("diagnostic command {program} failed: status={status:?}, stderr={stderr}")]
        Command { program: &'static str, status: Option<i32>, stderr: String },
        #[error("invalid diagnostic JSON on {surface}: {source}")]
        Json {
            surface: &'static str,
            #[source]
            source: serde_json::Error,
        },
        #[error("measurement precondition {reason:?} was not met at {path}")]
        Precondition { reason: Precondition, path: PathBuf },
        #[error("diagnostic {event} has {problem:?}")]
        Diagnostic { event: &'static str, problem: TraceProblem },
        #[error("population mismatch on {surface}: expected {expected}, observed {observed}")]
        Population { surface: &'static str, expected: usize, observed: usize },
        #[error("activation was latched for {alloc}")]
        ActivationLatched { alloc: AllocationId },
        #[error("quiescence left unconfirmed allocations: {unconfirmed:?}")]
        Quiescence { unconfirmed: BTreeMap<AllocationId, GuestNetworkError> },
        #[error("full audit reported damaged allocation attachments: {damaged:?}")]
        AttachmentDamage { damaged: BTreeMap<AllocationId, GuestNetworkError> },
        #[error("allocation {alloc} process did not terminate by SIGKILL: {status}")]
        ProcessTermination { alloc: AllocationId, status: std::process::ExitStatus },
        #[error("foreign {surface} complement changed")]
        Complement { surface: String },
        #[error("shared guest-network owner: {0}")]
        Guest(#[from] GuestNetworkError),
        #[error("shared guest-network audit: {0}")]
        GuestAudit(#[from] SharedGuestNetworkAuditError),
        #[error("worker shared-owner call: {0}")]
        Worker(#[from] MtlsSharedOwnerError),
        #[error("worker allocation install: {0}")]
        Install(#[from] MtlsInterceptInstallError),
        #[error("worker allocation stop: {0}")]
        Stop(#[from] MtlsInterceptStopError),
        #[error("worker shutdown: {0}")]
        WorkerShutdown(#[from] MtlsInterceptOwnerShutdownError),
        #[error("DNS owner: {0}")]
        Dns(#[from] DnsResponderError),
        #[error("real observation store: {0}")]
        Store(#[from] ObservationStoreError),
        #[error("real resolve adapter: {0}")]
        Resolve(#[from] MtlsResolveError),
        #[error("support input parse: {0}")]
        Id(#[from] overdrive_core::id::IdParseError),
        #[error("support cgroup path parse: {0}")]
        CgroupPath(#[from] overdrive_core::cgroup::CgroupPathError),
        #[error("bridge-guard cleanup: {0}")]
        BridgeGuard(#[from] overdrive_netlink::nft::bridge::BridgeGuardError),
        #[error("bridge-guard test-support input: {0}")]
        BridgeGuardInput(#[from] overdrive_netlink::nft::bridge::BridgeGuardValidationError),
        #[error("shared-infrastructure observation/restoration: {0}")]
        Netlink(#[from] overdrive_netlink::NetlinkError),
        #[error("support task join: {0}")]
        Join(#[from] tokio::task::JoinError),
        #[error("measurement error={primary:?}; cleanup failures={cleanup:?}")]
        Cleanup { primary: Option<Box<Self>>, cleanup: Vec<Self> },
        #[error("diagnostic subscriber installation: {0}")]
        Subscriber(#[from] tracing::subscriber::SetGlobalDefaultError),
    }

    type Result<T> = std::result::Result<T, Error>;

    fn io(operation: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io { operation, path: path.into(), source }
    }

    fn read(path: impl AsRef<Path>) -> Result<String> {
        let path = path.as_ref();
        std::fs::read_to_string(path).map_err(|source| io("read", path, source))
    }

    fn output(program: &'static str, args: &[&str]) -> Result<std::process::Output> {
        Command::new(program).args(args).output().map_err(|source| io("spawn", program, source))
    }

    fn command(program: &'static str, args: &[&str]) -> Result<String> {
        let result = output(program, args)?;
        if !result.status.success() {
            return Err(Error::Command {
                program,
                status: result.status.code(),
                stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&result.stdout).into_owned())
    }

    fn diagnostic(program: &'static str, args: &[&str]) -> Result<Value> {
        serde_json::from_str(&command(program, args)?)
            .map_err(|source| Error::Json { surface: program, source })
    }

    fn write_json(path: &Path, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|source| Error::Json { surface: "receipt", source })?;
        std::fs::write(path, bytes).map_err(|source| io("write", path, source))
    }

    struct Raw {
        file: File,
        path: PathBuf,
    }

    impl Raw {
        fn open(path: PathBuf) -> Result<Self> {
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .map_err(|source| io("create raw samples", &path, source))?;
            Ok(Self { file, path })
        }

        fn record(&mut self, value: &Value) -> Result<()> {
            serde_json::to_writer(&mut self.file, value)
                .map_err(|source| Error::Json { surface: "raw samples", source })?;
            self.file
                .write_all(b"\n")
                .map_err(|source| io("append raw sample", &self.path, source))?;
            self.file.flush().map_err(|source| io("flush raw sample", &self.path, source))
        }
    }

    #[derive(Clone, Default)]
    struct Diagnostics(Arc<parking_lot::Mutex<Vec<Value>>>);

    #[derive(Default)]
    struct Fields(BTreeMap<String, Value>);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().to_owned(), json!(format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), json!(value));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().to_owned(), json!(value));
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.insert(field.name().to_owned(), json!(value));
        }
    }

    impl<S: tracing::Subscriber> Layer<S> for Diagnostics {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != DIAGNOSTIC_TARGET {
                return;
            }
            let mut fields = Fields::default();
            event.record(&mut fields);
            self.0.lock().push(json!(fields.0));
        }
    }

    impl Diagnostics {
        fn clear(&self) {
            self.0.lock().clear();
        }

        fn take(&self, expected: &'static str) -> Result<Value> {
            let values = std::mem::take(&mut *self.0.lock());
            let matching =
                values.into_iter().filter(|value| value["event"] == expected).collect::<Vec<_>>();
            match matching.as_slice() {
                [] => Err(Error::Diagnostic { event: expected, problem: TraceProblem::Missing }),
                [one] => Ok(one.clone()),
                _ => Err(Error::Diagnostic { event: expected, problem: TraceProblem::Duplicate }),
            }
        }
    }

    fn field_u64(event: &Value, name: &str, kind: &'static str) -> Result<u64> {
        event[name]
            .as_u64()
            .ok_or(Error::Diagnostic { event: kind, problem: TraceProblem::MissingField })
    }

    fn event_duration(event: &Value, prefix: &str, kind: &'static str) -> Result<Duration> {
        let secs = field_u64(event, &format!("{prefix}_secs"), kind)?;
        let nanos = field_u64(event, &format!("{prefix}_subsec_nanos"), kind)?;
        let subsec = u32::try_from(nanos)
            .ok()
            .filter(|nanos| *nanos < 1_000_000_000)
            .ok_or(Error::Diagnostic { event: kind, problem: TraceProblem::InvalidDuration })?;
        Ok(Duration::new(secs, subsec))
    }

    fn source_and_host() -> Result<Value> {
        let virtualized = output("systemd-detect-virt", &[])?;
        let cpuinfo = read("/proc/cpuinfo")?;
        if command("uname", &["-m"])?.trim() != "x86_64"
            || virtualized.status.code() != Some(1)
            || String::from_utf8_lossy(&virtualized.stdout).trim() != "none"
            || cpuinfo.split_whitespace().any(|word| word == "hypervisor")
            || Path::new("/sys/hypervisor/type").exists()
        {
            return Err(Error::Precondition {
                reason: Precondition::NativePhysicalHost,
                path: "/proc/cpuinfo".into(),
            });
        }
        let marker = read(".overdrive-metal-source")?;
        let lease = read("/run/lock/overdrive-metal-shared.owner")?;
        let parse = |text: &str, key: &str| {
            text.lines().find_map(|line| line.strip_prefix(key)).map(str::to_owned)
        };
        // Canonical metal sync deliberately excludes .git by default. The
        // launcher computes this commit and dirty/untracked-source digest on
        // the editing host, writes the marker after sync, and native-preflight
        // verifies the exact marker under this run's lease. A remote Git HEAD
        // may therefore be absent or stale; it is not the synced source's
        // identity. Keep the marker's commit/digest and lease cross-check.
        let hex = |value: &str| value.bytes().all(|byte| byte.is_ascii_hexdigit());
        let sha = parse(&marker, "commit=")
            .filter(|value| matches!(value.len(), 40 | 64) && hex(value))
            .ok_or(Error::Precondition {
                reason: Precondition::SourceMarker,
                path: ".overdrive-metal-source".into(),
            })?;
        let source_digest = parse(&marker, "source_digest=")
            .filter(|value| value.len() == 64 && hex(value))
            .ok_or(Error::Precondition {
                reason: Precondition::SourceMarker,
                path: ".overdrive-metal-source".into(),
            })?;
        let workspace = parse(&marker, "workspace=").filter(|value| !value.is_empty()).ok_or(
            Error::Precondition {
                reason: Precondition::SourceMarker,
                path: ".overdrive-metal-source".into(),
            },
        )?;
        let lease_pid = parse(&lease, "pid=").filter(|pid| pid.parse::<u32>().is_ok());
        if parse(&lease, "commit=").as_deref() != Some(&sha)
            || parse(&lease, "workspace=").as_deref() != Some(&workspace)
            || parse(&lease, "action=").as_deref() != Some("run")
            || lease_pid.is_none_or(|pid| !Path::new(&format!("/proc/{pid}")).is_dir())
        {
            return Err(Error::Precondition {
                reason: Precondition::CanonicalLease,
                path: "/run/lock/overdrive-metal-shared.owner".into(),
            });
        }
        Ok(json!({
            "measured_source_sha": sha,
            "canonical_source_digest": source_digest,
            "executable_sha256": command("sha256sum", &["/proc/self/exe"])?.split_whitespace().next(),
            "cpu_model": cpuinfo.lines().find_map(|line| line.strip_prefix("model name").and_then(|value| value.split_once(':')).map(|(_, model)| model.trim())),
            "logical_cpus": cpuinfo.lines().filter(|line| line.starts_with("processor\t")).count(),
            "memory": read("/proc/meminfo")?,
            "dmi_product": read("/sys/class/dmi/id/product_name")?.trim(),
            "uname_r": command("uname", &["-r"])?.trim(),
            "cloud_hypervisor_version": command("cloud-hypervisor", &["--version"])?.trim(),
            "virtualization": "none",
            "canonical_native_preflight": "infra/metal/native-preflight.sh (cargo xtask metal run)",
        }))
    }

    fn source_manifest(dir: &Path) -> Result<()> {
        // Enumerate the actual synced inputs. The remote .git/index is not
        // synchronized by default and cannot enumerate the measured source.
        fn collect(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
            for entry in std::fs::read_dir(directory)
                .map_err(|source| io("enumerate synced source", directory, source))?
            {
                let entry =
                    entry.map_err(|source| io("read synced source entry", directory, source))?;
                let path = entry.path();
                if path.is_dir() {
                    if entry.file_name() != "target" && entry.file_name() != ".git" {
                        collect(&path, paths)?;
                    }
                } else if path.is_file() {
                    paths.push(path);
                }
            }
            Ok(())
        }
        let mut paths = Vec::new();
        collect(Path::new("crates"), &mut paths)?;
        collect(Path::new(".cargo"), &mut paths)?;
        for name in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"] {
            let path = PathBuf::from(name);
            if path.is_file() {
                paths.push(path);
            }
        }
        paths.sort();
        paths.dedup();
        let mut manifest = String::new();
        for chunk in paths.chunks(128) {
            let args = chunk
                .iter()
                .map(|path| {
                    path.to_str().ok_or_else(|| {
                        io(
                            "source checksum path",
                            path,
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "source checksum path is not UTF-8",
                            ),
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            manifest.push_str(&command("sha256sum", &args)?);
        }
        let path = dir.join("source-files.sha256");
        std::fs::write(&path, manifest).map_err(|source| io("write source manifest", path, source))
    }

    fn stable(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                // Only mutable execution counters are excluded; all identity,
                // schema, handle, ownership, and ordering facts are preserved.
                if let Some(Value::Object(counter)) = fields.get_mut("counter") {
                    counter.remove("packets");
                    counter.remove("bytes");
                }
                fields.remove("run_time_ns");
                fields.remove("run_cnt");
                for value in fields.values_mut() {
                    stable(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    stable(value);
                }
            }
            _ => {}
        }
    }

    fn raw_snapshot() -> Result<Value> {
        Ok(json!({
            "links": diagnostic("ip", &["-j", "-d", "link", "show"] )?,
            "nft": diagnostic("nft", &["-j", "list", "ruleset"] )?,
            "policy_rules": diagnostic("ip", &["-j", "rule", "show"] )?,
            "table_100": diagnostic("ip", &["-j", "route", "show", "table", "100"] )?,
            "bpf_programs": diagnostic("bpftool", &["-j", "prog", "show"] )?,
            "bpf_maps": diagnostic("bpftool", &["-j", "map", "show"] )?,
            "bpf_links": diagnostic("bpftool", &["-j", "link", "show"] )?,
        }))
    }

    fn normalized_snapshot(mut value: Value) -> Value {
        stable(&mut value);
        if let Some(links) = value["links"].as_array_mut() {
            for link in links {
                if link["linkinfo"]["info_kind"] == "bridge"
                    && link["linkinfo"]["info_data"]["gc_timer"].is_number()
                    && let Some(data) = link["linkinfo"]["info_data"].as_object_mut()
                {
                    // Linux IFLA_BR_GC_TIMER is remaining gc_work.timer time,
                    // not bridge configuration (v7.0 br_netlink/br_stp_timer).
                    // A read-only native two-second probe proved this field
                    // alone ticks; raw snapshots retain its exact values.
                    data.remove("gc_timer");
                }
            }
        }
        value
    }

    #[allow(
        clippy::struct_excessive_bools,
        reason = "independent presence observations from existing kernel ports; these are not lifecycle state flags"
    )]
    struct PriorKernelState {
        ip_program: Option<nft::SharedIpInterceptIdentity>,
        bridge: Option<ObservedLinkIdentity>,
        bridge_guard: bool,
        mark_guard: bool,
        fwmark_rule: bool,
        local_route: bool,
        pin_directories: BTreeMap<PathBuf, Value>,
    }

    fn bridge_guard_spec() -> Result<BridgeGuardSpec> {
        Ok(BridgeGuardSpec::new(
            "overdrive-mtls".to_owned(),
            "prerouting".to_owned(),
            "managed_taps".to_owned(),
            -300,
            0x295a,
            0x295b,
        )?)
    }

    fn pin_directories() -> Result<BTreeMap<PathBuf, Value>> {
        fn visit(path: &Path, directories: &mut BTreeMap<PathBuf, Value>) -> Result<()> {
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|source| io("observe pin hierarchy", path, source))?;
            if !metadata.is_dir() {
                // A pin or foreign entry is not an empty reusable directory.
                // Stale map ownership must be resolved before the fixture.
                return Err(Error::Precondition {
                    reason: Precondition::ExistingOwnedKernelState,
                    path: path.to_path_buf(),
                });
            }
            directories.insert(path.to_path_buf(), json!({"inode": metadata.ino(), "device": metadata.dev(), "mode": metadata.mode(), "uid": metadata.uid(), "gid": metadata.gid()}));
            for entry in std::fs::read_dir(path)
                .map_err(|source| io("enumerate pin hierarchy", path, source))?
            {
                let entry = entry.map_err(|source| io("read pin hierarchy entry", path, source))?;
                visit(&entry.path(), directories)?;
            }
            Ok(())
        }
        let mut directories = BTreeMap::new();
        if Path::new(PIN_ROOT).exists() {
            visit(Path::new(PIN_ROOT), &mut directories)?;
        }
        Ok(directories)
    }

    fn inspect_prior_kernel_state(before: &Value) -> Result<PriorKernelState> {
        // A retained empty graph can outlive its worker. A live worker cannot
        // be replaced by a second benchmark owner, even with zero allocations.
        for entry in std::fs::read_dir("/proc")
            .map_err(|source| io("observe live owners", "/proc", source))?
        {
            let entry = entry.map_err(|source| io("read process entry", "/proc", source))?;
            let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == std::process::id() {
                continue;
            }
            let path = entry.path().join("exe");
            let executable = match std::fs::read_link(&path) {
                Ok(executable) => executable,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(io("observe process executable", path, source)),
            };
            let name = executable
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .trim_end_matches(" (deleted)");
            if matches!(name, "overdrive" | "cloud-hypervisor" | "netns_density_benchmark") {
                return Err(Error::Precondition { reason: Precondition::LiveOwner, path });
            }
        }
        let links = before["links"].as_array().into_iter().flatten();
        if links.filter_map(|link| link["ifname"].as_str()).any(|name| name.starts_with("ovd-tp-"))
            || before["links"].as_array().into_iter().flatten().any(|link| link["master"] == BRIDGE)
        {
            return Err(Error::Precondition {
                reason: Precondition::ExistingOwnedKernelState,
                path: PIN_ROOT.into(),
            });
        }
        let slice = Path::new(CGROUP_ROOT).join(SLICE);
        if slice.exists() {
            verify_scopes(&slice, &BTreeSet::new())?;
        }
        let state = nft::observe_shared_ip_intercept_state()?;
        if state.as_ref().is_some_and(|state| {
            !state.managed_guest_ips().is_empty()
                || !state.outbound_sources().is_empty()
                || !state.inbound_destinations().is_empty()
        }) {
            return Err(Error::Precondition {
                reason: Precondition::ExistingOwnedKernelState,
                path: "nft:ip/overdrive-mtls/members".into(),
            });
        }
        let bridge_guard = match nft::bridge::observe(&bridge_guard_spec()?, &BTreeSet::new())? {
            nft::bridge::BridgeGuardObservation::Exact { .. } => true,
            nft::bridge::BridgeGuardObservation::Absent { .. } => false,
            nft::bridge::BridgeGuardObservation::Conflict { .. } => {
                return Err(Error::Precondition {
                    reason: Precondition::ExistingOwnedKernelState,
                    path: "nft:bridge/overdrive-mtls".into(),
                });
            }
        };
        let mark_guard = nft::observe_intercept_mark_guard()?;
        let (bridge, gateway, fwmark_rule, local_route) = block_on_host_netlink(|| async {
            let client = Client::new()?;
            let bridge = client.observe_link_identity(BRIDGE).await?;
            let gateway = if bridge.is_some() {
                client.observe_addr(BRIDGE, Ipv4Addr::new(100, 95, 0, 1), 16).await?
            } else {
                false
            };
            Ok((
                bridge,
                gateway,
                client.fib_rule_fwmark_present(1, 100).await?,
                client.local_route_present(100, "lo").await?,
            ))
        })?;
        if bridge.as_ref().is_some_and(|bridge| {
            bridge.kind != ObservedLinkKind::Bridge
                || bridge.mac != Some(overdrive_core::dataplane::GUEST_BRIDGE_MAC)
                || !gateway
        }) {
            return Err(Error::Precondition {
                reason: Precondition::ExistingOwnedKernelState,
                path: BRIDGE.into(),
            });
        }
        Ok(PriorKernelState {
            ip_program: state.map(|state| state.identity().clone()),
            bridge,
            bridge_guard,
            mark_guard,
            fwmark_rule,
            local_route,
            pin_directories: pin_directories()?,
        })
    }

    fn verify_scopes(slice: &Path, expected: &BTreeSet<String>) -> Result<()> {
        let observed = std::fs::read_dir(slice)
            .map_err(|source| io("read scope complement", slice, source))?
            .map(|entry| entry.map_err(|source| io("read scope", slice, source)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        if observed != *expected || !read(slice.join("cgroup.procs"))?.trim().is_empty() {
            return Err(Error::Precondition {
                reason: Precondition::ForeignWorkload,
                path: slice.to_path_buf(),
            });
        }
        Ok(())
    }

    struct Fixture {
        pool: GuestAddressPool,
        owner: Arc<dyn SharedGuestNetworkOwner>,
        worker: Arc<MtlsInterceptWorker>,
        intercept: HostMtlsIntercept,
        resolve: Arc<ServiceBackendsResolve>,
        dns: Arc<dyn GuestDns>,
        dns_task: Option<tokio::task::JoinHandle<()>>,
        plans: Vec<GuestNetworkPlan>,
        children: BTreeMap<AllocationId, Child>,
        cgroups: CgroupManager,
        created_slice: bool,
        created_parent: bool,
        created_bpf_parent: bool,
    }

    impl Fixture {
        fn new(dir: &Path) -> Result<Self> {
            let clock: Arc<dyn Clock> = Arc::new(SystemClock);
            let store: Arc<dyn ObservationStore> =
                Arc::new(LocalObservationStore::open(dir.join("observations.redb"))?);
            let frontend = FrontendAddrAllocator::new();
            let resolve =
                Arc::new(ServiceBackendsResolve::new(Arc::clone(&store), frontend.clone()));
            let identity: Arc<dyn IdentityRead> = Arc::new(IdentityMgr::new(None));
            let enforcement: Arc<dyn MtlsEnforcement> =
                Arc::new(HostMtlsEnforcement::new(identity, MtlsLimits::default()));
            let intercept = HostMtlsIntercept::new();
            let intercept_port: Arc<dyn MtlsIntercept> = Arc::new(intercept.clone());
            let resolve_port: Arc<dyn MtlsResolve> = resolve.clone();
            let worker = Arc::new(MtlsInterceptWorker::new(
                enforcement,
                resolve_port,
                Arc::clone(&clock),
                intercept_port,
            ));
            let gateway = Ipv4Addr::new(100, 95, 0, 1);
            let dns =
                HostGuestDnsFactory.responder(GuestDnsDeps { store, clock, gateway, frontend });
            Ok(Self {
                pool: GuestAddressPool::new(
                    Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 0), 16),
                    BRIDGE.to_owned(),
                    gateway,
                    gateway,
                ),
                owner: e18_test_support::host_owner(),
                worker,
                intercept,
                resolve,
                dns,
                dns_task: None,
                plans: Vec::with_capacity(POPULATION),
                children: BTreeMap::new(),
                cgroups: CgroupManager::new(CGROUP_ROOT.into(), Arc::new(RealCgroupFs::new())),
                created_slice: !Path::new(CGROUP_ROOT).join(SLICE).exists(),
                created_parent: !Path::new(CGROUP_ROOT).join("overdrive.slice").exists(),
                created_bpf_parent: !Path::new("/sys/fs/bpf/overdrive").exists(),
            })
        }

        async fn start(&mut self, trace: &Diagnostics) -> Result<()> {
            self.owner.probe_startup().await?;
            self.owner.converge_shared().await?;
            self.resolve.probe().await?;
            self.worker.start_shared_owner().await?;
            // Refuse missing mutex instrumentation before creating the full
            // population. This warm-up observation is not a density sample.
            trace.clear();
            self.worker.audit_shared_owner().await?;
            let event = trace.take(HOLD_EVENT)?;
            let _duration = event_duration(&event, "hold", HOLD_EVENT)?;
            self.dns.probe().await?;
            self.dns_task = Some(tokio::spawn(Arc::clone(&self.dns).serve()));
            Ok(())
        }

        async fn attach(&mut self, profile: Profile, index: usize) -> Result<()> {
            let alloc = AllocationId::new(&format!("e18-{index:05}"))?;
            let plan = e18_test_support::assign(&self.pool, alloc)?;
            // Retain before the first effect so failed provisioning is still
            // driven through ordinary awaited teardown during cleanup.
            self.plans.push(plan.clone());
            self.owner.provision(&plan).await?;
            let service_ports =
                profile.ports().iter().filter_map(|port| NonZeroU16::new(*port)).collect();
            let spec = AllocationSpec {
                alloc: plan.alloc().clone(),
                identity: SpiffeId::new(&format!(
                    "spiffe://overdrive.local/workload/e18/alloc/{index:05}"
                ))?,
                driver: DriverPayload::Vm(VmPayload {
                    command: "/bin/true".to_owned(),
                    args: Vec::new(),
                    kernel: "/e18/no-vmm/kernel".into(),
                    rootfs: "/e18/no-vmm/rootfs".into(),
                }),
                resources: Resources { cpu_milli: 50, memory_bytes: 32 * 1024 * 1024 },
                probe_descriptors: Vec::new(),
                network: Some(plan.assignment().clone()),
                service_ports,
            };
            self.worker.start_alloc(&spec).await?;
            if self.owner.activate(&plan).await? != TapActivation::Raised {
                return Err(Error::ActivationLatched { alloc: plan.alloc().clone() });
            }
            Ok(())
        }

        fn verify_population(&self, profile: Profile, expected: usize) -> Result<Value> {
            let members = self
                .intercept
                .observe_shared_state()
                .map_err(|source| Error::Worker(MtlsSharedOwnerError::Intercept { source }))?
                .ok_or(Error::Population {
                    surface: "shared IP program",
                    expected: 1,
                    observed: 0,
                })?;
            let expected_ips =
                self.plans.iter().map(|plan| plan.assignment().address).collect::<BTreeSet<_>>();
            let expected_destinations = expected_ips
                .iter()
                .flat_map(|ip| {
                    profile.ports().iter().map(move |port| std::net::SocketAddrV4::new(*ip, *port))
                })
                .collect::<BTreeSet<_>>();
            for (surface, wanted, actual) in [
                ("managed_guest_ips", expected, members.members.managed_guest_ips.len()),
                ("outbound_sources", expected, members.members.outbound_sources.len()),
                (
                    "inbound_destinations",
                    expected * profile.ports().len(),
                    members.members.inbound_destinations.len(),
                ),
            ] {
                if actual != wanted {
                    return Err(Error::Population { surface, expected: wanted, observed: actual });
                }
            }
            if members.members.managed_guest_ips != expected_ips
                || members.members.outbound_sources != expected_ips
                || members.members.inbound_destinations != expected_destinations
            {
                return Err(Error::Complement {
                    surface: "exact allocation IP members".to_owned(),
                });
            }
            let taps = diagnostic("ip", &["-j", "-d", "link", "show", "master", BRIDGE])?;
            let observed_names = taps
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|link| link["ifname"].as_str().map(str::to_owned))
                .collect::<BTreeSet<_>>();
            let names = self
                .plans
                .iter()
                .map(|plan| plan.assignment().tap.clone())
                .collect::<BTreeSet<_>>();
            if names != observed_names {
                return Err(Error::Population {
                    surface: "bridge TAP membership",
                    expected,
                    observed: observed_names.len(),
                });
            }
            self.verify_tap_flags(true)?;
            Ok(json!({"ip_members": format!("{:?}", members.members), "bridge_links": taps}))
        }

        fn verify_tap_flags(&self, up: bool) -> Result<()> {
            for plan in &self.plans {
                let path = PathBuf::from(format!("/sys/class/net/{}/flags", plan.assignment().tap));
                let flags = read(&path)?;
                let active = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16)
                    .ok()
                    .map(|flags| flags & 1 != 0);
                if active != Some(up) {
                    return Err(Error::Complement {
                        surface: format!("{} administrative state", plan.assignment().tap),
                    });
                }
            }
            Ok(())
        }

        async fn quiesce(&self, trace: &Diagnostics) -> Result<(Duration, Value)> {
            trace.clear();
            let started = Instant::now();
            let result = self.owner.quiesce_managed_taps().await?;
            let wall = started.elapsed();
            if !result.unconfirmed.is_empty() {
                return Err(Error::Quiescence { unconfirmed: result.unconfirmed });
            }
            let event = trace.take(DOWN_EVENT)?;
            let confirmed = field_u64(&event, "confirmed_taps", DOWN_EVENT)?;
            if usize::try_from(confirmed).ok() != Some(self.plans.len()) {
                return Err(Error::Diagnostic {
                    event: DOWN_EVENT,
                    problem: TraceProblem::WrongPopulation,
                });
            }
            let last = self.plans.last().ok_or(Error::Diagnostic {
                event: DOWN_EVENT,
                problem: TraceProblem::WrongPopulation,
            })?;
            if event["last_alloc"].as_str() != Some(&last.alloc().to_string())
                || event["last_tap"].as_str() != Some(last.assignment().tap.as_str())
            {
                return Err(Error::Diagnostic {
                    event: DOWN_EVENT,
                    problem: TraceProblem::WrongLastAttachment,
                });
            }
            if event_duration(&event, "elapsed", DOWN_EVENT)? > wall {
                return Err(Error::Diagnostic {
                    event: DOWN_EVENT,
                    problem: TraceProblem::InvalidDuration,
                });
            }
            self.verify_tap_flags(false)?;
            Ok((wall, event))
        }

        async fn samples(&self, trace: &Diagnostics, raw: &mut Raw) -> Result<()> {
            for sample in 0..SAMPLES {
                let started = Instant::now();
                let audit = self.owner.audit_shared().await?;
                let wall = started.elapsed();
                if !audit.damaged.is_empty() {
                    return Err(Error::AttachmentDamage { damaged: audit.damaged });
                }
                raw.record(&json!({"kind": "owner_full_audit", "sample": sample, "owner": "shared_guest_network", "wall_ns": wall.as_nanos()}))?;
                trace.clear();
                let started = Instant::now();
                self.worker.audit_shared_owner().await?;
                let wall = started.elapsed();
                let event = trace.take(HOLD_EVENT)?;
                if event_duration(&event, "hold", HOLD_EVENT)? > wall {
                    return Err(Error::Diagnostic {
                        event: HOLD_EVENT,
                        problem: TraceProblem::InvalidDuration,
                    });
                }
                raw.record(&json!({"kind": "owner_full_audit", "sample": sample, "owner": "mtls_worker", "wall_ns": wall.as_nanos(), "element_effects_hold": event}))?;
                let started = Instant::now();
                self.dns.audit().await?;
                raw.record(&json!({"kind": "owner_full_audit", "sample": sample, "owner": "guest_dns", "wall_ns": started.elapsed().as_nanos()}))?;
                let (wall, down) = self.quiesce(trace).await?;
                raw.record(&json!({"kind": "quiesce", "sample": sample, "wall_ns": wall.as_nanos(), "last_tap_down": down}))?;
                let started = Instant::now();
                self.owner.restore_quiesced_taps().await?;
                let wall = started.elapsed();
                self.verify_tap_flags(true)?;
                raw.record(
                    &json!({"kind": "restore", "sample": sample, "wall_ns": wall.as_nanos()}),
                )?;
            }
            Ok(())
        }

        async fn populate_processes(&mut self, raw: &mut Raw, cohort: &'static str) -> Result<()> {
            for plan in &self.plans {
                let scope = CgroupPath::for_alloc(plan.alloc());
                self.cgroups.create_workload_scope(&scope).await.map_err(|source| {
                    io("create allocation scope", scope.resolve(Path::new(CGROUP_ROOT)), source)
                })?;
                let child = tokio::process::Command::new("/bin/sleep")
                    .arg("86400")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|source| io("spawn live allocation process", "/bin/sleep", source))?;
                let pid = child.id().ok_or(Error::Precondition {
                    reason: Precondition::UnexpectedProcess,
                    path: scope.resolve(Path::new(CGROUP_ROOT)),
                })?;
                self.children.insert(plan.alloc().clone(), child);
                self.cgroups.place_pid_in_scope(&scope, pid).await.map_err(|source| {
                    io(
                        "place live allocation process",
                        scope.resolve(Path::new(CGROUP_ROOT)),
                        source,
                    )
                })?;
                raw.record(&json!({"kind": "live_process", "cohort": cohort, "alloc": plan.alloc().to_string(), "scope": scope.as_str(), "pid": pid}))?;
            }
            self.verify_processes()
        }

        fn verify_processes(&self) -> Result<()> {
            let scopes = self
                .plans
                .iter()
                .map(|plan| format!("{}.scope", plan.alloc()))
                .collect::<BTreeSet<_>>();
            verify_scopes(&Path::new(CGROUP_ROOT).join(SLICE), &scopes)?;
            for (alloc, child) in &self.children {
                let path = CgroupPath::for_alloc(alloc).resolve(Path::new(CGROUP_ROOT));
                let pid = child.id().ok_or(Error::Precondition {
                    reason: Precondition::UnexpectedProcess,
                    path: path.clone(),
                })?;
                for entry in std::fs::read_dir(&path)
                    .map_err(|source| io("read allocation scope children", &path, source))?
                {
                    let entry = entry.map_err(|source| io("read scope child", &path, source))?;
                    if entry.path().is_dir() {
                        return Err(Error::Precondition {
                            reason: Precondition::ForeignWorkload,
                            path: entry.path(),
                        });
                    }
                }
                if read(path.join("cgroup.procs"))?.trim() != pid.to_string()
                    || !read(format!("/proc/{pid}/cgroup"))?
                        .lines()
                        .any(|line| line == format!("0::/{SLICE}/{alloc}.scope"))
                    || read(format!("/proc/{pid}/status"))?
                        .lines()
                        .any(|line| line.starts_with("State:") && line.contains("Z (zombie)"))
                {
                    return Err(Error::Precondition {
                        reason: Precondition::UnexpectedProcess,
                        path,
                    });
                }
            }
            Ok(())
        }

        async fn reap_killed(&mut self) -> Result<()> {
            while let Some((alloc, mut child)) = self.children.pop_first() {
                let status = child
                    .wait()
                    .await
                    .map_err(|source| io("reap allocation process", alloc.to_string(), source))?;
                if status.signal() != Some(libc::SIGKILL) {
                    return Err(Error::ProcessTermination { alloc, status });
                }
            }
            Ok(())
        }

        async fn kills(&mut self, raw: &mut Raw) -> Result<()> {
            self.populate_processes(raw, "ordered_per_vm").await?;
            let mut writes = Vec::with_capacity(self.plans.len());
            let loop_started = Instant::now();
            for plan in &self.plans {
                let scope = CgroupPath::for_alloc(plan.alloc());
                let started = Instant::now();
                self.cgroups.cgroup_kill(&scope).await.map_err(|source| {
                    io("per-VM cgroup.kill", scope.resolve(Path::new(CGROUP_ROOT)), source)
                })?;
                writes.push((plan.alloc().clone(), started.elapsed()));
            }
            let loop_wall = loop_started.elapsed();
            // Persist outside K's measured interval: filesystem logging and
            // child reaping are not part of the production kill-write loop.
            for (order, (alloc, wall)) in writes.into_iter().enumerate() {
                raw.record(&json!({"kind": "per_vm_kill_write", "order": order, "alloc": alloc.to_string(), "wall_ns": wall.as_nanos()}))?;
            }
            raw.record(&json!({"kind": "all_allocation_kill_loop", "ordered_allocations": self.plans.len(), "wall_ns": loop_wall.as_nanos()}))?;
            self.reap_killed().await?;
            self.populate_processes(raw, "populated_workloads_slice").await?;
            // The already-approved workloads_slice constructor remains RED
            // until 09-01. Use the existing validated FromStr support surface;
            // do not implement that later production method in this step.
            let slice: CgroupPath = SLICE.parse()?;
            let started = Instant::now();
            self.cgroups.cgroup_kill(&slice).await.map_err(|source| {
                io(
                    "populated workloads-slice cgroup.kill",
                    slice.resolve(Path::new(CGROUP_ROOT)),
                    source,
                )
            })?;
            raw.record(&json!({"kind": "workloads_slice_kill_write", "live_allocations": self.plans.len(), "wall_ns": started.elapsed().as_nanos()}))?;
            self.reap_killed().await
        }

        async fn cleanup(&mut self) -> Vec<Error> {
            let mut failures = Vec::new();
            // Cleanup kills only retained support children, never a global
            // slice. Normal measurement already reaped both full cohorts.
            for (alloc, mut child) in std::mem::take(&mut self.children) {
                if let Err(source) = child.kill().await {
                    failures.push(io("cleanup child", alloc.to_string(), source));
                }
            }
            for plan in self.plans.iter().rev() {
                if let Err(source) = self.worker.stop_alloc(plan.alloc()).await {
                    failures.push(source.into());
                }
                match self.owner.teardown(plan).await {
                    Ok(()) => e18_test_support::release(&self.pool, plan.alloc()),
                    Err(source) => failures.push(source.into()),
                }
                if let Err(source) =
                    self.cgroups.remove_workload_scope(&CgroupPath::for_alloc(plan.alloc())).await
                {
                    failures.push(io("cleanup allocation scope", plan.alloc().to_string(), source));
                }
            }
            if let Err(source) = self.worker.shutdown_owner().await {
                failures.push(source.into());
            }
            self.dns.stop();
            if let Some(task) = self.dns_task.take()
                && let Err(source) = task.await
            {
                failures.push(source.into());
            }
            e18_test_support::shutdown_resolve(&self.resolve).await;
            failures
        }
    }

    fn cleanup_shared(
        prior: &PriorKernelState,
        created_slice: bool,
        created_parent: bool,
        created_bpf_parent: bool,
    ) -> Result<()> {
        // Published-worker shutdown deliberately retains the empty current IP
        // program. Restore the previously observed semantic graph through the
        // existing conditional adapter, or remove only our newly created one.
        let current = nft::observe_shared_ip_intercept_state()?;
        if current.as_ref().is_some_and(|state| {
            !state.managed_guest_ips().is_empty()
                || !state.outbound_sources().is_empty()
                || !state.inbound_destinations().is_empty()
        }) {
            return Err(Error::Complement {
                surface: "shared IP members after owner shutdown".to_owned(),
            });
        }
        let current_identity = current.as_ref().map(nft::SharedIpInterceptState::identity);
        if current_identity != prior.ip_program.as_ref() {
            nft::replace_shared_ip_intercept_atomically(
                current_identity,
                prior.ip_program.as_ref(),
            )?;
        }
        if !prior.mark_guard && nft::observe_intercept_mark_guard()? {
            nft::delete_table("overdrive-mtls-guard")?;
        }
        block_on_host_netlink(|| async {
            let client = Client::new()?;
            if !prior.fwmark_rule && client.fib_rule_fwmark_present(1, 100).await? {
                client.delete_unique_fib_rule_fwmark(1, 100).await?;
            }
            if !prior.local_route && client.local_route_present(100, "lo").await? {
                client.delete_unique_local_route(100, "lo").await?;
            }
            Ok(())
        })?;
        if !prior.bridge_guard {
            match nft::bridge::delete_owned_guard(&bridge_guard_spec()?, &BTreeSet::new())? {
                BridgeGuardDeleteOutcome::Absent { .. }
                | BridgeGuardDeleteOutcome::Deleted { .. } => {}
                BridgeGuardDeleteOutcome::Conflict { .. } => {
                    return Err(Error::Complement {
                        surface: "owned bridge guard at cleanup".to_owned(),
                    });
                }
            }
        }
        let links = Path::new(PIN_ROOT).join("links");
        if links.exists()
            && std::fs::read_dir(&links)
                .map_err(|source| io("read owned link pins", &links, source))?
                .next()
                .is_some()
        {
            return Err(Error::Complement {
                surface: "allocation link pins after teardown".to_owned(),
            });
        }
        for name in ["endpoints", "counters"] {
            let path = Path::new(PIN_ROOT).join("maps").join(name);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(io("unpin fixture map", path, source)),
            }
        }
        for path in [links, Path::new(PIN_ROOT).join("maps"), PathBuf::from(PIN_ROOT)] {
            if prior.pin_directories.contains_key(&path) {
                continue;
            }
            match std::fs::remove_dir(&path) {
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(io("remove empty fixture pin directory", path, source)),
            }
        }
        let bpf_parent = Path::new("/sys/fs/bpf/overdrive");
        if created_bpf_parent && bpf_parent.exists() {
            std::fs::remove_dir(bpf_parent)
                .map_err(|source| io("remove empty fixture BPF pin parent", bpf_parent, source))?;
        }
        let observed_bridge =
            block_on_host_netlink(|| async { Client::new()?.observe_link_identity(BRIDGE).await })?;
        if let Some(prior_bridge) = &prior.bridge {
            let mut observed = observed_bridge.ok_or_else(|| Error::Complement {
                surface: "pre-existing bridge vanished".to_owned(),
            })?;
            let was_up = observed.up;
            observed.up = prior_bridge.up;
            if observed != *prior_bridge {
                return Err(Error::Complement {
                    surface: "pre-existing bridge identity changed".to_owned(),
                });
            }
            if was_up != prior_bridge.up {
                block_on_host_netlink(|| async {
                    let client = Client::new()?;
                    if prior_bridge.up {
                        client.set_link_up(BRIDGE).await
                    } else {
                        client.set_link_down(BRIDGE).await
                    }
                })?;
            }
        } else if observed_bridge.is_some() {
            block_on_host_netlink(|| async { Client::new()?.del_link(BRIDGE).await })?;
        }
        if pin_directories()? != prior.pin_directories {
            return Err(Error::Complement { surface: "pre-existing pin directories".to_owned() });
        }
        if created_slice && Path::new(CGROUP_ROOT).join(SLICE).exists() {
            std::fs::remove_dir(Path::new(CGROUP_ROOT).join(SLICE))
                .map_err(|source| io("remove fixture slice", SLICE, source))?;
        }
        if created_parent && Path::new(CGROUP_ROOT).join("overdrive.slice").exists() {
            std::fs::remove_dir(Path::new(CGROUP_ROOT).join("overdrive.slice"))
                .map_err(|source| io("remove fixture slice parent", "overdrive.slice", source))?;
        }
        Ok(())
    }

    pub async fn run(args: Args) -> Result<()> {
        let substrate = source_and_host()?;
        let dir = args.out.join(args.profile.name());
        if dir.exists() {
            return Err(Error::Precondition { reason: Precondition::ExistingReceipt, path: dir });
        }
        let trace = Diagnostics::default();
        tracing::subscriber::set_global_default(Registry::default().with(trace.clone()))?;
        let before_raw = raw_snapshot()?;
        let before = normalized_snapshot(before_raw.clone());
        let prior = inspect_prior_kernel_state(&before)?;
        std::fs::create_dir_all(&dir)
            .map_err(|source| io("create profile raw directory", &dir, source))?;
        source_manifest(&dir)?;
        write_json(&dir.join("foreign-before.json"), &before)?;
        write_json(&dir.join("foreign-before-raw.json"), &before_raw)?;
        write_json(
            &dir.join("prior-shared-state.json"),
            &json!({
                "ip_program": format!("{:?}", prior.ip_program), "bridge": format!("{:?}", prior.bridge),
                "bridge_guard": prior.bridge_guard, "mark_guard": prior.mark_guard,
                "fwmark_rule": prior.fwmark_rule, "local_route": prior.local_route,
                "pin_directories": prior.pin_directories,
            }),
        )?;
        let mut raw = Raw::open(dir.join("samples.jsonl"))?;
        let method = json!({
            "receipt_id": "M-ND295-E18", "classification": "benchmark (non-EDD)",
            "profile": args.profile.name(), "N": POPULATION, "M": POPULATION * args.profile.ports().len(),
            "tcp_ports": args.profile.ports(), "owner_call_order": ["shared_guest_network", "mtls_worker", "guest_dns"],
            "samples_per_owner_and_quiesce_restore": SAMPLES,
            "sample_state": "all samples warm: full-population independent read-back precedes sample 0; later samples follow complete quiesce/restore cycles",
            "warmup": "one empty worker audit and one active attachment quiesce/restore, excluded from density samples",
            "clock": "std::time::Instant; nanoseconds; full awaited calls including scheduling/lock wait",
            "mutex_hold": "private feature-gated diagnostic: after acquisition through guard release; excludes lock wait",
            "last_tap_down": "private feature-gated diagnostic stamped after each actual successful TAP read-back; last successful timestamp emitted after the pass; independent sysfs read-back outside the timed interval",
            "kill_method": "RealCgroupFs; one live /bin/sleep per allocation; per-VM writes in AllocationId order; full repopulation before one workloads-slice write; reaping/logging outside loop timing",
            "runtime": "Tokio multi-thread, 4 runtime workers, no VMM population or service TCP listeners",
            "statistics": "raw ordered samples only; no bound, maximum, fit, R15 threshold decision, or source value is derived here",
            "source": substrate,
            "source_correlation": "canonical metal marker's editing-host commit and dirty/untracked-source digest, cross-checked with the active run lease and native-preflight; executable SHA256 and actual synced source-files.sha256 (no remote Git dependency)",
            "complement_normalization": "raw snapshots retained separately; existing execution-counter normalization plus numeric links[*].linkinfo.info_data.gc_timer only for info_kind=bridge, proven read-only countdown by Linux 7.0 semantics and native no-mutation probe; every identity/config/ownership field retained",
        });
        write_json(&dir.join("method.json"), &method)?;
        let mut fixture = Fixture::new(&dir)?;
        let measured = async {
            fixture.start(&trace).await?;
            fixture.attach(args.profile, 0).await?;
            let _warmup = fixture.quiesce(&trace).await?;
            fixture.owner.restore_quiesced_taps().await?;
            for index in 1..POPULATION {
                fixture.attach(args.profile, index).await?;
            }
            let inventory = fixture.verify_population(args.profile, POPULATION)?;
            write_json(&dir.join("population-before.json"), &inventory)?;
            write_json(&dir.join("population-kernel-before.json"), &raw_snapshot()?)?;
            fixture.samples(&trace, &mut raw).await?;
            if matches!(args.profile, Profile::E18T1Port4) {
                fixture.kills(&mut raw).await?;
            }
            let inventory = fixture.verify_population(args.profile, POPULATION)?;
            write_json(&dir.join("population-after.json"), &inventory)?;
            write_json(&dir.join("population-kernel-after.json"), &raw_snapshot()?)?;
            Ok::<(), Error>(())
        }
        .await;
        let mut cleanup = fixture.cleanup().await;
        let created_slice = fixture.created_slice;
        let created_parent = fixture.created_parent;
        let created_bpf_parent = fixture.created_bpf_parent;
        drop(fixture);
        if let Err(error) =
            cleanup_shared(&prior, created_slice, created_parent, created_bpf_parent)
        {
            cleanup.push(error);
        }
        match raw_snapshot() {
            Ok(after_raw) => {
                if let Err(error) = write_json(&dir.join("foreign-after-raw.json"), &after_raw) {
                    cleanup.push(error);
                }
                let after = normalized_snapshot(after_raw);
                if let Err(error) = write_json(&dir.join("foreign-after.json"), &after) {
                    cleanup.push(error);
                }
                if after != before {
                    cleanup.push(Error::Complement {
                        surface: "links/nft/policy-route/BPF object inventory".to_owned(),
                    });
                }
            }
            Err(error) => cleanup.push(error),
        }
        raw.record(&json!({
            "kind": "completion", "measurement_ok": measured.is_ok(), "cleanup_ok": cleanup.is_empty(),
            "measurement_error": measured.as_ref().err().map(|error| format!("{error:?}")),
            "cleanup_errors": cleanup.iter().map(|error| format!("{error:?}")).collect::<Vec<_>>(),
        }))?;
        if measured.is_err() || !cleanup.is_empty() {
            return Err(Error::Cleanup { primary: measured.err().map(Box::new), cleanup });
        }
        // A complete receipt is published only after every required real call,
        // diagnostic, populated kill and independent cleanup complement passed.
        write_json(
            &dir.join("receipt-complete.json"),
            &json!({"receipt_id": "M-ND295-E18", "profile": args.profile.name(), "raw_samples": "samples.jsonl", "method": "method.json", "source": "source-files.sha256", "complete": true}),
        )?;
        eprintln!("E18 raw receipt completed at {}", dir.display());
        Ok(())
    }
    #[cfg(test)]
    mod normalization_properties {
        use super::{json, normalized_snapshot};
        use proptest::prelude::*;

        proptest! {
            /// CONTRACT_SHAPE: pure-function.
            #[test]
            fn only_the_numeric_bridge_gc_countdown_is_omitted(
                timer in any::<u64>(), configuration in any::<u64>(), ifindex in 1_u64..u64::MAX,
            ) {
                let before = json!({"links": [{"ifindex": ifindex, "flags": ["NO-CARRIER"],
                    "linkinfo": {"info_kind": "bridge", "info_data": {"gc_timer": timer, "ageing_time": configuration}}}]});
                let mut clock_advanced = before.clone();
                clock_advanced["links"][0]["linkinfo"]["info_data"]["gc_timer"] = json!(timer.wrapping_add(1));
                prop_assert_eq!(normalized_snapshot(before.clone()), normalized_snapshot(clock_advanced.clone()));
                let mut configured = before.clone();
                configured["links"][0]["linkinfo"]["info_data"]["ageing_time"] = json!(configuration.wrapping_add(1));
                prop_assert_ne!(normalized_snapshot(before.clone()), normalized_snapshot(configured));
                let mut replaced = before.clone();
                replaced["links"][0]["ifindex"] = json!(ifindex + 1);
                prop_assert_ne!(normalized_snapshot(before.clone()), normalized_snapshot(replaced));
                let mut other_kind = before.clone();
                other_kind["links"][0]["linkinfo"]["info_kind"] = json!("veth");
                clock_advanced["links"][0]["linkinfo"]["info_kind"] = json!("veth");
                prop_assert_ne!(normalized_snapshot(other_kind), normalized_snapshot(clock_advanced));
                let mut malformed = before.clone();
                malformed["links"][0]["linkinfo"]["info_data"]["gc_timer"] = json!("not a timer");
                prop_assert_ne!(normalized_snapshot(before), normalized_snapshot(malformed));
            }
        }
    }
}
