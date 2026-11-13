//! Probe library: a per-module guest broker (shape A) and its client.
//!
//! The broker is a detached host process that boots ONE guest for a test
//! binary and serves `RUN <test>` requests over a unix socket, executing
//! `<binary> --exact <test> --nocapture` inside that guest through the guest
//! peer. Each nextest test process is a client of it.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Group written into the `overdrive_test_placement` record so the repo's
/// wrapper places the test as `guest-declared` (host half in a local-mode
/// vsock namespace). The probe never uses the macro's own guest hand-off.
pub const GROUP: &str = "cgroup-default";

/// Builds the `<full name>\0<group>\0` placement record.
pub const fn record<const N: usize>(full: &str, group: &str) -> [u8; N] {
    let mut out = [0_u8; N];
    let (f, g) = (full.as_bytes(), group.as_bytes());
    let mut i = 0;
    while i < f.len() {
        out[i] = f[i];
        i += 1;
    }
    let mut j = 0;
    while j < g.len() {
        out[f.len() + 1 + j] = g[j];
        j += 1;
    }
    out
}

/// Emits a placement record for the enclosing test fn and keeps it linked.
#[macro_export]
macro_rules! declare_placement {
    ($test:expr) => {
        const FULL: &str = ::core::concat!(::core::module_path!(), "::", $test);
        const LEN: usize = FULL.len() + 1 + $crate::GROUP.len() + 1;
        #[used]
        #[unsafe(link_section = "overdrive_test_placement")]
        static RECORD: [u8; LEN] = $crate::record::<LEN>(FULL, $crate::GROUP);
        ::core::hint::black_box(&RECORD);
    };
}

/// What a request returns.
#[derive(Debug)]
pub struct Reply {
    pub exit: i32,
    pub runs_served: u32,
    pub broker_pid: u32,
    pub broker_netns: String,
    pub exec_ms: u128,
    pub console: String,
}

pub fn netns_id() -> String {
    fs::read_link("/proc/self/ns/net").map(|p| p.display().to_string()).unwrap_or_else(|e| format!("?{e}"))
}

/// The in-guest body every probe test runs: names the guest it ran in.
pub fn body(name: &str) {
    let read = |p: &str| fs::read_to_string(p).map(|s| s.trim().to_owned()).unwrap_or_else(|e| format!("?{e}"));
    println!(
        "MODGUEST_BODY test={name} boot_id={} kernel={} pid={} uptime={}",
        read("/proc/sys/kernel/random/boot_id"),
        read("/proc/sys/kernel/osrelease"),
        std::process::id(),
        read("/proc/uptime"),
    );
    if name.ends_with("_slow") {
        println!("MODGUEST_BODY {name} sleeping 45s");
        std::thread::sleep(std::time::Duration::from_secs(45));
    }
    if name.ends_with("t_fail") {
        panic!("MODGUEST_BODY deliberate failure in {name}");
    }
}

fn start_time_of(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    rest.split_whitespace().nth(19).map(str::to_owned)
}

/// Nearest ancestor whose comm is `cargo-nextest`: the broker's lifetime anchor.
pub fn nextest_ancestor() -> Option<(u32, String)> {
    let mut pid = std::process::id();
    for _ in 0..32 {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let open = stat.find('(')?;
        let close = stat.rfind(')')?;
        let comm = &stat[open + 1..close];
        let ppid: u32 = stat[close + 2..].split_whitespace().nth(1)?.parse().ok()?;
        if comm.contains("nextest") {
            return Some((pid, start_time_of(pid)?));
        }
        if ppid <= 1 {
            return None;
        }
        pid = ppid;
    }
    None
}

pub fn alive(pid: u32, start: &str) -> bool {
    start_time_of(pid).is_some_and(|s| s == start)
}

fn socket_dir() -> PathBuf {
    // Short, run-scoped, shared by every test process of the run.
    let run = std::env::var("OVERDRIVE_METAL_RUN_ID").unwrap_or_else(|_| "norun".into());
    let dir = PathBuf::from("/tmp").join(format!("modguest-{run}"));
    let _ = fs::create_dir_all(&dir);
    dir
}

fn try_connect(sock: &Path) -> Option<UnixStream> {
    UnixStream::connect(sock).ok()
}

/// Runs `test` of `exe` in the module's shared guest, starting the broker if
/// this is the first arrival.
pub fn run_in_shared_guest(broker_bin: &Path, key: &str, exe: &Path, test: &str) -> Result<Reply, String> {
    let dir = socket_dir();
    let sock = dir.join(format!("{key}.sock"));
    let mut stream = match try_connect(&sock) {
        Some(s) => s,
        None => start_broker_and_connect(broker_bin, &dir, &sock, key, exe)?,
    };
    writeln!(stream, "RUN\t{test}").map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut header = String::new();
    reader.read_line(&mut header).map_err(|e| e.to_string())?;
    let f: Vec<&str> = header.trim_end().split('\t').collect();
    if f.len() != 7 || f[0] != "EXIT" {
        return Err(format!("bad broker reply {header:?}"));
    }
    let len: usize = f[6].parse().map_err(|e| format!("{e}"))?;
    let mut console = vec![0_u8; len];
    reader.read_exact(&mut console).map_err(|e| e.to_string())?;
    Ok(Reply {
        exit: f[1].parse().map_err(|e| format!("{e}"))?,
        runs_served: f[2].parse().map_err(|e| format!("{e}"))?,
        broker_pid: f[3].parse().map_err(|e| format!("{e}"))?,
        broker_netns: f[4].to_owned(),
        exec_ms: f[5].parse().map_err(|e| format!("{e}"))?,
        console: String::from_utf8_lossy(&console).into_owned(),
    })
}

fn start_broker_and_connect(
    broker_bin: &Path,
    dir: &Path,
    sock: &Path,
    key: &str,
    exe: &Path,
) -> Result<UnixStream, String> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{key}.lock")))
        .map_err(|e| e.to_string())?;
    // SAFETY: flock on an fd we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err("flock failed".into());
    }
    if let Some(s) = try_connect(sock) {
        return Ok(s); // another process started it while we waited for the lock
    }
    let _ = fs::remove_file(sock);
    let (anchor_pid, anchor_start) = nextest_ancestor().unwrap_or((0, String::new()));
    let log_path = dir.join(format!("{key}.broker.log"));
    let log = File::create(&log_path).map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(broker_bin);
    command
        .arg(sock)
        .arg(exe)
        .arg(anchor_pid.to_string())
        .arg(&anchor_start)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // SAFETY: setsid in the forked child before exec; no allocation.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| format!("spawning broker: {e}"))?;
    eprintln!("MODGUEST_HOST started broker pid={} anchor_nextest_pid={anchor_pid}", child.id());
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(240) {
        if let Some(s) = try_connect(sock) {
            return Ok(s);
        }
        if let Ok(Some(status)) = child.try_wait() {
            let log = fs::read_to_string(&log_path).unwrap_or_default();
            return Err(format!("broker exited early ({status}); log:\n{log}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("broker never became connectable".into())
}
