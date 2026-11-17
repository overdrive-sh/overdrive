//! V-1 probe (throwaway): descriptor-scoped BPF run statistics.
//!
//! Subcommands (all invoked by the orchestrator, `run`, via self-exec):
//!   run <out.log>          orchestrator: the whole experiment, prints evidence lines
//!   holder                 enable_stats(RunTime), print "HELD pid=<pid>", sleep forever
//!   connect <addr> <n>     join the cgroup given in $NBP_CGROUP, do n loopback connect+close
//!
//! The fixture is the pig-connect-cost / bpf-stats-overhead one: a fresh cgroup, a fresh
//! bpffs pin dir, `EbpfDataplane::new_with_pin_dir` with both production connect4
//! programs, loopback connect-and-close with SO_LINGER 0.
use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use aya::programs::loaded_programs;
use aya::sys::{enable_stats, Stats};
use overdrive_dataplane::EbpfDataplane;

const SYSCTL: &str = "/proc/sys/kernel/bpf_stats_enabled";
const TARGET: &str = "anon_inode:bpf-stats";

fn sysctl() -> String {
    std::fs::read_to_string(SYSCTL).map(|s| s.trim().to_owned()).unwrap_or_else(|e| format!("<{e}>"))
}

fn connect_n(addr: SocketAddr, n: u32) {
    for _ in 0..n {
        let stream = TcpStream::connect(addr).expect("connect");
        let linger = libc::linger { l_onoff: 1, l_linger: 0 };
        // SAFETY: live socket fd, valid pointer/len.
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
    }
}

/// The DDD-7 scan pattern: every /proc/<pid>/fd/<n> whose link target is exactly TARGET.
/// Returns (pid, fd) pairs, plus how many /proc/<pid>/fd directories were unreadable.
fn scan_holders() -> (Vec<(u32, u32)>, u32, Vec<String>) {
    let mut found = Vec::new();
    let mut unreadable = 0;
    let mut distinct_anon_targets: BTreeSet<String> = BTreeSet::new();
    for e in std::fs::read_dir("/proc").expect("/proc").flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else {
            unreadable += 1;
            continue;
        };
        for fd in fds.flatten() {
            let Ok(t) = std::fs::read_link(fd.path()) else { continue };
            let t = t.to_string_lossy().into_owned();
            if t.starts_with("anon_inode:") {
                distinct_anon_targets.insert(t.clone());
            }
            if t == TARGET {
                if let Some(n) = fd.file_name().to_str().and_then(|s| s.parse().ok()) {
                    found.push((pid, n));
                }
            }
        }
    }
    (found, unreadable, distinct_anon_targets.into_iter().collect())
}

fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program).args(args).output().expect("spawn");
    assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Veth;
impl Veth {
    fn new() -> Self {
        let _ = Command::new("ip").args(["link", "del", "nbp1h"]).output();
        run("ip", &["link", "add", "nbp1h", "type", "veth", "peer", "name", "nbp1p"]);
        run("ip", &["link", "set", "nbp1h", "up"]);
        run("ip", &["link", "set", "nbp1p", "up"]);
        Self
    }
}
impl Drop for Veth {
    fn drop(&mut self) {
        let _ = Command::new("ip").args(["link", "del", "nbp1h"]).output();
    }
}

fn tracked_ids() -> Vec<u32> {
    let mut v: Vec<u32> = loaded_programs()
        .filter_map(Result::ok)
        .filter(|p| p.name_as_str() == Some("cgroup_connect4"))
        .map(|p| p.id())
        .collect();
    v.sort_unstable();
    v
}

fn snap(ids: &[u32]) -> Vec<(u64, u128)> {
    ids.iter()
        .map(|id| {
            let i = loaded_programs().filter_map(Result::ok).find(|p| p.id() == *id).expect("prog");
            (i.run_count(), i.run_time().as_nanos())
        })
        .collect()
}

fn delta(a: &[(u64, u128)], b: &[(u64, u128)]) -> Vec<(u64, u128)> {
    a.iter().zip(b).map(|(x, y)| (y.0 - x.0, y.1 - x.1)).collect()
}

fn spawn_connect(addr: SocketAddr, n: u32, cgroup: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["connect", &addr.to_string(), &n.to_string()])
        .env("NBP_CGROUP", cgroup)
        .spawn()
        .expect("spawn connect")
}

fn phase(label: &str, ids: &[u32], addr: SocketAddr, n: u32, cgroup: &Path) -> Vec<(u64, u128)> {
    let before = snap(ids);
    let s = sysctl();
    let mut c = spawn_connect(addr, n, cgroup);
    assert!(c.wait().unwrap().success());
    let after = snap(ids);
    let d = delta(&before, &after);
    println!(
        "PHASE {label}: connects={n} sysctl_before_during={s} sysctl_after={} per-program(run_count_delta,run_time_ns_delta)={d:?}",
        sysctl()
    );
    d
}

fn wait_held(child: &mut Child) -> u32 {
    use std::io::{BufRead, BufReader};
    let mut r = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    r.read_line(&mut line).expect("holder line");
    println!("holder said: {}", line.trim());
    line.trim().strip_prefix("HELD pid=").and_then(|s| s.parse().ok()).expect("pid")
}

fn orchestrate() {
    println!("# uname -r: {}", String::from_utf8_lossy(&Command::new("uname").arg("-r").output().unwrap().stdout).trim());
    let lscpu = String::from_utf8_lossy(&Command::new("lscpu").output().unwrap().stdout).into_owned();
    for l in lscpu.lines().filter(|l| l.starts_with("Model name")) {
        println!("# {l}");
    }
    println!("# euid={}", unsafe { libc::geteuid() });
    println!("SYSCTL initial={}", sysctl());
    let (h0, unreadable, anon) = scan_holders();
    println!("SCAN initial: holders={h0:?} unreadable_proc_fd_dirs={unreadable} anon_inode_targets_seen={anon:?}");

    let pid = std::process::id();
    let cgroup = PathBuf::from(format!("/sys/fs/cgroup/nbp-v1-{pid}"));
    std::fs::create_dir(&cgroup).expect("mkdir cgroup");
    let pin = PathBuf::from(format!("/sys/fs/bpf/nbp-v1-{pid}"));
    let _ = std::fs::remove_dir_all(&pin);
    std::fs::create_dir_all(&pin).expect("pin dir");
    let _veth = Veth::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in listener.incoming() {
            drop(s);
        }
    });

    let before_progs: BTreeSet<u32> = tracked_ids().into_iter().collect();
    let dataplane = EbpfDataplane::new_with_pin_dir("nbp1h", "nbp1p", &pin, &cgroup).expect("dataplane");
    let ids: Vec<u32> = tracked_ids().into_iter().filter(|i| !before_progs.contains(i)).collect();
    println!("TRACKED connect4 program ids={ids:?} (expect 2)");
    assert_eq!(ids.len(), 2);

    // Phase 0: control, no holder.
    let d0 = phase("0-no-holder-control", &ids, addr, 1_000, &cgroup);

    // Phase 1: a holder process enables stats; the sysctl is never written.
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .arg("holder")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn holder");
    let hpid = wait_held(&mut holder);
    let (h1, unreadable1, _) = scan_holders();
    println!("SCAN during-hold: holders={h1:?} expected_holder_pid={hpid} unreadable={unreadable1} target_string={TARGET:?}");
    println!("SYSCTL during-hold={}", sysctl());
    let d1 = phase("1-holder-live", &ids, addr, 5_000, &cgroup);

    // Phase 1b (edge): a second holder; kill the first; stats must remain on (refcount).
    let mut holder2 = Command::new(std::env::current_exe().unwrap()).arg("holder").stdout(Stdio::piped()).spawn().expect("spawn holder2");
    let h2pid = wait_held(&mut holder2);
    let (h1b, _, _) = scan_holders();
    println!("SCAN two-holders: {h1b:?} (expect {hpid} and {h2pid})");
    holder.kill().expect("kill -9 holder1");
    holder.wait().unwrap();
    let (h1c, _, _) = scan_holders();
    println!("SCAN after kill holder1 (holder2 alive): {h1c:?}");
    let d1b = phase("1b-one-holder-killed-other-live", &ids, addr, 1_000, &cgroup);

    // Phase 2: kill -9 the last holder; nothing is left.
    holder2.kill().expect("kill -9 holder2"); // Child::kill is SIGKILL
    holder2.wait().unwrap();
    let (h2, unreadable2, _) = scan_holders();
    println!("SCAN after-kill: holders={h2:?} unreadable={unreadable2}");
    println!("SYSCTL after-kill={}", sysctl());
    let d2 = phase("2-after-kill-9", &ids, addr, 1_000, &cgroup);

    // Phase 3 (edge): holder exits cleanly (descriptor close on exit, not kill).
    let mut holder3 = Command::new(std::env::current_exe().unwrap()).args(["holder", "exit-after-ms", "1500"]).stdout(Stdio::piped()).spawn().expect("spawn holder3");
    let _ = wait_held(&mut holder3);
    holder3.wait().unwrap();
    let (h3, _, _) = scan_holders();
    let d3 = phase("3-after-clean-exit", &ids, addr, 1_000, &cgroup);
    println!("SCAN after-clean-exit: {h3:?}");

    println!("--- VERDICT INPUTS ---");
    let per = |d: &[(u64, u128)]| d.iter().map(|x| x.0).collect::<Vec<_>>();
    println!("run_count deltas  control={:?} held(5000)={:?} one-of-two-held={:?} after-kill={:?} after-clean-exit={:?}", per(&d0), per(&d1), per(&d1b), per(&d2), per(&d3));
    for (i, x) in d1.iter().enumerate() {
        println!("held prog#{i}: run_time_ns/run_count = {:.1} ns per invocation", x.1 as f64 / x.0 as f64);
    }
    drop(dataplane);
    std::fs::write(Path::new("/sys/fs/cgroup/cgroup.procs"), pid.to_string()).ok();
    let _ = std::fs::remove_dir(&cgroup);
    let _ = std::fs::remove_dir_all(&pin);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("holder") => {
            let _fd: OwnedFd = enable_stats(Stats::RunTime).expect("enable_stats(RunTime)");
            println!("HELD pid={}", std::process::id());
            use std::io::Write;
            std::io::stdout().flush().unwrap();
            match args.get(2).map(String::as_str) {
                Some("exit-after-ms") => {
                    std::thread::sleep(std::time::Duration::from_millis(args[3].parse().unwrap()));
                }
                _ => loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                },
            }
        }
        Some("connect") => {
            let cg = std::env::var("NBP_CGROUP").expect("NBP_CGROUP");
            std::fs::write(Path::new(&cg).join("cgroup.procs"), std::process::id().to_string()).expect("join cgroup");
            connect_n(args[2].parse().expect("addr"), args[3].parse().expect("n"));
        }
        _ => orchestrate(),
    }
}
