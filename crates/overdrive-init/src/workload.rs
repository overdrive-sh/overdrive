//! Workload creation in the listen-hook cgroup.

#![allow(
    unsafe_code,
    reason = "clone3 and execvp are the narrow syscall boundary required by CLONE_INTO_CGROUP"
)]

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

use nix::libc;

const CLONE_INTO_CGROUP: u64 = 1 << 33;

#[repr(C)]
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

/// Creates the workload directly in the cgroup containing the listen hooks.
/// An `execvp` failure retains the command path's existing guest EXIT status
/// of 127; a `clone3` failure is returned to PID 1 as the existing Spawn error.
pub fn spawn_in_cgroup(argv: &[String], cgroup: RawFd) -> io::Result<i32> {
    let command = argv
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "workload argv is empty"))?;
    let c_argv = argv
        .iter()
        .map(|argument| CString::new(argument.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidInput, source))?;
    let mut argv_ptrs: Vec<*const libc::c_char> =
        c_argv.iter().map(|argument| argument.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());

    let cgroup = u64::try_from(cgroup)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid workload cgroup fd"))?;
    let clone_args = CloneArgs {
        flags: CLONE_INTO_CGROUP,
        pidfd: 0,
        child_tid: 0,
        parent_tid: 0,
        exit_signal: libc::SIGCHLD as u64,
        stack: 0,
        stack_size: 0,
        tls: 0,
        set_tid: 0,
        set_tid_size: 0,
        cgroup,
    };
    // SAFETY: `clone_args` follows the Linux `clone_args` layout; the child starts
    // with the same stack and only calls async-signal-safe libc operations
    // before replacing its image with `execvp`.
    let child = unsafe {
        libc::syscall(libc::SYS_clone3, &raw const clone_args, std::mem::size_of::<CloneArgs>())
    };
    if child < 0 {
        return Err(io::Error::last_os_error());
    }
    if child == 0 {
        // SAFETY: this branch is the child returned by clone3. It creates the
        // process group expected by the existing supervisor and then execs
        // the operator command with inherited stdio and environment.
        unsafe {
            libc::setpgid(0, 0);
            libc::execvp(c_argv[0].as_ptr(), argv_ptrs.as_ptr());
            libc::_exit(127);
        }
    }
    i32::try_from(child)
        .map_err(|_| io::Error::other(format!("clone3 returned invalid pid for {command}")))
}
