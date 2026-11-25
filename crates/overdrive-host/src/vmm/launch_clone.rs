//! The VMM spawn primitive (ADR-0352, feature delta § "A VMM is born inside its
//! scope"): a raw `clone3` with `CLONE_INTO_CGROUP | CLONE_PIDFD`, so the VMM
//! process is a member of its allocation's cgroup scope before its first
//! instruction, followed by the audited launch sequence of ADR-0129 and
//! ADR-0143 run in the clone child.
//!
//! This is the crate's one production `#[allow(unsafe_code)]` module. The
//! parent-side waits, signal, pipes and descriptor handling use safe `rustix`
//! and `std` wrappers; only the `clone3` call and the code the child runs
//! between it and `execve` are unsafe.
//!
//! # Safety argument
//!
//! * After `clone3` the child is a single-threaded copy of a multithreaded
//!   parent. A lock another thread held at the clone (an allocator arena, a
//!   stdio lock, a tokio internal) stays held for ever in the child. From the
//!   instant `clone3` returns `0` until `execve`, the child therefore
//!   allocates nothing, takes no lock, runs no destructor and runs no code
//!   that can panic or unwind. It makes only direct system calls and the
//!   async-signal-safe libc calls `dup2`, `sigemptyset`, `sigprocmask`,
//!   `sigaction`, `write`, `execve` and `_exit`.
//! * Everything the child needs is built before the clone: the NUL-terminated
//!   absolute path, the `argv` and `envp` pointer arrays, the seccomp program,
//!   the descriptor numbers and the error pipe. The child never returns to Rust
//!   code: it ends in `execve` or `_exit`.
//! * Descriptors are opened `O_CLOEXEC` before the clone, so a concurrent
//!   `exec` elsewhere in `serve` cannot inherit them, and every descriptor the
//!   child `dup2`s from or writes to is moved to a number of at least 3 first,
//!   so no `dup2` onto 0 to 2 can clobber a source.
//! * `exit_signal = SIGCHLD` makes the child an ordinary child that `waitid`
//!   reaps without `__WCLONE`.

#![allow(unsafe_code)]

use std::ffi::{CString, OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::process::{Signal, WaitId, WaitIdOptions, WaitIdStatus};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use super::launch_seccomp::VmmLaunchSeccompFilter;

/// `clone3` flag: place the child in the cgroup named by `clone_args.cgroup`
/// (`include/uapi/linux/sched.h:109 @ v7.0`).
const CLONE_INTO_CGROUP: u64 = 0x2_0000_0000;
/// `clone3` flag: install a pidfd for the child in the parent's table.
const CLONE_PIDFD: u64 = 0x0000_1000;
/// `CLONE_ARGS_SIZE_VER2`: the size of the `clone_args` that carries `cgroup`.
const CLONE_ARGS_SIZE_VER2: usize = 88;
/// The lowest descriptor number a parent-side source may hold: 0 to 2 are
/// `dup2` targets in the child.
const FIRST_NON_STDIO_FD: RawFd = 3;
/// How long the reaper waits before it asks `waitid` again when the pidfd is
/// readable and the leader is not yet reapable (a delayed leader).
const DELAYED_LEADER_POLL: Duration = Duration::from_millis(10);
/// The exit status a child that fails a step ends with.
const CHILD_FAILURE_STATUS: libc::c_int = 127;
/// The step label of a refused placement, which happens in `clone3` itself.
pub(super) const JOINING_THE_CGROUP: &str = "joining the cgroup";
/// The `PATH` `execvp` falls back to when the variable is unset.
const DEFAULT_SEARCH_PATH: &str = "/bin:/usr/bin";

/// The kernel's `struct clone_args` at `CLONE_ARGS_SIZE_VER2`.
#[repr(C)]
#[derive(Default)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

const _: () = assert!(std::mem::size_of::<CloneArgs>() == CLONE_ARGS_SIZE_VER2);

/// A step the clone child runs between `clone3` and `execve`, in this order.
/// The discriminant is the byte the child writes to the error pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum LaunchStep {
    /// Empty signal mask and `SIGPIPE` back to its default disposition.
    SignalReset = 1,
    /// The `dup2` calls onto descriptors 0 to 2.
    Stdio = 2,
    /// `close_range(3, ~0, CLOSE_RANGE_CLOEXEC)`.
    CloseRange = 3,
    /// `prctl(PR_SET_NO_NEW_PRIVS)`.
    NoNewPrivs = 4,
    /// The seccomp filter load.
    SeccompLoad = 5,
    /// `execve`.
    Execve = 6,
}

impl LaunchStep {
    /// The step named by an error-pipe byte; an unknown byte is `Execve`, the
    /// parent never guesses a step from a malformed report.
    pub(super) const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::SignalReset,
            2 => Self::Stdio,
            3 => Self::CloseRange,
            4 => Self::NoNewPrivs,
            5 => Self::SeccompLoad,
            _ => Self::Execve,
        }
    }

    /// The error-pipe byte of this step.
    pub(super) const fn code(self) -> u8 {
        self as u8
    }

    /// The operator-facing name of the step.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::SignalReset => "resetting signal state",
            Self::Stdio => "wiring standard I/O",
            Self::CloseRange => "closing inherited descriptors",
            Self::NoNewPrivs => "setting no_new_privs",
            Self::SeccompLoad => "loading the seccomp filter",
            Self::Execve => "executing the launch program",
        }
    }
}

/// What the child reports on the error pipe: a failing step and its `errno`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChildReport {
    pub(super) step: LaunchStep,
    pub(super) errno: i32,
}

/// The five bytes the child writes: the step byte, then `errno` as a
/// native-endian `i32`.
const REPORT_LEN: usize = 5;

impl ChildReport {
    /// Encode as the five bytes the child writes.
    pub(super) const fn encode(self) -> [u8; REPORT_LEN] {
        let errno = self.errno.to_ne_bytes();
        [self.step.code(), errno[0], errno[1], errno[2], errno[3]]
    }

    /// Decode what the parent read to end-of-file. No bytes is `None` (the
    /// child reached `exec`); five bytes are a report; any other length is a
    /// child that died mid-write, reported as `Execve` with `EIO`.
    pub(super) fn decode(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [] => None,
            [step, a, b, c, d] => Some(Self {
                step: LaunchStep::from_code(*step),
                errno: i32::from_ne_bytes([*a, *b, *c, *d]),
            }),
            _ => Some(Self { step: LaunchStep::Execve, errno: libc::EIO }),
        }
    }
}

/// A launch step that failed, carried inside an [`io::Error`] so the error's
/// `kind` is the step's `errno` kind and the adapter can name the step.
#[derive(Debug)]
pub(super) struct StepFailure {
    /// The operator-facing step name.
    pub(super) step: &'static str,
    /// The `errno` the step failed with.
    pub(super) source: io::Error,
}

impl fmt::Display for StepFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed: {}", self.step, self.source)
    }
}

impl std::error::Error for StepFailure {}

fn step_error(step: &'static str, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), StepFailure { step, source })
}

/// Map a child's wait status to the adapter-agnostic exit code and signal.
/// `CLD_EXITED` yields the code; `CLD_KILLED` and `CLD_DUMPED` the signal.
pub(super) fn exit_of(status: &WaitIdStatus) -> (Option<i32>, Option<u8>) {
    let signal = status.terminating_signal().and_then(|number| u8::try_from(number).ok());
    (status.exit_status(), signal)
}

/// The prepared launch: what runs, where it is born and what confines it.
pub(super) struct LaunchRequest<'a> {
    /// The program as the caller names it (`argv[0]`); resolved over `PATH`
    /// before the clone when it has no `/`.
    pub(super) program: &'a OsStr,
    /// `argv[1..]`.
    pub(super) args: &'a [OsString],
    /// The scope's directory: the cgroup the child is born in.
    pub(super) scope_dir: &'a Path,
    /// The seccomp program the child loads before `execve`.
    pub(super) filter: &'a VmmLaunchSeccompFilter,
}

/// A launched child: its guard and the read end of its stderr pipe.
pub(super) struct Launched {
    pub(super) child: LaunchedChild,
    pub(super) stderr: OwnedFd,
}

/// Owner of a launched child's pidfd. Dropped while it still owns the pidfd,
/// it kills the child and hands the pidfd to a detached reap, so a launch
/// that is abandoned never leaves a running VMM or a zombie.
pub(super) struct LaunchedChild {
    pid: u32,
    pidfd: Option<OwnedFd>,
    runtime: Option<tokio::runtime::Handle>,
}

impl LaunchedChild {
    /// The child's pid.
    pub(super) const fn pid(&self) -> u32 {
        self.pid
    }

    /// Register the pidfd with the reactor and hand it to the reaper. Must be
    /// called inside a tokio runtime. On failure the guard is dropped, which
    /// kills and reaps the child.
    pub(super) fn into_watch(mut self) -> io::Result<ChildWatch> {
        let Some(pidfd) = self.pidfd.take() else {
            unreachable!("a LaunchedChild owns its pidfd until into_watch takes it")
        };
        match AsyncFd::try_with_interest(pidfd, Interest::READABLE) {
            Ok(ready) => Ok(ChildWatch { ready }),
            Err(failure) => {
                let (pidfd, source) = failure.into_parts();
                self.pidfd = Some(pidfd);
                Err(source)
            }
        }
    }

    /// Take the pidfd out of the guard, leaving nothing for `Drop` to kill.
    const fn take_pidfd(&mut self) -> Option<OwnedFd> {
        self.pidfd.take()
    }
}

impl Drop for LaunchedChild {
    fn drop(&mut self) {
        let Some(pidfd) = self.pidfd.take() else { return };
        // `ESRCH` is "already gone", not an error.
        let _ = rustix::process::pidfd_send_signal(&pidfd, Signal::KILL);
        match self.runtime.take() {
            Some(runtime) => {
                runtime.spawn(async move {
                    if let Ok(ready) = AsyncFd::with_interest(pidfd, Interest::READABLE) {
                        let _ = ChildWatch { ready }.wait().await;
                    }
                });
            }
            None => {
                // No runtime to reap on: the child was just sent SIGKILL.
                let _ =
                    rustix::process::waitid(WaitId::PidFd(pidfd.as_fd()), WaitIdOptions::EXITED);
            }
        }
    }
}

/// The reaper's handle on a child: pollable exit and a pidfd kill.
pub(super) struct ChildWatch {
    ready: AsyncFd<OwnedFd>,
}

impl ChildWatch {
    /// Send `SIGKILL` to the child. `ESRCH` is "already gone", not an error.
    pub(super) fn kill(&self) {
        let _ = rustix::process::pidfd_send_signal(self.ready.get_ref(), Signal::KILL);
    }

    /// Wait for the child to exit and reap it: pidfd readability, then
    /// `waitid(P_PIDFD, WEXITED | WNOHANG)` until it reports the exit.
    pub(super) async fn wait(&self) -> io::Result<WaitIdStatus> {
        loop {
            let _ready = self.ready.readable().await?;
            let reaped = rustix::process::waitid(
                WaitId::PidFd(self.ready.get_ref().as_fd()),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG,
            );
            match reaped {
                Ok(Some(status)) => return Ok(status),
                Ok(None) | Err(rustix::io::Errno::INTR) => {
                    tokio::time::sleep(DELAYED_LEADER_POLL).await;
                }
                Err(errno) => return Err(errno.into()),
            }
        }
    }
}

/// Resolve `program` to a path `execve` can take, by the search `execvp`
/// performs over the caller's `PATH`: a name with a `/` is used as given;
/// otherwise the first executable candidate wins, and when none is executable
/// but one exists the first existing candidate is returned so its `execve`
/// reports `EACCES`. No candidate at all is `NotFound`.
fn resolve_program(program: &OsStr) -> io::Result<PathBuf> {
    if program.as_bytes().contains(&b'/') {
        return Ok(PathBuf::from(program));
    }
    let search = std::env::var_os("PATH").unwrap_or_else(|| OsString::from(DEFAULT_SEARCH_PATH));
    let mut first_existing = None;
    for directory in std::env::split_paths(&search) {
        let directory =
            if directory.as_os_str().is_empty() { PathBuf::from(".") } else { directory };
        let candidate = directory.join(program);
        let is_file = std::fs::metadata(&candidate).is_ok_and(|meta| meta.is_file());
        if !is_file {
            continue;
        }
        if rustix::fs::access(&candidate, rustix::fs::Access::EXEC_OK).is_ok() {
            return Ok(candidate);
        }
        first_existing.get_or_insert(candidate);
    }
    first_existing.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
}

fn c_string(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// A NUL-terminated pointer array over strings that outlive it.
fn pointer_array(strings: &[CString]) -> Vec<*const libc::c_char> {
    let mut array: Vec<*const libc::c_char> = Vec::with_capacity(strings.len() + 1);
    array.extend(strings.iter().map(|string| string.as_ptr()));
    array.push(std::ptr::null());
    array
}

/// Move `fd` to a descriptor number of at least [`FIRST_NON_STDIO_FD`] and
/// close the original.
fn moved_above_stdio(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() >= FIRST_NON_STDIO_FD {
        return Ok(fd);
    }
    rustix::io::fcntl_dupfd_cloexec(&fd, FIRST_NON_STDIO_FD).map_err(io::Error::from)
}

fn pipe_above_stdio() -> io::Result<(OwnedFd, OwnedFd)> {
    let (reader, writer) = io::pipe()?;
    Ok((moved_above_stdio(OwnedFd::from(reader))?, moved_above_stdio(OwnedFd::from(writer))?))
}

/// What the clone child reads: every pointer and descriptor it needs, built
/// before the clone.
struct ChildPlan {
    path: *const libc::c_char,
    argv: *const *const libc::c_char,
    envp: *const *const libc::c_char,
    null_device: RawFd,
    stderr_sink: RawFd,
    error_pipe: RawFd,
    filter: libc::sock_fprog,
}

/// Start the VMM inside `request.scope_dir` and run the launch sequence in the
/// child. Synchronous and blocking: run it in `spawn_blocking`. A launch that
/// returns `Ok` has a child that reached `execve`; an `Err` leaves no process.
pub(super) fn launch(request: &LaunchRequest<'_>) -> io::Result<Launched> {
    let program = resolve_program(request.program)?;
    let path = c_string(program.as_os_str())?;
    let mut argv_strings = Vec::with_capacity(request.args.len() + 1);
    argv_strings.push(c_string(request.program)?);
    for argument in request.args {
        argv_strings.push(c_string(argument)?);
    }
    let mut envp_strings = Vec::new();
    for (name, value) in std::env::vars_os() {
        let mut entry = name;
        entry.push("=");
        entry.push(value);
        envp_strings.push(c_string(&entry)?);
    }
    let argv = pointer_array(&argv_strings);
    let envp = pointer_array(&envp_strings);

    let scope = rustix::fs::open(
        request.scope_dir,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|errno| step_error(JOINING_THE_CGROUP, errno.into()))?;
    let scope = moved_above_stdio(scope)?;

    let null_device = moved_above_stdio(OwnedFd::from(
        std::fs::File::options().read(true).write(true).open("/dev/null")?,
    ))?;
    let (stderr_reader, stderr_sink) = pipe_above_stdio()?;
    let (error_reader, error_writer) = pipe_above_stdio()?;

    let plan = ChildPlan {
        path: path.as_ptr(),
        argv: argv.as_ptr(),
        envp: envp.as_ptr(),
        null_device: null_device.as_raw_fd(),
        stderr_sink: stderr_sink.as_raw_fd(),
        error_pipe: error_writer.as_raw_fd(),
        filter: libc::sock_fprog {
            len: u16::try_from(request.filter.program().len())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?,
            filter: request.filter.program().as_ptr().cast_mut(),
        },
    };
    let runtime = tokio::runtime::Handle::try_current().ok();

    let mut child = spawn_in_scope(&plan, &scope, runtime)?;

    // The parent's copies of the child's ends are closed so the error pipe
    // reaches end-of-file once the child execs or ends.
    drop((scope, null_device, stderr_sink, error_writer));
    let mut report = Vec::with_capacity(REPORT_LEN);
    std::fs::File::from(error_reader).read_to_end(&mut report)?;
    match ChildReport::decode(&report) {
        None => Ok(Launched { child, stderr: stderr_reader }),
        Some(failure) => {
            // The child is ending in `_exit`: reap it so no zombie remains.
            if let Some(pidfd) = child.take_pidfd() {
                let _ =
                    rustix::process::waitid(WaitId::PidFd(pidfd.as_fd()), WaitIdOptions::EXITED);
            }
            Err(step_error(failure.step.as_str(), io::Error::from_raw_os_error(failure.errno)))
        }
    }
}

/// The one `clone3`. The child never returns from this function.
fn spawn_in_scope(
    plan: &ChildPlan,
    scope: &OwnedFd,
    runtime: Option<tokio::runtime::Handle>,
) -> io::Result<LaunchedChild> {
    let mut pidfd: libc::c_int = -1;
    let scope_fd = u64::try_from(scope.as_raw_fd())
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let arguments = CloneArgs {
        flags: CLONE_INTO_CGROUP | CLONE_PIDFD,
        pidfd: std::ptr::addr_of_mut!(pidfd) as u64,
        exit_signal: libc::SIGCHLD as u64,
        cgroup: scope_fd,
        ..CloneArgs::default()
    };
    // SAFETY: `arguments` is a live `clone_args` of `CLONE_ARGS_SIZE_VER2`
    // bytes whose `pidfd` field points at the live `pidfd` integer and whose
    // `cgroup` field is a descriptor this function holds open. No `CLONE_VM`,
    // `CLONE_THREAD` or `stack`, so the child is a copy-on-write copy that
    // runs on its own copy of this stack, like `fork`.
    let created = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            std::ptr::addr_of!(arguments),
            std::mem::size_of::<CloneArgs>(),
        )
    };
    if created == 0 {
        // SAFETY: this is the clone child; `plan` was built before the clone
        // and `child_after_clone` obeys the module's safety argument.
        unsafe { child_after_clone(plan) }
    }
    if created < 0 {
        return Err(step_error(JOINING_THE_CGROUP, io::Error::last_os_error()));
    }
    // SAFETY: `CLONE_PIDFD` stored a fresh descriptor in `pidfd`, owned by no
    // one else; the guard owns it from here, with no fallible step between.
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd) };
    let pid = u32::try_from(created).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    Ok(LaunchedChild { pid, pidfd: Some(pidfd), runtime })
}

/// Report the failing step and `errno` on the error pipe and end the child.
///
/// # Safety
///
/// Callable only in the clone child: it uses `write` and `_exit` only.
unsafe fn child_fail(error_pipe: RawFd, step: LaunchStep) -> ! {
    // SAFETY: `__errno_location` returns the calling thread's errno slot.
    let errno = unsafe { *libc::__errno_location() };
    let message = ChildReport { step, errno }.encode();
    // SAFETY: `message` is a live five-byte buffer; `write` and `_exit` are
    // async-signal-safe.
    unsafe {
        libc::write(error_pipe, message.as_ptr().cast(), message.len());
        libc::_exit(CHILD_FAILURE_STATUS)
    }
}

/// The clone child's whole life: the audited launch sequence, then `execve`.
///
/// # Safety
///
/// Callable only in the clone child of a multithreaded parent. It allocates
/// nothing, takes no lock, runs no destructor and cannot panic: every step is
/// one direct system call or an async-signal-safe libc call.
unsafe fn child_after_clone(plan: &ChildPlan) -> ! {
    // SAFETY: every call below is a system call or async-signal-safe libc call
    // over values built before the clone (see the module's safety argument).
    unsafe {
        let mut empty: libc::sigset_t = std::mem::zeroed();
        if libc::sigemptyset(&raw mut empty) == -1
            || libc::sigprocmask(libc::SIG_SETMASK, &raw const empty, std::ptr::null_mut()) == -1
        {
            child_fail(plan.error_pipe, LaunchStep::SignalReset);
        }
        let mut default_action: libc::sigaction = std::mem::zeroed();
        default_action.sa_sigaction = libc::SIG_DFL;
        if libc::sigaction(libc::SIGPIPE, &raw const default_action, std::ptr::null_mut()) == -1 {
            child_fail(plan.error_pipe, LaunchStep::SignalReset);
        }

        if libc::dup2(plan.null_device, libc::STDIN_FILENO) == -1
            || libc::dup2(plan.null_device, libc::STDOUT_FILENO) == -1
            || libc::dup2(plan.stderr_sink, libc::STDERR_FILENO) == -1
        {
            child_fail(plan.error_pipe, LaunchStep::Stdio);
        }

        if libc::syscall(
            libc::SYS_close_range,
            FIRST_NON_STDIO_FD.cast_unsigned(),
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        ) == -1
        {
            child_fail(plan.error_pipe, LaunchStep::CloseRange);
        }

        if libc::prctl(
            libc::PR_SET_NO_NEW_PRIVS,
            1 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        ) == -1
        {
            child_fail(plan.error_pipe, LaunchStep::NoNewPrivs);
        }

        if libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            0,
            &raw const plan.filter,
        ) == -1
        {
            child_fail(plan.error_pipe, LaunchStep::SeccompLoad);
        }

        libc::execve(plan.path, plan.argv, plan.envp);
        child_fail(plan.error_pipe, LaunchStep::Execve)
    }
}
