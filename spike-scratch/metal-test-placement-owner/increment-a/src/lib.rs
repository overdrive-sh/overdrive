//! Probe library: the owner election client, pidfd and /proc helpers.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Group written into the placement record so the repo's wrapper places the
/// test as `guest-declared` (host half in a local-mode vsock namespace).
pub const GROUP: &str = "cgroup-default";

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

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub fn ns_link(kind: &str) -> String {
    fs::read_link(format!("/proc/self/ns/{kind}"))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("?{e}"))
}

pub fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `pidfd_open(pid, 0)`; `pid` is interpreted in the CALLER's PID namespace.
pub fn pidfd_open(pid: u32) -> Result<OwnedFd, i32> {
    // SAFETY: plain syscall with integer arguments.
    let r = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if r < 0 {
        return Err(errno());
    }
    // SAFETY: the kernel returned a fresh fd that we now own.
    Ok(unsafe { OwnedFd::from_raw_fd(r as RawFd) })
}

/// `pidfd_send_signal(fd, 0, NULL, 0)`: a permission and liveness probe.
pub fn pidfd_probe(fd: &OwnedFd) -> Result<(), i32> {
    // SAFETY: plain syscall; null siginfo is allowed.
    let r = unsafe {
        libc::syscall(libc::SYS_pidfd_send_signal, fd.as_raw_fd(), 0, std::ptr::null::<libc::siginfo_t>(), 0)
    };
    if r < 0 { Err(errno()) } else { Ok(()) }
}

/// Is `fd` readable within `timeout_ms` (a pidfd is readable once the task exited)?
pub fn readable(fd: RawFd, timeout_ms: i32) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut p, 1, timeout_ms) > 0 && p.revents & libc::POLLIN != 0 }
}

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub state: char,
    pub nspid: String,
}

fn parse_stat(pid: u32) -> Option<(String, char, u32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let comm = stat[open + 1..close].to_owned();
    let mut rest = stat[close + 2..].split_whitespace();
    let state = rest.next()?.chars().next()?;
    let ppid: u32 = rest.next()?.parse().ok()?;
    Some((comm, state, ppid))
}

/// NSpid chain of the calling process (host pid first). Reads `/proc/self`:
/// inside a new PID namespace `getpid()` is NOT the host pid.
pub fn self_nspid() -> String {
    nspid_path("self")
}

pub fn nspid_of(pid: u32) -> String {
    nspid_path(&pid.to_string())
}

fn nspid_path(who: &str) -> String {
    fs::read_to_string(format!("/proc/{who}/status"))
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| l.strip_prefix("NSpid:").map(|v| v.split_whitespace().collect::<Vec<_>>().join("/")))
        })
        .unwrap_or_default()
}

/// `None` once the process is gone; otherwise its `/proc/<pid>/stat` state.
pub fn proc_state(pid: u32) -> Option<char> {
    parse_stat(pid).map(|(_, s, _)| s)
}

pub fn scan_procs() -> Vec<Proc> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if let Some((comm, state, ppid)) = parse_stat(pid) {
            out.push(Proc { pid, ppid, comm, state, nspid: nspid_of(pid) });
        }
    }
    out
}

/// Every process below `root` by parent links, as seen from the host `/proc`.
pub fn descendants(root: u32) -> Vec<Proc> {
    let all = scan_procs();
    let mut found: Vec<Proc> = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for c in all.iter().filter(|c| c.ppid == p && c.pid != root) {
            if !found.iter().any(|f| f.pid == c.pid) {
                found.push(c.clone());
                frontier.push(c.pid);
            }
        }
    }
    found
}

pub fn first_nspid(pid_text: &str) -> u32 {
    pid_text.split('/').next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Where one owner lives.
#[derive(Clone, Debug)]
pub struct OwnerSpec {
    pub dir: PathBuf,
    pub key: String,
    pub anchor: u32,
    pub vm: bool,
    pub pidns: bool,
    pub cid: u32,
    pub startup_ms: u64,
}

impl OwnerSpec {
    pub fn file(&self, ext: &str) -> PathBuf {
        self.dir.join(format!("{}.{ext}", self.key))
    }
    pub fn sock(&self) -> PathBuf {
        self.file("sock")
    }
}

fn connect(sock: &Path) -> Option<UnixStream> {
    UnixStream::connect(sock).ok()
}

/// Reaches the owner of `spec`, electing one if there is none. With
/// `use_lock` the election is an exclusive `flock` (DDD-8); without it the
/// arrival just starts one when the connect is refused (the control).
/// Returns the stream and whether THIS call started an owner.
pub fn ensure_owner(spec: &OwnerSpec, bin: &Path, use_lock: bool) -> Result<(UnixStream, bool), String> {
    if let Some(s) = connect(&spec.sock()) {
        return Ok((s, false));
    }
    let lock = if use_lock {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(spec.file("lock"))
            .map_err(|e| format!("lock open: {e}"))?;
        // SAFETY: flock on an fd we own.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err("flock failed".into());
        }
        if let Some(s) = connect(&spec.sock()) {
            return Ok((s, false));
        }
        Some(f)
    } else {
        None
    };
    let _ = fs::remove_file(spec.sock());
    let log: File = OpenOptions::new().create(true).append(true).open(spec.file("owner.log")).map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(bin);
    cmd.arg(&spec.dir).arg(&spec.key).arg(spec.anchor.to_string());
    if !spec.pidns {
        cmd.arg("--no-pidns");
    }
    if spec.vm {
        cmd.arg("--vm");
    }
    cmd.arg(format!("--cid={}", spec.cid)).arg(format!("--startup-ms={}", spec.startup_ms));
    cmd.stdin(Stdio::null()).stdout(Stdio::from(log)).stderr(Stdio::from(log_err));
    // SAFETY: setsid in the forked child before exec; no allocation.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("spawning owner: {e}"))?;
    let started = Instant::now();
    let mut outer_status = None;
    while started.elapsed() < Duration::from_secs(120) {
        if let Some(s) = connect(&spec.sock()) {
            drop(lock);
            return Ok((s, true));
        }
        if outer_status.is_none() {
            if let Ok(Some(st)) = child.try_wait() {
                outer_status = Some(st);
                // pidns mode: the outer process exits right after forking the
                // owner proper; a non-zero status is a refusal to start.
                if !st.success() {
                    let log = fs::read_to_string(spec.file("owner.log")).unwrap_or_default();
                    return Err(format!("owner process exited {st}; log:\n{log}"));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !spec.pidns {
        std::mem::forget(child);
    }
    Err("owner never became connectable".into())
}

/// One request line, one reply line.
pub fn request(sock: &Path, line: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(sock).map_err(|e| format!("connect {}: {e}", sock.display()))?;
    writeln!(stream, "{line}").map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply.trim_end().to_owned())
}

#[derive(Debug, Clone)]
pub struct Hello {
    pub ns_pid: u32,
    pub host_pid: u32,
    pub netns: String,
    pub pidns: String,
    pub started_ms: u128,
}

pub fn hello(sock: &Path) -> Result<Hello, String> {
    let r = request(sock, "HELLO")?;
    let f: Vec<&str> = r.split('\t').collect();
    if f.len() != 6 || f[0] != "OWNER" {
        return Err(format!("bad HELLO reply {r:?}"));
    }
    Ok(Hello {
        ns_pid: f[1].parse().map_err(|e| format!("{e}"))?,
        host_pid: f[2].parse().map_err(|e| format!("{e}"))?,
        netns: f[3].into(),
        pidns: f[4].into(),
        started_ms: f[5].parse().map_err(|e| format!("{e}"))?,
    })
}
