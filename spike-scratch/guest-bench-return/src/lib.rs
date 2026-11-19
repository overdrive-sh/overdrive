//! Throwaway DESIGN spike for `guest-benchmark-substrate` (O3, O6).
//!
//! Two native benchmark families whose bodies boot a guest-kit guest from inside the
//! `cargo bench` process (the benchmark form's closed command), run the prepared in-guest
//! payload `guest-sampler`, and bring its output back to the host half:
//!
//! - `guest-boot-cost` (V-A): image build, launch -> peer dialled, trivial exec, power-off ->
//!   VMM exit, and the per-exec round trip in a warm guest.
//! - `guest-bench-return` (V-B): a block of u64 timings returned over the console or vsock.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_core::guest_transport::GUEST_CID_BASE;
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestInit, GuestKit, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::{VMADDR_CID_ANY, VMADDR_PORT_ANY, VsockListener};
use overdrive_testing_macros::overdrive_bench;

const SAMPLER_GUEST_PATH: &str = "/opt/spike/guest-sampler";
const EXEC_BOUND: Duration = Duration::from_secs(120);
/// Outside every registered `cid_bands` band (the highest ends below 0x2100).
const CID_FIRST_OFFSET: u32 = 0x3000;

static KIT: OnceLock<GuestKit> = OnceLock::new();
static CID_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn fail(msg: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(msg)
}

fn kit(staging: &Path) -> Result<&'static GuestKit, ReplicateFailure> {
    if let Some(kit) = KIT.get() {
        return Ok(kit);
    }
    // Spike-local adaptation, no `crates/` change: `vm_fixture::workspace_root()` resolves the
    // workspace by `cargo locate-project --workspace` from the cwd, and `cargo bench` runs a
    // bench with cwd = the package dir (this spike's own workspace). Enter the repository root
    // so the fixture finds `overdrive-init` and the prepared artefacts the lane step staged.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(&repo).map_err(|e| fail(&format!("chdir {}: {e}", repo.display())))?;
    let kit = GuestKit::provision(staging).map_err(|e| fail(&format!("kit provision: {e}")))?;
    Ok(KIT.get_or_init(|| kit))
}

fn next_cid() -> u32 {
    let n = CID_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 256;
    GUEST_CID_BASE + CID_FIRST_OFFSET + n
}

/// The prepared in-guest payload: built by `cargo bench` beside the bench binary
/// (`target/release/guest-sampler`), or `$GUEST_SAMPLER`.
fn sampler_host_path() -> Result<PathBuf, ReplicateFailure> {
    if let Some(p) = std::env::var_os("GUEST_SAMPLER") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| fail(&format!("current_exe: {e}")))?;
    let candidate = exe
        .parent()
        .and_then(Path::parent)
        .map(|d| d.join("guest-sampler"))
        .ok_or_else(|| fail("no target dir above the bench binary"))?;
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(fail(&format!("guest-sampler not built at {}", candidate.display())))
    }
}

struct Booted {
    guest: overdrive_testing::guest_kit::ScriptedGuest,
    _work: WorkDir,
    image_ns: u64,
    boot_ns: u64,
}

fn ns(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Image build + scripted boot, timed in two parts.
fn boot_guest(staging: &Path) -> Result<Booted, ReplicateFailure> {
    let kit = kit(staging)?;
    let work = WorkDir::create(staging, "gbr").map_err(|e| fail(&format!("workdir: {e}")))?;
    let spec = GuestImageSpec {
        init: GuestInit::ScriptedPeer,
        forwarder_module: true,
        loopback_module: false,
        extra: vec![GuestFile {
            host: sampler_host_path()?,
            guest: PathBuf::from(SAMPLER_GUEST_PATH),
            with_shared_libraries: true,
        }],
    };
    let t0 = Instant::now();
    let image = kit.image(work.path(), "bench", &spec).map_err(|e| fail(&format!("image: {e}")))?;
    let image_ns = ns(t0.elapsed());
    let t1 = Instant::now();
    let guest = kit
        .boot_scripted(&image, next_cid(), &work.path().join("run"))
        .map_err(|e| fail(&format!("boot_scripted: {e}")))?;
    let boot_ns = ns(t1.elapsed());
    Ok(Booted { guest, _work: work, image_ns, boot_ns })
}

#[allow(clippy::cast_precision_loss)]
fn f(v: u64) -> f64 {
    v as f64
}

// ---------------------------------------------------------------------------------------
// V-A: guest boot cost
// ---------------------------------------------------------------------------------------

#[overdrive_bench(
    family = "guest-boot-cost",
    substrate = native,
    arms = ["fresh-guest", "fresh-guest-control"],
    replicates = 30,
    metric(name = "image_ns", unit = ns, table = "lifecycle"),
    metric(name = "boot_to_peer_ns", unit = ns, table = "lifecycle"),
    metric(name = "first_exec_ns", unit = ns, table = "lifecycle"),
    metric(name = "warm_exec_ns", unit = ns, table = "lifecycle", series),
    metric(name = "shutdown_ns", unit = ns, table = "lifecycle"),
    metric(name = "total_ns", unit = ns, table = "lifecycle"),
    smoke(none),
)]
fn guest_boot_cost(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let staging = default_staging_root();
    let total = Instant::now();
    let mut b = boot_guest(&staging)?;
    let argv = [SAMPLER_GUEST_PATH, "1", "console"];

    let t = Instant::now();
    let status =
        b.guest.peer().exec(&argv, EXEC_BOUND).map_err(|e| fail(&format!("first exec: {e}")))?;
    let first_exec = ns(t.elapsed());
    if status != 0 {
        return Err(fail(&format!("payload exit status {status}")));
    }
    let mut warm = Vec::new();
    for _ in 0..20 {
        let t = Instant::now();
        b.guest.peer().exec(&argv, EXEC_BOUND).map_err(|e| fail(&format!("warm exec: {e}")))?;
        warm.push(f(ns(t.elapsed())));
    }

    let t = Instant::now();
    b.guest.take_peer().power_off().map_err(|e| fail(&format!("power_off: {e}")))?;
    let exited = b
        .guest
        .vm
        .wait_exit(Duration::from_secs(60))
        .map_err(|e| fail(&format!("wait_exit: {e}")))?;
    if exited.is_none() {
        return Err(fail("the VMM did not exit within 60 s of power_off"));
    }
    let shutdown = ns(t.elapsed());

    rep.record("image_ns", f(b.image_ns))?;
    rep.record("boot_to_peer_ns", f(b.boot_ns))?;
    rep.record("first_exec_ns", f(first_exec))?;
    rep.record_series("warm_exec_ns", &warm)?;
    rep.record("shutdown_ns", f(shutdown))?;
    rep.record("total_ns", f(ns(total.elapsed())))?;
    Ok(())
}

// ---------------------------------------------------------------------------------------
// V-B: return path
// ---------------------------------------------------------------------------------------

#[allow(clippy::cast_possible_truncation)]
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

struct Block {
    samples: Vec<u64>,
    bytes_on_wire: usize,
    integrity_ok: bool,
}

fn parse_console(console: &str, n: usize) -> Result<Block, String> {
    let begin_tag = format!("OVDSAMPLES-BEGIN {n}");
    let begin = console.rfind(&begin_tag).ok_or("no BEGIN marker in the console")?;
    let rest = &console[begin..];
    let end = rest.find("OVDSAMPLES-END").ok_or("no END marker in the console")?;
    let body = &rest[begin_tag.len()..end];
    let trailer = rest[end..].lines().next().unwrap_or_default().trim();
    let wire = end + trailer.len() + begin_tag.len();
    let mut samples = Vec::with_capacity(n);
    for tok in body.split_ascii_whitespace() {
        samples.push(tok.parse::<u64>().map_err(|e| format!("bad token {tok:?}: {e}"))?);
    }
    let mut bytes = Vec::with_capacity(samples.len() * 8);
    for s in &samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    let want = format!("OVDSAMPLES-END {n} {:016x}", fnv(&bytes));
    Ok(Block { integrity_ok: samples.len() == n && trailer == want, samples, bytes_on_wire: wire })
}

fn read_vsock_block(listener: &VsockListener, n: usize) -> Result<Block, String> {
    let (stream, _) = listener
        .accept_within(Duration::from_secs(60))
        .map_err(|e| format!("accept: {e}"))?
        .ok_or("the guest never connected over vsock")?;
    let want = n * 8 + 8;
    let mut buf = Vec::with_capacity(want);
    let mut chunk = vec![0u8; 1 << 16];
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.read_within(&mut chunk, left).map_err(|e| format!("read: {e}"))? {
            None => return Err("vsock read timed out".into()),
            Some(0) => break,
            Some(k) => buf.extend_from_slice(&chunk[..k]),
        }
    }
    if buf.len() != want {
        return Err(format!("short vsock block: {} of {want} bytes", buf.len()));
    }
    let (data, sum) = buf.split_at(n * 8);
    let samples: Vec<u64> =
        data.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap_or([0; 8]))).collect();
    let sum = u64::from_le_bytes(sum.try_into().unwrap_or([0; 8]));
    Ok(Block { integrity_ok: sum == fnv(data), samples, bytes_on_wire: buf.len() })
}

#[overdrive_bench(
    family = "guest-bench-return",
    substrate = native,
    arms = ["console-1k", "vsock-1k", "console-100k", "vsock-100k", "console-1m", "vsock-1m"],
    replicates = 10,
    metric(name = "guest_sample_ns", unit = ns, table = "returned", series),
    metric(name = "samples_returned", unit = count, table = "returned"),
    metric(name = "bytes_on_wire", unit = bytes, table = "returned"),
    metric(name = "integrity_ok", unit = count, table = "returned"),
    metric(name = "exec_to_block_ns", unit = ns, table = "returned"),
    smoke(none),
)]
fn guest_bench_return(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let staging = default_staging_root();
    let arm = rep.arm().to_owned();
    let (mode, size) = arm.split_once('-').ok_or_else(|| fail("arm shape"))?;
    let n: usize = match size {
        "1k" => 1_000,
        "100k" => 100_000,
        "1m" => 1_000_000,
        other => return Err(fail(&format!("size {other}"))),
    };
    let mut b = boot_guest(&staging)?;
    let count = n.to_string();
    let t = Instant::now();
    let block = if mode == "console" {
        let argv = [SAMPLER_GUEST_PATH, count.as_str(), "console"];
        let status =
            b.guest.peer().exec(&argv, EXEC_BOUND).map_err(|e| fail(&format!("exec: {e}")))?;
        if status != 0 {
            return Err(fail(&format!("payload exit status {status}")));
        }
        parse_console(&b.guest.vm.console(), n).map_err(|e| fail(&e))?
    } else {
        let listener = VsockListener::bind(VMADDR_CID_ANY, VMADDR_PORT_ANY)
            .map_err(|e| fail(&format!("host vsock bind: {e}")))?;
        let port = listener.port().to_string();
        // The guest process blocks in `write` once the vsock buffer fills, and `exec` returns
        // only when it exits: the host must read concurrently or the block never completes.
        let reader = std::thread::spawn(move || read_vsock_block(&listener, n));
        let argv = [SAMPLER_GUEST_PATH, count.as_str(), "vsock", port.as_str()];
        let status =
            b.guest.peer().exec(&argv, EXEC_BOUND).map_err(|e| fail(&format!("exec: {e}")))?;
        if status != 0 {
            return Err(fail(&format!("payload exit status {status}")));
        }
        reader
            .join()
            .map_err(|_| fail("vsock reader thread panicked"))?
            .map_err(|e| fail(&e))?
    };
    let elapsed = ns(t.elapsed());

    let _ = b.guest.take_peer().power_off();
    let _ = b.guest.vm.wait_exit(Duration::from_secs(60));

    rep.record("exec_to_block_ns", f(elapsed))?;
    rep.record("samples_returned", block.samples.len() as f64)?;
    rep.record("bytes_on_wire", block.bytes_on_wire as f64)?;
    rep.record("integrity_ok", if block.integrity_ok { 1.0 } else { 0.0 })?;
    if !block.integrity_ok {
        return Err(fail("integrity check failed (count or checksum mismatch)"));
    }
    if n > 100_000 {
        return Ok(()); // the 1M block's integrity and size are the evidence; skip the series
    }
    let values: Vec<f64> = block.samples.iter().map(|s| *s as f64).collect();
    rep.record_series("guest_sample_ns", &values)?;
    Ok(())
}
