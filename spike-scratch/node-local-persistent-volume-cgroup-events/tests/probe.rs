//! PROBE: can `kill_scope` wait on `cgroup.events` instead of retrying `rmdir`?
//!
//! Hypothesis (H): `populated 0` in `cgroup.events` is set in the same
//! `css_set_lock` critical section that wakes `rmdir`'s dying-task wait
//! (`do_cgroup_task_dead`, `kernel/cgroup/cgroup.c:7072-7101 @ v7.0`), so (H1) it
//! never reads 0 while `rmdir` would still sleep, (H2) an `rmdir` issued after
//! `populated 0` returns at once, and (H3) a poll/inotify waiter on `cgroup.events`
//! is woken and (H4) can be bounded by a timer while a concurrent `rmdir` cannot.
//!
//! Prediction: while a device is suspended and the scope's last task is held in
//! `do_exit` (close of a block device with dirty pages -> `sync_blockdev`), the scope
//! reads `populated 1` and a concurrent `rmdir` sleeps in D state; on resume the task
//! dies, `populated 0` and the `rmdir` return within a few ms of each other; an
//! `rmdir` after `populated 0` returns `Ok` in well under 20 ms in EVERY trial; the
//! poll waiter times out on schedule while `rmdir` is stuck.
//!
//! Falsification: any trial where `populated 0` was observed and the next `rmdir`
//! blocked (> 100 ms) or failed `EBUSY`; any trial where a waiter missed
//! `populated 0` that the 200 us oracle saw; `populated 0` while the child was alive.
//!
//! Every test runs inside a fresh `cgroup-default` / `volume-pool` guest (the guest
//! has `dmsetup`, `losetup`, device-mapper and cgroup v2). Nothing touches the host.

#![allow(clippy::all, clippy::pedantic, clippy::restriction)]

use overdrive_testing::overdrive_test;
use std::ffi::CString;
use std::fs;
use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const CG: &str = "/sys/fs/cgroup";
const HOLD_MS: u64 = 1500;
const DELAYS: [u64; 4] = [0, 10, 50, 500];

fn run(prog: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(prog).args(args).output().map_err(|e| format!("spawn {prog}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
    } else {
        Err(format!("{prog} {args:?}: {:?}: {}", o.status.code(), String::from_utf8_lossy(&o.stderr)))
    }
}

/// A dm linear device over a loop-backed file; `Drop` resumes and removes it.
struct Dev {
    name: String,
    path: String,
    loopdev: String,
    img: String,
}

impl Dev {
    fn new(tag: &str) -> Dev {
        let img = format!("/tmp/cgev-{tag}.img");
        let f = fs::File::create(&img).expect("img");
        f.set_len(64 << 20).expect("size");
        drop(f);
        let loopdev = run("losetup", &["--find", "--show", &img]).expect("losetup");
        let name = format!("cgev-{tag}");
        let table = format!("0 131072 linear {loopdev} 0");
        run("dmsetup", &["create", &name, "--table", &table]).expect("dmsetup create");
        let path = format!("/dev/mapper/{name}");
        // The guest has no udev: create the node from the dm major:minor ourselves.
        let mm = run("dmsetup", &["info", "-c", "--noheadings", "--separator", ":", "-o", "major,minor", &name]).expect("dmsetup info");
        let (maj, min) = mm.trim().split_once(':').expect("major:minor");
        let (maj, min): (u32, u32) = (maj.trim().parse().unwrap(), min.trim().parse().unwrap());
        let cp = CString::new(path.clone()).unwrap();
        assert_eq!(unsafe { libc::mknod(cp.as_ptr(), libc::S_IFBLK | 0o600, libc::makedev(maj, min)) }, 0, "mknod");
        Dev { name, path, loopdev, img }
    }
    fn suspend(&self) {
        run("dmsetup", &["suspend", &self.name]).expect("dmsetup suspend");
    }
    fn resume(&self) {
        run("dmsetup", &["resume", &self.name]).expect("dmsetup resume");
    }
    fn dm_state(&self) -> String {
        run("dmsetup", &["info", "-c", "--noheadings", "-o", "name,attr,open", &self.name]).unwrap_or_else(|e| e)
    }
}

impl Drop for Dev {
    fn drop(&mut self) {
        let _ = run("dmsetup", &["resume", &self.name]);
        for _ in 0..50 {
            if run("dmsetup", &["remove", "--force", &self.name]).is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = run("losetup", &["-d", &self.loopdev]);
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(&self.img);
        eprintln!("SPIKE cleanup dev={} done", self.name);
    }
}

/// A cgroup v2 scope; `Drop` kills and removes it (best effort, bounded).
struct Scope {
    path: String,
}

impl Scope {
    fn new(name: &str) -> Scope {
        let parent = format!("{CG}/cgev.slice");
        let _ = fs::create_dir(&parent);
        let path = format!("{parent}/{name}.scope");
        fs::create_dir(&path).expect("mkdir scope");
        Scope { path }
    }
    fn events(&self) -> String {
        fs::read_to_string(format!("{}/cgroup.events", self.path)).unwrap_or_else(|e| format!("ERR {e}"))
    }
    fn populated(&self) -> Option<bool> {
        let e = fs::read_to_string(format!("{}/cgroup.events", self.path)).ok()?;
        e.lines().find_map(|l| l.strip_prefix("populated ")).map(|v| v.trim() == "1")
    }
    fn kill(&self) {
        let _ = fs::write(format!("{}/cgroup.kill", self.path), "1");
    }
    fn procs(&self) -> String {
        fs::read_to_string(format!("{}/cgroup.procs", self.path)).unwrap_or_default().replace('\n', ",")
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.kill();
        for _ in 0..40 {
            if !std::path::Path::new(&self.path).exists() {
                return;
            }
            let c = CString::new(self.path.clone()).unwrap();
            // Only attempt rmdir when the scope reads empty: an rmdir on a scope whose
            // last task is held would sleep uninterruptibly. Devices are resumed first
            // by field order in the callers, so this normally succeeds at once.
            if self.populated() == Some(false) && unsafe { libc::rmdir(c.as_ptr()) } == 0 {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        eprintln!("SPIKE LEAK scope {} still present", self.path);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// buffered write to the device after suspend, then idle: the task is held in `do_exit`
    Buffered,
    /// O_DIRECT write to the suspended device: the task is held in the write, not exiting
    Direct,
}

/// Forks a child that joins `scope`, does its I/O and idles. Only async-signal-safe libc
/// calls run in the child.
fn spawn_child(scope: &Scope, dev: &Dev, mode: Mode, offset: i64) -> (i32, i32) {
    let procs = CString::new(format!("{}/cgroup.procs", scope.path)).unwrap();
    let devp = CString::new(dev.path.clone()).unwrap();
    let mut buf: *mut libc::c_void = std::ptr::null_mut();
    unsafe {
        assert_eq!(libc::posix_memalign(&mut buf, 4096, 4096), 0);
        libc::memset(buf, 0x5a, 4096);
    }
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            let zero = b"0";
            let p = libc::open(procs.as_ptr(), libc::O_WRONLY);
            libc::write(p, zero.as_ptr().cast(), 1);
            libc::close(p);
            let flags = if mode == Mode::Direct { libc::O_RDWR | libc::O_DIRECT } else { libc::O_RDWR };
            let d = libc::open(devp.as_ptr(), flags);
            if d < 0 {
                let e = *libc::__errno_location() as u8;
                libc::write(fds[1], b"E".as_ptr().cast(), 1);
                libc::write(fds[1], &e as *const u8 as *const _, 1);
                libc::_exit(3);
            }
            if mode == Mode::Direct {
                libc::write(fds[1], b"r".as_ptr().cast(), 1); // about to block in the write
                libc::pwrite(d, buf, 4096, offset);
            } else {
                let n = libc::pwrite(d, buf, 4096, offset);
                libc::write(fds[1], if n == 4096 { b"r".as_ptr() } else { b"W".as_ptr() }.cast(), 1);
            }
            loop {
                libc::sleep(1000);
            }
        }
    }
    unsafe { libc::close(fds[1]) };
    (pid, fds[0])
}

fn proc_state(pid: i32) -> String {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat.rsplit(')').next().unwrap_or("").trim().chars().next().unwrap_or('?');
    let wchan = fs::read_to_string(format!("/proc/{pid}/wchan")).unwrap_or_else(|e| format!("ERR {e}"));
    let flags = stat.rsplit(')').next().unwrap_or("").split_whitespace().nth(6).unwrap_or("?").to_owned();
    format!("state={state} wchan={} flags={flags}", wchan.trim())
}

type Log = Arc<Mutex<Vec<(f64, String)>>>;

struct Clock {
    t0: Instant,
    log: Log,
}
impl Clock {
    fn ms(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }
    fn mark(&self, what: &str) -> f64 {
        let t = self.ms();
        self.log.lock().unwrap().push((t, what.to_owned()));
        t
    }
}

fn first(log: &Log, name: &str) -> Option<f64> {
    log.lock().unwrap().iter().find(|(_, w)| w.starts_with(name)).map(|(t, _)| *t)
}
fn all(log: &Log, name: &str) -> Vec<(f64, String)> {
    log.lock().unwrap().iter().filter(|(_, w)| w.starts_with(name)).cloned().collect()
}
fn fmt(v: Option<f64>) -> String {
    v.map_or("-".into(), |t| format!("{t:.2}"))
}

struct Outcome {
    scen: &'static str,
    trial: usize,
    resume: Option<f64>,
    oracle_pop0: Option<f64>,
    poll_pop0: Option<f64>,
    inotify_pop0: Option<f64>,
    reaped: Option<f64>,
    rmdir_thread_end: Option<f64>,
    rmdir_thread_errno: Option<i32>,
    after_pop0_dur: Option<f64>,
    after_pop0_errno: Option<i32>,
    poll_timeouts_before_pop0: usize,
    pre_resume: String,
    child_alive_at_pop0: Option<bool>,
}

/// One trial. `suspend`: hold the device for HOLD_MS after the kill. `rmdir_thread`: a
/// concurrent plain `rmdir` loop-free (one call) started 50 ms after the kill.
/// `rmdir_after_pop0`: the poll waiter calls `rmdir` immediately on `populated 0`.
fn trial(
    scen: &'static str,
    n: usize,
    dev: &Dev,
    mode: Mode,
    suspend: bool,
    rmdir_thread: Option<u64>,
    rmdir_after_pop0: bool,
) -> Outcome {
    let scope = Scope::new(&format!("{scen}-{n}"));
    let offset = (n as i64 % 1000) * 4096;
    if suspend {
        dev.suspend();
    }
    let (pid, rfd) = spawn_child(&scope, dev, mode, offset);
    let mut b = [0u8; 1];
    let got = unsafe { libc::read(rfd, b.as_mut_ptr().cast(), 1) };
    assert_eq!(got, 1);
    if b[0] != b'r' {
        let mut e = [0u8; 1];
        unsafe { libc::read(rfd, e.as_mut_ptr().cast(), 1) };
        let nodes: Vec<String> = fs::read_dir("/dev/mapper").map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        panic!("child setup failed: {} errno={} dev={} /dev/mapper={:?} dmsetup_ls={:?}", b[0] as char, e[0], dev.path, nodes, run("dmsetup", &["ls"]));
    }
    thread::sleep(Duration::from_millis(150));
    let pre_state = format!(
        "{} events=[{}] procs=[{}] dm=[{}]",
        proc_state(pid),
        scope.events().replace('\n', ";"),
        scope.procs(),
        dev.dm_state()
    );

    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let clock = Arc::new(Clock { t0: Instant::now(), log: log.clone() });
    let stop = Arc::new(AtomicBool::new(false));
    let events_path = format!("{}/cgroup.events", scope.path);
    let scope_path = scope.path.clone();
    let mut threads = Vec::new();

    // Oracle: 200 us polling reader of cgroup.events (ground truth for when it reads 0).
    {
        let (c, s, p) = (clock.clone(), stop.clone(), events_path.clone());
        threads.push(thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                if let Ok(e) = fs::read_to_string(&p) {
                    if e.contains("populated 0") {
                        c.mark("oracle_pop0");
                        return;
                    }
                } else {
                    c.mark("oracle_err");
                    return;
                }
                thread::sleep(Duration::from_micros(200));
            }
        }));
    }
    // inotify waiter: arm, check state, block (no timeout) -> raw IN_MODIFY wake behaviour.
    {
        let (c, s, p) = (clock.clone(), stop.clone(), events_path.clone());
        threads.push(thread::spawn(move || unsafe {
            let ifd = libc::inotify_init1(0);
            let cp = CString::new(p.clone()).unwrap();
            let wd = libc::inotify_add_watch(ifd, cp.as_ptr(), libc::IN_MODIFY);
            assert!(wd >= 0, "inotify_add_watch");
            loop {
                let e = match fs::read_to_string(&p) {
                    Ok(e) => e,
                    Err(_) => {
                        c.mark("inotify_gone");
                        return;
                    }
                };
                if e.contains("populated 0") {
                    c.mark("inotify_pop0");
                    return;
                }
                let mut pfd = libc::pollfd { fd: ifd, events: libc::POLLIN, revents: 0 };
                let r = libc::poll(&mut pfd, 1, 200);
                if r > 0 {
                    let mut buf = [0u8; 512];
                    libc::read(ifd, buf.as_mut_ptr().cast(), buf.len());
                    if c.log.lock().unwrap().len() < 200 {
                        c.mark("inotify_event");
                    }
                }
                if s.load(Ordering::Relaxed) {
                    return;
                }
            }
        }));
    }
    // poll(POLLPRI) waiter with a 250 ms bound: the cancellable wait the design wants.
    {
        let (c, s, p, scope_path) = (clock.clone(), stop.clone(), events_path.clone(), scope_path.clone());
        threads.push(thread::spawn(move || unsafe {
            let cp = CString::new(p.clone()).unwrap();
            let fd = libc::open(cp.as_ptr(), libc::O_RDONLY);
            let mut buf = [0u8; 128];
            loop {
                // read (re)arms the kernfs event counter, then test, then block.
                let n = libc::pread(fd, buf.as_mut_ptr().cast(), buf.len(), 0);
                if n <= 0 {
                    c.mark(&format!("poll_gone errno={}", *libc::__errno_location()));
                    return;
                }
                let text = String::from_utf8_lossy(&buf[..n.max(0) as usize]).into_owned();
                if text.contains("populated 0") {
                    c.mark("poll_pop0");
                    if rmdir_after_pop0 {
                        let sp = CString::new(scope_path.clone()).unwrap();
                        let t = c.mark("rmdir_after_pop0_start");
                        let r = libc::rmdir(sp.as_ptr());
                        let errno = if r == 0 { 0 } else { *libc::__errno_location() };
                        let e = c.mark(&format!("rmdir_after_pop0_end errno={errno}"));
                        c.mark(&format!("rmdir_after_pop0_dur {:.3}", e - t));
                    }
                    return;
                }
                let mut pfd = libc::pollfd { fd, events: libc::POLLPRI, revents: 0 };
                let r = libc::poll(&mut pfd, 1, 250);
                if r == 0 {
                    c.mark("poll_timeout");
                } else if c.log.lock().unwrap().len() < 200 {
                    c.mark("poll_wake");
                }
                if s.load(Ordering::Relaxed) {
                    return;
                }
            }
        }));
    }
    // waitpid thread.
    {
        let c = clock.clone();
        threads.push(thread::spawn(move || unsafe {
            let mut st = 0;
            libc::waitpid(pid, &mut st, 0);
            c.mark(&format!("reaped sig={}", libc::WTERMSIG(st)));
        }));
    }

    clock.mark("t_kill");
    scope.kill();
    if let Some(delay_ms) = rmdir_thread {
        let (c, sp) = (clock.clone(), scope_path.clone());
        let ev = events_path.clone();
        threads.push(thread::spawn(move || unsafe {
            thread::sleep(Duration::from_millis(delay_ms));
            let pre = format!("{} events=[{}]", proc_state(pid), fs::read_to_string(&ev).unwrap_or_else(|e| format!("ERR {e}")).replace('\n', ";"));
            let spc = CString::new(sp).unwrap();
            c.mark(&format!("rmdir_thread_start delay={delay_ms} pre: {pre}"));
            let r = libc::rmdir(spc.as_ptr());
            let errno = if r == 0 { 0 } else { *libc::__errno_location() };
            let post = format!("{} cgroup_of_child=[{}]", proc_state(pid), fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default().trim());
            c.mark(&format!("rmdir_thread_end errno={errno} post: {post}"));
        }));
    }
    let strict = !rmdir_after_pop0 && rmdir_thread.is_none();
    let mut resume_t = None;
    // sample the held state just before resume (for the D-state / wchan evidence)
    let mut held_state = String::new();
    if suspend {
        thread::sleep(Duration::from_millis(HOLD_MS - 100));
        held_state = format!(
            "{} events=[{}] procs=[{}] rmdir_thread_tasks=[{}] dm=[{}] child_cgroup=[{}]",
            proc_state(pid),
            scope.events().replace('\n', ";"),
            scope.procs(),
            held_threads(),
            dev.dm_state(),
            fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default().trim()
        );
        thread::sleep(Duration::from_millis(100));
        resume_t = Some(clock.mark("resume"));
        dev.resume();
    }
    // wait (bounded) for everything to settle
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut c_alive = None;
    while Instant::now() < deadline {
        // `poll_pop0` ends the poll waiter; the oracle ends on pop0 or, once a rmdir removed the scope, on a read error.
        if (first(&log, "oracle_pop0").is_some() || first(&log, "oracle_err").is_some())
            && (first(&log, "poll_pop0").is_some() || first(&log, "poll_gone").is_some())
            && (!strict || first(&log, "inotify_pop0").is_some())
            && first(&log, "reaped").is_some()
        {
            if rmdir_thread.is_none() || first(&log, "rmdir_thread_end").is_some() {
                break;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    if let Some(t) = first(&log, "oracle_pop0") {
        // child liveness at the moment populated 0 was first seen: reaped must not be far later
        c_alive = Some(first(&log, "reaped").map_or(true, |r| r > t + 50.0));
    }
    stop.store(true, Ordering::Relaxed);
    for t in threads {
        let _ = t.join();
    }
    // If the test's own rmdir never ran, Scope::drop removes the scope.
    let errno_of = |name: &str| {
        all(&log, name).first().and_then(|(_, w)| w.split("errno=").nth(1).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<i32>().ok()))
    };
    let after_dur = all(&log, "rmdir_after_pop0_dur").first().and_then(|(_, w)| w.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok()));
    let o = Outcome {
        scen,
        trial: n,
        resume: resume_t,
        oracle_pop0: first(&log, "oracle_pop0"),
        poll_pop0: first(&log, "poll_pop0"),
        inotify_pop0: first(&log, "inotify_pop0"),
        reaped: first(&log, "reaped"),
        rmdir_thread_end: first(&log, "rmdir_thread_end"),
        rmdir_thread_errno: errno_of("rmdir_thread_end"),
        after_pop0_dur: after_dur,
        after_pop0_errno: errno_of("rmdir_after_pop0_end"),
        poll_timeouts_before_pop0: all(&log, "poll_timeout").len(),
        pre_resume: held_state,
        child_alive_at_pop0: c_alive,
    };
    eprintln!(
        "SPIKE scen={} trial={} resume={} oracle_pop0={} poll_pop0={} inotify_pop0={} reaped={} rmdir_thread_end={}(errno={}) after_pop0_rmdir_dur={}(errno={}) poll_timeouts={} reaped_minus_pop0={} child_alive_at_pop0={:?}",
        o.scen, o.trial, fmt(o.resume), fmt(o.oracle_pop0), fmt(o.poll_pop0), fmt(o.inotify_pop0), fmt(o.reaped),
        fmt(o.rmdir_thread_end), o.rmdir_thread_errno.map_or("-".into(), |e| e.to_string()),
        fmt(o.after_pop0_dur), o.after_pop0_errno.map_or("-".into(), |e| e.to_string()), o.poll_timeouts_before_pop0,
        fmt(o.reaped.zip(o.oracle_pop0).map(|(r, p)| r - p)), o.child_alive_at_pop0
    );
    if n < 4 {
        eprintln!("SPIKE scen={scen} trial={n} PRE-KILL {pre_state}");
        eprintln!("SPIKE scen={scen} trial={n} HELD-BEFORE-RESUME {}", o.pre_resume);
        for (t, w) in log.lock().unwrap().iter() {
            eprintln!("SPIKE scen={scen} trial={n} timeline {t:9.2} ms  {w}");
        }
    }
    drop(scope);
    o
}

/// Threads of this process in D state and their wchan (the blocked `rmdir` thread).
fn held_threads() -> String {
    let mut out = Vec::new();
    if let Ok(d) = fs::read_dir("/proc/self/task") {
        for e in d.flatten() {
            let tid = e.file_name().to_string_lossy().into_owned();
            let stat = fs::read_to_string(e.path().join("stat")).unwrap_or_default();
            let st = stat.rsplit(')').next().unwrap_or("").trim().chars().next().unwrap_or('?');
            if st == 'D' {
                let w = fs::read_to_string(e.path().join("wchan")).unwrap_or_default();
                out.push(format!("tid{tid}:D:{w}"));
            }
        }
    }
    out.join(",")
}

fn leaks(dev_name: &str) {
    let cg = fs::read_dir(format!("{CG}/cgev.slice")).map(|d| d.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".scope")).count()).unwrap_or(0);
    let dm = run("dmsetup", &["ls"]).unwrap_or_default();
    let lo = run("losetup", &["-a"]).unwrap_or_default();
    eprintln!("SPIKE leakcheck({dev_name}) cgroup_scopes_left={cg} dmsetup_ls=[{}] losetup=[{}]", dm.replace('\n', ";"), lo.replace('\n', ";"));
}

fn uname() {
    eprintln!("SPIKE uname -r = {}", fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim());
    eprintln!("SPIKE /proc/version = {}", fs::read_to_string("/proc/version").unwrap_or_default().trim());
    let ks = fs::read_to_string("/proc/kallsyms").unwrap_or_default();
    for sym in ["cgroup_drain_dying", "cgroup_task_dead", "do_cgroup_task_dead", "cgroup_rmdir", "cgroup_destroy_locked", "cgroup_task_exit"] {
        let hit = ks.lines().filter(|l| l.split_whitespace().nth(2) == Some(sym) || l.split_whitespace().nth(2).is_some_and(|n| n.starts_with(&format!("{sym}."))));
        eprintln!("SPIKE kallsyms {sym}: {:?}", hit.collect::<Vec<_>>());
    }
}

fn stats(label: &str, v: &mut Vec<f64>) {
    if v.is_empty() {
        eprintln!("SPIKE stat {label}: n=0");
        return;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!("SPIKE stat {label}: n={} min={:.2} median={:.2} max={:.2}", v.len(), v[0], v[v.len() / 2], v[v.len() - 1]);
}

fn lines_ok(label: &str, ok: bool, detail: String) {
    eprintln!("SPIKE verdict {label}: {} {detail}", if ok { "HOLDS" } else { "VIOLATED" });
}

/// A: last task held in do_exit on a suspended device; a concurrent plain `rmdir` is
/// started; the poll waiter must keep timing out (bounded) while `rmdir` sleeps.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn a_held_in_exit_with_concurrent_rmdir() {
    uname();
    let dev = Dev::new("a");
    let mut res = Vec::new();
    for n in 0..20 {
        res.push(trial("A", n, &dev, Mode::Buffered, true, Some(DELAYS[n % DELAYS.len()]), false));
    }
    leaks("a");
    // On this kernel `rmdir` does not sleep: it removes the scope while the exiting task is
    // still held. Count the trials where it returned Ok before the resume (task still alive).
    let early_ok = res.iter().filter(|o| o.rmdir_thread_errno == Some(0) && o.rmdir_thread_end.zip(o.resume).is_some_and(|(e, r)| e < r)).count();
    let alive = res.iter().filter(|o| o.reaped.zip(o.resume).is_some_and(|(p, r)| p >= r)).count();
    lines_ok("A.rmdir_returned_Ok_before_resume_while_task_still_held", early_ok == res.len(), format!("{early_ok}/{}", res.len()));
    lines_ok("A.task_still_alive_after_that_rmdir_until_resume", alive == res.len(), format!("{alive}/{}", res.len()));
    let mut d_rm: Vec<f64> = res.iter().filter_map(|o| o.rmdir_thread_end.map(|e| e - DELAYS[o.trial % DELAYS.len()] as f64)).collect();
    stats("A rmdir duration (ms, minus its own start delay)", &mut d_rm);
    let mut d_reap: Vec<f64> = res.iter().filter_map(|o| Some(o.reaped? - o.resume?)).collect();
    stats("A resume->task reaped ms", &mut d_reap);
}

/// A2: same hold; no concurrent rmdir; `rmdir` issued immediately on the waiter's wake.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn a2_rmdir_immediately_after_populated0() {
    uname();
    let dev = Dev::new("a2");
    let mut res = Vec::new();
    for n in 0..40 {
        res.push(trial("A2", n, &dev, Mode::Buffered, true, None, n % 2 == 1));
    }
    leaks("a2");
    let mut dur: Vec<f64> = res.iter().filter_map(|o| o.after_pop0_dur).collect();
    stats("A2 rmdir duration right after populated 0 (ms)", &mut dur);
    let odd: Vec<&Outcome> = res.iter().filter(|o| o.trial % 2 == 1).collect();
    let bad = odd.iter().filter(|o| o.after_pop0_errno != Some(0) || o.after_pop0_dur.is_none_or(|d| d > 100.0)).count();
    lines_ok("A2.rmdir_after_populated0_never_blocks_or_EBUSY", bad == 0, format!("violations={bad}/{} (odd trials rmdir on the waiter's wake)", odd.len()));
    let even: Vec<&Outcome> = res.iter().filter(|o| o.trial % 2 == 0).collect();
    let mut d_pop: Vec<f64> = even.iter().filter_map(|o| Some(o.oracle_pop0? - o.resume?)).collect();
    stats("A2(even, no rmdir) resume->populated0 ms (oracle)", &mut d_pop);
    let mut d_poll: Vec<f64> = even.iter().filter_map(|o| Some(o.poll_pop0? - o.oracle_pop0?)).collect();
    stats("A2(even) oracle_pop0->poll_pop0 ms (poll POLLPRI waiter latency)", &mut d_poll);
    let mut d_ino: Vec<f64> = even.iter().filter_map(|o| Some(o.inotify_pop0? - o.oracle_pop0?)).collect();
    stats("A2(even) oracle_pop0->inotify_pop0 ms (inotify IN_MODIFY waiter latency)", &mut d_ino);
    let missed = even.iter().filter(|o| o.poll_pop0.is_none() || o.inotify_pop0.is_none() || o.oracle_pop0.is_none()).count();
    lines_ok("A2.no_waiter_missed_populated0", missed == 0, format!("missed={missed}/{}", even.len()));
    let early = even.iter().filter(|o| o.oracle_pop0.zip(o.resume).is_some_and(|(p, r)| p < r)).count();
    lines_ok("A2.populated0_never_before_resume_while_task_held", early == 0, format!("early={early}/{}", even.len()));
    let tmo: Vec<f64> = res.iter().map(|o| o.poll_timeouts_before_pop0 as f64).collect();
    eprintln!("SPIKE stat A2 poll 250ms timeouts while held: min={} max={}", tmo.iter().cloned().fold(f64::MAX, f64::min), tmo.iter().cloned().fold(0.0, f64::max));
    let mut d_reap: Vec<f64> = res.iter().filter_map(|o| Some(o.reaped? - o.oracle_pop0?)).collect();
    stats("A2 oracle_pop0->reaped ms", &mut d_reap);
}

/// B: the task is held in a direct write (not exiting). `rmdir` must say EBUSY; the
/// scope stays populated until the device resumes.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn b_held_in_running_write_not_exiting() {
    uname();
    let dev = Dev::new("b");
    let mut res = Vec::new();
    for n in 0..10 {
        res.push(trial("B", n, &dev, Mode::Direct, true, Some(50), false));
    }
    leaks("b");
    let ebusy = res.iter().filter(|o| o.rmdir_thread_errno == Some(libc::EBUSY)).count();
    lines_ok("B.rmdir_EBUSY_while_task_visible", ebusy == res.len(), format!("EBUSY={ebusy}/{}", res.len()));
    let held_ok = res.iter().all(|o| o.oracle_pop0.zip(o.resume).is_some_and(|(p, r)| p >= r));
    lines_ok("B.populated_stays_1_until_resume", held_ok, String::new());
    let mut d_pop: Vec<f64> = res.iter().filter_map(|o| Some(o.oracle_pop0? - o.resume?)).collect();
    stats("B resume->oracle_pop0 ms", &mut d_pop);
}

/// C: control, no held I/O: kill settles in milliseconds; waiters and rmdir agree.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
fn c_control_without_held_io() {
    uname();
    let dev = Dev::new("c");
    let mut res = Vec::new();
    for n in 0..40 {
        res.push(trial("C", n, &dev, Mode::Buffered, false, None, true));
    }
    leaks("c");
    let mut d: Vec<f64> = res.iter().filter_map(|o| o.oracle_pop0).collect();
    stats("C kill->populated0 ms", &mut d);
    let mut d_poll: Vec<f64> = res.iter().filter_map(|o| Some(o.poll_pop0? - o.oracle_pop0?)).collect();
    stats("C oracle_pop0->poll_pop0 ms", &mut d_poll);
    let mut dur: Vec<f64> = res.iter().filter_map(|o| o.after_pop0_dur).collect();
    stats("C rmdir duration after populated 0 (ms)", &mut dur);
    let bad = res.iter().filter(|o| o.after_pop0_errno != Some(0) || o.after_pop0_dur.is_none_or(|d| d > 100.0)).count();
    lines_ok("C.rmdir_after_populated0_never_blocks_or_EBUSY", bad == 0, format!("violations={bad}/{}", res.len()));
    let _ = std::io::stderr().flush();
}
