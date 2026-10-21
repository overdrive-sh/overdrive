//! V-EV-UNLOAD's ULP negative control and unload stress, run inside the
//! guest-kit microVM (GH #295; MODULE-REFCOUNT, ADR-0198; ADR-0204 rule 5;
//! feature-delta § *Validation plan*, "V-EV-UNLOAD", "Placement"; T-1,
//! U-EV-UNLOAD).
//!
//! **Why a guest.** Both cells unload `ovd_fwd`, and the ULP control leaves
//! it unloadable until the next boot (a socket that had kTLS installed after
//! attach keeps its module reference for the boot, rule 5). They need a
//! kernel boot of their own: the guest-kit microVM, whose PID 1 is the
//! scripted peer, so nothing in the guest holds the module the peer loaded
//! at boot. The guest is powered off after the cell, which ends that boot.
//!
//! **Two halves in one file.** The host cases (`kvm-tests`, native metal)
//! boot a guest per case through
//! `overdrive_testing::guest_lane::run_scripted_in_guest_cell` and run
//! exactly one in-guest cell of this same `integration` binary in it, staged
//! by `cargo xtask guest-kit-prepare` (the `overdrive-fwd` in-guest lane).
//! The in-guest cells (`in_guest_*`) fail closed outside a scripted guest.
//! An in-guest cell that is still ignored reports `0 passed`, so its pending
//! marker keeps the host case red as well: activating a cell means removing
//! both markers.
//!
//! Lane: `cargo xtask metal run -- cargo nextest run -p overdrive-fwd
//! --features integration-tests,kvm-tests -E 'test(unload_in_guest)'
//! --run-ignored only`. CIDs from the [`FWD_UNLOAD_IN_GUEST`] band.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::GUEST_FORWARDER_MODULE_PATH;
use overdrive_core::guest_transport::forwarder::{AbortResult, FlowForwarder, PairMode, PairSide};
use overdrive_fwd::KernelFlowForwarder;
use overdrive_testing::guest_kit::cid_bands::FWD_UNLOAD_IN_GUEST;
use overdrive_testing::guest_kit::{GuestKit, WorkDir, host_module_file};
use overdrive_testing::guest_lane::{InGuestTests, run_scripted_in_guest_cell};
use overdrive_testing::guest_peer::CONTROL_PORT_PARAM;
use overdrive_testing::vm_fixture::default_staging_root;

use super::support::{
    END_WAIT, Vsock, attach_stream, install_tls_ulp, tcp_pair, udp_pair, vsock_pair, wait_end_of,
};
use super::unload::{MODULE_DIR, delete_nonblock, refcnt, wait_refcnt};

/// Where the ULP control's `tls` module sits in the guest image; the
/// launcher (`run-with-modules`) loads it before the cell.
const TLS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/tls.ko";
/// Guest RAM of a case.
const GUEST_MIB: u32 = 512;
/// The named hold window of the ULP control: how long a reference that must
/// NOT come back is watched. It is the bound within which the control's
/// positive half sees a non-ULP pair's references come back
/// ([`END_WAIT`], `wait_refcnt`), so "still held" is read over a window in
/// which a release would have shown.
const ULP_HOLD_WINDOW: Duration = END_WAIT;

// ---------------------------------------------------------------------------
// The in-guest side
// ---------------------------------------------------------------------------

/// Fails closed outside a scripted guest-kit guest (whose command line
/// carries the scripted peer's control parameter).
fn in_scripted_guest() {
    let cmdline = std::fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    assert!(
        cmdline.contains(CONTROL_PORT_PARAM),
        "not inside a scripted guest-kit guest: this cell unloads ovd_fwd and runs only as the \
         in-guest half of unload_in_guest (never on a host)"
    );
}

/// The guest's precondition: the peer loaded the kit's module at boot and
/// nothing holds it.
fn precondition_idle_guest_module() {
    in_scripted_guest();
    assert!(Path::new(MODULE_DIR).exists(), "precondition: the peer loaded ovd_fwd at boot");
    assert_eq!(refcnt(), 0, "precondition: nothing in the guest holds ovd_fwd");
}

/// Asserts `refcnt` reads `want` throughout `window` (a hold, watched at
/// 50 ms steps), for a reference that must not be given back.
fn assert_refcnt_holds(want: u64, window: Duration, why: &str) {
    let end = Instant::now() + window;
    loop {
        assert_eq!(refcnt(), want, "{why}");
        if Instant::now() >= end {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// `finit_module` of the kit's module in the guest; `Ok` or the errno.
fn load_guest_module() -> Result<(), i32> {
    let file = File::open(GUEST_FORWARDER_MODULE_PATH)
        .unwrap_or_else(|e| panic!("the kit's module at {GUEST_FORWARDER_MODULE_PATH}: {e}"));
    // SAFETY: a descriptor we own for the call and an empty NUL-terminated
    // parameter string; flags 0.
    let rc =
        unsafe { libc::syscall(libc::SYS_finit_module, file.as_raw_fd(), c"".as_ptr(), 0_i32) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) }
}

/// Every kernel log line so far (`/dev/kmsg`, read non-blocking to its end).
fn kernel_log() -> Vec<String> {
    let mut kmsg = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/kmsg")
        .expect("/dev/kmsg is readable as root");
    let mut lines = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        match kmsg.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => lines.push(String::from_utf8_lossy(&buf[..n]).into_owned()),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            // A record overwritten while reading: keep going.
            Err(e) if e.raw_os_error() == Some(libc::EPIPE) => {}
            Err(e) => panic!("read /dev/kmsg: {e}"),
        }
    }
    lines
}

/// The kernel log lines that name `ovd_fwd`.
fn module_log_lines() -> Vec<String> {
    kernel_log().into_iter().filter(|l| l.contains("ovd_fwd")).collect()
}

/// The fault markers a broken unload would leave in the log.
fn fault_lines() -> Vec<String> {
    const MARKERS: [&str; 3] = ["Oops", "general protection fault", "BUG: unable to handle"];
    kernel_log().into_iter().filter(|l| MARKERS.iter().any(|m| l.contains(m))).collect()
}

/// V-EV-UNLOAD's ULP negative control, in the guest (step 4). First the
/// positive half: a pair attached, aborted and its record read gives its
/// two references back (`refcnt` returns to the context's 1). Then kTLS is
/// installed on an attached TCP half (a test-only stimulus; no owner does
/// it, ADR-0200) and the pair is aborted and its record read: `refcnt`
/// stays one above the context's, the unload stays refused after the
/// context closes (`refcnt` 1, `EWOULDBLOCK`), the module stays loaded, and
/// the module logs the socket. The guest is then powered off, which ends
/// this boot of the module.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn in_guest_a_socket_given_a_ulp_after_attach_keeps_its_reference_for_the_boot() {
    precondition_idle_guest_module();
    let fwd = KernelFlowForwarder::open().expect("open a context");
    assert_eq!(refcnt(), 1, "the context's reference");

    // Positive half: without a ULP the references come back.
    let (v, _v_far) = vsock_pair();
    let (t, _t_far) = tcp_pair();
    let id = attach_stream(&fwd, &v, &t);
    assert_eq!(refcnt(), 3);
    assert_eq!(fwd.abort(id).expect("abort"), AbortResult::Aborted);
    let _ = wait_end_of(&fwd, id);
    wait_refcnt(1);

    // The ULP control.
    let logged_before = module_log_lines().len();
    let (v2, _v2_far) = vsock_pair();
    let (t2, _t2_far) = tcp_pair();
    let id2 = attach_stream(&fwd, &v2, &t2);
    install_tls_ulp(&t2);
    assert_eq!(fwd.abort(id2).expect("abort"), AbortResult::Aborted);
    let _ = wait_end_of(&fwd, id2);
    assert_refcnt_holds(
        2,
        ULP_HOLD_WINDOW,
        "the ULP socket keeps its reference above the context's (the positive half saw the \
         references come back without a ULP)",
    );
    drop(fwd);
    // The context's reference comes back (a positive transition, polled).
    wait_refcnt(1);
    assert_refcnt_holds(1, ULP_HOLD_WINDOW, "the ULP socket's reference outlives the context");
    assert_eq!(delete_nonblock(), Err(libc::EWOULDBLOCK), "the unload stays refused");
    assert!(Path::new(MODULE_DIR).exists(), "the module stays loaded");
    assert!(
        module_log_lines().len() > logged_before,
        "the module logs the socket that kept its reference"
    );
    assert!(fault_lines().is_empty(), "no fault: {:#?}", fault_lines());
    drop((v, t, v2, t2));
}

/// One attached pair and one datagram association of an iteration, with
/// their far ends.
struct Hooked {
    stream: (Vsock, Vsock, TcpStream, TcpStream),
    dgram: (Vsock, Vsock, UdpSocket, UdpSocket),
}

/// V-EV-UNLOAD's unload stress, in the guest (step 4): for a bounded
/// duration, a loop of open, attach (stream pairs and datagram
/// associations), end (abort and record), close and unload, while sender
/// threads on every CPU push bytes into every far end — so the forwarder
/// sends on every vsock and UDP half and `sock_wfree` runs from the guest's
/// virtio-vsock transmit work and the network stack while pairs end — and
/// another thread attempts `delete_module` continuously, reloading the
/// module whenever its attempt succeeds. Oracles: while the loop holds a
/// context, the module is never gone and the context answers; the kernel
/// log gains no fault (no oops); at least
/// one unload succeeded between iterations, so the loop exercised the
/// unload.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn in_guest_the_unload_races_pair_ends_with_senders_on_every_half_and_never_faults() {
    const DURATION: Duration = Duration::from_secs(60);
    const PAIRS: usize = 4;
    precondition_idle_guest_module();
    let faults_before = fault_lines().len();
    let stop = Arc::new(AtomicBool::new(false));
    let unloads = Arc::new(AtomicU64::new(0));
    let deleter = thread::spawn({
        let stop = Arc::clone(&stop);
        let unloads = Arc::clone(&unloads);
        move || {
            while !stop.load(Ordering::SeqCst) {
                match delete_nonblock() {
                    Ok(()) => {
                        unloads.fetch_add(1, Ordering::SeqCst);
                        load_guest_module().unwrap_or_else(|e| panic!("reload: errno {e}"));
                    }
                    Err(e) if e == libc::EWOULDBLOCK || e == libc::EBUSY || e == libc::ENOENT => {}
                    Err(e) => panic!("delete_module: errno {e}"),
                }
            }
        }
    });
    let cpus = thread::available_parallelism().map_or(2, std::num::NonZero::get);
    let end = Instant::now() + DURATION;
    let mut iterations = 0u64;
    while Instant::now() < end {
        // The deleter may have the module out for an instant: retry the open.
        let fwd = loop {
            match KernelFlowForwarder::open() {
                Ok(fwd) => break fwd,
                Err(_) if Instant::now() < end => thread::sleep(Duration::from_millis(1)),
                Err(e) => panic!("no context could be opened: {e}"),
            }
        };
        let hooked: Vec<Hooked> = (0..PAIRS)
            .map(|_| {
                let (v, v_far) = vsock_pair();
                let (t, t_far) = tcp_pair();
                attach_stream(&fwd, &v, &t);
                let (dv, dv_far) = vsock_pair();
                let (u, u_far) = udp_pair();
                // A sender never blocks: a full buffer with no reset must end
                // the iteration at `senders_stop`, not hang the scope's join.
                v_far.set_nonblocking(true);
                t_far.set_nonblocking(true).expect("non-blocking TCP far end");
                u_far.set_nonblocking(true).expect("non-blocking UDP far end");
                fwd.attach(&dv, &u, PairMode::Datagram { udp: PairSide::B })
                    .expect("attach a datagram association");
                Hooked { stream: (v, v_far, t, t_far), dgram: (dv, dv_far, u, u_far) }
            })
            .collect();
        let senders_stop = Arc::new(AtomicBool::new(false));
        thread::scope(|s| {
            for chunk in hooked.chunks(PAIRS.div_ceil(cpus).max(1)) {
                let senders_stop = Arc::clone(&senders_stop);
                s.spawn(move || {
                    let payload = vec![0x5a_u8; 16 * 1024];
                    while !senders_stop.load(Ordering::SeqCst) {
                        for h in chunk {
                            let _ = (&h.stream.3).write(&payload);
                            let _ = (&h.stream.1).write(&payload);
                            let _ = h.dgram.3.send(&payload[..1200]);
                        }
                    }
                });
            }
            thread::sleep(Duration::from_millis(50));
            assert!(Path::new(MODULE_DIR).exists(), "the module is never gone while held");
            fwd.inventory().expect("the held context answers");
            fwd.abort_all().expect("end every pair while senders run");
            senders_stop.store(true, Ordering::SeqCst);
        });
        while fwd.take_end().expect("take_end").is_some() {}
        drop(fwd);
        drop(hooked);
        iterations += 1;
    }
    stop.store(true, Ordering::SeqCst);
    deleter.join().expect("the deleter thread");
    if !Path::new(MODULE_DIR).exists() {
        load_guest_module().unwrap_or_else(|e| panic!("final reload: errno {e}"));
    }
    let faults = fault_lines();
    assert_eq!(faults.len(), faults_before, "no fault in the kernel log: {faults:#?}");
    assert!(iterations > 10, "the loop ran ({iterations} iterations)");
    assert!(unloads.load(Ordering::SeqCst) > 0, "the unload succeeded between iterations");
}

// ---------------------------------------------------------------------------
// The host side
// ---------------------------------------------------------------------------

/// Boots one scripted guest at the band's `slot` with `modules` and runs
/// the in-guest `cell` of this binary in it.
fn run_in_guest(slot: u32, cell: &str, with_tls: bool) {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
    let tests = InGuestTests::prepared("overdrive-fwd", "integration").expect(
        "precondition: the overdrive-fwd in-guest binary is staged (cargo xtask guest-kit-prepare)",
    );
    let work = WorkDir::create(&staging, "fwd-unload-in-guest").expect("work dir");
    let modules = if with_tls {
        vec![
            host_module_file(&kit, work.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))
                .expect("precondition: the running kernel ships the tls module"),
        ]
    } else {
        Vec::new()
    };
    let run = run_scripted_in_guest_cell(
        &kit,
        &tests,
        cell,
        FWD_UNLOAD_IN_GUEST.raw(slot),
        work.path(),
        &modules,
        GUEST_MIB,
    )
    .expect("the guest boots and runs the cell");
    assert!(
        run.passed(),
        "the in-guest cell {cell} must run and pass (exit {}); guest console:\n{}",
        run.exit_status,
        run.console
    );
}

/// V-EV-UNLOAD's ULP negative control, in a guest of its own.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn a_ulp_socket_keeps_the_module_for_the_boot_in_a_real_guest() {
    run_in_guest(
        0,
        "integration::unload_in_guest::in_guest_a_socket_given_a_ulp_after_attach_keeps_its_reference_for_the_boot",
        true,
    );
}

/// V-EV-UNLOAD's unload stress, in a guest of its own.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 4"]
fn the_unload_stress_never_faults_in_a_guest() {
    run_in_guest(
        1,
        "integration::unload_in_guest::in_guest_the_unload_races_pair_ends_with_senders_on_every_half_and_never_faults",
        false,
    );
}
