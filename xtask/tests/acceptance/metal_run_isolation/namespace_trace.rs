//! Test-only syscall scheduling for a real verification-read resource failure.
//! No syscall result is replaced: the owned tracee's actual RLIMIT_NOFILE is
//! temporarily exhausted for its actual child-ns-mode read, then restored.
#![allow(
    clippy::undocumented_unsafe_blocks,
    reason = "all ptrace/prlimit calls target this fixture's TRACEME descendant set"
)]
use super::support::Fixture;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn c_error() -> io::Error {
    io::Error::last_os_error()
}
fn trace(
    request: libc::c_uint,
    pid: libc::pid_t,
    address: usize,
    data: usize,
) -> io::Result<libc::c_long> {
    let result = unsafe {
        libc::ptrace(request, pid, address as *mut libc::c_void, data as *mut libc::c_void)
    };
    if result == -1 { Err(c_error()) } else { Ok(result) }
}
fn pathname(pid: libc::pid_t, address: u64) -> io::Result<String> {
    let mut bytes = Vec::new();
    for offset in (0..4096).step_by(8) {
        let value = trace(
            libc::PTRACE_PEEKDATA,
            pid,
            usize::try_from(address).map_err(io::Error::other)? + offset,
            0,
        )?;
        for byte in value.to_ne_bytes() {
            if byte == 0 {
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(byte);
        }
    }
    Err(io::Error::other("unterminated tracee pathname"))
}
struct Descendants(BTreeSet<libc::pid_t>);
impl Drop for Descendants {
    fn drop(&mut self) {
        for pid in &self.0 {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
        for pid in &self.0 {
            let mut status = 0;
            unsafe {
                libc::waitpid(*pid, &raw mut status, libc::__WALL);
            }
        }
    }
}
/// Exhaust only the owned process's actual verification-open descriptor budget.
#[expect(
    clippy::too_many_lines,
    reason = "one scoped event loop owns tracees, real resource exhaustion and restoration"
)]
pub(super) fn deny_verification_open(fixture: &Fixture, inner: &Command) -> io::Result<Output> {
    let stdout = fixture.temp.path().join("traced-wrapper.stdout");
    let stderr = fixture.temp.path().join("traced-wrapper.stderr");
    let mut command = Command::new(inner.get_program());
    command.args(inner.get_args());
    for (key, value) in inner.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    command
        .stdout(Stdio::from(fs::File::create(&stdout)?))
        .stderr(Stdio::from(fs::File::create(&stderr)?));
    unsafe {
        command.pre_exec(|| {
            if libc::ptrace(
                libc::PTRACE_TRACEME,
                0,
                std::ptr::null_mut::<libc::c_void>(),
                std::ptr::null_mut::<libc::c_void>(),
            ) == -1
            {
                return Err(c_error());
            }
            Ok(())
        });
    }
    let child = command.spawn()?;
    let root = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;
    let mut owned = Descendants(BTreeSet::from([root]));
    let mut net_creation = BTreeMap::new();
    let mut net_creations = 0_u32;
    let mut exhausted = BTreeMap::<libc::pid_t, libc::rlimit64>::new();
    let mut injected = false;
    let mut final_status = None;
    let options = libc::PTRACE_O_TRACESYSGOOD
        | libc::PTRACE_O_TRACEFORK
        | libc::PTRACE_O_TRACEVFORK
        | libc::PTRACE_O_TRACECLONE
        | libc::PTRACE_O_TRACEEXEC
        | libc::PTRACE_O_EXITKILL;
    let mut status = 0;
    if unsafe { libc::waitpid(root, &raw mut status, 0) } != root {
        return Err(c_error());
    }
    trace(libc::PTRACE_SETOPTIONS, root, 0, usize::try_from(options).map_err(io::Error::other)?)?;
    trace(libc::PTRACE_SYSCALL, root, 0, 0)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while !owned.0.is_empty() {
        if Instant::now() >= deadline {
            return Err(io::Error::other("owned namespace trace exceeded fixture bound"));
        }
        let mut stopped = None;
        for pid in owned.0.iter().copied() {
            let mut value = 0;
            let result =
                unsafe { libc::waitpid(pid, &raw mut value, libc::__WALL | libc::WNOHANG) };
            if result == pid {
                stopped = Some((pid, value));
                break;
            }
            if result < 0 {
                return Err(c_error());
            }
        }
        let Some((pid, status)) = stopped else {
            std::thread::sleep(Duration::from_micros(100));
            continue;
        };
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            if pid == root {
                final_status = Some(status);
            }
            owned.0.remove(&pid);
            continue;
        }
        let signal = libc::WSTOPSIG(status);
        let event = status >> 16;
        if event == libc::PTRACE_EVENT_FORK
            || event == libc::PTRACE_EVENT_VFORK
            || event == libc::PTRACE_EVENT_CLONE
        {
            let mut descendant: libc::c_ulong = 0;
            trace(libc::PTRACE_GETEVENTMSG, pid, 0, (&raw mut descendant) as usize)?;
            owned.0.insert(libc::pid_t::try_from(descendant).map_err(io::Error::other)?);
        }
        if signal == (libc::SIGTRAP | 0x80) {
            let mut registers = unsafe { std::mem::zeroed::<libc::user_regs_struct>() };
            trace(libc::PTRACE_GETREGS, pid, 0, (&raw mut registers) as usize)?;
            let mut info = unsafe { std::mem::zeroed::<libc::ptrace_syscall_info>() };
            trace(
                libc::PTRACE_GET_SYSCALL_INFO,
                pid,
                std::mem::size_of_val(&info),
                (&raw mut info) as usize,
            )?;
            let entry = info.op == libc::PTRACE_SYSCALL_INFO_ENTRY;
            if !entry && info.op != libc::PTRACE_SYSCALL_INFO_EXIT {
                return Err(io::Error::other("unexpected owned syscall stop kind"));
            }
            if entry {
                let syscall = i64::try_from(registers.orig_rax).unwrap_or(-1);
                let creates = if syscall == libc::SYS_unshare || syscall == libc::SYS_clone {
                    registers.rdi & u64::try_from(libc::CLONE_NEWNET).map_err(io::Error::other)?
                        != 0
                } else if syscall == libc::SYS_clone3 {
                    trace(
                        libc::PTRACE_PEEKDATA,
                        pid,
                        usize::try_from(registers.rdi).map_err(io::Error::other)?,
                        0,
                    )? & libc::c_long::from(libc::CLONE_NEWNET)
                        != 0
                } else {
                    false
                };
                net_creation.insert(pid, creates);
                let address = if syscall == libc::SYS_open {
                    Some(registers.rdi)
                } else if syscall == libc::SYS_openat || syscall == libc::SYS_openat2 {
                    Some(registers.rsi)
                } else {
                    None
                };
                if net_creations >= 2
                    && !injected
                    && let Some(address) = address
                {
                    let path = pathname(pid, address)?;
                    if path.ends_with("/ns_mode") || path == "ns_mode" {
                        let mut previous = unsafe { std::mem::zeroed::<libc::rlimit64>() };
                        if unsafe {
                            libc::prlimit64(
                                pid,
                                libc::RLIMIT_NOFILE,
                                std::ptr::null(),
                                &raw mut previous,
                            )
                        } != 0
                        {
                            return Err(c_error());
                        }
                        let limit = libc::rlimit64 { rlim_cur: 0, rlim_max: previous.rlim_max };
                        if unsafe {
                            libc::prlimit64(
                                pid,
                                libc::RLIMIT_NOFILE,
                                &raw const limit,
                                std::ptr::null_mut(),
                            )
                        } != 0
                        {
                            return Err(c_error());
                        }
                        exhausted.insert(pid, previous);
                        injected = true;
                    }
                }
            } else {
                if net_creation.remove(&pid) == Some(true) && i64::try_from(registers.rax).is_ok() {
                    net_creations += 1;
                }
                if let Some(previous) = exhausted.remove(&pid) {
                    if unsafe {
                        libc::prlimit64(
                            pid,
                            libc::RLIMIT_NOFILE,
                            &raw const previous,
                            std::ptr::null_mut(),
                        )
                    } != 0
                    {
                        return Err(c_error());
                    }
                    if registers.rax != u64::MAX - 23 {
                        return Err(io::Error::other(
                            "real verification open did not return EMFILE",
                        ));
                    }
                }
            }
        }
        let delivered = if signal == (libc::SIGTRAP | 0x80)
            || signal == libc::SIGTRAP
            || signal == libc::SIGSTOP
        {
            0
        } else {
            signal
        };
        trace(libc::PTRACE_SYSCALL, pid, 0, usize::try_from(delivered).map_err(io::Error::other)?)?;
    }
    if !injected {
        return Err(io::Error::other(
            "actual adapter never opened child namespace verification after creating both namespaces",
        ));
    }
    Ok(Output {
        status: std::process::ExitStatus::from_raw(
            final_status.ok_or_else(|| io::Error::other("missing owned wrapper status"))?,
        ),
        stdout: fs::read(stdout)?,
        stderr: fs::read(stderr)?,
    })
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn fixture_exhausts_only_a_real_owned_child_namespace_verification_read() {
    let fixture = Fixture::new_accessible();
    let before = ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
        .map(|path| fs::read(path).expect("initial mode observation"));
    let mut command = Command::new("unshare");
    command.args(["--net","bash","-c","printf 'local\n' > /proc/sys/net/vsock/child_ns_mode; exec unshare --net cat /proc/sys/net/vsock/ns_mode"]);
    let output =
        deny_verification_open(&fixture, &command).expect("real verification-open fixture");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        ["/proc/sys/net/vsock/ns_mode", "/proc/sys/net/vsock/child_ns_mode"]
            .map(|path| fs::read(path).expect("initial mode after fixture")),
        before
    );
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
fn fixture_causes_real_child_mode_and_namespace_capacity_refusals_in_owned_contexts() {
    use super::support::workspace_file;
    let fixture = Fixture::new_accessible();
    let initial = [
        "/proc/sys/net/vsock/ns_mode",
        "/proc/sys/net/vsock/child_ns_mode",
        "/proc/sys/user/max_net_namespaces",
    ]
    .map(|path| fs::read(path).expect("initial namespace inputs"));
    for (step, diagnostic) in
        [("child_mode", "Read-only file system"), ("local", "No space left on device")]
    {
        let mut command = Command::new("unshare");
        if step == "local" {
            command.args(["--user", "--map-root-user"]);
        } else {
            command.args(["--mount", "--propagation", "private"]);
        }
        let denial = fixture.temp.path().join("namespace-denial");
        super::support::executable(
            &denial,
            &super::support::text(&workspace_file("infra/metal/tests/fixtures/namespace-denial")),
        );
        command.arg(denial).arg(step).arg(fixture.temp.path())
            .args(["unshare","--net","bash","-c","set -e; printf 'local\\n' > /proc/sys/net/vsock/child_ns_mode; exec unshare --net cat /proc/sys/net/vsock/ns_mode"])
            .env("LC_ALL","C");
        let output = command.output().expect("real scoped namespace-resource fixture");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{step}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
    }
    assert_eq!(
        [
            "/proc/sys/net/vsock/ns_mode",
            "/proc/sys/net/vsock/child_ns_mode",
            "/proc/sys/user/max_net_namespaces"
        ]
        .map(|path| fs::read(path).expect("unchanged initial namespace inputs")),
        initial
    );
}
