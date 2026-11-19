//! Throwaway DESIGN spike V-B2 (guest-benchmark-substrate): can a production-`overdrive-init`
//! guest return a framed sample block on its console when the host reads the console only after
//! the program's EXIT, the guest's power-off and the VMM's exit?
//!
//! Host half mirrors `guest_lane::run_in_guest_cell_attempt` (not public: Holder/read_line are
//! re-implemented here) with the operator program swapped for `init-return-guest`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use overdrive_bench::{Replicate, ReplicateFailure};
use overdrive_core::guest_transport::{GUEST_CID_BASE, GUEST_NODE_GATEWAY};
use overdrive_core::vm::beacon::BeaconMessage;
use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::{
    GuestBoot, GuestFile, GuestImageSpec, GuestInit, GuestKit, LOOPBACK_MODULE_GUEST_PATH, WorkDir,
};
use overdrive_testing::guest_lane::{
    HostListeners, LANE_LAUNCH_TOKEN, READY_BOUND, lane_workload_addr,
};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::{VsockListener, VsockStream};
use overdrive_testing_macros::overdrive_bench;

const GUEST_BIN_PATH: &str = "/opt/spike/init-return-guest";
const CELL_BOUND: Duration = Duration::from_secs(120);
const CID_FIRST_OFFSET: u32 = 0x3000;
const ACCEPT_WARM: usize = 20;
const ACCEPT_N: usize = 500;
const RT_WARM: usize = 200;
const RT_N: usize = 5000;

static KIT: OnceLock<GuestKit> = OnceLock::new();
static CID_SEQ: AtomicU32 = AtomicU32::new(0);
static REP_SEQ: AtomicU64 = AtomicU64::new(0);
static EVIDENCE: OnceLock<PathBuf> = OnceLock::new();

fn fail(msg: &str) -> ReplicateFailure {
    ReplicateFailure::fixture(msg)
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn kit(staging: &Path) -> Result<&'static GuestKit, ReplicateFailure> {
    if let Some(kit) = KIT.get() {
        return Ok(kit);
    }
    // Spike-local cwd adaptation (prior spike's finding; the design's PD-7 removes the need).
    std::env::set_current_dir(repo()).map_err(|e| fail(&format!("chdir: {e}")))?;
    let kit = GuestKit::provision(staging).map_err(|e| fail(&format!("kit provision: {e}")))?;
    Ok(KIT.get_or_init(|| kit))
}

fn next_cid() -> u32 {
    GUEST_CID_BASE + CID_FIRST_OFFSET + CID_SEQ.fetch_add(1, Ordering::Relaxed) % 256
}

fn guest_bin_host_path() -> Result<PathBuf, ReplicateFailure> {
    let exe = std::env::current_exe().map_err(|e| fail(&format!("current_exe: {e}")))?;
    let c = exe
        .parent()
        .and_then(Path::parent)
        .map(|d| d.join("init-return-guest"))
        .ok_or_else(|| fail("no target dir"))?;
    if c.is_file() { Ok(c) } else { Err(fail(&format!("init-return-guest not built at {}", c.display()))) }
}

fn evidence_dir() -> &'static PathBuf {
    EVIDENCE.get_or_init(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let d = repo()
            .join("spike-scratch/guest-bench-init-return/evidence")
            .join(format!("{ts}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    })
}

struct Holder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Holder {
    fn start(listener: VsockListener, cid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                let mut held: Vec<VsockStream> = Vec::new();
                let mut buf = [0u8; 4096];
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept_within(Duration::from_millis(50)) {
                        Ok(Some((stream, peer))) if peer.cid == cid => held.push(stream),
                        Ok(_) => {}
                        Err(_) => return,
                    }
                    held.retain(|s| !matches!(s.read_within(&mut buf, Duration::ZERO), Ok(Some(0)) | Err(_)));
                }
            }
        });
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn read_line(stream: &VsockStream, bound: Duration) -> io::Result<Option<String>> {
    let deadline = Instant::now() + bound;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.read_within(&mut byte, left)? {
            None => return Ok(None),
            Some(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Some(_) if byte[0] == b'\n' => {
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            Some(_) => line.push(byte[0]),
        }
    }
}

fn ns(d: Duration) -> f64 {
    d.as_nanos() as f64
}

fn fnv(values: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in values {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

#[derive(Default, Debug)]
struct Parsed {
    /// Some(values) iff exactly one begin..end with matching n, count and checksum.
    intact: Option<Vec<u64>>,
    why: String,
    /// Lines strictly between begin and end that are not `bench-values: <metric>` lines.
    interleaved: usize,
}

fn parse_series(console: &str, metric: &str) -> Parsed {
    let begin_p = format!("bench-series-begin: {metric} ");
    let vals_p = format!("bench-values: {metric} ");
    let end_p = format!("bench-series-end: {metric} ");
    let mut p = Parsed::default();
    let mut begins = 0;
    let mut state = 0; // 0 outside, 1 inside, 2 ended
    let (mut n_begin, mut n_end, mut sum_end) = (0usize, 0usize, 0u64);
    let mut vals: Vec<u64> = Vec::new();
    let mut bad = false;
    for line in console.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(r) = line.strip_prefix(&begin_p) {
            begins += 1;
            state = 1;
            n_begin = r.trim().parse().unwrap_or(usize::MAX);
        } else if state == 1 {
            if let Some(r) = line.strip_prefix(&vals_p) {
                for t in r.split(',') {
                    match t.trim().parse::<u64>() {
                        Ok(v) => vals.push(v),
                        Err(_) => bad = true,
                    }
                }
            } else if let Some(r) = line.strip_prefix(&end_p) {
                let mut it = r.split_whitespace();
                n_end = it.next().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
                sum_end = it.next().and_then(|s| u64::from_str_radix(s, 16).ok()).unwrap_or(0);
                state = 2;
            } else {
                p.interleaved += 1;
            }
        }
    }
    p.why = if begins != 1 {
        format!("{begins} begin lines")
    } else if state != 2 {
        "no end line (tail loss)".to_owned()
    } else if bad {
        "unparseable value token".to_owned()
    } else if n_begin != n_end || n_end != vals.len() {
        format!("count mismatch begin={n_begin} end={n_end} parsed={}", vals.len())
    } else if fnv(&vals) != sum_end {
        "checksum mismatch".to_owned()
    } else {
        String::new()
    };
    if p.why.is_empty() {
        p.intact = Some(vals);
    }
    p
}

#[overdrive_bench(
    family = "guest-init-return-lead",
    substrate = native,
    arms = ["bare", "lead-newline"],
    replicates = 30,
    metric(name = "rtt_ns", unit = ns, table = "returned", series),
    metric(name = "accept_ns", unit = ns, table = "returned", series),
    metric(name = "rtt_intact", unit = count, table = "returned"),
    metric(name = "accept_intact", unit = count, table = "returned"),
    metric(name = "both_directions_ok", unit = count, table = "returned"),
    metric(name = "interleaved_lines_in_blocks", unit = count, table = "returned"),
    metric(name = "kernel_lines_after_block", unit = count, table = "returned"),
    metric(name = "console_bytes", unit = bytes, table = "returned"),
    metric(name = "block_bytes", unit = bytes, table = "returned"),
    metric(name = "exit_status", unit = count, table = "returned"),
    metric(name = "guest_workload_ns", unit = ns, table = "cycle"),
    metric(name = "guest_print_ns", unit = ns, table = "cycle"),
    metric(name = "image_ns", unit = ns, table = "cycle"),
    metric(name = "launch_to_ready_ns", unit = ns, table = "cycle"),
    metric(name = "exec_to_exit_ns", unit = ns, table = "cycle"),
    metric(name = "exit_to_vmm_exit_ns", unit = ns, table = "cycle"),
    metric(name = "cycle_ns", unit = ns, table = "cycle"),
    smoke(none),
)]
fn guest_init_return(rep: &mut Replicate) -> Result<(), ReplicateFailure> {
    let n = REP_SEQ.fetch_add(1, Ordering::Relaxed);
    let staging = default_staging_root();
    let cycle = Instant::now();
    let kit = kit(&staging)?;
    let cid = next_cid();
    let work = WorkDir::create(&staging, "gir").map_err(|e| fail(&format!("workdir: {e}")))?;
    let spec = GuestImageSpec {
        init: GuestInit::OverdriveInit,
        forwarder_module: true,
        loopback_module: true,
        extra: vec![GuestFile {
            host: guest_bin_host_path()?,
            guest: PathBuf::from(GUEST_BIN_PATH),
            with_shared_libraries: true,
        }],
    };
    let t = Instant::now();
    let image = kit.image(work.path(), "gir", &spec).map_err(|e| fail(&format!("image: {e}")))?;
    let image_ns = ns(t.elapsed());

    let t_launch = Instant::now();
    let listeners = HostListeners::bind().map_err(|e| fail(&format!("listeners: {e}")))?;
    let token = listeners.transport_token(cid);
    let HostListeners { beacon, stream: _stream, control } = listeners;
    let _holder = Holder::start(control, cid);
    let vm = kit
        .boot(&GuestBoot {
            image: &image,
            cid,
            cmdline_extra: vec![token],
            memory_mib: 512,
            vcpus: 2,
            run_dir: &work.path().join("run"),
        })
        .map_err(|e| fail(&format!("boot: {e}")))?;
    let save = |console: &str, tag: &str| {
        let _ = std::fs::write(evidence_dir().join(format!("rep-{n:03}-{tag}.console.log")), console);
    };
    let deadline = Instant::now() + READY_BOUND;
    let conn = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match beacon.accept_within(left) {
            Ok(Some((s, peer))) if peer.cid == cid => break s,
            Ok(Some(_)) => {}
            _ => {
                save(&vm.console(), "noready");
                return Err(fail("no beacon within READY_BOUND"));
            }
        }
    };
    let ready = read_line(&conn, READY_BOUND)
        .ok()
        .flatten()
        .ok_or_else(|| {
            save(&vm.console(), "noready");
            fail("no READY")
        })?;
    if !matches!(ready.parse::<BeaconMessage>(), Ok(BeaconMessage::Ready { .. })) {
        return Err(fail(&format!("expected READY, got {ready:?}")));
    }
    let launch_to_ready_ns = ns(t_launch.elapsed());

    let workload = lane_workload_addr(cid);
    let mut argv: Vec<String> =
        [GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES, LOOPBACK_MODULE_GUEST_PATH, "--"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
    argv.extend([
        GUEST_BIN_PATH.to_owned(),
        workload.to_string(),
        GUEST_NODE_GATEWAY.to_string(),
        LANE_LAUNCH_TOKEN.to_owned(),
        ACCEPT_WARM.to_string(),
        ACCEPT_N.to_string(),
        RT_WARM.to_string(),
        RT_N.to_string(),
        if rep.arm().starts_with("lead") { "lead" } else { "bare" }.to_owned(),
    ]);
    let t_exec = Instant::now();
    (&conn)
        .write_all(format!("{}\n", BeaconMessage::Exec { argv }).as_bytes())
        .map_err(|e| fail(&format!("EXEC: {e}")))?;
    let exit_deadline = Instant::now() + CELL_BOUND;
    let exit_status = loop {
        let left = exit_deadline.saturating_duration_since(Instant::now());
        let line = read_line(&conn, left)
            .map_err(|e| {
                save(&vm.console(), "noexit");
                fail(&format!("reading EXIT: {e}"))
            })?
            .ok_or_else(|| {
                save(&vm.console(), "noexit");
                fail("no EXIT within bound")
            })?;
        match line.parse::<BeaconMessage>() {
            Ok(BeaconMessage::Listen { .. }) => {}
            Ok(BeaconMessage::Exit { status }) => break status,
            other => return Err(fail(&format!("expected LISTEN/EXIT, got {line:?} ({other:?})"))),
        }
    };
    let exec_to_exit_ns = ns(t_exec.elapsed());

    // The console is read ONLY now: after EXIT, power-off and VMM exit.
    let t_x = Instant::now();
    let exited = vm.wait_exit(Duration::from_secs(60)).map_err(|e| fail(&format!("wait_exit: {e}")))?;
    let exit_to_vmm_exit_ns = ns(t_x.elapsed());
    if exited.is_none() {
        save(&vm.console(), "novmmexit");
        return Err(fail("VMM did not exit within 60 s of EXIT"));
    }
    let console = vm.console();
    save(&console, "console");

    let rtt = parse_series(&console, "rtt_ns");
    let acc = parse_series(&console, "accept_ns");
    let both = console.contains("direction-check: app_to_vsock=ok")
        && console.contains("direction-check: vsock_to_app_echoed=ok");
    // Kernel lines after the last end line = evidence the console was drained past the block.
    let last_end = console.rfind("bench-series-end: rtt_ns ");
    let after = last_end
        .map(|i| console[i..].lines().skip(1).filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let block_bytes: usize = console
        .lines()
        .filter(|l| l.starts_with("bench-"))
        .map(|l| l.len() + 1)
        .sum();
    let guest_kernel = console
        .lines()
        .find_map(|l| l.strip_prefix("init-return-guest-kernel: "))
        .unwrap_or("?")
        .to_owned();

    let intact_r = rtt.intact.is_some();
    let intact_a = acc.intact.is_some();
    eprintln!(
        "V-B2 rep {n} arm={} cid={cid:#x} exit={exit_status} rtt_intact={intact_r}({}) accept_intact={intact_a}({}) both_dirs={both} interleaved={}+{} after_block_lines={after} console_bytes={} guest_kernel={guest_kernel} cycle_ms={}",
        rep.arm(),
        rtt.why,
        acc.why,
        rtt.interleaved,
        acc.interleaved,
        console.len(),
        cycle.elapsed().as_millis()
    );
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(evidence_dir().join("summary.tsv")) {
        let _ = writeln!(
            f,
            "{n}\t{}\t{cid:#x}\t{exit_status}\t{intact_r}\t{}\t{intact_a}\t{}\t{both}\t{}\t{}\t{after}\t{}\t{block_bytes}\t{guest_kernel}",
            rep.arm(), rtt.why, acc.why, rtt.interleaved, acc.interleaved, console.len()
        );
    }

    let tail = |p: &str| console.lines().find_map(|l| l.strip_prefix(p)).and_then(|v| v.trim().parse::<f64>().ok());
    if let Some(v) = tail("init-return-workload-ns: ") {
        rep.record("guest_workload_ns", v)?;
    }
    if let Some(v) = tail("init-return-print-ns: ") {
        rep.record("guest_print_ns", v)?;
    }
    rep.record("image_ns", image_ns)?;
    rep.record("launch_to_ready_ns", launch_to_ready_ns)?;
    rep.record("exec_to_exit_ns", exec_to_exit_ns)?;
    rep.record("exit_to_vmm_exit_ns", exit_to_vmm_exit_ns)?;
    rep.record("exit_status", f64::from(exit_status))?;
    rep.record("rtt_intact", f64::from(u8::from(intact_r)))?;
    rep.record("accept_intact", f64::from(u8::from(intact_a)))?;
    rep.record("both_directions_ok", f64::from(u8::from(both)))?;
    rep.record("interleaved_lines_in_blocks", (rtt.interleaved + acc.interleaved) as f64)?;
    rep.record("kernel_lines_after_block", after as f64)?;
    rep.record("console_bytes", console.len() as f64)?;
    rep.record("block_bytes", block_bytes as f64)?;
    if let Some(v) = &rtt.intact {
        rep.record_series("rtt_ns", &v.iter().map(|x| *x as f64).collect::<Vec<_>>())?;
    }
    if let Some(v) = &acc.intact {
        rep.record_series("accept_ns", &v.iter().map(|x| *x as f64).collect::<Vec<_>>())?;
    }
    drop(work);
    rep.record("cycle_ns", ns(cycle.elapsed()))?;
    Ok(())
}
