//! R-55 per-connect cost receipt (throwaway, report-only).
//!
//! Workload: loopback connect-and-close from a process inside a dedicated cgroup
//! that carries the dataplane's `connect4` programs. Arm A = only
//! `cgroup_connect4_service` attached (the gateway link is detached right after
//! construction); arm B = both programs attached. 30 interleaved runs per arm.
//! Per run: `ProgramInfo::run_time()/run_count()` of the new `connect4` programs
//! (bpf_stats_enabled=1) plus wall time per connect. Raw CSV on stdout.
use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use aya::programs::loaded_programs;
use overdrive_dataplane::EbpfDataplane;

const RUNS: u32 = 30;
const WARMUP: u32 = 500;
const CONNECTS: u32 = 5_000;

fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().expect("spawn");
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Veth;
impl Veth {
    fn new() -> Self {
        let _ = Command::new("ip").args(["link", "del", "pgch"]).output();
        run("ip", &["link", "add", "pgch", "type", "veth", "peer", "name", "pgcp"]);
        run("ip", &["link", "set", "pgch", "up"]);
        run("ip", &["link", "set", "pgcp", "up"]);
        Self
    }
}
impl Drop for Veth {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", "pgch"]).output();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    A,
    B,
}

struct Sample {
    arm: Arm,
    /// connect4 programs that ran during the timed loop (run_count > 0).
    attached: usize,
    wall_ns_per_connect: f64,
    // (run_count, run_time_ns) per connect4 program, ascending program id
    programs: Vec<(u64, u128)>,
}

fn program_ids() -> BTreeSet<u32> {
    loaded_programs().filter_map(Result::ok).map(|p| p.id()).collect()
}

/// Link ids of cgroup links by program id, from `bpftool link show` (plain text).
fn cgroup_links() -> Vec<(u32, u32)> {
    run("bpftool", &["link", "show"])
        .lines()
        .filter_map(|line| {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() >= 4 && t[0].ends_with(':') && t[1] == "cgroup" && t[2] == "prog" {
                Some((t[0].trim_end_matches(':').parse().ok()?, t[3].parse().ok()?))
            } else {
                None
            }
        })
        .collect()
}

fn one_run(arm: Arm, addr: SocketAddr, cgroup: &Path, pin: &Path) -> Sample {
    let before = program_ids();
    let dataplane =
        EbpfDataplane::new_with_pin_dir("pgch", "pgcp", pin, cgroup).expect("dataplane");
    let mut connect4: Vec<(u32, String)> = loaded_programs()
        .filter_map(Result::ok)
        .filter(|p| !before.contains(&p.id()))
        .filter(|p| p.name_as_str() == Some("cgroup_connect4"))
        .map(|p| (p.id(), p.name_as_str().unwrap_or("").to_owned()))
        .collect();
    connect4.sort();
    assert_eq!(connect4.len(), 2, "service + gateway programs loaded: {connect4:?}");
    let gateway_prog = connect4[1].0; // loaded after the service program: higher id
    if arm == Arm::A {
        let links = cgroup_links();
        let link = links.iter().find(|(_, prog)| *prog == gateway_prog).expect("gateway link").0;
        run("bpftool", &["link", "detach", "id", &link.to_string()]);
    }
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
                let info = loaded_programs()
                    .filter_map(Result::ok)
                    .find(|p| p.id() == *id)
                    .expect("program still loaded");
                (info.run_count(), info.run_time().as_nanos())
            })
            .collect()
    };
    let start = snap(&connect4);
    let t0 = Instant::now();
    for _ in 0..CONNECTS {
        connect();
    }
    let wall = t0.elapsed();
    let end = snap(&connect4);
    drop(dataplane);
    let programs: Vec<(u64, u128)> =
        start.iter().zip(&end).map(|(s, e)| (e.0 - s.0, e.1 - s.1)).collect();
    let attached = programs.iter().filter(|p| p.0 > 0).count();
    Sample {
        arm,
        attached,
        wall_ns_per_connect: wall.as_nanos() as f64 / f64::from(CONNECTS),
        programs,
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let stats_path = "/proc/sys/kernel/bpf_stats_enabled";
    let prior = std::fs::read_to_string(stats_path).expect("read bpf_stats_enabled");
    std::fs::write(stats_path, "1").expect("enable bpf stats");

    let cgroup = PathBuf::from(format!("/sys/fs/cgroup/pig-connect-cost-{}", std::process::id()));
    std::fs::create_dir(&cgroup).expect("mkdir cgroup");
    std::fs::write(cgroup.join("cgroup.procs"), std::process::id().to_string()).expect("enter");
    let pin = PathBuf::from(format!("/sys/fs/bpf/pig-connect-cost-{}", std::process::id()));
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

    println!("arm,run,connect4_programs_that_ran,wall_ns_per_connect,prog_idx,run_count,run_time_ns");
    let mut samples: Vec<(u32, Sample)> = Vec::new();
    for i in 0..RUNS {
        let order = if i % 2 == 0 { [Arm::A, Arm::B] } else { [Arm::B, Arm::A] };
        for arm in order {
            let s = one_run(arm, addr, &cgroup, &pin);
            let label = if arm == Arm::A { "A" } else { "B" };
            for (idx, (count, time)) in s.programs.iter().enumerate() {
                println!(
                    "{label},{i},{},{:.1},{idx},{count},{time}",
                    s.attached, s.wall_ns_per_connect
                );
            }
            samples.push((i, s));
        }
    }

    let summarize = |arm: Arm| {
        let sel: Vec<&Sample> = samples.iter().filter(|(_, s)| s.arm == arm).map(|(_, s)| s).collect();
        let wall = median(sel.iter().map(|s| s.wall_ns_per_connect).collect());
        let hook_per_connect = median(
            sel.iter()
                .map(|s| {
                    // Only programs still attached ran; detached run_count stays 0.
                    let time: u128 = s.programs.iter().map(|p| p.1).sum();
                    let count: u64 = s.programs.iter().map(|p| p.0).max().unwrap_or(1);
                    time as f64 / count as f64
                })
                .collect(),
        );
        (wall, hook_per_connect)
    };
    let (wall_a, hook_a) = summarize(Arm::A);
    let (wall_b, hook_b) = summarize(Arm::B);
    println!("# median wall ns/connect   A={wall_a:.1} B={wall_b:.1} delta={:.1}", wall_b - wall_a);
    println!("# median hook ns/connect   A={hook_a:.1} B={hook_b:.1} delta={:.1}", hook_b - hook_a);

    // restore
    std::fs::write(Path::new("/sys/fs/cgroup/cgroup.procs"), std::process::id().to_string()).ok();
    let _ = std::fs::remove_dir(&cgroup);
    let _ = std::fs::remove_dir_all(&pin);
    std::fs::write(stats_path, prior.trim()).expect("restore bpf stats");
}
