#![allow(clippy::print_stdout, clippy::print_stderr, clippy::expect_used)]
//! `cargo verifier-regress` — Tier 4 verifier-complexity regression gate.
//!
//! Walks every recorded program baseline under
//! `perf-baseline/main/verifier-budget/`, loads the compiled BPF
//! object via aya, reads each program's
//! [`aya::programs::ProgramInfo::verified_instruction_count`] (kernel
//! ≥5.16), and feeds the records to the pure
//! [`overdrive_dataplane::verifier_budget::evaluate`] decision fn.
//! Fails with structured breach output (program / metric / baseline /
//! measured / threshold per breach):
//!
//! - `>5%` growth vs baseline
//! - `>10% of ceiling` proximity (within 10% of the 1M `CAP_BPF`
//!   ceiling)
//! - Baseline names a program the BPF object does not contain
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

use overdrive_dataplane::verifier_budget::{
    BaselineRecord, GateOutcome, GatePolicy, MeasuredRecord, evaluate, parse_baseline_file,
    render_failure, render_measured,
};

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
    let bpf_object = workspace_root.join("target/bpf/overdrive_bpf.o");

    if !bpf_object.exists() {
        bail!("BPF object {} not found. Run `cargo xtask bpf-build` first.", bpf_object.display());
    }

    let baselines = read_baselines(&baseline_dir)?;
    if baselines.is_empty() {
        bail!(
            "no baselines found under {} — at least one `<name>.txt` file with a `prog=… verified_insns=…` line is required",
            baseline_dir.display()
        );
    }

    let candidates = measure(&bpf_object, &baselines)?;
    eprintln!("verifier-regress: measurements:\n{}", render_measured(&candidates));

    let policy = GatePolicy::default();
    match evaluate(&baselines, &candidates, &policy) {
        GateOutcome::Pass => {
            eprintln!("verifier-regress: pass — {} program(s) within budget", baselines.len());
            Ok(())
        }
        GateOutcome::Fail { breaches } => {
            eprint!("{}", render_failure(&breaches));
            bail!("verifier-regress gate failed: {} breach(es)", breaches.len())
        }
    }
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

#[cfg(target_os = "linux")]
fn measure(
    bpf_object: &std::path::Path,
    baselines: &[BaselineRecord],
) -> Result<Vec<MeasuredRecord>> {
    use std::path::Path;

    use aya::{EbpfLoader, programs::Xdp};
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
    let _service_map = HashOfMapsHandle::<ServiceKey, u32>::new_pinned_with_array_inner(
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
        // All baselined programs are XDP today. If/when TC programs
        // ship a baseline, extend with a SchedClassifier branch (or
        // dispatch on the ProgramSection) — the verified-insn read
        // is identical across program types.
        let xdp: &mut Xdp = program
            .try_into()
            .wrap_err_with(|| format!("program `{}` is not an XDP program", baseline.program))?;
        xdp.load().wrap_err_with(|| format!("xdp.load() for `{}`", baseline.program))?;
        let info = xdp.info().wrap_err_with(|| format!("xdp.info() for `{}`", baseline.program))?;
        let insns = info.verified_instruction_count().ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "kernel did not report verified_instruction_count for `{}`; \
                 requires kernel ≥5.16 (`bpf_prog_info.verified_insns`)",
                baseline.program
            )
        })?;
        records.push(MeasuredRecord {
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
    drop(_service_map);
    let _ = std::fs::remove_file(&pin_path);
    let _ = std::fs::remove_dir(&pin_dir);

    Ok(records)
}

#[cfg(not(target_os = "linux"))]
fn measure(
    _bpf_object: &std::path::Path,
    _baselines: &[BaselineRecord],
) -> Result<Vec<MeasuredRecord>> {
    bail!(
        "verifier-regress requires Linux (kernel ≥5.16) to read \
         `bpf_prog_info.verified_insns`. On macOS run via \
         `cargo xtask lima run -- cargo verifier-regress`."
    )
}
