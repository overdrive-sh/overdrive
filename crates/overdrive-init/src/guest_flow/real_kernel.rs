//! Real-kernel half of R5-59 (b), the guest seam's accept-path equivalence
//! (D-UP-19, PD-5; D-UP-18): `GuestFlowOwner` over `KernelGuestOwnerIo` and
//! `KernelFlowForwarder` serves the same connection table
//! ([`ACCEPT_PATH_CASES`]) with the same per-connection outcome as the sim half
//! (`tests::tcp_accept::accept_path_outcomes_by_first_record`, over
//! `SimGuestOwnerIo`): a connection with its `TcpAccept` request buffered is
//! served — the application accepts it and the bytes written after the
//! request reach it — and one with no byte yields no application connection
//! and its V_g is closed. Order-insensitive, compared per connection.
//!
//! **Lane.** Inside an Overdrive microVM guest, as the guest's operator
//! command, through `cargo xtask metal run --` with `kvm-tests` (see
//! `transport::real_kernel`): the production `overdrive-init` (PID 1) owns
//! `GUEST_INBOUND_PORT`; this cell opens the connections over the guest's vsock
//! loopback transport (`VMADDR_CID_LOCAL`), which delivers them to that
//! listener through the same `virtio_transport_recv_pkt` path a host
//! connection takes (`net/vmw_vsock/vsock_loopback.c:142-143 @ v7.0`), and
//! plays the application on `workload_addr:<port>`. It fails closed outside a
//! guest and when the guest kernel has no vsock loopback transport.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::doc_markdown,
    reason = "real-kernel test cells: a failed precondition panics"
)]

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddrV4, TcpListener};
use std::os::fd::{AsRawFd, OwnedFd};
use std::str::FromStr;
use std::time::{Duration, Instant};

use nix::sys::socket::{AddressFamily, SockFlag, SockType, VsockAddr, connect, shutdown, socket};
use overdrive_core::guest_transport::GUEST_INBOUND_PORT;
use overdrive_core::guest_transport::wire::{FlowId, FlowOrigin, FlowRequest};
use overdrive_core::net::ListenPort;

use super::test_support::{ACCEPT_PATH_CASES, AcceptPathCase};
use crate::transport::{GuestTransportToken, parse_transport_token};

/// The guest's transport token; panics (fails closed) outside a guest.
fn in_guest() -> GuestTransportToken {
    let cmdline = fs::read_to_string("/proc/cmdline").expect("/proc/cmdline is readable");
    assert!(
        cmdline.contains("overdrive.transport="),
        "not running inside an Overdrive guest: this cell runs as a microVM guest's operator command"
    );
    parse_transport_token(&cmdline).expect("the guest's token parses")
}

/// A vsock STREAM connection to the guest's own inbound listener over the
/// loopback transport.
fn connect_inbound() -> File {
    let fd: OwnedFd = socket(AddressFamily::Vsock, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .expect("a vsock socket");
    let addr = VsockAddr::new(nix::libc::VMADDR_CID_LOCAL, GUEST_INBOUND_PORT.as_u32());
    connect(fd.as_raw_fd(), &addr).unwrap_or_else(|e| {
        panic!("connect to the guest's own GUEST_INBOUND_PORT over vsock loopback failed ({e}): the guest kernel needs CONFIG_VSOCKETS_LOOPBACK")
    });
    File::from(fd)
}

/// The application's accept on `listener` within `bound`, or `None`.
fn accept_within(listener: &TcpListener, bound: Duration) -> Option<std::net::TcpStream> {
    listener.set_nonblocking(true).expect("non-blocking listener");
    let deadline = Instant::now() + bound;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return None,
            Err(e) => panic!("accept failed: {e}"),
        }
    }
}

/// The per-connection outcome both halves compare: served with the payload,
/// or no application connection.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Served(Vec<u8>),
    NoApplicationConnection,
}

/// R5-59 (b), real half: the production guest owner's outcome per connection
/// of [`ACCEPT_PATH_CASES`] equals the sim half's — served or not, by its
/// first record.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_accept_path_matches_the_sim_per_connection() {
    let token = in_guest();
    let mut listeners: BTreeMap<u16, TcpListener> = BTreeMap::new();
    let mut outcomes = BTreeMap::new();
    for case in ACCEPT_PATH_CASES {
        let AcceptPathCase { seq, port, request_buffered, payload } = case;
        let listener = listeners.entry(port).or_insert_with(|| {
            TcpListener::bind(SocketAddrV4::new(token.workload_addr, port))
                .expect("the application listens")
        });
        let mut vsock = connect_inbound();
        let outcome = if request_buffered {
            let request = FlowRequest::TcpAccept {
                flow: FlowId::new(FlowOrigin::Host, seq).expect("small seq"),
                peer: token.gateway,
                listen_port: ListenPort::from_str(&port.to_string()).expect("non-zero"),
                launch: token.launch,
            };
            vsock.write_all(&request.encode()).expect("the request is written");
            let mut app =
                accept_within(listener, Duration::from_secs(5)).expect("the application accepts C");
            assert_eq!(
                app.peer_addr().expect("C's address").ip(),
                std::net::IpAddr::V4(token.gateway),
                "C is bound to the gateway"
            );
            vsock.write_all(payload).expect("the payload is written after the request");
            let mut got = vec![0u8; payload.len()];
            app.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
            app.read_exact(&mut got).expect("the payload reaches the application");
            app.shutdown(Shutdown::Both).ok();
            Outcome::Served(got)
        } else {
            shutdown(vsock.as_raw_fd(), nix::sys::socket::Shutdown::Write)
                .expect("the connection ends with no byte");
            let app = accept_within(listener, Duration::from_secs(2));
            assert!(app.is_none(), "{case:?}: no application connection");
            let mut rest = Vec::new();
            vsock.read_to_end(&mut rest).expect("V_g is closed by the guest owner");
            assert!(rest.is_empty());
            Outcome::NoApplicationConnection
        };
        outcomes.insert(seq, outcome);
    }
    for case in ACCEPT_PATH_CASES {
        let expected = if case.request_buffered {
            Outcome::Served(case.payload.to_vec())
        } else {
            Outcome::NoApplicationConnection
        };
        assert_eq!(outcomes.get(&case.seq), Some(&expected), "{case:?}");
    }
}

// ---------------------------------------------------------------------------
// D-22: the Tier-3 half of R5-28 and R5-31 — the V-24 shape in the in-guest
// owner lane
// ---------------------------------------------------------------------------
//
// **Lane** (`overdrive_testing::guest_lane::run_in_guest_owner_cell`,
// feature-delta § *DISTILL handoff*, "R5-28 / R5-31 Tier-3 half: the in-guest
// lane", D-22). PID 1 is the scripted peer, so the owner built here — the
// production `GuestFlowOwner` over `KernelGuestOwnerIo`, configured by the
// production `configure_guest_transport` with the real `ovd_fwd` on the
// Linux 7.0 guest — is the guest's **only** flow owner. The host side is the
// lane's scripted host peer over the real vhost-vsock: it opens V_h to the
// guest's inbound port, writes the `TcpAccept` request and nothing else, and
// closes V_h in the order each cell names; it holds and plays the control
// session. The two ends step each other over the lane's script channel.
//
// **Oracles, all read in the guest** (D-22 (1)–(5)): the owner through its
// crate-private `guest_observation()` (DG-5, QPROJ-5; no new accessor); this
// cell's application listener on `workload_addr:APP_PORT` (no C, or C reset,
// and no byte); the host peer's record of the control session (checked by
// the lane, which fails the case on a message for the aborted flow); the
// kernel shape on a V_g the cell accepts itself (the raw witness). R5-28's
// "counted" (`guest_flow.control_discarded`) has no in-guest read surface:
// it is discharged by the seeded-sim half alone, as is the host half's
// discard of a late `Paired`.
//
// Each case also serves a positive control flow end to end on the same lane
// (`CONTROL_FLOW`): the application accepts its C and the flow pairs, so
// "no C" for the aborted flow is not a lane that cannot serve at all.

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::socket::MsgFlags;
use overdrive_core::guest_transport::forwarder::ForwardOutcome;
use overdrive_core::guest_transport::wire::FLOW_REQUEST_LEN;
use overdrive_testing::guest_lane::{LaneScript, owner_lane};
use overdrive_testing::vsock::{VMADDR_CID_ANY, VsockListener};

use super::{
    GuestAcceptPhase, GuestFlowOwner, GuestSideObservation, KernelGuestOwnerIo, MonotonicGuestClock,
};
use crate::transport::{KernelGuestTransportOps, configure_guest_transport};

/// Every lane wait's bound.
const LANE_BOUND: Duration = Duration::from_secs(30);
/// How long the cell observes that something does not happen (a C, a byte).
const QUIET: Duration = Duration::from_secs(1);
/// R5-31: no guest connection of an aborted flow is open this long after the
/// abort.
const ABORT_SETTLE: Duration = Duration::from_secs(1);

/// The host-origin flow id `seq`.
fn host_flow(seq: u32) -> FlowId {
    FlowId::new(FlowOrigin::Host, seq).expect("a small host-origin sequence")
}

/// The application side of the cell: accepts every C on
/// `workload_addr:APP_PORT` on its own thread, as an application would.
struct App {
    accepted: Arc<Mutex<Vec<TcpStream>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl App {
    fn start(addr: SocketAddrV4) -> Self {
        let listener = TcpListener::bind(addr).expect("the application listens");
        listener.set_nonblocking(true).expect("non-blocking listener");
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let accepted = Arc::clone(&accepted);
            let stop = Arc::clone(&stop);
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => accepted.lock().expect("app lock").push(stream),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(e) => panic!("the application's accept failed: {e}"),
                    }
                }
            }
        });
        Self { accepted, stop, thread: Some(thread) }
    }

    fn count(&self) -> usize {
        self.accepted.lock().expect("app lock").len()
    }

    fn take_all(&self) -> Vec<TcpStream> {
        std::mem::take(&mut *self.accepted.lock().expect("app lock"))
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How an application connection C ended, read with a bound.
#[derive(Debug, PartialEq, Eq)]
enum AppEnd {
    /// `read` failed `ECONNRESET`.
    Reset,
    /// A clean end of stream.
    Eof,
    /// Bytes arrived (the count), or another error.
    Other(String),
}

/// Reads C until its end within `bound`: the bytes it carried and how it
/// ended.
fn app_end(c: &TcpStream, bound: Duration) -> (usize, AppEnd) {
    c.set_read_timeout(Some(bound)).expect("timeout");
    let mut r = c;
    let mut got = 0usize;
    let mut buf = [0u8; 4096];
    loop {
        match r.read(&mut buf) {
            Ok(0) => return (got, AppEnd::Eof),
            Ok(n) => got += n,
            Err(e) if e.kind() == ErrorKind::ConnectionReset => return (got, AppEnd::Reset),
            Err(e) => return (got, AppEnd::Other(e.to_string())),
        }
    }
}

/// The guest's only flow owner, configured by the production setup, with
/// the lane's script channel and the application.
struct OwnerCell {
    owner: GuestFlowOwner,
    script: LaneScript,
    app: App,
}

impl OwnerCell {
    /// Configures the guest transport with the production setup, constructs
    /// the production owner over the kernel seam, starts the application,
    /// and pumps the owner until the lane's host has accepted its control
    /// session.
    fn start() -> Self {
        let token = in_guest();
        let script = LaneScript::connect_from_guest();
        let mut ops = KernelGuestTransportOps::new();
        configure_guest_transport(&token, &mut ops).expect(
            "the production setup configures the guest transport (the cell is the guest's only \
             owner: PID 1 is the scripted peer)",
        );
        let owner = GuestFlowOwner::new(
            token,
            Box::new(KernelGuestOwnerIo::new(ops)),
            Arc::new(MonotonicGuestClock),
        );
        let app = App::start(SocketAddrV4::new(token.workload_addr, owner_lane::APP_PORT));
        let mut cell = Self { owner, script, app };
        cell.pump_until_heard(owner_lane::SESSION_ACCEPTED);
        assert!(cell.observe().session, "the owner holds its control session");
        cell
    }

    fn observe(&self) -> GuestSideObservation {
        self.owner.guest_observation()
    }

    /// One owner step (the next event, or the 1 s audit tick at most).
    fn dispatch(&mut self) {
        self.owner.dispatch_next().unwrap_or_else(|e| panic!("the guest owner failed: {e}"));
    }

    fn say(&self, line: &str) {
        self.script.say(line).expect("the script channel is open");
    }

    /// Pumps the owner until the host's next script line, which must be
    /// `line`.
    fn pump_until_heard(&mut self, line: &str) {
        let deadline = Instant::now() + LANE_BOUND;
        loop {
            if let Some(heard) = self.script.hear_within(Duration::ZERO).expect("script channel") {
                assert_eq!(heard, line, "the host's script is out of step");
                return;
            }
            assert!(Instant::now() < deadline, "no {line} from the host within {LANE_BOUND:?}");
            self.dispatch();
        }
    }

    /// Stops dispatching the owner, tells the host (`PAUSED`), and waits for
    /// the host's `line` without dispatching: a `V_h` the host opens
    /// meanwhile stays queued at the guest (its V_g unaccepted) until the
    /// owner runs again.
    fn pause_until_heard(&mut self, line: &str) {
        self.say(owner_lane::PAUSED);
        let heard = self.script.hear(line, LANE_BOUND).expect("script channel");
        assert_eq!(heard, line, "the host's script is out of step");
    }

    /// Pumps the owner until `done` holds of its observation.
    fn pump_until(
        &mut self,
        what: &str,
        bound: Duration,
        done: impl Fn(&GuestSideObservation) -> bool,
    ) -> GuestSideObservation {
        let deadline = Instant::now() + bound;
        loop {
            let obs = self.observe();
            if done(&obs) {
                return obs;
            }
            assert!(Instant::now() < deadline, "{what} not reached within {bound:?}: {obs:?}");
            self.dispatch();
        }
    }

    /// Pumps the owner for `d` (whole dispatches).
    fn pump_for(&mut self, d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            self.dispatch();
        }
    }

    /// The positive control: the host opens `CONTROL_FLOW` and keeps V_h
    /// open; the application accepts its C and the flow pairs; the host then
    /// closes V_h and C is reset with no byte. Proves this lane serves a flow
    /// end to end, so an aborted flow's "no C" has teeth.
    fn serve_control_flow(&mut self) {
        self.serve_control_flow_checking(None);
    }

    /// [`Self::serve_control_flow`] after a flow the host aborted, whose V_g
    /// was queued BEFORE the control flow's V_h was opened (D-22 (1), (2),
    /// (5)). The listener accepts in order, so the control flow's pairing
    /// shows the owner has accepted and processed the aborted flow's V_g:
    /// only then, after `ABORT_SETTLE` more of dispatching, is the aborted
    /// flow checked: the owner holds nothing of it, it attached no pair (the
    /// control flow's is the only attach since `attached_before`, the owner's
    /// attach count before either flow), and every application connection
    /// that arrived before the control flow's is reset and carried no byte.
    /// Checking before the owner has run would pass trivially while it is
    /// paused.
    fn serve_control_flow_after_abort(&mut self, aborted: FlowId, attached_before: u64) {
        self.serve_control_flow_checking(Some((aborted, attached_before)));
    }

    fn serve_control_flow_checking(&mut self, aborted: Option<(FlowId, u64)>) {
        let flow = host_flow(owner_lane::CONTROL_FLOW);
        let before = self.app.count();
        self.pump_until_heard(owner_lane::REQUESTED);
        self.pump_until("the control flow paired", LANE_BOUND, |o| {
            matches!(o.accepted.get(&flow), Some(GuestAcceptPhase::Paired { .. }))
        });
        let mut strays = if let Some((aborted, attached_before)) = aborted {
            self.pump_for(ABORT_SETTLE);
            let obs = self.observe();
            assert!(
                !obs.accepted.contains_key(&aborted) && !obs.flows.contains_key(&aborted),
                "the aborted flow is gone once the owner has processed it: {obs:?}"
            );
            assert_eq!(
                obs.forwarder_attached,
                attached_before + 1,
                "only the control flow attached a pair: {obs:?}"
            );
            self.app.take_all()
        } else {
            Vec::new()
        };
        let deadline = Instant::now() + LANE_BOUND;
        while self.app.count() + strays.len() == before {
            assert!(Instant::now() < deadline, "the control flow's C never reached the app");
            self.dispatch();
        }
        strays.extend(self.app.take_all());
        let c = strays.pop().expect("the control flow's C");
        for stray in strays {
            let (bytes, end) = app_end(&stray, QUIET);
            assert_eq!(
                (bytes, end),
                (0, AppEnd::Reset),
                "an aborted flow's C is reset, empty (the control flow's C is the last)"
            );
        }
        self.say(owner_lane::PAIRED);
        self.pump_until_heard(owner_lane::CLOSED);
        let (bytes, end) = app_end(&c, LANE_BOUND);
        assert_eq!((bytes, end), (0, AppEnd::Reset), "the host's close resets the control C");
        self.pump_until("the control flow ended", LANE_BOUND, |o| {
            !o.accepted.contains_key(&flow) && !o.flows.contains_key(&flow)
        });
    }
}

/// `POLLRDHUP` (`include/uapi/asm-generic/poll.h`); nix 0.30's `PollFlags`
/// declares no constant for it, so the libc bit is carried as-is.
const POLLRDHUP: PollFlags = PollFlags::from_bits_retain(nix::libc::POLLRDHUP);

fn poll_revents(fd: &impl std::os::fd::AsFd, events: PollFlags) -> PollFlags {
    let mut fds = [PollFd::new(fd.as_fd(), events)];
    poll(&mut fds, PollTimeout::ZERO).expect("poll");
    fds[0].revents().unwrap_or(PollFlags::empty())
}

/// V-24's kernel shape on a V_g the cell accepted (D-22 (4)): the request
/// read succeeds, then EOF, `POLLRDHUP`, and `EPIPE` on the first write.
fn assert_v24_shape(v_g: &mut File) {
    let mut request = [0u8; FLOW_REQUEST_LEN];
    v_g.read_exact(&mut request).expect("the request read itself succeeds");
    let mut more = [0u8; 16];
    assert_eq!(v_g.read(&mut more).expect("a read after the request"), 0, "then EOF");
    assert!(poll_revents(v_g, POLLRDHUP).contains(POLLRDHUP), "V_g reports POLLRDHUP");
    let written = nix::sys::socket::send(v_g.as_raw_fd(), b"x", MsgFlags::MSG_NOSIGNAL);
    assert_eq!(written, Err(nix::errno::Errno::EPIPE), "the first write fails EPIPE");
}

/// D-22 (4): the V-24 shape re-witnessed in the lane, on a V_g this cell
/// accepts with its own raw listener under the host's script: in the order
/// "V_g still queued when the host closes" and in the order "V_g accepted
/// before the host writes and closes", the request read succeeds, then
/// EOF, `POLLRDHUP` and `EPIPE` on the first write. The owner-path cells
/// rest on this kernel fact, observed here on the lane's own kernel.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 5"]
fn the_v24_kernel_shape_holds_in_both_accept_orders() {
    let _token = in_guest();
    let mut script = LaneScript::connect_from_guest();
    let raw = VsockListener::bind(VMADDR_CID_ANY, owner_lane::RAW_PORT).expect("the raw listener");
    script.say(owner_lane::LISTENING).expect("script");

    // Order 1: V_g still queued when the host closes V_h.
    assert_eq!(script.hear("RAW-QUEUED", LANE_BOUND).expect("script"), owner_lane::RAW_QUEUED);
    let (v_g, _) = raw.accept_within(LANE_BOUND).expect("accept").expect("the queued V_g");
    assert_v24_shape(&mut File::from(std::os::fd::OwnedFd::from(v_g)));

    // Order 2: V_g accepted before the host writes the request and closes.
    assert_eq!(
        script.hear("RAW-CONNECTED", LANE_BOUND).expect("script"),
        owner_lane::RAW_CONNECTED
    );
    let (v_g, _) = raw.accept_within(LANE_BOUND).expect("accept").expect("the connected V_g");
    script.say(owner_lane::RAW_ACCEPTED).expect("script");
    assert_eq!(script.hear("CLOSED", LANE_BOUND).expect("script"), owner_lane::CLOSED);
    assert_v24_shape(&mut File::from(std::os::fd::OwnedFd::from(v_g)));
    script.say(owner_lane::DONE).expect("script");
}

/// R5-31, quiescence shape: the host aborted a host-opened flow before the
/// guest accepted V_g (request written, V_h closed, V_g still queued). The
/// production guest owner ends the flow: it never pairs, no C reaches the
/// application and no byte; within 1 s the owner holds nothing of it. The
/// lane checks the control session carries no message for the flow. The
/// positive control flow is then served end to end.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_flow_aborted_before_its_v_g_is_accepted_ends_with_no_application_connection() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pause_until_heard(owner_lane::QUEUED);
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    cell.say(owner_lane::DONE);
}

/// R5-31, session-loss shape: the host aborted the flow by control-session
/// loss — request written, V_h closed, then the session closed, V_g still
/// queued — and the guest owner reconnects. The flow ends with no C and no
/// byte; the owner holds its session again after the reconnect; the new
/// session carries no message for the flow (checked by the lane). The
/// positive control flow is then served on the new session.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_flow_aborted_by_session_loss_ends_and_the_session_is_held_again() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pause_until_heard(owner_lane::QUEUED_THEN_SESSION_CLOSED);
    cell.pump_until_heard(owner_lane::SESSION_ACCEPTED);
    let obs = cell.observe();
    assert!(obs.session, "the owner holds its session again after the reconnect: {obs:?}");
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    cell.say(owner_lane::DONE);
}

/// R5-28, the guest-accepted order: the guest owner has accepted V_g, read
/// the request and issued its connect to the application (`Accepted`) when
/// the host closes V_h having written nothing after the request. The
/// guest half ends through that close: C, if it was connected, is reset with
/// no byte; within 1 s the owner holds nothing of the flow. The lane checks
/// the guest wrote at most the flow's (late) `Paired` and answers it with
/// nothing.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn a_flow_the_host_closes_after_the_guest_accepted_it_ends_on_the_guest() {
    let mut cell = OwnerCell::start();
    let flow = host_flow(owner_lane::ABORTED_FLOW);
    let attached_before = cell.observe().forwarder_attached;
    cell.pump_until_heard(owner_lane::REQUESTED);
    cell.pump_until("the flow accepted", LANE_BOUND, |o| o.accepted.contains_key(&flow));
    cell.say(owner_lane::ACCEPTED);
    cell.script
        .hear("CLOSED", LANE_BOUND)
        .map(|l| assert_eq!(l, owner_lane::CLOSED))
        .expect("script");
    let settle = Instant::now() + ABORT_SETTLE;
    cell.pump_until("the flow ended", ABORT_SETTLE, |o| {
        !o.accepted.contains_key(&flow) && !o.flows.contains_key(&flow)
    });
    while Instant::now() < settle {
        cell.dispatch();
    }
    for c in cell.app.take_all() {
        assert_eq!(app_end(&c, QUIET), (0, AppEnd::Reset), "C is reset and carried no byte");
    }
    let obs = cell.observe();
    assert_eq!(
        (obs.accepted.contains_key(&flow), obs.flows.contains_key(&flow)),
        (false, false),
        "nothing of the flow is left after the owner has run for 1 s: {obs:?}"
    );
    // Whether the owner paired the flow before it saw the close depends on
    // its dispatch order, so the cell pins the invariant instead: a pair the
    // owner attached for this flow ended through the close and yielded
    // exactly one consumed end record, an abort; a flow it never attached
    // yielded none.
    let attached = obs.forwarder_attached - attached_before;
    assert_eq!(
        obs.consumed_ends, attached,
        "one consumed end record per pair the close ended, none for an unattached flow: {obs:?}"
    );
    if attached > 0 {
        let end = obs.last_consumed_end.expect("the consumed end record is kept");
        assert!(
            matches!(end.outcome, ForwardOutcome::Aborted { .. }),
            "a pair the close ended is an abort: {end:?}"
        );
    }
    cell.serve_control_flow();
    cell.say(owner_lane::DONE);
}

/// R5-28, the not-yet-held order: the host sends `Abort` for a flow the
/// guest does not hold; the owner runs for `QUIET` with it (and discards
/// it — its count is discharged by the seeded-sim half) and keeps its
/// session; then the host opens, requests and closes that flow's V_h while
/// the owner is paused (V_g still queued). The flow ends through the host's
/// close with no C and no byte. The lane checks the guest sent no reply to
/// the `Abort` and no message for the flow.
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
fn an_abort_for_a_flow_the_guest_does_not_hold_changes_nothing_and_the_flow_ends_on_the_close() {
    let mut cell = OwnerCell::start();
    let attached = cell.observe().forwarder_attached;
    cell.pump_until_heard(owner_lane::ABORT_SENT);
    cell.pump_for(QUIET);
    let obs = cell.observe();
    assert!(obs.session, "the discarded Abort did not end the session: {obs:?}");
    assert!(
        !obs.accepted.contains_key(&host_flow(owner_lane::ABORTED_FLOW)),
        "an Abort for an unheld flow creates nothing: {obs:?}"
    );
    cell.pause_until_heard(owner_lane::QUEUED);
    cell.serve_control_flow_after_abort(host_flow(owner_lane::ABORTED_FLOW), attached);
    assert!(cell.observe().session, "the session is still held after the flow's end");
    cell.say(owner_lane::DONE);
}

/// R5-31's burst: `BURST` host-opened flows paired, then every V_h closed
/// at once by the host. Every C is reset with no byte and, 1 s after the
/// host's close, the owner holds none of them. The abort-to-reset latency
/// (from the cell hearing `CLOSED-ALL` to each C's reset) is reported as p50
/// / p99 and not asserted (P-38: p99 112 ms on 2 vCPUs, from guest
/// scheduling).
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step 5"]
#[expect(clippy::print_stdout, reason = "R5-31's abort-to-reset latency is reported, not asserted")]
fn a_burst_of_host_aborts_resets_every_application_connection() {
    let mut cell = OwnerCell::start();
    let burst = owner_lane::BURST_FIRST..owner_lane::BURST_FIRST + owner_lane::BURST;
    cell.pump_until_heard(owner_lane::REQUESTED);
    let flows: Vec<FlowId> = burst.map(host_flow).collect();
    cell.pump_until("every burst flow paired", LANE_BOUND * 4, |o| {
        flows.iter().all(|f| matches!(o.accepted.get(f), Some(GuestAcceptPhase::Paired { .. })))
    });
    let deadline = Instant::now() + LANE_BOUND;
    while cell.app.count() < flows.len() {
        assert!(Instant::now() < deadline, "only {} Cs reached the app", cell.app.count());
        cell.dispatch();
    }
    let cs = cell.app.take_all();
    for c in &cs {
        c.set_nonblocking(true).expect("non-blocking C");
    }
    cell.say(owner_lane::ALL_PAIRED);
    cell.script
        .hear("CLOSED-ALL", LANE_BOUND)
        .map(|l| assert_eq!(l, owner_lane::CLOSED_ALL))
        .expect("script");
    let t0 = Instant::now();
    let reader = thread::spawn(move || {
        let mut ended: Vec<Option<(Duration, AppEnd, usize)>> = cs.iter().map(|_| None).collect();
        let mut buf = [0u8; 4096];
        while ended.iter().any(Option::is_none) && t0.elapsed() < LANE_BOUND {
            for (c, slot) in cs.iter().zip(ended.iter_mut()) {
                if slot.is_some() {
                    continue;
                }
                let mut r = c;
                match r.read(&mut buf) {
                    Ok(0) => *slot = Some((t0.elapsed(), AppEnd::Eof, 0)),
                    Ok(n) => *slot = Some((t0.elapsed(), AppEnd::Other(format!("{n} bytes")), n)),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                        *slot = Some((t0.elapsed(), AppEnd::Reset, 0));
                    }
                    Err(e) => *slot = Some((t0.elapsed(), AppEnd::Other(e.to_string()), 0)),
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
        ended
    });
    cell.pump_until("every burst flow ended", ABORT_SETTLE, |o| {
        flows.iter().all(|f| !o.accepted.contains_key(f) && !o.flows.contains_key(f))
    });
    let ended = reader.join().expect("the reader thread");
    let mut latencies: Vec<Duration> = Vec::with_capacity(ended.len());
    for (i, slot) in ended.into_iter().enumerate() {
        let (at, end, bytes) = slot.unwrap_or_else(|| panic!("burst C {i} never ended"));
        assert_eq!((end, bytes), (AppEnd::Reset, 0), "burst C {i} is reset with no byte");
        assert!(at <= ABORT_SETTLE, "burst C {i} still open {at:?} after the abort");
        latencies.push(at);
    }
    latencies.sort();
    let p = |q: usize| latencies[(latencies.len() * q / 100).min(latencies.len() - 1)];
    println!(
        "R5-31 burst of {}: abort-to-reset p50 {:?}, p99 {:?} (reported, not asserted)",
        latencies.len(),
        p(50),
        p(99)
    );
    cell.say(owner_lane::DONE);
}
