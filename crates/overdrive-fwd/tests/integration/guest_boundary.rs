//! The forwarder across a real VM boundary (GH #295, T-1; EVMOD-END,
//! EVMOD-ATTACH-END, EVMOD-CTL; ADR-0180, ADR-0210, ADR-0216): the cases the
//! host-role suite cannot reach, because the loopback transport has no
//! device release and no guest side.
//!
//! **The fixture.** `overdrive_testing::guest_kit`: a Cloud Hypervisor fork
//! microVM at a CID of the [`FWD_GUEST_BOUNDARY`] band, booted from the guest
//! kit (the metal host's stock 7.0 kernel and the `ovd_fwd.ko` built for it)
//! with the scripted guest vsock peer as PID 1. The peer runs no Overdrive
//! owner; the test scripts both sides: the host halves through this crate's
//! [`KernelFlowForwarder`], the guest halves — and, for the battery, the
//! guest's own `KernelFlowForwarder` context — through the peer's protocol.
//! A guest vsock connection to the host enters through vhost
//! (`drivers/vhost/vsock.c:555 @ v7.0`), so every host vsock half here is a
//! real vhost socket.
//!
//! **The device release (the orphan sweep).** Killing the VMM is the last
//! close of its `/dev/vhost-vsock` file: `vhost_vsock_dev_release`
//! (`drivers/vhost/vsock.c:721-750 @ v7.0`) sweeps every host socket of the
//! CID, setting `sk_err = ECONNRESET` and calling `sk_error_report`
//! (`:746-750`). The first three cases are the Tier-3 pair of the sim
//! arbiter reads `SimSocket::set_sk_err` / `report_error` that the
//! `fwd_pair` and `owner_flows` conformance drivers make (QPROJ-7, QPROJ-8):
//! a live pair aborts `SocketError { A, ECONNRESET }` and resets its TCP far
//! end; a half swept before attach is refused `HalfEnded { A, ECONNRESET }`;
//! a watched half's record is queued before its holder observes the
//! `ECONNRESET` (the record-before-original order, DR2-CLOSE F-S1).
//!
//! **The battery across the boundary.** A host application ↔ host pair ↔
//! vhost ↔ virtio-vsock ↔ guest pair ↔ guest application: bytes both ways,
//! a clean end each way, an application reset from each side, and a VMM
//! exit with bytes in flight.
//!
//! Lane: `integration-tests,kvm-tests` on the native x86_64 metal host,
//! `cargo xtask metal run -- cargo nextest run -p overdrive-fwd --features
//! integration-tests,kvm-tests`; this binary is in `host-kernel-shared`.
//! Precondition: the host has the `ovd_fwd` build for its release loaded
//! (EVMOD-LOAD; `infra/metal`). Every case is pending DELIVER step 4 (the
//! module, its adapter and `cargo xtask kmod-build`) on top of step 1 (the
//! fork's vhost-kernel vsock device). Each case launches one VMM and ends it
//! within its bound; the fixture kills a VMM a failed case left running.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpStream};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use overdrive_core::guest_transport::forwarder::{
    AbortCause, FlowForwarder, ForwardId, ForwardOutcome, ForwarderError, PairMode, PairSide,
};
use overdrive_core::guest_transport::wire::KernelErrno;
use overdrive_fwd::KernelFlowForwarder;
use overdrive_testing::guest_app::{Direction, StreamCheck, StreamGen};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{GuestImageSpec, GuestKit, ScriptedGuest, WorkDir};
use overdrive_testing::guest_peer::{
    DrainEnd, GuestPeerError, PeerCommand, PeerOutcome, PeerRead, PeerReply, PeerSide,
};
use overdrive_testing::vm_fixture::default_staging_root;
use overdrive_testing::vsock::{VMADDR_CID_ANY, VMADDR_CID_HOST, VsockListener, VsockStream};

use super::support::{
    END_WAIT, FarEnd, assert_far_tcp_reset, bounded, live, open_context, read_until_end,
    reset_close, tcp_pair, wait_end_of,
};

/// The host port a guest of slot `n` dials for its data connections.
const fn data_port(slot: u32) -> u32 {
    30_000 + slot
}

/// Bytes each direction of the battery moves.
const BATTERY_BYTES: u64 = 2 << 20;

/// One booted scripted guest of the band's `slot`.
struct Boundary {
    guest: ScriptedGuest,
    slot: u32,
    cid: u32,
    _work: WorkDir,
}

impl Boundary {
    fn boot(slot: u32, name: &str) -> Self {
        let staging = default_staging_root();
        let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
        let work = WorkDir::create(&staging, &format!("fwd-boundary-{name}")).expect("work dir");
        let image =
            kit.image(work.path(), name, &GuestImageSpec::scripted()).expect("the scripted image");
        let cid = FWD_GUEST_BOUNDARY.raw(slot);
        let guest = kit
            .boot_scripted(&image, cid, &work.path().join("run"))
            .expect("the scripted guest boots and dials its control port");
        Self { guest, slot, cid, _work: work }
    }

    /// A host vsock half: the guest connects `handle` to the host's data
    /// port; returns the host's accepted socket, whose peer is this CID.
    fn host_half_from_guest(&mut self, handle: &str) -> VsockStream {
        let listener = VsockListener::bind(VMADDR_CID_ANY, data_port(self.slot))
            .expect("bind the host data port");
        self.guest
            .peer()
            .vsock_connect(handle, VMADDR_CID_HOST, data_port(self.slot), None)
            .expect("the guest connects to the host");
        let (stream, peer) = listener
            .accept_within(END_WAIT)
            .expect("accept")
            .expect("the guest's connection is in the host's accept queue");
        assert_eq!(peer.cid, self.cid, "the host sees the guest's real CID");
        stream
    }

    fn kill_vmm(&mut self) {
        let status = self.guest.vm.kill().expect("kill the VMM");
        assert!(!status.success(), "a killed VMM does not exit cleanly: {status}");
    }
}

fn attach(fwd: &KernelFlowForwarder, vsock: &VsockStream, tcp: &TcpStream) -> ForwardId {
    fwd.attach(vsock as &dyn AsFd, tcp as &dyn AsFd, PairMode::Stream)
        .expect("attach the host pair")
}

fn device_release_reset() -> ForwardOutcome {
    ForwardOutcome::Aborted {
        cause: AbortCause::SocketError { side: PairSide::A, errno: KernelErrno::ECONNRESET },
    }
}

// ---------------------------------------------------------------------------
// The device release's orphan sweep (drivers/vhost/vsock.c:721-750 @ v7.0)
// ---------------------------------------------------------------------------

/// A live host pair whose vsock half's VMM exits aborts
/// `SocketError { A, ECONNRESET }` (the sweep's `sk_err`), and its TCP far
/// end reads a reset, never EOF (R5-45 (b), R5-10 guest half).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_vmm_exit_aborts_a_live_host_pair_with_the_device_releases_reset() {
    let mut rig = Boundary::boot(0, "sweep-pair");
    let fwd = open_context();
    let v_h = rig.host_half_from_guest("v");
    let (t, t_far) = tcp_pair();
    let id = attach(&fwd, &v_h, &t);
    rig.guest.peer().write("v", b"before-the-exit").expect("the guest writes");
    let mut got = [0u8; 15];
    (&mut bounded(&t_far)).read_exact(&mut got).expect("the bytes reach the TCP far end");
    assert_eq!(&got, b"before-the-exit");

    rig.kill_vmm();

    let end = wait_end_of(&fwd, id);
    assert_eq!(end.outcome, device_release_reset(), "the sweep's ECONNRESET classifies the pair");
    assert_far_tcp_reset(&t_far);
    assert_eq!(live(&fwd), (0, 0, 0), "nothing live or queued after the record");
    drop((v_h, t));
}

/// A host vsock half the sweep reached before its attach is refused
/// `HalfEnded { A, ECONNRESET }` (EVMOD-ATTACH-END, ADR-0210): the attach
/// changes nothing and the owner ends the flow.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_vsock_half_swept_before_attach_is_refused_half_ended_with_econnreset() {
    let mut rig = Boundary::boot(1, "sweep-attach");
    let fwd = open_context();
    let v_h = rig.host_half_from_guest("v");
    rig.kill_vmm();
    let (t, _t_far) = tcp_pair();
    let refused = fwd.attach(&v_h as &dyn AsFd, &t as &dyn AsFd, PairMode::Stream);
    match refused {
        Err(ForwarderError::HalfEnded { side, errno }) => {
            assert_eq!(side, PairSide::A);
            assert_eq!(errno, KernelErrno::ECONNRESET);
        }
        other => panic!("a swept half is refused HalfEnded {{ A, ECONNRESET }}, got {other:?}"),
    }
    assert_eq!(live(&fwd), (0, 0, 0), "a refused attach changes nothing");
}

/// A watched host vsock half (a dial's `V_h`, ADR-0216) swept by the device
/// release: the watch's record is queued before its holder, blocked in
/// `read`, observes the end, and the holder's read fails `ECONNRESET`
/// (EVMOD-STREAM-END (d); DR2-CLOSE F-S1).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_watched_half_records_the_sweep_before_its_holder_observes_the_reset() {
    let mut rig = Boundary::boot(2, "sweep-watch");
    let fwd = Arc::new(open_context());
    let v_h = rig.host_half_from_guest("v");
    let watch = fwd.watch(&v_h as &dyn AsFd).expect("watch V_h");
    let holder_fwd = Arc::clone(&fwd);
    let holder_v = v_h.try_clone().expect("the holder's handle");
    let holder = thread::spawn(move || {
        let mut buf = [0u8; 64];
        let read = holder_v.read_within(&mut buf, END_WAIT * 3);
        // The first thing the holder does on waking: look for the record.
        let record_at_wake = holder_fwd.take_end().expect("take_end");
        (read.map_err(|e| e.raw_os_error()), record_at_wake)
    });
    thread::sleep(Duration::from_millis(200));
    rig.kill_vmm();
    let (read, record) = holder.join().expect("the holder thread");
    assert_eq!(read, Err(Some(104)), "the holder's read fails ECONNRESET, never EOF");
    let record = record.expect("the record is queued before the holder observes the end");
    assert_eq!(record.id, watch);
    assert!(
        matches!(
            record.outcome,
            ForwardOutcome::Aborted { cause: AbortCause::SocketError { errno, .. } }
                if errno == KernelErrno::ECONNRESET
        ),
        "the sweep's class: {:?}",
        record.outcome
    );
    assert_eq!(live(&fwd), (0, 0, 0));
}

// ---------------------------------------------------------------------------
// The battery across the VM boundary
// ---------------------------------------------------------------------------

/// host app ↔ host pair ↔ vhost ↔ virtio-vsock ↔ guest pair ↔ guest app.
struct ForwardedFlow {
    rig: Boundary,
    fwd: KernelFlowForwarder,
    host_id: ForwardId,
    guest_id: u64,
    host_app: TcpStream,
    _v_h: VsockStream,
    _h_srv: TcpStream,
}

impl ForwardedFlow {
    fn establish(slot: u32, name: &str) -> Self {
        let mut rig = Boundary::boot(slot, name);
        let v_h = rig.host_half_from_guest("v");
        let peer = rig.guest.peer();
        peer.forwarder_open("f").expect("the guest opens a forwarder context");
        let listen = peer
            .tcp_listen("tl", SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .expect("the guest application's listener");
        peer.tcp_connect("app", listen).expect("the guest application connects");
        assert!(
            peer.tcp_accept("srv", "tl", END_WAIT).expect("accept"),
            "the guest accepts its own connection"
        );
        let guest_id = peer
            .forwarder_attach_stream("f", "v", "srv")
            .expect("the attach command")
            .unwrap_or_else(|refused| panic!("the guest attaches its pair: {refused}"));
        let fwd = open_context();
        let (h_srv, host_app) = tcp_pair();
        let host_id = attach(&fwd, &v_h, &h_srv);
        Self { rig, fwd, host_id, guest_id, host_app, _v_h: v_h, _h_srv: h_srv }
    }

    fn guest_end(&mut self) -> PeerOutcome {
        let end = self
            .rig
            .guest
            .peer()
            .forwarder_take_end("f", END_WAIT)
            .expect("the guest's end-record command")
            .expect("the guest pair's end record");
        assert_eq!(end.id, self.guest_id);
        end.outcome
    }
}

/// Bytes cross the VM boundary byte-exact both ways, a clean half-close from
/// each application is carried after its last byte, and both pairs complete
/// (R2-END, CLEAN-FULL-END; R5-10's clean-close regression).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn bytes_cross_the_vm_boundary_both_ways_and_clean_ends_complete_both_pairs() {
    let mut flow = ForwardedFlow::establish(3, "battery-clean");
    let tag = "battery-clean";
    let mut writer = flow.host_app.try_clone().expect("the host app's writer");
    let host_writer = thread::spawn(move || {
        let mut stream_gen = StreamGen::new("battery-clean", Direction::PeerToGuest);
        let mut chunk = vec![0u8; 64 * 1024];
        let mut left = BATTERY_BYTES;
        while left > 0 {
            let n = usize::try_from(left.min(chunk.len() as u64)).expect("chunk");
            stream_gen.fill(&mut chunk[..n]);
            writer.write_all(&chunk[..n]).expect("the host app writes");
            left -= n as u64;
        }
        writer.shutdown(Shutdown::Write).expect("the host app half-closes");
    });
    let peer = flow.rig.guest.peer();
    peer.spawn(
        "drain",
        PeerCommand::Drain { handle: "app".into(), tag: tag.into(), timeout_ms: 10_000 },
    )
    .expect("the guest app drains");
    let pumped = peer
        .pump("app", tag, BATTERY_BYTES, Duration::from_secs(120))
        .expect("the guest app writes");
    assert_eq!(pumped, BATTERY_BYTES);
    peer.shutdown("app", Shutdown::Write).expect("the guest app half-closes");

    let mut check = StreamCheck::new(tag, Direction::GuestToPeer);
    let mut buf = vec![0u8; 64 * 1024];
    flow.host_app.set_read_timeout(Some(END_WAIT)).expect("timeout");
    loop {
        match flow.host_app.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => check
                .check(&buf[..n])
                .unwrap_or_else(|at| panic!("the guest's stream is corrupt at {at}")),
            Err(e) => panic!("the host app reads a clean end, got {e}"),
        }
    }
    assert_eq!(check.verified(), BATTERY_BYTES, "every byte, then a clean EOF");
    host_writer.join().expect("the host app's writer");
    let drained = loop {
        if let Some(reply) = flow.rig.guest.peer().join("drain", END_WAIT).expect("join") {
            break reply;
        }
    };
    assert_eq!(drained, PeerReply::Drained { len: BATTERY_BYTES, end: DrainEnd::Eof });

    let host_end = wait_end_of(&flow.fwd, flow.host_id);
    assert_eq!(host_end.outcome, ForwardOutcome::Completed);
    assert_eq!((host_end.moved_ab, host_end.moved_ba), (BATTERY_BYTES, BATTERY_BYTES));
    assert_eq!(flow.guest_end(), PeerOutcome::Completed);
}

/// The guest application resets its connection: the guest pair aborts
/// `SocketError { B, ECONNRESET }`, the guest forwarder ends `V_g` with a
/// direction open, the host pair aborts `PeerAbort { A }` and the host
/// application reads a reset, never EOF (R5-45 (b)).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_guest_application_reset_reaches_the_host_application_as_a_reset() {
    let mut flow = ForwardedFlow::establish(4, "battery-guest-rst");
    flow.rig.guest.peer().write("app", b"from-the-guest").expect("the guest app writes");
    let mut got = [0u8; 14];
    (&mut bounded(&flow.host_app)).read_exact(&mut got).expect("the host app reads");
    assert_eq!(&got, b"from-the-guest");

    flow.rig.guest.peer().reset("app").expect("the guest app resets");

    assert_far_tcp_reset(&flow.host_app);
    assert_eq!(
        wait_end_of(&flow.fwd, flow.host_id).outcome,
        ForwardOutcome::Aborted { cause: AbortCause::PeerAbort { side: PairSide::A } }
    );
    assert_eq!(flow.guest_end(), PeerOutcome::SocketError(PeerSide::B, 104));
}

/// The host application resets its connection: the host pair aborts
/// `SocketError { B, ECONNRESET }`, the guest pair aborts
/// `PeerAbort { A }`, and the guest application's read fails `ECONNRESET`
/// (R5-45 (b), the other direction).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_host_application_reset_reaches_the_guest_application_as_a_reset() {
    let mut flow = ForwardedFlow::establish(5, "battery-host-rst");
    (&mut &flow.host_app).write_all(b"from-the-host").expect("the host app writes");
    let read = flow.rig.guest.peer().read("app", 13, END_WAIT).expect("the guest app reads");
    assert_eq!(read, PeerRead::Data(b"from-the-host".to_vec()));

    // `reset_close` sets `SO_LINGER{1,0}` on the socket and closes one
    // descriptor; the flow's own descriptor is closed with it, so the
    // socket's last close sends the RST.
    reset_close(flow.host_app.try_clone().expect("clone"));
    drop(std::mem::replace(&mut flow.host_app, tcp_pair().0));

    assert_eq!(
        wait_end_of(&flow.fwd, flow.host_id).outcome,
        ForwardOutcome::Aborted {
            cause: AbortCause::SocketError { side: PairSide::B, errno: KernelErrno::ECONNRESET }
        }
    );
    match flow.rig.guest.peer().read("app", 64, END_WAIT) {
        Err(GuestPeerError::Remote { errno, .. }) => {
            assert_eq!(errno, 104, "the guest application's read fails ECONNRESET");
        }
        other => panic!("the guest application reads a reset, never EOF: {other:?}"),
    }
    assert_eq!(flow.guest_end(), PeerOutcome::PeerAbort(PeerSide::A));
}

/// The VMM exits with bytes flowing both ways: the device release aborts the
/// host pair `SocketError { A, ECONNRESET }` and the host application reads
/// a reset, never EOF (the battery's `kill` shape across the boundary).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_vmm_exit_mid_transfer_resets_the_host_application() {
    let mut flow = ForwardedFlow::establish(6, "battery-kill");
    let tag = "battery-kill";
    flow.rig
        .guest
        .peer()
        .spawn(
            "pump",
            PeerCommand::Pump { handle: "app".into(), tag: tag.into(), len: u64::MAX / 2 },
        )
        .expect("the guest app writes without end");
    let mut check = StreamCheck::new(tag, Direction::GuestToPeer);
    let mut buf = vec![0u8; 64 * 1024];
    let host_app = bounded(&flow.host_app);
    let start = Instant::now();
    while check.verified() < 256 * 1024 {
        let n = (&mut &*host_app).read(&mut buf).expect("bytes flow");
        assert!(n > 0, "no EOF while the guest writes");
        check.check(&buf[..n]).unwrap_or_else(|at| panic!("corrupt at {at}"));
        assert!(start.elapsed() < END_WAIT, "the flow moves bytes");
    }

    flow.rig.kill_vmm();

    let (_, far) = read_until_end(host_app);
    assert_eq!(far, FarEnd::Error(ErrorKind::ConnectionReset), "a reset, never EOF");
    assert_eq!(wait_end_of(&flow.fwd, flow.host_id).outcome, device_release_reset());
    assert_eq!(live(&flow.fwd), (0, 0, 0), "nothing live or queued after the record");
}
