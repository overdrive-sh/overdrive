//! Tier-3 acceptance: `VmHostState::kill_scope` never removes a populated scope
//! (RB-3, ADR-0349, ADR-0351; feature delta § "A scope kill's settle wait is
//! bounded", Tests).
//!
//! # Scenario family
//!
//! The scope of a VM whose last task is held in exit (a flush parked on a
//! suspended device, in `exit_task_work`, before the task leaves its cgroup)
//! reads `populated 1` for as long as the device and the claim are held. The
//! running kernel's `rmdir` of such a scope returns `Ok` in about a millisecond
//! with the task alive and holding the device (spike
//! `findings-cgroup-events.md`), so a `kill_scope` that retries `rmdir` reports
//! "scope absent" for a VM that is not gone, and a lease released on that
//! report can hand the held CID to a new launch. The contract:
//!
//! * with the scope's last task held in exit, `kill_scope` returns
//!   `Err(ResourceBusy)` at its bound, the scope directory is present for the
//!   whole hold and the task is alive;
//! * after the drain it returns `Ok` and the scope is absent;
//! * a scope that empties during the wait returns `Ok` without waiting out the
//!   bound;
//! * a scope that another remover deletes during the wait is absent, so
//!   `kill_scope` returns `Ok`. The removal does not wake a waiter on
//!   `cgroup.events` (`kernfs_drain_open_files`, `fs/kernfs/file.c:811 @ v7.0`,
//!   notifies no poller), and a later read of the removed file fails with
//!   `ENODEV` (`fs/kernfs/file.c:174,260 @ v7.0`); either is the scope's
//!   absence, not a failure.
//!
//! # Owner path, fault stimulus, oracle
//!
//! The production `RealVmHostState::kill_scope` over the real cgroup v2
//! filesystem. The fault enters at the kernel device only: a `dm-linear` device
//! over a loop file is suspended with a buffered write dirty, and the task that
//! wrote it is killed, so its final close blocks in `sync_blockdev`. No
//! production seam is used or invented. Oracle: the return value's kind, the
//! scope directory, `/proc/<pid>` and `cgroup.events`.
//!
//! # Tier and placement
//!
//! Tier 3, a `cgroup-default` / `volume-pool` guest (it has `dmsetup`,
//! `losetup`, device-mapper and cgroup v2; no udev, so the device node is made
//! here). Example-only at this layer (Mandate 9).

#![allow(
    clippy::doc_markdown,
    unsafe_code,
    reason = "the fixture forks a task that joins the scope and holds a device (the spike's discipline)"
)]

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use overdrive_core::AllocationId;
use overdrive_core::cgroup::CgroupPath;
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_host::RealVmHostState;
use overdrive_testing::overdrive_test;
use serial_test::serial;

/// The cgroup root every scope of this file resolves under. The scope of
/// allocation `a` is `<ROOT>/overdrive.slice/workloads.slice/<a>.scope`.
const ROOT: &str = "/sys/fs/cgroup/ksk-test";

fn run(program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("spawn {program}: {error}"));
    assert!(
        out.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A `dm-linear` device over a loop file. `Drop` resumes and removes it.
struct Device {
    name: String,
    path: String,
    backing: String,
    image: String,
}

impl Device {
    fn new(tag: &str) -> Self {
        let image = format!("/tmp/ksk-{tag}.img");
        let file = std::fs::File::create(&image).expect("create the image");
        file.set_len(64 << 20).expect("size the image");
        drop(file);
        let backing = run("losetup", &["--find", "--show", &image]);
        let name = format!("ksk-{tag}");
        run("dmsetup", &["create", &name, "--table", &format!("0 131072 linear {backing} 0")]);
        let path = format!("/dev/mapper/{name}");
        let numbers = run(
            "dmsetup",
            &["info", "-c", "--noheadings", "--separator", ":", "-o", "major,minor", &name],
        );
        let (major, minor) = numbers.trim().split_once(':').expect("major:minor");
        let major: u32 = major.trim().parse().expect("major");
        let minor: u32 = minor.trim().parse().expect("minor");
        let c_path = CString::new(path.clone()).expect("path");
        // The guest has no udev: make the node from the device's own numbers.
        std::fs::create_dir_all("/dev/mapper").expect("mapper dir");
        // SAFETY: `mknod` is called with a valid NUL-terminated path.
        let made = unsafe {
            libc::mknod(c_path.as_ptr(), libc::S_IFBLK | 0o600, libc::makedev(major, minor))
        };
        assert_eq!(made, 0, "mknod of the device node");
        Self { name, path, backing, image }
    }

    fn suspend(&self) {
        run("dmsetup", &["suspend", &self.name]);
    }

    fn resume(&self) {
        run("dmsetup", &["resume", &self.name]);
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _resumed = Command::new("dmsetup").args(["resume", &self.name]).status();
        for _ in 0..50 {
            let removed = Command::new("dmsetup").args(["remove", "--force", &self.name]).status();
            if removed.is_ok_and(|status| status.success()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _detached = Command::new("losetup").args(["-d", &self.backing]).status();
        let _node = std::fs::remove_file(&self.path);
        let _image = std::fs::remove_file(&self.image);
    }
}

fn alloc(name: &str) -> AllocationId {
    AllocationId::new(name).expect("valid allocation id")
}

/// The scope directory `kill_scope` resolves for `alloc`, created with its parents.
fn make_scope(alloc: &AllocationId) -> PathBuf {
    let path = CgroupPath::for_alloc(alloc).resolve(Path::new(ROOT));
    std::fs::create_dir_all(&path).expect("create the scope");
    path
}

fn populated(scope: &Path) -> bool {
    std::fs::read_to_string(scope.join("cgroup.events"))
        .ok()
        .and_then(|events| {
            events.lines().find_map(|line| line.strip_prefix("populated ").map(|v| v.trim() == "1"))
        })
        .unwrap_or(false)
}

fn state_of(pid: i32) -> char {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| stat.rsplit(')').next().and_then(|rest| rest.trim().chars().next()))
        .unwrap_or('?')
}

/// Fork a child that joins `scope`, opens `device` and writes one buffered
/// block, reports on the returned pipe, then idles. Only async-signal-safe
/// calls run in the child.
fn spawn_writer(scope: &Path, device: &Device) -> (i32, i32) {
    let procs = CString::new(format!("{}/cgroup.procs", scope.display())).expect("procs");
    let node = CString::new(device.path.clone()).expect("node");
    let mut buf: *mut libc::c_void = std::ptr::null_mut();
    // SAFETY: `posix_memalign` and `memset` on a fresh allocation of the stated size.
    unsafe {
        assert_eq!(libc::posix_memalign(&raw mut buf, 4096, 4096), 0);
        libc::memset(buf, 0x5a, 4096);
    }
    let mut pipe = [0_i32; 2];
    // SAFETY: `pipe` writes two descriptors into the array.
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    // SAFETY: the child below calls only async-signal-safe functions.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork");
    if pid == 0 {
        // SAFETY: child process; async-signal-safe calls only, ending in a loop.
        unsafe {
            let zero = b"0";
            let procs_fd = libc::open(procs.as_ptr(), libc::O_WRONLY);
            libc::write(procs_fd, zero.as_ptr().cast(), 1);
            libc::close(procs_fd);
            let fd = libc::open(node.as_ptr(), libc::O_RDWR);
            if fd < 0 {
                libc::write(pipe[1], b"E".as_ptr().cast(), 1);
                libc::_exit(3);
            }
            let wrote = libc::pwrite(fd, buf, 4096, 0);
            let mark: &[u8] = if wrote == 4096 { b"r" } else { b"W" };
            libc::write(pipe[1], mark.as_ptr().cast(), 1);
            loop {
                libc::sleep(1000);
            }
        }
    }
    // SAFETY: closing the parent's copy of the write end.
    unsafe { libc::close(pipe[1]) };
    (pid, pipe[0])
}

fn wait_ready(read_end: i32) {
    let mut byte = [0_u8; 1];
    // SAFETY: reading one byte into a one-byte buffer.
    let n = unsafe { libc::read(read_end, byte.as_mut_ptr().cast(), 1) };
    assert_eq!((n, byte[0]), (1, b'r'), "the child joined the scope and wrote its block");
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
}

fn host() -> RealVmHostState {
    RealVmHostState::new(
        PathBuf::from(ROOT),
        PathBuf::from(ROOT).join("run"),
        PathBuf::from(ROOT).join("index"),
    )
}

/// Wait up to `max` for `condition`.
fn eventually(max: Duration, condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

/// RB-3, ADR-0349. `kill_scope` never removes a populated scope: with
/// the scope's last task held in exit by a suspended device, it returns
/// `ResourceBusy` at its bound with the directory present and the task alive;
/// after the drain it returns `Ok` and the scope is gone.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the scope directory, the task,
/// the return value.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[serial(cgroup)]
fn kill_scope_never_removes_a_scope_whose_last_task_is_held_in_exit() {
    let device = Device::new("held");
    let id = alloc("ksk-held-0");
    let scope = make_scope(&id);
    // The device is suspended before the task writes: `dmsetup suspend` flushes dirty pages first,
    // so a block dirtied afterwards is what the task's final close has to write back, and cannot.
    device.suspend();
    let (pid, ready) = spawn_writer(&scope, &device);
    wait_ready(ready);
    // SAFETY: SIGKILL to the child this test forked.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert!(
        eventually(Duration::from_secs(5), || state_of(pid) == 'D' && populated(&scope)),
        "precondition: the killed task is held in exit (state D) and its scope reads populated 1"
    );
    let rt = runtime();
    let started = Instant::now();

    let result = rt.block_on(host().kill_scope(&CgroupPath::for_alloc(&id)));

    let error = result
        .expect_err("kill_scope returns ResourceBusy while the scope's last task is held in exit");
    assert_eq!(error.kind(), std::io::ErrorKind::ResourceBusy, "the busy outcome: {error}");
    assert!(
        started.elapsed() >= Duration::from_millis(2500),
        "a busy return always takes the bound (3 s), got {:?}",
        started.elapsed()
    );
    assert!(scope.exists(), "the scope directory is present for the whole hold");
    assert!(
        Path::new(&format!("/proc/{pid}")).exists(),
        "the held task is alive and still holds the device"
    );
    assert!(populated(&scope), "the scope still reads populated 1");

    device.resume();
    assert!(
        eventually(Duration::from_secs(10), || !populated(&scope)),
        "after the drain the scope reads populated 0"
    );
    rt.block_on(host().kill_scope(&CgroupPath::for_alloc(&id)))
        .expect("after the drain kill_scope returns Ok");
    assert!(!scope.exists(), "Ok means the scope directory is absent");
}

/// GUARD (RB-3, ADR-0349). A scope that empties during the wait returns `Ok`
/// without waiting out the bound, with the scope removed: the control with no
/// held I/O.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[serial(cgroup)]
fn guard_a_scope_that_empties_during_the_wait_returns_ok_before_the_bound() {
    let device = Device::new("free");
    let id = alloc("ksk-free-0");
    let scope = make_scope(&id);
    let (pid, ready) = spawn_writer(&scope, &device);
    wait_ready(ready);
    let rt = runtime();
    let started = Instant::now();

    rt.block_on(host().kill_scope(&CgroupPath::for_alloc(&id)))
        .expect("a scope with no held I/O settles");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a scope that empties during the wait does not wait out the bound, took {:?}",
        started.elapsed()
    );
    assert!(!scope.exists(), "the scope is removed");
    assert!(
        !populated(&scope) && eventually(Duration::from_secs(5), || state_of(pid) != 'D'),
        "the task is gone from the scope"
    );
}

/// RB-3, ADR-0349 (absent scope is `Ok`). Another remover deletes the scope
/// while `kill_scope` waits on its `cgroup.events`: the wait ends and
/// `kill_scope` returns `Ok`, because the scope it was asked to remove is gone.
/// The second remover is the test's own `rmdir`, which the running kernel
/// accepts for a scope whose last task is held in exit (the spike's finding), so
/// the removal lands under the wait.
///
/// PENDING (RED): the removal wakes no waiter, so `kill_scope` waits out its
/// bound and returns `ResourceBusy` for a scope that is already gone (observed
/// on metal: `Err(Kind(ResourceBusy))`); an `ENODEV` read, if a wake did arrive,
/// would return as a non-busy error. Neither is mapped to the absent scope.
///
/// CONTRACT_SHAPE: bounded-change. Universe: the return value, the scope
/// directory.
#[overdrive_test(guest = "cgroup-default", profile = "volume-pool")]
#[test]
#[serial(cgroup)]
#[should_panic(expected = "kill_scope returns Ok when another remover deletes the scope mid-wait")]
fn kill_scope_returns_ok_when_another_remover_deletes_the_scope_mid_wait() {
    let device = Device::new("raced");
    let id = alloc("ksk-raced-0");
    let scope = make_scope(&id);
    device.suspend();
    let (pid, ready) = spawn_writer(&scope, &device);
    wait_ready(ready);
    // SAFETY: SIGKILL to the child this test forked.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert!(
        eventually(Duration::from_secs(5), || state_of(pid) == 'D' && populated(&scope)),
        "precondition: the killed task is held in exit (state D) and its scope reads populated 1"
    );
    let other_remover = {
        let scope = scope.clone();
        std::thread::spawn(move || {
            // Well inside the 3 s bound, after `kill_scope` is waiting on `cgroup.events`.
            std::thread::sleep(Duration::from_millis(500));
            std::fs::remove_dir(&scope)
        })
    };
    let rt = runtime();

    let result = rt.block_on(host().kill_scope(&CgroupPath::for_alloc(&id)));

    other_remover
        .join()
        .expect("the other remover ran")
        .expect("precondition: the other remover's rmdir of the held scope succeeds");
    assert!(!scope.exists(), "the scope is absent");
    assert!(
        result.is_ok(),
        "kill_scope returns Ok when another remover deletes the scope mid-wait, got {result:?}"
    );
}
