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
//! **Two roles in one test entry.** On the host (`kvm-tests`, native metal),
//! each entry boots a guest through
//! `overdrive_testing::guest_lane::run_scripted_in_guest_cell` and selects
//! that same exact entry in this staged `integration` binary. Inside the
//! scripted guest the entry runs its private in-guest body. The runner
//! accepts only `test result: ok. 1 passed`, so selecting no guest test can
//! never pass vacuously.
//!
//! Lane: `cargo xtask metal run -- cargo nextest run -p overdrive-fwd
//! --features integration-tests,kvm-tests -E 'test(unload_in_guest)'
//! --no-fail-fast`. CIDs from the [`FWD_UNLOAD_IN_GUEST`] band.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use nix::libc;
use overdrive_core::guest_transport::GUEST_FORWARDER_MODULE_PATH;
use overdrive_core::guest_transport::forwarder::{
    AbortResult, FlowForwarder, ForwardId, ForwarderError, PairMode, PairSide,
};
use overdrive_core::guest_transport::wire::DatagramFrame;
use overdrive_fwd::KernelFlowForwarder;
use overdrive_testing::guest_kit::cid_bands::FWD_UNLOAD_IN_GUEST;
use overdrive_testing::guest_kit::{GuestKit, WorkDir, host_module_file};
use overdrive_testing::guest_lane::{InGuestTests, run_scripted_in_guest_cell};
use overdrive_testing::guest_peer::CONTROL_PORT_PARAM;
use overdrive_testing::vm_fixture::default_staging_root;

use super::support::{
    END_WAIT, Vsock, VsockListener, attach_stream, install_tls_ulp, tcp_pair, udp_pair, vsock_pair,
    wait_end_of,
};
use super::unload::{MODULE_DIR, delete_nonblock, refcnt, wait_refcnt};

/// Where the ULP control's `tls` module sits in the guest image; the
/// launcher (`run-with-modules`) loads it before the cell.
const TLS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/tls.ko";
/// Guest RAM of a case.
const GUEST_MIB: u32 = 4_096;
/// The named hold window of the ULP control: how long a reference that must
/// NOT come back is watched. It is the bound within which the control's
/// positive half sees a non-ULP pair's references come back
/// ([`END_WAIT`], `wait_refcnt`), so "still held" is read over a window in
/// which a release would have shown.
const ULP_HOLD_WINDOW: Duration = END_WAIT;

// ---------------------------------------------------------------------------
// The in-guest side
// ---------------------------------------------------------------------------

/// Whether this test binary is running inside a scripted guest-kit guest
/// (whose command line carries the scripted peer's control parameter).
fn in_scripted_guest() -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    cmdline.contains(CONTROL_PORT_PARAM)
}

/// The guest's precondition: the peer loaded the kit's module at boot and
/// nothing holds it.
fn precondition_idle_guest_module() {
    assert!(
        in_scripted_guest(),
        "not inside a scripted guest-kit guest: this cell unloads ovd_fwd and runs only as the \
         in-guest half of unload_in_guest (never on a host)"
    );
    assert!(Path::new(MODULE_DIR).exists(), "precondition: the peer loaded ovd_fwd at boot");
    assert_eq!(refcnt(), 0, "precondition: nothing in the guest holds ovd_fwd");
}

/// Raises the soft descriptor limit to the hard limit for the bounded lane
/// topology calculated by the stress body.
fn raise_nofile(at_least: u64) {
    // SAFETY: valid rlimit out-parameter.
    let mut lim: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: valid rlimit out-parameter.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut lim) }, 0, "getrlimit");
    assert!(lim.rlim_max >= at_least, "precondition: RLIMIT_NOFILE hard limit >= {at_least}");
    lim.rlim_cur = lim.rlim_max;
    // SAFETY: valid rlimit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) }, 0, "setrlimit");
}

/// Pins this sender to one of the guest's CPUs, so a successful byte count
/// proves that CPU actually drove receive load during the stress.
fn pin_current_thread(cpu: usize) {
    // SAFETY: `set` is initialized before the syscall and names a CPU in the
    // fixture's contiguous 0..vCPU range.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
    }
    // SAFETY: PID 0 means this thread; `set` is a live cpu_set_t.
    assert_eq!(
        unsafe {
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw const set)
        },
        0,
        "pin sender to guest CPU {cpu}: {}",
        std::io::Error::last_os_error()
    );
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

/// Two stream pairs joined by one vsock connection. Each TCP far end drives
/// one direction through both ending pairs, so one vsock connection supplies
/// two independently identified pair lifetimes.
struct StreamChain {
    ids: [ForwardId; 2],
    _vsock: (Vsock, Vsock),
    tcp_a: (TcpStream, TcpStream),
    tcp_b: (TcpStream, TcpStream),
}

/// One datagram association retained in every lane so unload races both
/// stream pairs and UDP-backed associations.
struct DatagramHook {
    id: ForwardId,
    _vsock_half: Vsock,
    vsock_far: Vsock,
    _udp_half: UdpSocket,
    udp_far: UdpSocket,
}

#[derive(Clone, Copy)]
enum TrafficItem<'a> {
    Stream(&'a StreamChain),
    Datagram(&'a DatagramHook),
}

impl TrafficItem<'_> {
    fn send(&self, payload: &[u8], frame: &[u8]) -> u64 {
        let lengths = match self {
            Self::Stream(chain) => {
                [(&chain.tcp_a.1).write(payload).ok(), (&chain.tcp_b.1).write(payload).ok()]
            }
            Self::Datagram(hook) => {
                [hook.udp_far.send(&payload[..1200]).ok(), (&hook.vsock_far).write(frame).ok()]
            }
        };
        lengths
            .into_iter()
            .flatten()
            .map(|length| u64::try_from(length).expect("write length fits u64"))
            .sum()
    }
}

/// One independently locked control lane. IDs are unique within their
/// context; lanes let the stress overlap the per-context ABORT cleanup wait
/// without changing the module's serialized context-management contract.
struct StressLane {
    fwd: KernelFlowForwarder,
    streams: Vec<StreamChain>,
    datagram: DatagramHook,
    ids: Vec<ForwardId>,
}

impl StressLane {
    fn attach(fwd: KernelFlowForwarder, stream_chains: usize) -> Self {
        let vsock_listener = VsockListener::bind();
        let tcp_listener = TcpListener::bind("127.0.0.1:0").expect("stress TCP listener");
        let tcp_addr = tcp_listener.local_addr().expect("stress TCP listener address");
        let tcp_pair = || {
            let far = TcpStream::connect(tcp_addr).expect("stress TCP connect");
            let (half, _) = tcp_listener.accept().expect("stress TCP accept");
            (half, far)
        };
        let streams: Vec<StreamChain> = (0..stream_chains)
            .map(|_| {
                let vsock_b = Vsock::connect(vsock_listener.port());
                let vsock_a = vsock_listener.accept();
                let tcp_a = tcp_pair();
                let tcp_b = tcp_pair();
                tcp_a.1.set_nonblocking(true).expect("non-blocking TCP far end");
                tcp_b.1.set_nonblocking(true).expect("non-blocking TCP far end");
                let ids = [
                    attach_stream(&fwd, &vsock_a, &tcp_a.0),
                    attach_stream(&fwd, &vsock_b, &tcp_b.0),
                ];
                StreamChain { ids, _vsock: (vsock_a, vsock_b), tcp_a, tcp_b }
            })
            .collect();
        let vsock_far = Vsock::connect(vsock_listener.port());
        let vsock_half = vsock_listener.accept();
        let (udp_half, udp_far) = udp_pair();
        vsock_far.set_nonblocking(true);
        udp_far.set_nonblocking(true).expect("non-blocking UDP far end");
        let datagram_id = fwd
            .attach(&vsock_half, &udp_half, PairMode::Datagram { udp: PairSide::B })
            .expect("attach a datagram association");
        let datagram = DatagramHook {
            id: datagram_id,
            _vsock_half: vsock_half,
            vsock_far,
            _udp_half: udp_half,
            udp_far,
        };
        let mut ids: Vec<ForwardId> = streams.iter().flat_map(|item| item.ids).collect();
        ids.push(datagram.id);
        let expected: BTreeSet<ForwardId> = ids.iter().copied().collect();
        assert_eq!(expected.len(), ids.len(), "ids are never reused within a context");
        Self { fwd, streams, datagram, ids }
    }

    fn end_from_peers(&self) {
        for stream in &self.streams {
            let _ = stream.tcp_a.1.shutdown(std::net::Shutdown::Both);
            let _ = stream.tcp_b.1.shutdown(std::net::Shutdown::Both);
        }
        // SAFETY: a plain shutdown on the owned far descriptor; ABORT may
        // have won the race already, so ENOTCONN is an accepted loser.
        let _ =
            unsafe { libc::shutdown(self.datagram.vsock_far.as_fd().as_raw_fd(), libc::SHUT_RDWR) };
    }
}

/// V-EV-SAFE and V-EV-UNLOAD's unload stress, in the guest (step 4): for a bounded
/// duration, a loop of open, attach (stream pairs and datagram
/// associations), end (abort and record), close and unload, while sender
/// threads on every CPU push bytes into every far end — so the forwarder
/// sends on every vsock and UDP half and `sock_wfree` runs from the guest's
/// virtio-vsock transmit work and the network stack while pairs end — and
/// another thread attempts `delete_module` continuously, reloading the
/// module whenever its attempt succeeds. Oracles: while the loop holds a
/// context, the module is never gone and the context answers; every actual
/// end record is counted once by id; `ABORT` races `PAIR_STATE` on those ids;
/// every pinned CPU accepts traffic; the kernel log gains no fault (no oops);
/// at least 100,000 pairs end; at least
/// one unload succeeded between iterations, so the loop exercised the
/// unload.
/// CONTRACT_SHAPE: bounded-change.
fn in_guest_the_unload_races_pair_ends_with_senders_on_every_half_and_never_faults() {
    const DURATION: Duration = Duration::from_secs(60);
    // OVD_FWD_ABORT intentionally holds one context's ops_lock until the id
    // leaves the table and all of that context's cleanup has completed. Use
    // independent contexts so those waits overlap, as distinct owners do,
    // while retaining singular ABORT and PAIR_STATE calls for every id.
    const CONTROL_LANES: usize = 88;
    const STREAM_CHAINS_PER_LANE: usize = 8;
    const TRAFFIC_ITEMS_PER_ITERATION: usize = CONTROL_LANES * (STREAM_CHAINS_PER_LANE + 1);
    const PAIR_END_MIN: u64 = 100_000;
    const TRAFFIC_START_BOUND: Duration = Duration::from_secs(1);
    // Pin one tight-loop sender to every vCPU the disposable guest exposes.
    precondition_idle_guest_module();
    let descriptors = CONTROL_LANES * (STREAM_CHAINS_PER_LANE * 6 + 4) + 64;
    raise_nofile(u64::try_from(descriptors).expect("descriptor count fits u64"));
    let faults_before = fault_lines().len();
    let stop = Arc::new(AtomicBool::new(false));
    let unloads = Arc::new(AtomicU64::new(0));
    let pair_state_calls = AtomicU64::new(0);
    let pair_state_live = AtomicU64::new(0);
    let pair_state_ended = AtomicU64::new(0);
    let abort_won = AtomicU64::new(0);
    let abort_already_ended = AtomicU64::new(0);
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
    let cpus = thread::available_parallelism().expect("guest CPU count").get();
    assert!(cpus >= 2, "the V-EV-SAFE fixture needs at least two guest vCPUs");
    let end = Instant::now() + DURATION;
    let mut iterations = 0u64;
    let mut pair_ends = 0u64;
    let mut bytes_by_cpu = vec![0u64; cpus];
    'stress: while Instant::now() < end {
        // The deleter may have the module out for an instant: retry the open.
        let fwd = loop {
            match KernelFlowForwarder::open() {
                Ok(fwd) => break fwd,
                Err(ForwarderError::DeviceAbsent { source })
                    if source.kind() == ErrorKind::NotFound
                        || source.raw_os_error() == Some(libc::ENODEV) =>
                {
                    if Instant::now() >= end {
                        break 'stress;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("no context could be opened: {e:?}"),
            }
        };
        let mut contexts = Vec::with_capacity(CONTROL_LANES);
        contexts.push(fwd);
        for _ in 1..CONTROL_LANES {
            contexts.push(KernelFlowForwarder::open().expect("module held by the first context"));
        }
        let lanes = thread::scope(|scope| {
            contexts
                .into_iter()
                .map(|context| {
                    scope.spawn(move || StressLane::attach(context, STREAM_CHAINS_PER_LANE))
                })
                .map(|handle| handle.join().expect("stress lane setup"))
                .collect::<Vec<_>>()
        });
        assert!(Path::new(MODULE_DIR).exists(), "the module is never gone while held");
        for lane in &lanes {
            let before = lane.fwd.inventory().expect("the held context answers");
            assert_eq!(
                (before.live_pairs, before.live_watches, before.queued_ends, before.attached),
                (
                    u32::try_from(lane.ids.len()).expect("lane count fits u32"),
                    0,
                    0,
                    u64::try_from(lane.ids.len()).expect("lane count fits u64")
                ),
                "every id is live before the concurrent control calls"
            );
        }
        let senders_stop = Arc::new(AtomicBool::new(false));
        let bytes_this_iteration: Vec<AtomicU64> = (0..cpus).map(|_| AtomicU64::new(0)).collect();
        let peer_end_start = Barrier::new(CONTROL_LANES * 2);
        let abort_start = Barrier::new(CONTROL_LANES * 2);
        let traffic: Vec<TrafficItem<'_>> = lanes
            .iter()
            .flat_map(|lane| {
                lane.streams
                    .iter()
                    .map(TrafficItem::Stream)
                    .chain(std::iter::once(TrafficItem::Datagram(&lane.datagram)))
            })
            .collect();
        let traffic_started = thread::scope(|s| {
            for (cpu, (accepted, chunk)) in bytes_this_iteration
                .iter()
                .zip(traffic.chunks(TRAFFIC_ITEMS_PER_ITERATION.div_ceil(cpus)))
                .enumerate()
            {
                let senders_stop = Arc::clone(&senders_stop);
                s.spawn(move || {
                    pin_current_thread(cpu);
                    let payload = vec![0x5a_u8; 16 * 1024];
                    let frame = DatagramFrame::new(&payload[..1200])
                        .expect("1,200-byte datagram fits")
                        .encode();
                    while !senders_stop.load(Ordering::SeqCst) {
                        for item in chunk {
                            accepted.fetch_add(item.send(&payload, &frame), Ordering::Relaxed);
                        }
                    }
                });
            }
            // The senders are tight non-blocking loops. Wait only until each
            // pinned CPU has delivered bytes, then keep them running through
            // the control race. This is a safety workload; the throughput
            // benchmark remains GH #326.
            let traffic_deadline = Instant::now() + TRAFFIC_START_BOUND;
            while bytes_this_iteration.iter().any(|count| count.load(Ordering::Relaxed) == 0)
                && Instant::now() < traffic_deadline
            {
                thread::yield_now();
            }
            let traffic_started =
                bytes_this_iteration.iter().all(|count| count.load(Ordering::Relaxed) > 0);

            // Start singular ABORT and PAIR_STATE paths together on every
            // lane. Each lane preserves the same-id race; independent
            // contexts overlap the cleanup waits that ops_lock serializes.
            let mut controls = Vec::with_capacity(CONTROL_LANES * 3);
            for lane in &lanes {
                controls.push(s.spawn(|| {
                    peer_end_start.wait();
                    lane.end_from_peers();
                    abort_start.wait();
                }));
                controls.push(s.spawn(|| {
                    peer_end_start.wait();
                    for &id in &lane.ids {
                        let live = lane.fwd.pair_state(id).expect("PAIR_STATE while the id ends");
                        pair_state_calls.fetch_add(1, Ordering::Relaxed);
                        if live.is_some() {
                            pair_state_live.fetch_add(1, Ordering::Relaxed);
                        } else {
                            pair_state_ended.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }));
                controls.push(s.spawn(|| {
                    // Every peer has issued its end stimulus before singular
                    // ABORT begins; callbacks/free work remain concurrent.
                    abort_start.wait();
                    for &id in &lane.ids {
                        match lane.fwd.abort(id).expect("ABORT while the id ends") {
                            AbortResult::Aborted => {
                                abort_won.fetch_add(1, Ordering::Relaxed);
                            }
                            AbortResult::AlreadyEnded => {
                                abort_already_ended.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }));
            }
            for control in controls {
                control.join().expect("the control race thread");
            }
            senders_stop.store(true, Ordering::SeqCst);
            traffic_started
        });
        assert!(traffic_started, "every pinned guest CPU delivered bytes before the control race");
        for (total, current) in bytes_by_cpu.iter_mut().zip(&bytes_this_iteration) {
            let accepted = current.load(Ordering::Relaxed);
            assert!(accepted > 0, "the sender pinned to this guest CPU delivered bytes");
            *total = total.checked_add(accepted).expect("per-CPU byte count fits u64");
        }
        for lane in &lanes {
            for &id in &lane.ids {
                assert!(
                    lane.fwd.pair_state(id).expect("PAIR_STATE after ABORT").is_none(),
                    "ended id {id} is absent from its context's live table"
                );
                pair_state_calls.fetch_add(1, Ordering::Relaxed);
                pair_state_ended.fetch_add(1, Ordering::Relaxed);
            }
            let mut unseen: BTreeSet<ForwardId> = lane.ids.iter().copied().collect();
            while let Some(record) = lane.fwd.take_end().expect("take_end") {
                assert!(
                    unseen.remove(&record.id),
                    "one record for known id {}; no duplicate within its context",
                    record.id
                );
            }
            assert!(unseen.is_empty(), "one end record for every id; missing {unseen:?}");
            let observed = u64::try_from(lane.ids.len()).expect("lane count fits u64");
            pair_ends = pair_ends.checked_add(observed).expect("pair-end count fits u64");
            let after = lane.fwd.inventory().expect("inventory after every record is read");
            assert_eq!(
                (after.live_pairs, after.live_watches, after.queued_ends, after.attached,),
                (0, 0, 0, observed),
                "the record count agrees with the context's live and attached counters"
            );
            assert_eq!(
                after.completed.checked_add(after.aborted).expect("end counters fit u64"),
                observed,
                "every attached id is accounted as completed or aborted"
            );
            assert_eq!(after.unclassified_eof, 0, "every socket end retained its class");
        }
        drop(lanes);
        iterations += 1;
    }
    stop.store(true, Ordering::SeqCst);
    deleter.join().expect("the deleter thread");
    if !Path::new(MODULE_DIR).exists() {
        load_guest_module().unwrap_or_else(|e| panic!("final reload: errno {e}"));
    }
    let faults = fault_lines();
    let unload_count = unloads.load(Ordering::SeqCst);
    let state_calls = pair_state_calls.load(Ordering::Relaxed);
    let state_live = pair_state_live.load(Ordering::Relaxed);
    let state_ended = pair_state_ended.load(Ordering::Relaxed);
    let aborts = abort_won.load(Ordering::Relaxed);
    let stale_aborts = abort_already_ended.load(Ordering::Relaxed);
    eprintln!(
        "V-EV-SAFE pair_ends={pair_ends} pair_state_calls={state_calls} \
         pair_state_live={state_live} pair_state_ended={state_ended} \
         aborts={aborts} already_ended={stale_aborts} iterations={iterations} \
         unloads={unload_count} bytes_by_cpu={bytes_by_cpu:?}"
    );
    assert_eq!(faults.len(), faults_before, "no fault in the kernel log: {faults:#?}");
    assert!(iterations > 10, "the loop ran ({iterations} iterations)");
    assert!(unload_count > 0, "the unload succeeded between iterations");
    assert!(
        pair_ends >= PAIR_END_MIN,
        "V-EV-SAFE observed {pair_ends} actual, unique end records; required at least {PAIR_END_MIN}"
    );
    assert_eq!(
        state_calls,
        pair_ends.checked_mul(2).expect("PAIR_STATE call count fits u64"),
        "PAIR_STATE ran once during each ABORT race and once after each id ended"
    );
    assert!(
        state_ended >= pair_ends,
        "PAIR_STATE observed every id absent after its end ({state_ended}/{pair_ends})"
    );
    assert_eq!(
        aborts.checked_add(stale_aborts).expect("ABORT call count fits u64"),
        pair_ends,
        "one singular ABORT was issued while every id ended"
    );
    assert!(aborts > 0, "ABORT won against at least one ending id");
    assert!(stale_aborts > 0, "at least one socket end beat its racing ABORT");
    assert!(
        bytes_by_cpu.iter().all(|&bytes| bytes > 0),
        "every pinned guest CPU drove receive load"
    );
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
fn a_ulp_socket_keeps_the_module_for_the_boot_in_a_real_guest() {
    if in_scripted_guest() {
        in_guest_a_socket_given_a_ulp_after_attach_keeps_its_reference_for_the_boot();
    } else {
        run_in_guest(
            0,
            "integration::unload_in_guest::a_ulp_socket_keeps_the_module_for_the_boot_in_a_real_guest",
            true,
        );
    }
}

/// V-EV-UNLOAD's unload stress, in a guest of its own.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_unload_stress_never_faults_in_a_guest() {
    if in_scripted_guest() {
        in_guest_the_unload_races_pair_ends_with_senders_on_every_half_and_never_faults();
    } else {
        run_in_guest(
            1,
            "integration::unload_in_guest::the_unload_stress_never_faults_in_a_guest",
            false,
        );
    }
}
