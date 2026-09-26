//! GH #295 D-295-R2 / R4 — the owner-side TAP queue attach port
//! (`overdrive_netlink::attach_tap_queue`, FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the new `overdrive-netlink` TUN helper and the `attach_tap_queue` contract)) against real kernel TAPs.
//!
//! Every case runs as root in the Lima VM on scratch TAPs named outside the
//! production `ovd-tp-` family, each deleted by RAII on every exit path. The
//! `overdrive-netlink` integration binary is in the `host-kernel-shared`
//! nextest group.

use std::fs;
use std::os::fd::{AsRawFd, BorrowedFd};

use overdrive_netlink::{
    Client, TapLinkState, TapQueueError, attach_tap_queue, block_on_host_netlink,
    create_persistent_tap,
};

/// `IFF_TAP | IFF_NO_PI | IFF_VNET_HDR | IFF_PERSIST`: the exact flags of a
/// queue attached to a persistent single-queue vnet-header TAP (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (`TapQueueError::Flags` and postcondition 1 of the `attach_tap_queue` contract)).
pub(super) const PERSISTENT_VNET_TAP_FLAGS: u16 = 0x5802;
const IFF_PERSIST: u16 = 0x0800;
/// `IFF_TAP | IFF_NO_PI | IFF_VNET_HDR`: what the attach requests.
const REQUESTED_QUEUE_FLAGS: u16 = 0x5002;

/// Fail closed unless the test process is root (the Lima runner's uid).
pub(super) fn require_root(test: &str) {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    assert_eq!(euid, 0, "{test} creates kernel TAPs and must run as root (cargo xtask lima run)");
}

/// A scratch-TAP name unique to this process and case, outside `ovd-tp-`.
pub(super) fn scratch_name(tag: &str) -> String {
    let name = format!("nd{tag}{:06}", std::process::id() % 1_000_000);
    assert!(name.len() < libc::IFNAMSIZ, "scratch TAP name {name} fits IFNAMSIZ");
    name
}

/// The persistent-TAP state `name` reads back through rtnetlink.
pub(super) fn tap_state(name: &str) -> TapLinkState {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.observe_persistent_tap(&owned).await })
        .unwrap_or_else(|error| panic!("reading back TAP {name}: {error}"))
}

/// Raise `name` administratively.
fn raise(name: &str) {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.set_link_up(&owned).await })
        .unwrap_or_else(|error| panic!("raising TAP {name}: {error}"));
}

/// Lower `name` administratively.
fn lower(name: &str) {
    let owned = name.to_owned();
    block_on_host_netlink(|| async move { Client::new()?.set_link_down(&owned).await })
        .unwrap_or_else(|error| panic!("lowering TAP {name}: {error}"));
}

/// A persistent single-queue TAP owned by the root launcher (uid 0), created
/// down, and deleted when dropped.
pub(super) struct ScratchTap {
    name: String,
}

impl ScratchTap {
    pub(super) fn create(tag: &str) -> Self {
        let name = scratch_name(tag);
        // A crashed earlier run of this pid is not possible, but a leftover
        // device under the same name would make every assertion meaningless.
        assert_eq!(tap_state(&name), TapLinkState::Absent, "scratch TAP {name} must not pre-exist");
        create_persistent_tap(&name, 0)
            .unwrap_or_else(|error| panic!("creating scratch TAP {name}: {error}"));
        Self { name }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for ScratchTap {
    fn drop(&mut self) {
        delete_link(&self.name);
    }
}

/// Deletes a scratch name that must stay absent, should a failing attach have
/// left a device behind.
pub(super) struct AbsentNameGuard(pub(super) String);

impl Drop for AbsentNameGuard {
    fn drop(&mut self) {
        delete_link(&self.0);
    }
}

/// Delete `name` if present; a failure is logged, never absorbed silently.
fn delete_link(name: &str) {
    let owned = name.to_owned();
    let deleted = block_on_host_netlink(|| async move { Client::new()?.del_link(&owned).await });
    if let Err(error) = deleted {
        eprintln!("cleanup: deleting scratch link {name} failed: {error}");
    }
}

/// `TUNGETIFF` on an attached queue descriptor.
pub(super) fn queue_flags(queue: BorrowedFd<'_>) -> u16 {
    // SAFETY: `ifreq` is a plain C struct; the all-zero pattern is valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    // SAFETY: `queue` is an open `/dev/net/tun` descriptor for the duration of
    // the call and `request` is a valid, writable `ifreq`.
    let rc = unsafe { libc::ioctl(queue.as_raw_fd(), libc::TUNGETIFF, &raw mut request) };
    assert_eq!(rc, 0, "TUNGETIFF on the queue: {}", std::io::Error::last_os_error());
    // SAFETY: `TUNGETIFF` writes `ifru_flags`.
    let flags = unsafe { request.ifr_ifru.ifru_flags };
    u16::from_ne_bytes(flags.to_ne_bytes())
}

/// The `iff:` interface a `/dev/net/tun` descriptor is attached to, as the
/// kernel reports it in `fdinfo`.
fn attached_interface(queue: BorrowedFd<'_>) -> Option<String> {
    let fdinfo = fs::read_to_string(format!("/proc/self/fdinfo/{}", queue.as_raw_fd()))
        .expect("reading the queue's fdinfo");
    fdinfo.lines().find_map(|line| line.strip_prefix("iff:").map(|iface| iface.trim().to_owned()))
}

fn descriptor_flags(queue: BorrowedFd<'_>) -> (libc::c_int, libc::c_int) {
    // SAFETY: `queue` is open for the duration of both `fcntl` reads, which
    // take no pointer arguments.
    let status = unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_GETFL) };
    // SAFETY: as above.
    let descriptor = unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_GETFD) };
    assert!(status >= 0 && descriptor >= 0, "fcntl on the queue failed");
    (status, descriptor)
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-38 — A queue is handed over only from an exact, down, persistent
/// TAP: the success path.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-03 (S-ND295-38)"]
fn a_down_persistent_tap_hands_over_exactly_one_vnet_header_queue() {
    require_root("a_down_persistent_tap_hands_over_exactly_one_vnet_header_queue");
    let tap = ScratchTap::create("qa");
    let before = tap_state(tap.name());
    assert_eq!(before, TapLinkState::Persistent { up: false, owner_uid: Some(0) });

    let queue = attach_tap_queue(tap.name()).expect("a down persistent TAP hands over a queue");

    assert_eq!(queue.name(), tap.name(), "the queue names its TAP");
    assert_eq!(
        queue_flags(queue.as_fd()),
        PERSISTENT_VNET_TAP_FLAGS,
        "the queue carries exactly the persistent single-queue vnet-header TAP flags"
    );
    assert_eq!(attached_interface(queue.as_fd()).as_deref(), Some(tap.name()));
    let (status, descriptor) = descriptor_flags(queue.as_fd());
    assert_eq!(status & libc::O_ACCMODE, libc::O_RDWR, "the queue is read-write");
    assert_ne!(status & libc::O_NONBLOCK, 0, "the queue is non-blocking");
    assert_ne!(descriptor & libc::FD_CLOEXEC, 0, "the queue is close-on-exec");
    assert_eq!(tap_state(tap.name()), before, "the attach neither raised nor re-owned the TAP");

    // Dropping the queue releases it: a second attach succeeds and the TAP
    // is still the same down persistent root-owned device.
    drop(queue);
    let again = attach_tap_queue(tap.name()).expect("a released queue can be attached again");
    assert_eq!(queue_flags(again.as_fd()), PERSISTENT_VNET_TAP_FLAGS);
    drop(again);
    assert_eq!(tap_state(tap.name()), before, "the TAP outlives every queue unchanged");
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-38 — Every attach precondition violation (a raised TAP, a TAP that
/// already has a queue, a name that would create a fresh device) is refused
/// with its own typed cause, and the attach changes no TAP.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 05-03 (S-ND295-38)"]
fn every_attach_precondition_violation_is_refused_with_its_own_typed_cause() {
    require_root("every_attach_precondition_violation_is_refused_with_its_own_typed_cause");

    // A raised TAP.
    let raised = ScratchTap::create("qb");
    raise(raised.name());
    let raised_state = tap_state(raised.name());
    assert_eq!(raised_state, TapLinkState::Persistent { up: true, owner_uid: Some(0) });
    match attach_tap_queue(raised.name()) {
        Err(TapQueueError::NotDown { name }) => assert_eq!(name, raised.name()),
        other => panic!("a raised TAP must be refused NotDown, got {other:?}"),
    }
    assert_eq!(tap_state(raised.name()), raised_state, "the refusal did not lower the TAP");
    // The refused attach closed its descriptor: once the TAP is down again a
    // fresh attach is not busy.
    lower(raised.name());
    drop(attach_tap_queue(raised.name()).expect("the NotDown refusal released its queue"));

    // A TAP that already has a queue.
    let busy = ScratchTap::create("qc");
    let held = attach_tap_queue(busy.name()).expect("the first queue attaches");
    let busy_state = tap_state(busy.name());
    match attach_tap_queue(busy.name()) {
        Err(TapQueueError::Attach { name, source }) => {
            assert_eq!(name, busy.name());
            assert_eq!(source.raw_os_error(), Some(libc::EBUSY), "the kernel's EBUSY is preserved");
        }
        other => panic!("a second attach must be refused Attach(EBUSY), got {other:?}"),
    }
    assert_eq!(queue_flags(held.as_fd()), PERSISTENT_VNET_TAP_FLAGS, "the held queue is intact");
    assert_eq!(tap_state(busy.name()), busy_state, "the refusal changed no TAP state");
    drop(held);

    // A name that would create a fresh, non-persistent device.
    let absent = scratch_name("qd");
    let _absent_guard = AbsentNameGuard(absent.clone());
    assert_eq!(tap_state(&absent), TapLinkState::Absent);
    match attach_tap_queue(&absent) {
        Err(TapQueueError::Flags { name, observed }) => {
            assert_eq!(name, absent);
            assert_eq!(observed & IFF_PERSIST, 0, "the fresh device is not persistent");
            assert_eq!(
                observed & REQUESTED_QUEUE_FLAGS,
                REQUESTED_QUEUE_FLAGS,
                "the observed flags are the requested ones without IFF_PERSIST: {observed:#06x}"
            );
        }
        other => panic!("an absent name must be refused Flags, got {other:?}"),
    }
    assert_eq!(
        tap_state(&absent),
        TapLinkState::Absent,
        "closing the refused queue destroyed the transient device"
    );
}

/// Outcome anchor: OUT-ND295-BORN-CAPTURED.
/// S-ND295-39 — A root-owned TAP refuses any process that holds no queue: an
/// unprivileged process with the VM uid and no capability is refused as not
/// permitted.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 06-02 (S-ND295-39)"]
fn a_root_owned_tap_refuses_an_unprivileged_attach_with_eperm() {
    require_root("a_root_owned_tap_refuses_an_unprivileged_attach_with_eperm");
    let tap = ScratchTap::create("qe");
    let owner_before = tap_state(tap.name());
    assert_eq!(owner_before, TapLinkState::Persistent { up: false, owner_uid: Some(0) });

    let verdict = unprivileged_child_attach(tap.name());
    assert_eq!(
        verdict,
        ChildVerdict::RefusedNotPermitted,
        "an unprivileged uid-4200 process must be refused Attach(EPERM) on a root-owned TAP"
    );

    // The refusal left the queue free and the owner unchanged.
    let queue = attach_tap_queue(tap.name()).expect("root attaches the still-free queue");
    assert_eq!(queue_flags(queue.as_fd()), PERSISTENT_VNET_TAP_FLAGS);
    drop(queue);
    assert_eq!(tap_state(tap.name()), owner_before, "the TAP owner still reads back uid 0");
}

/// The VM uid/gid the confined VMM runs as.
const VM_ID: libc::uid_t = 4200;

/// What the credential-dropped child observed. Each outcome is a distinct
/// exit status, so the parent names the exact failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildVerdict {
    RefusedNotPermitted,
    RefusedOtherwise,
    Attached,
    CredentialDropFailed,
    CapabilitiesRemain,
    StatusUnreadable,
    Panicked,
    Unknown(i32),
}

impl ChildVerdict {
    const fn code(self) -> i32 {
        match self {
            Self::RefusedNotPermitted => 0,
            Self::RefusedOtherwise => 10,
            Self::Attached => 11,
            Self::CredentialDropFailed => 12,
            Self::CapabilitiesRemain => 13,
            Self::StatusUnreadable => 15,
            Self::Panicked => 14,
            Self::Unknown(code) => code,
        }
    }

    const fn from_code(code: i32) -> Self {
        match code {
            0 => Self::RefusedNotPermitted,
            10 => Self::RefusedOtherwise,
            11 => Self::Attached,
            12 => Self::CredentialDropFailed,
            13 => Self::CapabilitiesRemain,
            14 => Self::Panicked,
            15 => Self::StatusUnreadable,
            other => Self::Unknown(other),
        }
    }
}

/// Fork a child that drops to uid/gid 4200 with no supplementary group and
/// an empty effective capability set, then attaches `name` through the
/// production port. The child never returns into the test harness: it ends
/// with `_exit`.
fn unprivileged_child_attach(name: &str) -> ChildVerdict {
    // SAFETY: the test process has no other thread holding a lock the child
    // needs; the child only runs `child_attach` and then `_exit`s.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
    if pid == 0 {
        let verdict =
            std::panic::catch_unwind(|| child_attach(name)).unwrap_or(ChildVerdict::Panicked);
        // SAFETY: `_exit` ends the forked child without running the parent's
        // atexit handlers or unwinding into the test harness.
        unsafe { libc::_exit(verdict.code()) }
    }
    let mut status = 0;
    // SAFETY: `pid` is this process's child and `status` is writable.
    let waited = unsafe { libc::waitpid(pid, &raw mut status, 0) };
    assert_eq!(waited, pid, "waitpid: {}", std::io::Error::last_os_error());
    assert!(libc::WIFEXITED(status), "the child ended abnormally (status {status:#x})");
    ChildVerdict::from_code(libc::WEXITSTATUS(status))
}

fn child_attach(name: &str) -> ChildVerdict {
    // SAFETY: plain credential syscalls on the calling (only) thread of the
    // child; the group list pointer is null with length zero.
    let dropped = unsafe {
        libc::setgroups(0, std::ptr::null()) == 0
            && libc::setresgid(VM_ID, VM_ID, VM_ID) == 0
            && libc::setresuid(VM_ID, VM_ID, VM_ID) == 0
    };
    if !dropped {
        eprintln!("child: credential drop failed: {}", std::io::Error::last_os_error());
        return ChildVerdict::CredentialDropFailed;
    }
    // Leaving uid 0 for a non-zero uid clears the permitted and effective sets;
    // read it back rather than assume it.
    let status = match fs::read_to_string("/proc/self/status") {
        Ok(status) => status,
        Err(error) => {
            eprintln!("child: reading /proc/self/status failed: {error}");
            return ChildVerdict::StatusUnreadable;
        }
    };
    let no_capabilities = status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .is_some_and(|mask| mask.trim().chars().all(|digit| digit == '0'));
    if !no_capabilities {
        eprintln!("child: effective capabilities remain after the drop");
        return ChildVerdict::CapabilitiesRemain;
    }
    match attach_tap_queue(name) {
        Err(TapQueueError::Attach { source, .. }) if source.raw_os_error() == Some(libc::EPERM) => {
            ChildVerdict::RefusedNotPermitted
        }
        Err(error) => {
            eprintln!("child: attach refused with an unexpected cause: {error:?}");
            ChildVerdict::RefusedOtherwise
        }
        Ok(_) => ChildVerdict::Attached,
    }
}
