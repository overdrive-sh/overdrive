//! V-EV-UNLOAD — the real-kernel cell of the module's unload safety
//! (MODULE-REFCOUNT, 2026-10-20; EVMOD-LIFE as amended, ADR-0198; ADR-0204
//! rule 5; feature-delta § *Validation plan*, "V-EV-UNLOAD").
//!
//! Oracles: `delete_module("ovd_fwd", O_NONBLOCK)` — refused `EWOULDBLOCK`
//! while any module reference is held (`kernel/module/main.c:744-751`, `:828
//! @ v7.0`), `0` at zero references — and `/sys/module/ovd_fwd/refcnt`
//! (`:889-896`). An open context holds one reference (the misc device's
//! `fops.owner`); every hooked socket holds one (`__module_get` at attach or
//! watch, `module_put` as the free work's last statement).
//!
//! **Placement.** The cell unloads the module, so it runs only where no other
//! run depends on the loaded module: the native metal host under its
//! exclusive lease (`cargo xtask metal run --`, `kvm-tests`), never the
//! shared Lima VM, where an unload would remove a module another workspace's
//! `serve` test holds. Within the metal run every `host-kernel-shared` test
//! (this whole binary and every `serve` test binary) is serialized, so no
//! sibling holds a context while a case runs. Each case first checks that
//! nothing holds the module (`refcnt` 0) and that the loaded module is this
//! workspace's build, and ends with this workspace's build loaded again, so
//! the lane's state is as it found it.
//!
//! **Not here.** The ULP negative control (a socket that had kTLS installed
//! after attach keeps its reference for the boot) and the unload stress each
//! need a kernel boot of their own — the ULP control leaves the module
//! unloadable until a reboot — and run inside the guest-kit microVM
//! (`unload_in_guest.rs`, T-1, U-EV-UNLOAD), not on the metal host.
//!
//! **The step-K version oracle.** The step-K context cell checks the refused
//! unload by the module staying listed and a reopened context; the request
//! number of `OVD_FWD_VERSION` is written with the adapter in step 4
//! (`src/uapi.rs`), so "the context still answers `OVD_FWD_VERSION`" is the
//! step-4 cell [`an_open_context_answers_its_version_while_the_unload_is_refused`].

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::time::Instant;

use nix::libc;
use overdrive_core::guest_transport::forwarder::{FlowForwarder, PairMode, PairSide};
use overdrive_fwd::KernelFlowForwarder;
use overdrive_testing::forwarder_module::{built_module_path, kmod_dir, require_forwarder_module};

use super::support::{
    END_WAIT, attach_stream, open_context, tcp_pair, udp_pair, vsock_pair, wait_end, wait_end_of,
};

pub(super) const MODULE_DIR: &str = "/sys/module/ovd_fwd";
const REFCNT: &str = "/sys/module/ovd_fwd/refcnt";

/// The module's reference count.
pub(super) fn refcnt() -> u64 {
    std::fs::read_to_string(REFCNT)
        .unwrap_or_else(|e| panic!("precondition: ovd_fwd is loaded ({REFCNT}: {e})"))
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("{REFCNT} is a number: {e}"))
}

/// Waits (bounded by [`END_WAIT`]) until `refcnt` reads `want`: the free
/// work drops a hooked socket's reference after an RCU grace period.
pub(super) fn wait_refcnt(want: u64) {
    let deadline = Instant::now() + END_WAIT;
    loop {
        let got = refcnt();
        if got == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "refcnt stayed {got}, expected {want} within {END_WAIT:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// `delete_module("ovd_fwd", O_NONBLOCK)`: `Ok` or the errno.
pub(super) fn delete_nonblock() -> Result<(), i32> {
    // SAFETY: a valid NUL-terminated module name and the O_NONBLOCK flag; the
    // kernel's own reference check decides whether anything is unloaded.
    let rc =
        unsafe { libc::syscall(libc::SYS_delete_module, c"ovd_fwd".as_ptr(), libc::O_NONBLOCK) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().expect("errno"))
    }
}

/// This workspace's built module for the running kernel.
fn workspace_module() -> PathBuf {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").expect("osrelease");
    built_module_path(&kmod_dir(), release.trim())
}

/// `finit_module` of this workspace's build; panics naming the errno.
fn load_workspace_module() {
    let path = workspace_module();
    let file = std::fs::File::open(&path)
        .unwrap_or_else(|e| panic!("precondition: {} is built ({e})", path.display()));
    // SAFETY: a descriptor we own for the call and an empty NUL-terminated
    // parameter string; flags 0.
    let rc =
        unsafe { libc::syscall(libc::SYS_finit_module, file.as_raw_fd(), c"".as_ptr(), 0_i32) };
    assert_eq!(
        rc,
        0,
        "finit_module({}) after the unload: {}",
        path.display(),
        std::io::Error::last_os_error()
    );
}

/// Opens one forwarder context on the device directly (step K has the
/// device and no adapter yet): one module reference while it is open.
fn open_raw_context() -> OwnedFd {
    // SAFETY: a valid NUL-terminated path; the returned descriptor is owned
    // below exactly once.
    let fd = unsafe { libc::open(c"/dev/ovd_fwd".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    assert!(fd >= 0, "open /dev/ovd_fwd: {}", std::io::Error::last_os_error());
    // SAFETY: `fd` is a fresh descriptor this function owns.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// The case's preconditions: this workspace's build is loaded and nothing
/// holds it.
fn precondition_idle_workspace_module() {
    require_forwarder_module().unwrap_or_else(|e| panic!("precondition: {e}"));
    assert_eq!(
        refcnt(),
        0,
        "precondition: nothing holds ovd_fwd (no serve test or other run concurrently; this \
         cell runs only on the metal host under its lease)"
    );
}

/// Step K, the context half. With a context open, `refcnt` is 1 and the
/// unload is refused `EWOULDBLOCK` with the module still present; once the
/// context is closed, `refcnt` is 0, the unload succeeds and the module is
/// gone; this workspace's build then loads again and a new context opens.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step K"]
fn an_open_context_refuses_the_unload_and_its_close_lets_it_succeed() {
    precondition_idle_workspace_module();

    let context = open_raw_context();
    assert_eq!(refcnt(), 1, "an open context holds exactly one module reference");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "unload refused while a context is open");
    assert!(std::path::Path::new(MODULE_DIR).exists(), "the refused unload left the module loaded");
    assert_eq!(refcnt(), 1, "the refused unload changed no reference");

    drop(context);
    assert_eq!(refcnt(), 0, "the context's close released its reference");
    assert_eq!(delete_nonblock(), Ok(()), "the unload succeeds at zero references");
    assert!(!std::path::Path::new(MODULE_DIR).exists(), "the module is gone after the unload");

    load_workspace_module();
    require_forwarder_module().unwrap_or_else(|e| panic!("the reload is this workspace's: {e}"));
    let again = open_raw_context();
    assert_eq!(refcnt(), 1, "the reloaded module serves a new context");
    drop(again);
    assert_eq!(refcnt(), 0);
}

/// Step 4, the socket half. With one context open, an attached pair adds two
/// references (one per hooked socket, `refcnt` 3) and a watch one more (4);
/// the unload is refused `EWOULDBLOCK` at each. After `ABORT_ALL` and the
/// records are read, `refcnt` returns to 1 within the bounded wait (the
/// grace period and the free work). A pair still live when the context
/// closes (the release path) also gives its references back: `refcnt` 0.
/// The unload then succeeds, the module is loaded again, and the forwarder
/// probe passes on it.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn hooked_sockets_hold_the_module_until_their_free_work_and_a_reload_then_serves() {
    precondition_idle_workspace_module();

    let fwd = open_context();
    assert_eq!(refcnt(), 1, "the context's reference");
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    attach_stream(&fwd, &v, &t);
    assert_eq!(refcnt(), 3, "one reference per hooked half");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "refused with a live pair");
    let (w, _w_far) = vsock_pair();
    fwd.watch(&w).expect("watch a connected vsock half");
    assert_eq!(refcnt(), 4, "the watched socket's reference");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "refused with a live watch");

    assert_eq!(fwd.abort_all().expect("abort_all"), 2, "the pair and the watch were aborted");
    let first = wait_end(&fwd);
    let second = wait_end(&fwd);
    assert_ne!(first.id, second.id, "one record per id");
    wait_refcnt(1);
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "the context still holds one reference");

    // The release path: a pair live when the context closes.
    let (v2, _v2_far) = vsock_pair();
    let (t2, _t2_far) = tcp_pair();
    attach_stream(&fwd, &v2, &t2);
    assert_eq!(refcnt(), 3);
    drop(fwd);
    wait_refcnt(0);

    assert_eq!(delete_nonblock(), Ok(()), "the unload succeeds once every reference is gone");
    load_workspace_module();
    let reloaded = KernelFlowForwarder::open().expect("the reloaded module opens a context");
    reloaded.probe().expect("the forwarder probe passes on the reloaded module");
    drop(reloaded);
    wait_refcnt(0);
    drop((v, t, w, v2, t2));
}

/// Step 4, the socket half in datagram mode: an attached datagram
/// association (a vsock half carrying frames and a UDP half) holds one
/// reference per hooked socket — `refcnt` 3 with the context's — and the
/// unload is refused `EWOULDBLOCK`; after its abort and once its end record
/// is read, `refcnt` returns to the context's 1 within the bounded wait.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_datagram_association_holds_one_reference_per_hooked_socket_until_its_record_is_read() {
    precondition_idle_workspace_module();

    let fwd = open_context();
    assert_eq!(refcnt(), 1, "the context's reference");
    let (v, _v_far) = vsock_pair();
    let (u, _u_far) = udp_pair();
    let id = fwd
        .attach(&v, &u, PairMode::Datagram { udp: PairSide::B })
        .expect("attach a datagram association");
    assert_eq!(refcnt(), 3, "one reference per hooked half of the association");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "refused with a live association");

    fwd.abort(id).expect("abort the association");
    let end = wait_end_of(&fwd, id);
    assert_eq!(end.id, id, "the association's record was read");
    wait_refcnt(1);
    drop(fwd);
    wait_refcnt(0);
    drop((v, u));
}

/// Step 4, the context half's full oracle: with a context open the unload is
/// refused `EWOULDBLOCK`, the module still answers `OVD_FWD_VERSION` (the
/// forwarder probe's `Version` stage, on the probe's own context, passes
/// with every later stage) and the held context still answers an ioctl
/// (`inventory()`). After the context's close the unload succeeds and this
/// workspace's build loads again.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn an_open_context_answers_its_version_while_the_unload_is_refused() {
    precondition_idle_workspace_module();

    let fwd = KernelFlowForwarder::open().expect("open a context");
    assert_eq!(refcnt(), 1, "the context's reference");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "refused while the context is open");
    fwd.probe().expect("the module answers OVD_FWD_VERSION and serves the probe");
    let inventory = fwd.inventory().expect("the held context answers inventory()");
    assert_eq!((inventory.live_pairs, inventory.live_watches), (0, 0));
    wait_refcnt(1);
    drop(fwd);
    wait_refcnt(0);
    assert_eq!(delete_nonblock(), Ok(()), "the unload succeeds once the context is closed");
    load_workspace_module();
    require_forwarder_module().unwrap_or_else(|e| panic!("the reload is this workspace's: {e}"));
}
