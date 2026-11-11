#![allow(clippy::print_stdout, clippy::print_stderr, clippy::expect_used)]
//! `cargo verifier-regress` — Tier 4 verifier-complexity regression gate.
//!
//! Walks every recorded program baseline under
//! `perf-baseline/main/verifier-budget/`, loads the recorded dataplane or
//! guest BPF object via aya, reads each program's
//! [`aya::programs::ProgramInfo::verified_instruction_count`] (kernel
//! ≥5.16), then feeds the records to the pure
//! [`overdrive_dataplane::verifier_budget::evaluate`] decision fn.
//! Fails with structured breach output (program / metric / baseline /
//! measured / threshold per breach):
//!
//! - `>5%` growth vs baseline
//! - `>10% of ceiling` proximity (within 10% of the 1M `CAP_BPF`
//!   ceiling)
//! - Baseline names a program/object pair the gate does not measure
//! - A required guest listener program is absent or exceeds 500,000 verified
//!   instructions on the guest object
//!
//! # Why aya, not veristat
//!
//! libbpf 1.0+ removed the legacy `SEC("maps")` parser; aya 0.13.x
//! still emits maps in that legacy section, so libbpf-linked tools
//! (veristat, bpftool) refuse to load aya ELFs with
//! `libbpf: elf: legacy map definitions in 'maps' section are not
//! supported by libbpf v1.0+`. The signal we want —
//! `bpf_prog_info.verified_insns` — is the same field veristat
//! surfaces as `TOTAL_INSNS`; reading it via aya's typed
//! `ProgramInfo` bypasses libbpf entirely. Tracking `aya#913` for
//! the upstream resolution path (`HashMap` PR `#1367` and
//! `HashOfMaps` PR `#1446` collectively close it once they merge
//! and ship).
//!
//! User-facing invocation: `cargo verifier-regress` via the workspace
//! cargo alias in `.cargo/config.toml`. Direct invocation:
//! `cargo run -p overdrive-dataplane --bin verifier_regress`.

use std::path::PathBuf;
use std::process::ExitCode;

use color_eyre::eyre::{Result, WrapErr, bail};

#[allow(
    dead_code,
    reason = "gate module is binary-internal; some pub fns exist only for round-trip test coverage"
)]
mod verifier_budget;
use verifier_budget::{
    BaselineRecord, GateOutcome, GatePolicy, MeasuredRecord, evaluate, parse_baseline_file,
    render_failure, render_measured,
};

const DATAPLANE_BPF_OBJECT: &str = "target/bpf/overdrive_bpf.o";
const GUEST_BPF_OBJECT: &str = "target/bpf/overdrive_guest_bpf.o";
const GUEST_PROGRAM_LIMIT: u64 = 500_000;

#[derive(Clone, Copy)]
enum GuestProgramKind {
    SockOps,
    CgroupSock,
}

#[derive(Clone, Copy)]
struct GuestProgram {
    name: &'static str,
    kind: GuestProgramKind,
}

const GUEST_LISTENER_PROGRAMS: [GuestProgram; 3] = [
    GuestProgram { name: "listen_tcp", kind: GuestProgramKind::SockOps },
    GuestProgram { name: "post_bind4", kind: GuestProgramKind::CgroupSock },
    GuestProgram { name: "post_bind6", kind: GuestProgramKind::CgroupSock },
];

fn main() -> ExitCode {
    if let Err(err) = color_eyre::install() {
        eprintln!("failed to install color-eyre: {err}");
        return ExitCode::FAILURE;
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let workspace_root = workspace_root_dir()?;
    let baseline_dir = workspace_root.join("perf-baseline/main/verifier-budget");
    let dataplane_object = workspace_root.join(DATAPLANE_BPF_OBJECT);
    let guest_object = workspace_root.join(GUEST_BPF_OBJECT);

    for object in [&dataplane_object, &guest_object] {
        if !object.exists() {
            bail!("BPF object {} not found. Run `cargo xtask bpf-build` first.", object.display());
        }
    }

    let baselines = read_baselines(&baseline_dir)?;
    if baselines.is_empty() {
        bail!(
            "no baselines found under {} — at least one `<name>.txt` file with a `file=… prog=… verified_insns=…` line is required",
            baseline_dir.display()
        );
    }

    let dataplane_baselines: Vec<_> = baselines
        .iter()
        .filter(|baseline| baseline.object == DATAPLANE_BPF_OBJECT)
        .cloned()
        .collect();
    if dataplane_baselines.is_empty() {
        bail!("no dataplane baselines for object {DATAPLANE_BPF_OBJECT}");
    }
    let unsupported_objects: Vec<_> = baselines
        .iter()
        .map(|baseline| baseline.object.as_str())
        .filter(|object| *object != DATAPLANE_BPF_OBJECT && *object != GUEST_BPF_OBJECT)
        .collect();
    if !unsupported_objects.is_empty() {
        bail!("verifier baselines name unsupported BPF objects: {unsupported_objects:?}");
    }

    let mut candidates =
        measure_dataplane(&dataplane_object, DATAPLANE_BPF_OBJECT, &dataplane_baselines)?;
    let guest_candidates = measure_guest_object(&guest_object)?;
    candidates.extend(guest_candidates.iter().cloned());
    let kernel_release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| "unknown".to_owned());
    eprintln!("verifier-regress: kernel={}", kernel_release.trim());
    eprintln!("verifier-regress: measurements:\n{}", render_measured(&candidates));

    enforce_guest_program_limit(&guest_candidates)?;
    require_guest_baselines(&baselines)?;

    let policy = GatePolicy::default();
    match evaluate(&baselines, &candidates, &policy) {
        GateOutcome::Pass => {
            eprintln!(
                "verifier-regress: pass — {} program(s) within budget on kernel {}",
                baselines.len(),
                kernel_release.trim()
            );
            Ok(())
        }
        GateOutcome::Fail { breaches } => {
            eprint!("{}", render_failure(&breaches));
            bail!("verifier-regress gate failed: {} breach(es)", breaches.len())
        }
    }
}

fn enforce_guest_program_limit(candidates: &[MeasuredRecord]) -> Result<()> {
    for expected in GUEST_LISTENER_PROGRAMS {
        let candidate = candidates.iter().find(|candidate| {
            candidate.object == GUEST_BPF_OBJECT && candidate.program == expected.name
        });
        let Some(candidate) = candidate else {
            bail!(
                "guest BPF object {GUEST_BPF_OBJECT} did not produce a verifier measurement for {}",
                expected.name
            );
        };
        if candidate.verified_insns > GUEST_PROGRAM_LIMIT {
            bail!(
                "guest program {}::{} verified_insns={} exceeds the per-program limit {}",
                candidate.object,
                candidate.program,
                candidate.verified_insns,
                GUEST_PROGRAM_LIMIT
            );
        }
    }
    Ok(())
}

fn require_guest_baselines(baselines: &[BaselineRecord]) -> Result<()> {
    let missing: Vec<_> = GUEST_LISTENER_PROGRAMS
        .iter()
        .filter(|expected| {
            !baselines.iter().any(|baseline| {
                baseline.object == GUEST_BPF_OBJECT && baseline.program == expected.name
            })
        })
        .map(|expected| expected.name)
        .collect();
    if !missing.is_empty() {
        bail!(
            "guest verifier baseline is missing {missing:?} for {GUEST_BPF_OBJECT}; record the measured counts above in perf-baseline/main/verifier-budget"
        );
    }
    Ok(())
}

fn read_baselines(baseline_dir: &std::path::Path) -> Result<Vec<BaselineRecord>> {
    let mut baselines: Vec<BaselineRecord> = Vec::new();
    let entries = std::fs::read_dir(baseline_dir)
        .wrap_err_with(|| format!("read_dir({})", baseline_dir.display()))?;
    let mut baseline_paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.wrap_err("read_dir entry")?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("txt") {
            baseline_paths.push(path);
        }
    }
    baseline_paths.sort();
    for path in &baseline_paths {
        let text =
            std::fs::read_to_string(path).wrap_err_with(|| format!("read {}", path.display()))?;
        let mut records =
            parse_baseline_file(&text).wrap_err_with(|| format!("parse {}", path.display()))?;
        baselines.append(&mut records);
    }
    Ok(baselines)
}

/// Resolve the workspace root. Walks up from `CARGO_MANIFEST_DIR`
/// until it finds a `Cargo.toml` containing `[workspace]` (or returns
/// the env var directly if invoked from the workspace root via cargo
/// alias). Mirrors the resolution xtask used to do.
fn workspace_root_dir() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR is set by cargo at build time to the bin's
    // package dir (`crates/overdrive-dataplane`). Walk up to the
    // workspace root.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest_dir);
    loop {
        let candidate = p.join("Cargo.toml");
        if candidate.exists() {
            let body = std::fs::read_to_string(&candidate)
                .wrap_err_with(|| format!("read {}", candidate.display()))?;
            if body.contains("[workspace]") {
                return Ok(p);
            }
        }
        if !p.pop() {
            bail!("workspace root not found above {manifest_dir}");
        }
    }
}

// -------------------------------------------------------------------
// Linux-only measurement path — loads the BPF object via aya and
// reads `verified_instruction_count` per program. On non-Linux build
// targets the binary still compiles (so `cargo check` on macOS
// catches type errors), but `measure` returns a structured
// "Linux-only" error.
// -------------------------------------------------------------------

fn measure_dataplane(
    bpf_object: &std::path::Path,
    object_id: &str,
    baselines: &[BaselineRecord],
) -> Result<Vec<MeasuredRecord>> {
    use std::path::Path;

    use aya::{
        EbpfLoader,
        programs::{CgroupSockAddr, Xdp},
    };
    use overdrive_dataplane::maps::ServiceKey;
    use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;

    // Per-process pin dir under bpffs. Use a tempdir-like path so a
    // crashed prior run does not collide. The pin lifecycle ends
    // when the OwnedFd in HashOfMapsHandle drops (we drop both at
    // function exit) — kernel ref-counts the bpffs entry until the
    // last fd closes.
    let pin_dir = format!("/sys/fs/bpf/overdrive-verifier-regress-{}", std::process::id());
    std::fs::create_dir_all(&pin_dir).wrap_err_with(|| format!("create pin dir {pin_dir}"))?;
    // Best-effort cleanup helper: if the pinned outer map already
    // exists from a prior run, unlink before re-pinning.
    let pin_path = Path::new(&pin_dir).join("SERVICE_MAP");
    let _ = std::fs::remove_file(&pin_path);

    // Pre-pin SERVICE_MAP so aya's loader resolves the pinned FD via
    // `pinning = ByName` instead of trying (and failing) to create
    // the HoM from the ELF alone. Mirrors the production
    // `EbpfDataplane::new` shape and the working pattern in
    // `tests/integration/atomic_swap.rs`. Capacities match the
    // kernel-side `MAX_OUTER_ENTRIES` / `INNER_TABLE_SIZE` per
    // architecture.md § 5 / § 10.
    let service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
        "SERVICE_MAP",
        4096,
        overdrive_core::dataplane::MaglevTableSize::DEFAULT.get(),
        &pin_dir,
    )
    .wrap_err("pre-pin SERVICE_MAP")?;

    let mut bpf = EbpfLoader::new()
        .map_pin_path(&pin_dir)
        .allow_unsupported_maps()
        .load_file(bpf_object)
        .wrap_err_with(|| format!("EbpfLoader.load_file({})", bpf_object.display()))?;

    let mut records: Vec<MeasuredRecord> = Vec::new();
    for baseline in baselines {
        let program = bpf.program_mut(&baseline.program).ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "BPF object {} does not contain program `{}` named in baseline",
                bpf_object.display(),
                baseline.program,
            )
        })?;
        // ADR-0053 § 1 added the cgroup_sock_addr program type; the
        // verified-insn read is identical across program types (it's
        // a `ProgramInfo::verified_instruction_count` query against
        // a loaded program). Dispatch on the program section name —
        // the only naming convention shared with the kernel-side
        // attribute macros (`#[xdp]`, `#[cgroup_sock_addr(connect4)]`).
        let insns = if baseline.program.starts_with("cgroup_") {
            let cgroup: &mut CgroupSockAddr = program.try_into().wrap_err_with(|| {
                format!("program `{}` is not a CgroupSockAddr program", baseline.program)
            })?;
            cgroup
                .load()
                .wrap_err_with(|| format!("cgroup_sock_addr.load() for `{}`", baseline.program))?;
            let info = cgroup
                .info()
                .wrap_err_with(|| format!("cgroup_sock_addr.info() for `{}`", baseline.program))?;
            info.verified_instruction_count().ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "kernel did not report verified_instruction_count for `{}`; \
                     requires kernel ≥5.16 (`bpf_prog_info.verified_insns`)",
                    baseline.program
                )
            })?
        } else {
            let xdp: &mut Xdp = program.try_into().wrap_err_with(|| {
                format!("program `{}` is not an XDP program", baseline.program)
            })?;
            xdp.load().wrap_err_with(|| format!("xdp.load() for `{}`", baseline.program))?;
            let info =
                xdp.info().wrap_err_with(|| format!("xdp.info() for `{}`", baseline.program))?;
            info.verified_instruction_count().ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "kernel did not report verified_instruction_count for `{}`; \
                     requires kernel ≥5.16 (`bpf_prog_info.verified_insns`)",
                    baseline.program
                )
            })?
        };
        records.push(MeasuredRecord {
            object: object_id.to_owned(),
            program: baseline.program.clone(),
            verified_insns: u64::from(insns),
        });
    }

    // Drop ordering matters: drop bpf (and its loaded programs)
    // before _service_map, so the kernel-side ref-count releases the
    // outer map cleanly. Rust drops fields in declaration order
    // (locals in reverse-declaration order); the pin is freed once
    // the final fd closes.
    drop(bpf);
    drop(service_map);
    let _ = std::fs::remove_file(&pin_path);
    let _ = std::fs::remove_dir(&pin_dir);

    Ok(records)
}

fn measure_guest_object(guest_object: &std::path::Path) -> Result<Vec<MeasuredRecord>> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = guest_object;
        bail!("verifier-regress requires Linux to load the guest BPF object");
    }

    #[cfg(target_os = "linux")]
    {
        use aya::{
            EbpfLoader,
            programs::{CgroupSock, SockOps},
        };
        use std::path::PathBuf;

        let pin_dir =
            PathBuf::from(format!("/sys/fs/bpf/overdrive-verifier-guest-{}", std::process::id()));
        std::fs::create_dir_all(&pin_dir)
            .wrap_err_with(|| format!("create guest verifier pin dir {}", pin_dir.display()))?;
        let active_pin = ActiveCorrelationPin::create(pin_dir.clone())?;
        let mut bpf = EbpfLoader::new()
            .map_pin_path(&pin_dir)
            .allow_unsupported_maps()
            .load_file(guest_object)
            .wrap_err_with(|| format!("EbpfLoader.load_file({})", guest_object.display()))?;

        let mut records = Vec::with_capacity(GUEST_LISTENER_PROGRAMS.len());
        for expected in GUEST_LISTENER_PROGRAMS {
            let program = bpf.program_mut(expected.name).ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "guest BPF object {} does not contain required program `{}`",
                    guest_object.display(),
                    expected.name
                )
            })?;
            let insns = match expected.kind {
                GuestProgramKind::SockOps => {
                    let sock_ops: &mut SockOps = program.try_into().wrap_err_with(|| {
                        format!("guest program `{}` is not a SockOps program", expected.name)
                    })?;
                    sock_ops
                        .load()
                        .wrap_err_with(|| format!("sock_ops.load() for `{}`", expected.name))?;
                    sock_ops
                        .info()
                        .wrap_err_with(|| format!("sock_ops.info() for `{}`", expected.name))?
                        .verified_instruction_count()
                }
                GuestProgramKind::CgroupSock => {
                    let cgroup_sock: &mut CgroupSock = program.try_into().wrap_err_with(|| {
                        format!("guest program `{}` is not a CgroupSock program", expected.name)
                    })?;
                    cgroup_sock
                        .load()
                        .wrap_err_with(|| format!("cgroup_sock.load() for `{}`", expected.name))?;
                    cgroup_sock
                        .info()
                        .wrap_err_with(|| format!("cgroup_sock.info() for `{}`", expected.name))?
                        .verified_instruction_count()
                }
            }
            .ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "kernel did not report verified_instruction_count for guest program `{}`; \
                     requires kernel ≥5.16 (`bpf_prog_info.verified_insns`)",
                    expected.name
                )
            })?;
            records.push(MeasuredRecord {
                object: GUEST_BPF_OBJECT.to_owned(),
                program: expected.name.to_owned(),
                verified_insns: u64::from(insns),
            });
        }

        drop(bpf);
        drop(active_pin);
        Ok(records)
    }
}

// The guest object contains an `SK_STORAGE` map with pinning-by-name. Aya
// 0.13.1 needs the typed map pinned before it can reuse the map while loading
// the whole object; this mirrors `overdrive-init::transport::kernel`'s
// production setup and keeps the verifier measurement on that same ELF.
#[cfg(target_os = "linux")]
struct ActiveCorrelationPin {
    directory: std::path::PathBuf,
    _active_map: std::os::fd::OwnedFd,
}

#[cfg(target_os = "linux")]
impl ActiveCorrelationPin {
    fn create(directory: std::path::PathBuf) -> Result<Self> {
        use std::os::fd::AsRawFd as _;

        let active_path = directory.join("ACTIVE_CORRELATION");
        let _ = std::fs::remove_file(&active_path);
        let _ = std::fs::remove_file(directory.join("CHILD_CORRELATION"));
        let btf = load_active_correlation_btf()?;
        let active_map = create_active_correlation_map(btf.as_raw_fd())?;
        pin_bpf_object(active_map.as_raw_fd(), &active_path)?;
        Ok(Self { directory, _active_map: active_map })
    }
}

#[cfg(target_os = "linux")]
impl Drop for ActiveCorrelationPin {
    fn drop(&mut self) {
        for name in ["ACTIVE_CORRELATION", "CHILD_CORRELATION"] {
            let _ = std::fs::remove_file(self.directory.join(name));
        }
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(target_os = "linux")]
fn load_active_correlation_btf() -> Result<std::os::fd::OwnedFd> {
    let strings = b"\0int\0active\0";
    let mut types = Vec::new();
    types.extend_from_slice(&1_u32.to_ne_bytes());
    types.extend_from_slice(&(1_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&4_u32.to_ne_bytes());
    types.extend_from_slice(&32_u32.to_ne_bytes());
    types.extend_from_slice(&5_u32.to_ne_bytes());
    types.extend_from_slice(&(4_u32 << 24).to_ne_bytes());
    types.extend_from_slice(&24_u32.to_ne_bytes());
    let mut btf = Vec::new();
    btf.extend_from_slice(&0xeb9f_u16.to_ne_bytes());
    btf.push(1);
    btf.push(0);
    btf.extend_from_slice(&24_u32.to_ne_bytes());
    btf.extend_from_slice(&0_u32.to_ne_bytes());
    let type_length = u32::try_from(types.len()).wrap_err("active map BTF type length")?;
    let string_length = u32::try_from(strings.len()).wrap_err("active map BTF string length")?;
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&type_length.to_ne_bytes());
    btf.extend_from_slice(&string_length.to_ne_bytes());
    btf.extend_from_slice(&types);
    btf.extend_from_slice(strings);

    let mut attr = [0_u8; 128];
    put_u64(&mut attr, 0, btf.as_ptr() as u64);
    put_u32(&mut attr, 16, u32::try_from(btf.len()).wrap_err("active map BTF length")?);
    bpf_fd_syscall(BPF_BTF_LOAD, &mut attr).wrap_err("BPF_BTF_LOAD for ACTIVE_CORRELATION")
}

#[cfg(target_os = "linux")]
fn create_active_correlation_map(btf_fd: i32) -> Result<std::os::fd::OwnedFd> {
    let mut attr = [0_u8; 128];
    put_u32(&mut attr, 0, BPF_MAP_TYPE_SK_STORAGE);
    put_u32(&mut attr, 4, 4);
    put_u32(&mut attr, 8, 24);
    put_u32(&mut attr, 12, 0);
    put_u32(&mut attr, 16, BPF_F_NO_PREALLOC);
    attr[28..43].copy_from_slice(b"ACTIVE_CORREL\0\0");
    put_u32(&mut attr, 48, u32::try_from(btf_fd).wrap_err("active map BTF fd")?);
    put_u32(&mut attr, 52, 1);
    put_u32(&mut attr, 56, 2);
    bpf_fd_syscall(BPF_MAP_CREATE, &mut attr).wrap_err("BPF_MAP_CREATE for ACTIVE_CORRELATION")
}

#[cfg(target_os = "linux")]
fn pin_bpf_object(fd: i32, path: &std::path::Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let path = CString::new(path.as_os_str().as_bytes()).wrap_err("ACTIVE_CORRELATION pin path")?;
    let mut attr = [0_u8; 128];
    put_u64(&mut attr, 0, path.as_ptr() as u64);
    put_u32(&mut attr, 8, u32::try_from(fd).wrap_err("ACTIVE_CORRELATION map fd")?);
    bpf_syscall(BPF_OBJ_PIN, &mut attr).wrap_err("BPF_OBJ_PIN for ACTIVE_CORRELATION")?;
    Ok(())
}

#[cfg(target_os = "linux")]
const BPF_MAP_TYPE_SK_STORAGE: u32 = 24;
#[cfg(target_os = "linux")]
const BPF_F_NO_PREALLOC: u32 = 1;
#[cfg(target_os = "linux")]
const BPF_MAP_CREATE: i32 = 0;
#[cfg(target_os = "linux")]
const BPF_OBJ_PIN: i32 = 6;
#[cfg(target_os = "linux")]
const BPF_BTF_LOAD: i32 = 18;

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "the verifier gate must pin the guest sock-storage map before aya loads the embedded guest object"
)]
fn bpf_syscall(command: i32, attr: &mut [u8]) -> std::io::Result<()> {
    let result = unsafe { libc::syscall(libc::SYS_bpf, command, attr.as_mut_ptr(), attr.len()) };
    if result < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "the verifier gate must create the kernel BTF and sock-storage descriptors required by the guest object"
)]
fn bpf_fd_syscall(command: i32, attr: &mut [u8]) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd as _;

    let result = unsafe { libc::syscall(libc::SYS_bpf, command, attr.as_mut_ptr(), attr.len()) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let fd = i32::try_from(result)
        .map_err(|_| std::io::Error::other("BPF syscall returned an invalid file descriptor"))?;
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
}

#[cfg(target_os = "linux")]
fn put_u32(attr: &mut [u8], offset: usize, value: u32) {
    attr[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

#[cfg(target_os = "linux")]
fn put_u64(attr: &mut [u8], offset: usize, value: u64) {
    attr[offset..offset + 8].copy_from_slice(&value.to_ne_bytes());
}
