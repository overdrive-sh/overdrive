//! kernel.bpf_stats_enabled overhead probe (throwaway).
//!
//! Three arms, every one with BOTH production connect4 programs attached
//! (fresh `EbpfDataplane` + fresh cgroup per run); only the sysctl varies:
//!   OFF1 (bpf_stats_enabled=0), OFF2 (=0, the A/A control), ON (=1).
//! Blocks of one run per arm, arm order shuffled per block (seeded).
//! Per run: warm-up, N timed loopback connect-and-close (per-connect ns ->
//! median/mean), then a high-repeat BPF_PROG_TEST_RUN of the production
//! `xdp_service_map_lookup` (kernel-reported ns/run, miss path -> XDP_PASS).
//! Usage: bpf-stats-overhead <seed> <blocks> <csv-out>
use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsFd, AsRawFd};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use aya::programs::loaded_programs;
use overdrive_dataplane::sys::prog_test_run::prog_test_run;
use overdrive_dataplane::EbpfDataplane;

const WARMUP: u32 = 500;
const CONNECTS: u32 = 5_000;
const XDP_REPEAT: u32 = 1_000_000;
const STATS: &str = "/proc/sys/kernel/bpf_stats_enabled";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arm {
    Off1,
    Off2,
    On,
}
impl Arm {
    fn label(self) -> &'static str {
        match self {
            Arm::Off1 => "OFF1",
            Arm::Off2 => "OFF2",
            Arm::On => "ON",
        }
    }
    fn sysctl(self) -> &'static str {
        if self == Arm::On { "1" } else { "0" }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Restores the sysctl's original value on drop (also on panic unwind).
struct SysctlGuard(String);
impl Drop for SysctlGuard {
    fn drop(&mut self) {
        let _ = std::fs::write(STATS, self.0.trim());
    }
}

fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().expect("spawn");
    assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Veth;
impl Veth {
    fn new() -> Self {
        let _ = Command::new("ip").args(["link", "del", "bsoh"]).output();
        run("ip", &["link", "add", "bsoh", "type", "veth", "peer", "name", "bsop"]);
        run("ip", &["link", "set", "bsoh", "up"]);
        run("ip", &["link", "set", "bsop", "up"]);
        Self
    }
}
impl Drop for Veth {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", "bsoh"]).output();
    }
}

struct Sample {
    arm: Arm,
    sysctl_readback: String,
    wall_median_ns: f64,
    wall_mean_ns: f64,
    xdp_ns_per_run: u32,
    xdp_retval: u32,
    /// (run_count, run_time_ns) delta over timed loop + xdp test-run, per connect4 prog + xdp.
    stats_delta: Vec<(String, u64, u128)>,
}

fn program_ids() -> BTreeSet<u32> {
    loaded_programs().filter_map(Result::ok).map(|p| p.id()).collect()
}

/// Minimal Ethernet+IPv4+TCP SYN to a VIP that is not in SERVICE_MAP (miss -> XDP_PASS).
fn packet() -> Vec<u8> {
    let mut p = vec![0u8; 64];
    p[0..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x01]);
    p[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
    p[12..14].copy_from_slice(&[0x08, 0x00]);
    p[14] = 0x45;
    p[16..18].copy_from_slice(&(50u16).to_be_bytes());
    p[22] = 64;
    p[23] = 6;
    p[26..30].copy_from_slice(&[10, 250, 1, 1]);
    p[30..34].copy_from_slice(&[10, 96, 0, 1]);
    // ip checksum
    let mut sum: u32 = 0;
    for i in (14..34).step_by(2) {
        sum += u32::from(u16::from_be_bytes([p[i], p[i + 1]]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    p[24..26].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    p[34..36].copy_from_slice(&40000u16.to_be_bytes());
    p[36..38].copy_from_slice(&80u16.to_be_bytes());
    p[46] = 0x50;
    p[47] = 0x02;
    p
}

fn find_prog(id: u32) -> aya::programs::ProgramInfo {
    loaded_programs().filter_map(Result::ok).find(|p| p.id() == id).expect("program loaded")
}

fn one_run(arm: Arm, addr: SocketAddr, cgroup: &Path, pin: &Path) -> Sample {
    std::fs::write(STATS, arm.sysctl()).expect("set sysctl");
    let sysctl_readback = std::fs::read_to_string(STATS).unwrap().trim().to_owned();
    let before = program_ids();
    let dataplane =
        EbpfDataplane::new_with_pin_dir("bsoh", "bsop", pin, cgroup).expect("dataplane");
    let mut tracked: Vec<(u32, String)> = loaded_programs()
        .filter_map(Result::ok)
        .filter(|p| !before.contains(&p.id()))
        .filter(|p| matches!(p.name_as_str(), Some("cgroup_connect4") | Some("xdp_service_map")))
        .map(|p| (p.id(), p.name_as_str().unwrap_or("").to_owned()))
        .collect();
    tracked.sort();
    let c4 = tracked.iter().filter(|t| t.1 == "cgroup_connect4").count();
    assert_eq!(c4, 2, "both connect4 programs attached: {tracked:?}");
    let xdp_id = tracked.iter().find(|t| t.1.starts_with("xdp_service")).expect("xdp prog").0;

    let connect = || {
        let stream = TcpStream::connect(addr).expect("connect");
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: live socket fd, valid pointer/len for the call.
        unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                std::ptr::from_ref(&linger).cast(),
                std::mem::size_of::<libc::linger>() as libc::socklen_t,
            );
        }
        drop(stream);
    };
    for _ in 0..WARMUP {
        connect();
    }
    let snap = |ids: &[(u32, String)]| -> Vec<(u64, u128)> {
        ids.iter()
            .map(|(id, _)| {
                let i = find_prog(*id);
                (i.run_count(), i.run_time().as_nanos())
            })
            .collect()
    };
    let s0 = snap(&tracked);
    let mut times: Vec<u32> = Vec::with_capacity(CONNECTS as usize);
    for _ in 0..CONNECTS {
        let t = Instant::now();
        connect();
        times.push(t.elapsed().as_nanos() as u32);
    }
    // per-program cost: kernel-reported ns/run, miss path.
    let fd = find_prog(xdp_id).fd().expect("prog fd");
    let pkt = packet();
    let _warm = prog_test_run(fd.as_fd(), &pkt, 100_000).expect("test_run warm");
    let out = prog_test_run(fd.as_fd(), &pkt, XDP_REPEAT).expect("test_run");
    let s1 = snap(&tracked);
    drop(dataplane);
    let stats_delta: Vec<(String, u64, u128)> =
        tracked.iter().zip(s0.iter().zip(&s1)).map(|(t, (a, b))| (t.1.clone(), b.0 - a.0, b.1 - a.1)).collect();
    let mean = times.iter().map(|&t| f64::from(t)).sum::<f64>() / times.len() as f64;
    times.sort_unstable();
    Sample {
        arm,
        sysctl_readback,
        wall_median_ns: f64::from(times[times.len() / 2]),
        wall_mean_ns: mean,
        xdp_ns_per_run: out.duration_ns,
        xdp_retval: out.retval,
        stats_delta,
    }
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}
fn quantile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pos = q * (s.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    s[lo] + (s[hi] - s[lo]) * (pos - lo as f64)
}
/// Percentile bootstrap 95% CI for median(a) - median(b); resample runs with replacement.
fn boot_diff(a: &[f64], b: &[f64], rng: &mut Rng) -> (f64, f64, f64) {
    let point = median(a) - median(b);
    let mut d = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let ra: Vec<f64> = (0..a.len()).map(|_| a[rng.below(a.len())]).collect();
        let rb: Vec<f64> = (0..b.len()).map(|_| b[rng.below(b.len())]).collect();
        d.push(median(&ra) - median(&rb));
    }
    (point, quantile(&d, 0.025), quantile(&d, 0.975))
}

fn read(p: &str) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_else(|e| format!("<{e}>"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = args.get(1).map_or(1, |s| s.parse().expect("seed"));
    let blocks: u32 = args.get(2).map_or(30, |s| s.parse().expect("blocks"));
    let csv_path = args.get(3).cloned().unwrap_or_else(|| "runs.csv".into());
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);

    let prior = std::fs::read_to_string(STATS).expect("read bpf_stats_enabled");
    let _guard = SysctlGuard(prior.clone());
    println!("# seed={seed} blocks={blocks} connects/run={CONNECTS} warmup={WARMUP} xdp_repeat={XDP_REPEAT}");
    println!("# uname -r: {}", String::from_utf8_lossy(&Command::new("uname").arg("-r").output().unwrap().stdout).trim());
    println!("# sysctl bpf_stats_enabled prior value: {}", prior.trim());
    println!("# cpu governor cpu0: {}", read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"));
    println!("# intel_pstate no_turbo: {}", read("/sys/devices/system/cpu/intel_pstate/no_turbo"));
    println!("# cpufreq boost: {}", read("/sys/devices/system/cpu/cpufreq/boost"));
    println!("# smt active: {}", read("/sys/devices/system/cpu/smt/active"));
    println!("# cpu0 cur_freq kHz: {}", read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"));
    let lscpu = String::from_utf8_lossy(&Command::new("lscpu").output().unwrap().stdout).into_owned();
    for l in lscpu.lines().filter(|l| l.starts_with("Model name") || l.starts_with("CPU(s):") || l.starts_with("Thread(s)")) {
        println!("# {l}");
    }

    let cgroup = PathBuf::from(format!("/sys/fs/cgroup/bpf-stats-overhead-{}", std::process::id()));
    std::fs::create_dir(&cgroup).expect("mkdir cgroup");
    std::fs::write(cgroup.join("cgroup.procs"), std::process::id().to_string()).expect("enter");
    let pin = PathBuf::from(format!("/sys/fs/bpf/bpf-stats-overhead-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&pin);
    std::fs::create_dir_all(&pin).expect("pin dir");
    let _veth = Veth::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for s in listener.incoming() {
            drop(s);
        }
    });

    let mut csv = String::from("block,pos,arm,sysctl_readback,wall_median_ns,wall_mean_ns,xdp_ns_per_run,xdp_retval,connect4_run_count_delta,connect4_run_time_ns_delta,xdp_run_count_delta,xdp_run_time_ns_delta\n");
    let mut samples: Vec<Sample> = Vec::new();
    for b in 0..blocks {
        let mut order = [Arm::Off1, Arm::Off2, Arm::On];
        for i in (1..3).rev() {
            order.swap(i, rng.below(i + 1));
        }
        for (pos, arm) in order.into_iter().enumerate() {
            let s = one_run(arm, addr, &cgroup, &pin);
            // tracked sorted by id: connect4 x2 (service, gateway) then xdp (loaded later? sum robustly by name order)
            let (c4c, c4t): (u64, u128) = s.stats_delta.iter().filter(|p| p.0 == "cgroup_connect4").fold((0, 0), |a, p| (a.0 + p.1, a.1 + p.2));
            let (xc, xt): (u64, u128) = s.stats_delta.iter().filter(|p| p.0.starts_with("xdp_service")).fold((0, 0), |a, p| (a.0 + p.1, a.1 + p.2));
            let row = format!(
                "{b},{pos},{},{},{:.1},{:.1},{},{},{c4c},{c4t},{xc},{xt}",
                arm.label(), s.sysctl_readback, s.wall_median_ns, s.wall_mean_ns, s.xdp_ns_per_run, s.xdp_retval
            );
            println!("{row}");
            csv.push_str(&row);
            csv.push('\n');
            samples.push(s);
        }
    }
    std::fs::write(&csv_path, &csv).expect("write csv");

    let col = |arm: Arm, f: &dyn Fn(&Sample) -> f64| -> Vec<f64> {
        samples.iter().filter(|s| s.arm == arm).map(f).collect()
    };
    for (name, f) in [
        ("wall_median_ns/connect", (&|s: &Sample| s.wall_median_ns) as &dyn Fn(&Sample) -> f64),
        ("wall_mean_ns/connect", &|s: &Sample| s.wall_mean_ns),
        ("xdp_test_run_ns/run", &|s: &Sample| f64::from(s.xdp_ns_per_run)),
    ] {
        println!("# --- {name} ---");
        for arm in [Arm::Off1, Arm::Off2, Arm::On] {
            let v = col(arm, f);
            println!(
                "# {:5} n={} median-of-runs={:.2} IQR=[{:.2},{:.2}] ({:.2})",
                arm.label(), v.len(), median(&v), quantile(&v, 0.25), quantile(&v, 0.75),
                quantile(&v, 0.75) - quantile(&v, 0.25)
            );
        }
        let (on, o1, o2) = (col(Arm::On, f), col(Arm::Off1, f), col(Arm::Off2, f));
        let (p, lo, hi) = boot_diff(&on, &o1, &mut rng);
        let base = median(&o1);
        println!("# ON-OFF1  diff={p:.2} CI95=[{lo:.2},{hi:.2}]  ({:.2}% [{:.2}%,{:.2}%])", 100.0 * p / base, 100.0 * lo / base, 100.0 * hi / base);
        let (p, lo, hi) = boot_diff(&o2, &o1, &mut rng);
        println!("# OFF2-OFF1 (A/A) diff={p:.2} CI95=[{lo:.2},{hi:.2}]  ({:.2}% [{:.2}%,{:.2}%])", 100.0 * p / base, 100.0 * lo / base, 100.0 * hi / base);
    }
    let lie: u128 = samples.iter().filter(|s| s.arm != Arm::On).map(|s| s.stats_delta.iter().map(|p| p.2).sum::<u128>()).sum();
    let bad_readback = samples.iter().filter(|s| s.sysctl_readback != s.arm.sysctl()).count();
    println!("# OFF-arm run_time_ns growth total (must be 0 if stats truly off): {lie}");
    println!("# runs where sysctl readback != requested: {bad_readback}");

    std::fs::write(Path::new("/sys/fs/cgroup/cgroup.procs"), std::process::id().to_string()).ok();
    let _ = std::fs::remove_dir(&cgroup);
    let _ = std::fs::remove_dir_all(&pin);
}
