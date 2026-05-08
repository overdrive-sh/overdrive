#![allow(clippy::print_stdout, clippy::print_stderr, clippy::expect_used)]
//! `cargo xdp-perf` — Tier 4 XDP throughput / mean-latency
//! regression gate.
//!
//! Walks every recorded program baseline under
//! `perf-baseline/main/xdp-perf/`, loads the compiled BPF object via
//! aya, runs each program in-kernel via `BPF_PROG_TEST_RUN` with
//! `repeat=N`, then reads `bpf_prog_info.run_time_ns` / `run_cnt`
//! (gated on
//! [`aya::sys::enable_stats`](aya::sys::enable_stats) =
//! `Stats::RunTime`, kernel ≥5.8) and feeds the records to the
//! pure
//! [`overdrive_dataplane::xdp_perf_gate::evaluate`](overdrive_dataplane::xdp_perf_gate::evaluate)
//! decision fn. Fails with structured per-breach output:
//!
//! - `>5%` pps drop vs baseline
//! - `>10%` mean-ns rise vs baseline
//! - Baseline names a program the BPF object does not contain
//!
//! # Why `BPF_PROG_TEST_RUN`, not xdp-bench / xdp-trafficgen
//!
//! libbpf 1.0+ removed the legacy `SEC("maps")` parser; aya 0.13.x
//! still emits maps in that legacy section, so libbpf-linked tools
//! refuse to load aya ELFs (aya issue #913). `xdp-trafficgen`
//! v1.4.2 (Ubuntu Noble) has a separate bug: its kernel-support
//! probe sends a 4-byte packet, but kernel commit `6b3d638ca897`
//! requires `≥ ETH_HLEN` (14 bytes) — EINVAL. Fixed in xdp-tools
//! v1.5.4+ but not in the distro package.
//!
//! `BPF_PROG_TEST_RUN` with `repeat=N` runs the XDP program N
//! times in a tight kernel loop against a synthetic packet. No
//! external tool, no veth pair, no interface attachment — just a
//! loaded program fd. `ProgramInfo::run_time()` / `run_count()`
//! accumulate identically (the `BPF_PROG_RUN()` macro is
//! unconditional). More deterministic than live-traffic
//! measurement (no kernel networking stack jitter).
//!
//! See `docs/research/dataplane/xdp-trafficgen-einval-research.md`
//! for the full root cause analysis.
//!
//! # Architecture
//!
//! Mirrors the verifier-regress sibling: a tiny `main` shim, a
//! `run()` body, and a `#[cfg(target_os = "linux")]`-gated
//! `measure()` that owns aya + syscall invocations. Pure gate
//! logic lives in
//! `crates/overdrive-dataplane/src/xdp_perf_gate.rs`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use color_eyre::eyre::{Result, WrapErr, bail};

#[allow(
    dead_code,
    reason = "gate module is binary-internal; some pub fns exist only for round-trip test coverage"
)]
mod xdp_perf_gate;
use xdp_perf_gate::{
    BaselineRecord, GateOutcome, GatePolicy, MeasuredRecord, evaluate, parse_baseline_file,
    render_failure, render_measured,
};

#[derive(Parser, Debug, Clone, Copy)]
#[command(about = "Tier 4 XDP throughput / mean-latency regression gate")]
struct Args {
    /// Number of `BPF_PROG_TEST_RUN` iterations per program
    /// measurement window. The kernel runs the XDP program this
    /// many times in a tight loop against a synthetic UDP packet;
    /// larger values reduce noise at the cost of wall-clock per
    /// program.
    #[arg(long, default_value_t = 1_000_000)]
    num_iterations: u64,
}

fn main() -> ExitCode {
    if let Err(err) = color_eyre::install() {
        eprintln!("failed to install color-eyre: {err}");
        return ExitCode::FAILURE;
    }

    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:?}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<()> {
    let workspace_root = workspace_root_dir()?;
    let baseline_dir = workspace_root.join("perf-baseline/main/xdp-perf");
    let bpf_object = workspace_root.join("target/bpf/overdrive_bpf.o");

    if !bpf_object.exists() {
        bail!("BPF object {} not found. Run `cargo xtask bpf-build` first.", bpf_object.display());
    }

    let baselines = read_baselines(&baseline_dir)?;
    if baselines.is_empty() {
        bail!(
            "no baselines found under {} — at least one `<program>.txt` file with a `prog=… pps=… mean_ns=…` line is required",
            baseline_dir.display()
        );
    }

    let candidates = measure(args, &bpf_object, &baselines)?;
    eprintln!("xdp-perf: measurements:\n{}", render_measured(&candidates));

    let policy = GatePolicy::default();
    match evaluate(&baselines, &candidates, &policy) {
        GateOutcome::Pass => {
            eprintln!("xdp-perf: pass — {} program(s) within budget", baselines.len());
            Ok(())
        }
        GateOutcome::Fail { breaches } => {
            eprint!("{}", render_failure(&breaches));
            bail!("xdp-perf gate failed: {} breach(es)", breaches.len())
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

/// Resolve the workspace root by walking up from `CARGO_MANIFEST_DIR`
/// until a `Cargo.toml` containing `[workspace]` is found. Mirrors
/// the resolution `verifier_regress.rs` performs so both binaries
/// behave identically when invoked from anywhere in the tree.
fn workspace_root_dir() -> Result<PathBuf> {
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

#[cfg(target_os = "linux")]
fn measure(
    args: Args,
    bpf_object: &std::path::Path,
    baselines: &[BaselineRecord],
) -> Result<Vec<MeasuredRecord>> {
    use std::os::fd::AsFd;
    use std::path::Path;

    use aya::{
        EbpfLoader,
        programs::Xdp,
        sys::{Stats, enable_stats},
    };
    use overdrive_dataplane::maps::ServiceKey;
    use overdrive_dataplane::maps::hash_of_maps::HashOfMapsHandle;
    use overdrive_dataplane::sys::prog_test_run::prog_test_run;

    // Pre-pin SERVICE_MAP — same shape as verifier_regress, the
    // BPF object's `pinning = ByName` outer-HoM workaround per
    // ADR-0040 / aya issue #913.
    let pin_dir = format!("/sys/fs/bpf/overdrive-xdp-perf-{}", std::process::id());
    std::fs::create_dir_all(&pin_dir).wrap_err_with(|| format!("create pin dir {pin_dir}"))?;
    let pin_path = Path::new(&pin_dir).join("SERVICE_MAP");
    let _ = std::fs::remove_file(&pin_path);

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

    let _stats_fd = enable_stats(Stats::RunTime).wrap_err(
        "enable_stats(RunTime) — kernel ≥5.8 required; ensure CAP_SYS_ADMIN (run via `cargo xtask lima run --` or `sudo -E env`)",
    )?;

    let pkt = build_synthetic_udp_frame();
    let repeat = u32::try_from(args.num_iterations.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);

    let mut records: Vec<MeasuredRecord> = Vec::with_capacity(baselines.len());
    for baseline in baselines {
        let program = bpf.program_mut(&baseline.program).ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "BPF object {} does not contain program `{}` named in baseline",
                bpf_object.display(),
                baseline.program,
            )
        })?;
        let xdp: &mut Xdp = program
            .try_into()
            .wrap_err_with(|| format!("program `{}` is not an XDP program", baseline.program))?;
        xdp.load().wrap_err_with(|| format!("xdp.load() for `{}`", baseline.program))?;

        let info_pre = xdp
            .info()
            .wrap_err_with(|| format!("xdp.info() pre-test_run for `{}`", baseline.program))?;
        let cnt_pre = info_pre.run_count();
        let time_pre = info_pre.run_time();

        let prog_fd = xdp.fd().wrap_err_with(|| format!("xdp.fd() for `{}`", baseline.program))?;

        let wall_start = std::time::Instant::now();
        prog_test_run(prog_fd.as_fd(), &pkt, repeat).wrap_err_with(|| {
            format!("BPF_PROG_TEST_RUN for `{}` ({repeat} iterations)", baseline.program)
        })?;
        let wall_duration = wall_start.elapsed();

        let info_post = xdp
            .info()
            .wrap_err_with(|| format!("xdp.info() post-test_run for `{}`", baseline.program))?;
        let cnt_delta = info_post.run_count().saturating_sub(cnt_pre);
        let time_delta = info_post.run_time().saturating_sub(time_pre);

        if cnt_delta == 0 {
            bail!(
                "program `{}` saw zero invocations after {repeat} iterations in {:?}; \
                 verify the BPF object was compiled with the correct target and the program loads successfully",
                baseline.program,
                wall_duration,
            );
        }

        let pps = cnt_delta as f64 / wall_duration.as_secs_f64();
        let mean_ns =
            u64::try_from(time_delta.as_nanos() / u128::from(cnt_delta)).unwrap_or(u64::MAX);

        records.push(MeasuredRecord { program: baseline.program.clone(), pps, mean_ns });
    }

    drop(bpf);
    drop(_service_map);
    let _ = std::fs::remove_file(&pin_path);
    let _ = std::fs::remove_dir(&pin_dir);

    Ok(records)
}

/// Minimal Ethernet + IPv4 + UDP frame for `BPF_PROG_TEST_RUN`.
///
/// The kernel requires `data_size_in >= ETH_HLEN` (14 bytes) per
/// commit `6b3d638ca897`. This frame is 42 bytes — well above the
/// minimum — and exercises the XDP program's header-parsing path
/// (EtherType check → IPv4 parse → UDP protocol match → map lookup).
#[cfg(target_os = "linux")]
fn build_synthetic_udp_frame() -> Vec<u8> {
    let mut pkt = Vec::with_capacity(42);

    // Ethernet header (14 bytes)
    pkt.extend_from_slice(&[0x00; 6]); // dst MAC
    pkt.extend_from_slice(&[0x00; 6]); // src MAC
    pkt.extend_from_slice(&[0x08, 0x00]); // EtherType: IPv4

    // IPv4 header (20 bytes)
    pkt.push(0x45); // version=4, IHL=5 (20 bytes)
    pkt.push(0x00); // DSCP/ECN
    pkt.extend_from_slice(&0x001c_u16.to_be_bytes()); // total length: 28
    pkt.extend_from_slice(&[0x00; 2]); // identification
    pkt.extend_from_slice(&[0x00; 2]); // flags + frag offset
    pkt.push(0x40); // TTL: 64
    pkt.push(0x11); // protocol: UDP
    pkt.extend_from_slice(&[0x00; 2]); // header checksum (ignored by XDP)
    pkt.extend_from_slice(&[10, 0, 0, 1]); // src IP: 10.0.0.1
    pkt.extend_from_slice(&[10, 0, 0, 2]); // dst IP: 10.0.0.2

    // UDP header (8 bytes)
    pkt.extend_from_slice(&12345_u16.to_be_bytes()); // src port
    pkt.extend_from_slice(&80_u16.to_be_bytes()); // dst port
    pkt.extend_from_slice(&8_u16.to_be_bytes()); // length: 8 (header only)
    pkt.extend_from_slice(&[0x00; 2]); // checksum

    pkt
}

#[cfg(not(target_os = "linux"))]
fn measure(
    _args: Args,
    _bpf_object: &std::path::Path,
    _baselines: &[BaselineRecord],
) -> Result<Vec<MeasuredRecord>> {
    bail!(
        "xdp-perf requires Linux (kernel ≥5.8 for BPF_ENABLE_STATS). On macOS run via \
         `cargo xtask lima run -- cargo xdp-perf`."
    )
}
