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

/// Host-only tests: the error-pipe codec and the wait-status mapping. Neither
/// needs a guest; the status cases wait on ordinary children of the test.
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use proptest::prelude::*;
    use rustix::process::Pid;

    use super::*;
    use overdrive_testing::placement;

    placement!(host_pure);

    const EVERY_STEP: [LaunchStep; 6] = [
        LaunchStep::SignalReset,
        LaunchStep::Stdio,
        LaunchStep::CloseRange,
        LaunchStep::NoNewPrivs,
        LaunchStep::SeccompLoad,
        LaunchStep::Execve,
    ];

    fn any_step() -> impl Strategy<Value = LaunchStep> {
        proptest::sample::select(EVERY_STEP.to_vec())
    }

    #[test]
    fn every_step_byte_round_trips_and_the_bytes_are_one_to_six_in_launch_order() {
        let codes: Vec<u8> = EVERY_STEP.iter().map(|step| step.code()).collect();
        assert_eq!(codes, [1, 2, 3, 4, 5, 6], "the step bytes are the launch order");
        for step in EVERY_STEP {
            assert_eq!(LaunchStep::from_code(step.code()), step, "{step:?} round-trips");
        }
    }

    #[test]
    fn a_byte_that_names_no_step_is_execve() {
        for code in std::iter::once(0_u8).chain(7..=u8::MAX) {
            assert_eq!(LaunchStep::from_code(code), LaunchStep::Execve, "byte {code}");
        }
    }

    #[test]
    fn a_report_is_the_step_byte_then_the_native_endian_errno() {
        let report = ChildReport { step: LaunchStep::SeccompLoad, errno: libc::EACCES };
        let mut expected = vec![5_u8];
        expected.extend_from_slice(&libc::EACCES.to_ne_bytes());
        assert_eq!(report.encode().to_vec(), expected);
    }

    proptest! {
        #[test]
        fn a_report_decodes_to_what_was_encoded(step in any_step(), errno in any::<i32>()) {
            let report = ChildReport { step, errno };
            prop_assert_eq!(ChildReport::decode(&report.encode()), Some(report));
        }

        #[test]
        fn a_payload_of_the_wrong_length_decodes_as_execve_with_eio(
            bytes in proptest::collection::vec(any::<u8>(), 1..32)
                .prop_filter("not exactly a report", |bytes| bytes.len() != REPORT_LEN),
        ) {
            prop_assert_eq!(
                ChildReport::decode(&bytes),
                Some(ChildReport { step: LaunchStep::Execve, errno: libc::EIO }),
            );
        }

        #[test]
        fn a_five_byte_report_with_an_unknown_step_keeps_its_errno_and_names_execve(
            code in prop_oneof![Just(0_u8), 7..=u8::MAX],
            errno in any::<i32>(),
        ) {
            let mut bytes = vec![code];
            bytes.extend_from_slice(&errno.to_ne_bytes());
            prop_assert_eq!(
                ChildReport::decode(&bytes),
                Some(ChildReport { step: LaunchStep::Execve, errno }),
            );
        }
    }

    #[test]
    fn no_bytes_means_the_child_reached_exec() {
        assert_eq!(ChildReport::decode(&[]), None);
    }

    /// Wait for an ordinary child of this test the way the reaper does, with
    /// `waitid`, and return the status the mapping reads.
    #[allow(
        clippy::zombie_processes,
        reason = "the child is reaped by `waitid`, which clippy does not recognise as a wait"
    )]
    fn status_of(script: &str) -> WaitIdStatus {
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn /bin/sh");
        let pid = Pid::from_raw(i32::try_from(child.id()).expect("a pid fits i32"))
            .expect("a child pid is nonzero");
        rustix::process::waitid(WaitId::Pid(pid), WaitIdOptions::EXITED)
            .expect("waitid on the child")
            .expect("a blocking waitid returns the status")
    }

    #[test]
    fn a_normal_exit_maps_to_its_code_and_no_signal() {
        for code in [0, 1, 7, 127, 255] {
            assert_eq!(exit_of(&status_of(&format!("exit {code}"))), (Some(code), None));
        }
    }

    #[test]
    fn a_signal_death_maps_to_the_signal_and_no_code() {
        for (name, number) in [("KILL", 9_u8), ("TERM", 15), ("HUP", 1)] {
            assert_eq!(
                exit_of(&status_of(&format!("kill -{name} $$"))),
                (None, Some(number)),
                "SIG{name}",
            );
        }
    }
}

/// Guest tests (Tier 3): the spawn primitive against a real cgroup v2
/// filesystem and real processes. Each case launches ordinary programs
/// (`sleep`, `sh`) through [`launch`], the same call `create` makes, with the
/// production seccomp program.
#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixture: a failed precondition is a failed case"
)]
#[allow(
    clippy::doc_markdown,
    reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]
mod guest_tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::*;
    use overdrive_testing::overdrive_test;

    const CGROUP_ROOT: &str = "/sys/fs/cgroup";
    /// How long a case waits for a killed child to be reaped.
    const REAP_BOUND: Duration = Duration::from_secs(10);
    /// What `exit_of` reports: the exit code, the terminating signal.
    type Observed = (Option<i32>, Option<u8>);

    /// A cgroup the case launches into: a direct child of the cgroup2 mount.
    struct Scope(PathBuf);

    impl Scope {
        fn create(name: &str) -> Self {
            let dir =
                Path::new(CGROUP_ROOT).join(format!("launch-clone-{name}-{}", std::process::id()));
            std::fs::create_dir(&dir).expect("create the scope");
            Self(dir)
        }

        fn procs(&self) -> String {
            std::fs::read_to_string(self.0.join("cgroup.procs")).expect("read cgroup.procs")
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            // Best effort: the guest is discarded after the case.
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    fn launch_program(scope: &Scope, program: &str, args: &[&str]) -> io::Result<Launched> {
        let filter = VmmLaunchSeccompFilter::for_target().expect("a launch filter for the target");
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        launch(&LaunchRequest {
            program: OsStr::new(program),
            args: &args,
            scope_dir: &scope.0,
            filter: &filter,
        })
    }

    /// A process's descriptor table as `/proc/<pid>/fd` shows it: number to target.
    fn fd_table(pid: u32) -> Result<BTreeMap<i32, String>, String> {
        let mut table = BTreeMap::new();
        let entries = std::fs::read_dir(format!("/proc/{pid}/fd")).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let number: i32 = entry.file_name().to_string_lossy().parse().map_err(|_| "fd")?;
            let target = std::fs::read_link(entry.path()).map_err(|e| e.to_string())?;
            table.insert(number, target.to_string_lossy().into_owned());
        }
        Ok(table)
    }

    /// Wait until `pid` is blocked in a sleep system call (`nanosleep` or
    /// `clock_nanosleep`): its dynamic loader has finished, so its descriptor
    /// table holds only what it inherited. `launch` returns once the child has
    /// reached `execve`, not once the new program has started.
    fn wait_until_sleeping(pid: u32) {
        let deadline = std::time::Instant::now() + REAP_BOUND;
        loop {
            let syscall =
                std::fs::read_to_string(format!("/proc/{pid}/syscall")).unwrap_or_default();
            if matches!(syscall.split_whitespace().next(), Some("35" | "230")) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "process {pid} never blocked in a sleep call: `{}`",
                syscall.trim()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime")
    }

    async fn is_gone(pid: u32) -> bool {
        let deadline = tokio::time::Instant::now() + REAP_BOUND;
        while Path::new(&format!("/proc/{pid}")).exists() {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }

    /// Run `body` with the process's descriptors 0 to 2 closed, then restore
    /// them. The body returns data only: a failed assertion inside it would
    /// have nowhere to print.
    fn with_standard_descriptors_closed<R>(body: impl FnOnce() -> R) -> R {
        let mut saved = [-1; 3];
        for (slot, fd) in saved.iter_mut().zip(0..) {
            // SAFETY: `F_DUPFD_CLOEXEC` on a live descriptor returns a new one.
            let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, FIRST_NON_STDIO_FD) };
            assert!(copy >= FIRST_NON_STDIO_FD, "save standard descriptor {fd}");
            *slot = copy;
        }
        for fd in 0..3 {
            // SAFETY: closing a descriptor this test owns; it is restored below.
            unsafe { libc::close(fd) };
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        for (fd, copy) in (0..).zip(saved) {
            // SAFETY: `copy` is the live saved descriptor; `dup2` re-installs it on `fd`.
            unsafe {
                libc::dup2(copy, fd);
                libc::close(copy);
            }
        }
        match outcome {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// The test binary itself, launched as the VMM stand-in: the guest has no
    /// shell, so a child with a chosen behaviour is one of the `child_role_*`
    /// cases below, run by name.
    fn launch_role(scope: &Scope, role: &str) -> io::Result<Launched> {
        let executable = std::env::current_exe().expect("the test binary's path");
        let name = format!(
            "{}::{role}",
            module_path!().split_once("::").expect("a crate-qualified module path").1
        );
        launch_program(
            scope,
            executable.to_str().expect("a UTF-8 path"),
            &["--exact", &name, "--ignored", "--nocapture", "--test-threads=1"],
        )
    }

    /// Child role: end with exit status 7.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    #[ignore = "child role: launched by the cases that observe a child's exit"]
    fn child_role_exit_seven() {
        std::process::exit(7);
    }

    /// Child role: end on SIGKILL, raised against itself.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    #[ignore = "child role: launched by the cases that observe a child's exit"]
    fn child_role_kill_self() {
        // SAFETY: raising a signal against this process.
        unsafe { libc::raise(libc::SIGKILL) };
        std::thread::sleep(Duration::from_secs(600));
    }

    /// Child role: end on SIGABRT with core dumping allowed.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    #[ignore = "child role: launched by the cases that observe a child's exit"]
    fn child_role_abort_with_core() {
        let unlimited =
            libc::rlimit { rlim_cur: libc::RLIM_INFINITY, rlim_max: libc::RLIM_INFINITY };
        // SAFETY: `unlimited` is a live `rlimit`; `abort` ends the process.
        unsafe {
            libc::setrlimit(libc::RLIMIT_CORE, &raw const unlimited);
            libc::abort()
        }
    }

    /// Child role: write one line on standard error, then wait to be killed.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    #[ignore = "child role: launched by the case that observes standard I/O"]
    #[allow(clippy::print_stderr, reason = "the line on standard error is the case's oracle")]
    fn child_role_write_stderr_then_wait() {
        eprintln!("on-stderr");
        std::thread::sleep(Duration::from_secs(600));
    }

    /// (ADR-0352, ADR-0129) The launched process holds descriptors 0 to 2 and
    /// nothing else: a descriptor the parent left inheritable does not reach it.
    ///
    /// CONTRACT_SHAPE: bounded-change. Universe: the descriptor table of the
    /// launched process.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn a_launched_process_holds_no_inherited_descriptor_beyond_standard_io() {
        let scratch = tempfile::tempdir().expect("scratch directory");
        let stray_path = scratch.path().join("stray-parent-descriptor");
        let stray = File::create(&stray_path).expect("create the stray file");
        rustix::io::fcntl_setfd(&stray, rustix::io::FdFlags::empty())
            .expect("make the descriptor inheritable");
        let stray_target = stray_path.to_string_lossy().into_owned();

        // Contrast: an ordinary spawn inherits it, so its absence below is the
        // launch sequence's doing, not the descriptor's.
        let mut control = std::process::Command::new("sleep")
            .arg("600")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn the contrast child");
        wait_until_sleeping(control.id());
        let control_table = fd_table(control.id()).expect("the contrast table");
        control.kill().expect("kill the contrast child");
        control.wait().expect("reap the contrast child");
        assert!(
            control_table.values().any(|target| *target == stray_target),
            "contrast: an ordinary spawn inherits the stray descriptor: {control_table:?}"
        );

        let scope = Scope::create("hygiene");
        let launched = launch_program(&scope, "sleep", &["600"]).expect("launch sleep");
        wait_until_sleeping(launched.child.pid());
        let table = fd_table(launched.child.pid()).expect("the launched table");
        drop(launched);

        let numbers: Vec<i32> = table.keys().copied().collect();
        assert_eq!(numbers, [0, 1, 2], "only standard I/O is held: {table:?}");
        assert!(
            table.values().all(|target| *target != stray_target),
            "the stray descriptor did not reach the launched process: {table:?}"
        );
        assert_eq!(table[&0], "/dev/null", "standard input");
        assert_eq!(table[&1], "/dev/null", "standard output");
        assert!(table[&2].starts_with("pipe:"), "standard error is the capture pipe: {table:?}");
    }

    /// (ADR-0352, module safety argument) Standard I/O is correct when the
    /// parent's own descriptors 0 to 2 are closed: every source the child
    /// duplicates from sits at 3 or above, so no `dup2` clobbers one.
    ///
    /// CONTRACT_SHAPE: bounded-change. Universe: the descriptor table of the
    /// launched process, what arrives on its stderr pipe.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn standard_io_is_correct_when_the_parents_standard_descriptors_are_closed() {
        let scope = Scope::create("closed-stdio");

        let (probe_fd, table, stderr_text) = with_standard_descriptors_closed(|| {
            // With 0 to 2 closed, the next open takes the lowest number.
            let probe = File::open("/dev/null").expect("open a probe");
            let probe_fd = probe.as_raw_fd();
            drop(probe);

            let launched = launch_role(&scope, "child_role_write_stderr_then_wait")
                .map_err(|error| error.to_string());
            let (table, text) = match launched {
                Ok(launched) => {
                    // Read what the child writes on its capture pipe until the marker.
                    let mut pipe = File::from(launched.stderr.try_clone().expect("clone stderr"));
                    let mut seen = Vec::new();
                    let mut chunk = [0_u8; 256];
                    let outcome = loop {
                        match std::io::Read::read(&mut pipe, &mut chunk) {
                            Ok(0) => break Err("stderr closed before the marker".to_owned()),
                            Ok(count) => {
                                seen.extend_from_slice(&chunk[..count]);
                                if String::from_utf8_lossy(&seen).contains("on-stderr\n") {
                                    break Ok(String::from_utf8_lossy(&seen).into_owned());
                                }
                            }
                            Err(error) => break Err(error.to_string()),
                        }
                    };
                    (fd_table(launched.child.pid()), outcome)
                }
                Err(error) => (Err(error.clone()), Err(error)),
            };
            (probe_fd, table, text)
        });

        assert!(
            probe_fd < FIRST_NON_STDIO_FD,
            "the parent's low descriptors were free: {probe_fd}"
        );
        let table = table.expect("the launch succeeded with 0 to 2 closed");
        assert_eq!(
            table.get(&0).map(String::as_str),
            Some("/dev/null"),
            "standard input: {table:?}"
        );
        assert_eq!(
            table.get(&1).map(String::as_str),
            Some("/dev/null"),
            "standard output: {table:?}"
        );
        assert!(
            table.get(&2).is_some_and(|target| target.starts_with("pipe:")),
            "standard error is the capture pipe: {table:?}"
        );
        assert_eq!(
            table.keys().copied().collect::<Vec<_>>(),
            [0, 1, 2],
            "nothing else is held: {table:?}"
        );
        assert!(
            stderr_text.expect("the child wrote to stderr").contains("on-stderr\n"),
            "what the child writes on standard error arrives on the capture pipe"
        );
    }

    /// (ADR-0352) The exit is observed through the pidfd: code, signal and core
    /// dump map as `exit_of` says, the child is reaped by the observation (no
    /// zombie remains in its scope), and `kill` ends a running child.
    ///
    /// CONTRACT_SHAPE: bounded-change. Universe: the observed status, the
    /// process table entry, the scope's `cgroup.procs`.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn the_exit_is_observed_through_the_pidfd_and_the_child_is_reaped() {
        std::fs::write("/proc/sys/kernel/core_pattern", "core")
            .expect("set a core pattern the dump can honour");
        let work = tempfile::tempdir().expect("working directory for core files");
        std::env::set_current_dir(work.path()).expect("enter the working directory");
        let scope = Scope::create("reap");

        let cases: [(&str, Observed, bool); 3] = [
            ("child_role_exit_seven", (Some(7), None), false),
            ("child_role_kill_self", (None, Some(9)), false),
            ("child_role_abort_with_core", (None, Some(6)), true),
        ];
        runtime().block_on(async {
            for (script, expected, dumped) in cases {
                let launched = launch_role(&scope, script).expect("launch");
                let Launched { child, stderr: _stderr } = launched;
                let pid = child.pid();
                let watch = child.into_watch().expect("register the pidfd");
                let status = watch.wait().await.expect("the exit is observed");
                assert_eq!(exit_of(&status), expected, "mapping for `{script}`");
                assert_eq!(status.dumped(), dumped, "core dump for `{script}`");
                assert!(
                    !Path::new(&format!("/proc/{pid}")).exists(),
                    "`{script}`: observing the exit reaped the child (no zombie)"
                );
            }

            let Launched { child, stderr: _stderr } =
                launch_program(&scope, "sleep", &["600"]).expect("launch sleep");
            let pid = child.pid();
            let watch = child.into_watch().expect("register the pidfd");
            watch.kill();
            let status = watch.wait().await.expect("the kill is observed");
            assert_eq!(exit_of(&status), (None, Some(9)), "a killed child reports SIGKILL");
            assert!(is_gone(pid).await, "the killed child was reaped");
        });
        assert_eq!(scope.procs().trim(), "", "no member, zombie included, remains in the scope");
    }

    /// (ADR-0352) A launch that is abandoned leaves no VMM: a `create` future
    /// dropped after its `clone3` drops the blocking task's `Launched` unadopted,
    /// and the guard kills and reaps the child. Covered in both shapes the guard
    /// has: with a runtime (a detached reap) and without (a synchronous reap).
    ///
    /// CONTRACT_SHAPE: bounded-change. Universe: the process table entry, the
    /// scope's `cgroup.procs`.
    #[overdrive_test(guest = "cgroup-default")]
    #[test]
    fn an_abandoned_launch_is_killed_and_reaped_by_its_guard() {
        let scope = Scope::create("abandoned");
        let scope_dir = scope.0.clone();

        // With a runtime: `create` awaits `spawn_blocking(launch)`; dropping the
        // future drops the join handle and the closure's result is discarded.
        let rt = runtime();
        rt.block_on(async {
            let (sender, receiver) = std::sync::mpsc::channel();
            let task = tokio::task::spawn_blocking(move || {
                let filter = VmmLaunchSeccompFilter::for_target().expect("a launch filter");
                let args = [OsString::from("600")];
                let launched = launch(&LaunchRequest {
                    program: OsStr::new("sleep"),
                    args: &args,
                    scope_dir: &scope_dir,
                    filter: &filter,
                })?;
                sender.send(launched.child.pid()).expect("report the pid");
                Ok::<_, io::Error>(launched)
            });
            drop(task);
            let pid = tokio::task::spawn_blocking(move || receiver.recv_timeout(REAP_BOUND))
                .await
                .expect("the receive task")
                .expect("the abandoned launch reached clone3");
            assert!(is_gone(pid).await, "the abandoned VMM is killed and reaped");
        });
        drop(rt);
        assert_eq!(scope.procs().trim(), "", "nothing remains in the scope after the reap");

        // Without a runtime: the guard waits for the kill itself.
        let launched = launch_program(&scope, "sleep", &["600"]).expect("launch sleep");
        let pid = launched.child.pid();
        assert!(Path::new(&format!("/proc/{pid}")).exists(), "the child is alive before the drop");
        drop(launched);
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "with no runtime the drop returns only once the child is reaped"
        );
        assert_eq!(scope.procs().trim(), "", "nothing remains in the scope");
    }
}
