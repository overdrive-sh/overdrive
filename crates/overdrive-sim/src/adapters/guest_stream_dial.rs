//! `SimGuestStreamDial` — the sim binding of the guest-flow owner's
//! [`GuestStreamDial`] driving port.
//!
//! netns-density-295 INB-1, ADR-0173;
//! PROBE-NOSAMPLE, ADR-0187; the stream's end vocabulary EVMOD-STREAM-END and
//! EVMOD-STREAM-ABORT, approved 2026-10-17.
//!
//! Script-driven outcome injection, the same shape as
//! [`crate::adapters::probers`]: the harness scripts, per target address and
//! [`DialPurpose`], per target, or as the default for every target, what each
//! later `dial` returns — `Ok` with an in-memory [`GuestStream`] whose guest
//! side replies scripted bytes and ends its direction as scripted, or one
//! [`GuestDialError`] — at once, or later: a [`SimDialScript::Pending`] dial
//! parks until the harness answers it ([`SimGuestStreamDial::answer_pending`])
//! or the dialer drops its future (recorded as [`SimDial::cancelled`], the
//! cancellation the owner ends like a pairing deadline). Scripts are sticky:
//! every dial of a target returns that target's script until the harness
//! scripts it again. Lookup order: the `(target, purpose)` script, then the
//! target's, then the default.
//!
//! Inputs are accepted as the production owner accepts them: every target is
//! answered, never a panic. An unscripted target is answered
//! [`GuestDialError::NotServing`] — the production answer for an address that
//! is not an Active VM allocation's declared port.
//!
//! # The stream's end vocabulary (the production `GuestStream` contract)
//!
//! The in-memory stream answers as the owner's vsock adapter does (feature
//! delta § *Driving port — guest stream dial*, "How an end is resolved"):
//!
//! - **Reads.** The scripted reply in order; then, once the guest ended its
//!   direction, `Ok(0)` for [`GuestStreamEnd::HalfClose`] (the guest's vsock
//!   SHUTDOWN 0x2) or `Err(ConnectionReset)` for [`GuestStreamEnd::Abort`] (a
//!   guest reset, `0x3` with a direction open, or the VM's end). A guest
//!   that has not ended its direction parks the read until it does.
//! - **Owner abort** ([`SimDial::owner_abort`]: quiescence's first hold,
//!   teardown, session loss, the `Forwarder` recovery): every pending and
//!   later `read`, `write` and `shutdown_write` returns
//!   `Err(ConnectionAborted)` at once — a parked read is woken — and `end`
//!   reports `Some(Abort)`.
//! - **Writes.** Recorded while the host→guest direction is open, also after
//!   the guest half-closed; after `shutdown_write`, `Err(BrokenPipe)`; after
//!   a guest abort, `Err(ConnectionReset)`.
//! - **`shutdown_write`** delivers one SHUTDOWN 0x2 to the guest
//!   ([`SimDial::host_half_closed`]); a second call returns `Ok(())` and
//!   sends nothing.
//! - **Drop** delivers the close (`0x3`) to the guest ([`SimDial::host_closed`]).
//!
//! Every dial is recorded (target, purpose, the bytes the dialer wrote into
//! the returned stream, and how the host ended it), so tests observe the
//! calls the system under test made at this port.

#![allow(
    clippy::missing_const_for_fn,
    reason = "constructors build `Mutex<...>` fields, which are not const-constructible"
)]

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddrV4;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::Notify;

use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd, GuestStreamIo,
};

/// What a dial of a scripted target returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimDialScript {
    /// `dial` returns `Ok`: the guest accepted the connection. The returned
    /// stream yields `reply` to the dialer's reads, then ends the guest's
    /// direction with [`GuestStreamEnd::HalfClose`].
    Accept {
        /// The bytes the guest application sends back.
        reply: Vec<u8>,
    },
    /// `dial` returns `Ok`; the stream yields `reply`, then the guest ends
    /// its direction with `end` (`HalfClose`: the reads return `Ok(0)`;
    /// `Abort`: they return `Err(ConnectionReset)`).
    AcceptEnding {
        /// The bytes the guest application sends back.
        reply: Vec<u8>,
        /// How the guest ends its direction after `reply`.
        end: GuestStreamEnd,
    },
    /// `dial` returns `Ok`; the stream yields `reply` and the guest keeps its
    /// direction open — a read after `reply` parks — until the harness ends
    /// it ([`SimDial::guest_end`]) or the owner aborts the stream
    /// ([`SimDial::owner_abort`]).
    AcceptOpen {
        /// The bytes the guest application sends back.
        reply: Vec<u8>,
    },
    /// `dial` returns this refusal.
    Refuse(GuestDialError),
    /// `dial` parks until the harness answers it
    /// ([`SimGuestStreamDial::answer_pending`]) with one of the other
    /// scripts; a dial future dropped before the answer is recorded as
    /// [`SimDial::cancelled`] and returns nothing.
    Pending,
}

impl SimDialScript {
    /// A guest that accepts and sends nothing (a TCP-connect probe target).
    #[must_use]
    pub fn accept() -> Self {
        Self::Accept { reply: Vec::new() }
    }
}

/// The shared state of one sim stream: the guest side the harness drives and
/// the host side the dialer holds.
#[derive(Debug, Default)]
struct StreamState {
    /// The guest application's bytes not yet read by the dialer.
    unread: Vec<u8>,
    /// How the guest ended its direction, once it has.
    guest_end: Option<GuestStreamEnd>,
    /// The owner ended the stream (EVMOD-STREAM-ABORT).
    owner_aborted: bool,
    /// The bytes the dialer wrote toward the guest.
    written: Vec<u8>,
    /// The guest received the host's SHUTDOWN 0x2 (`shutdown_write`).
    host_half_closed: bool,
    /// The guest received the host's close `0x3` (the stream was dropped).
    host_closed: bool,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<StreamState>,
    changed: Notify,
}

/// The answer of a [`SimDialScript::Pending`] dial.
#[derive(Debug, Default)]
struct PendingState {
    /// The harness's answer, once given; never [`SimDialScript::Pending`].
    answer: Option<SimDialScript>,
    /// The dial future took the answer.
    taken: bool,
    /// The dial future was dropped before it took an answer.
    cancelled: bool,
}

#[derive(Debug, Default)]
struct Pending {
    state: Mutex<PendingState>,
    answered: Notify,
}

/// Marks a pending dial cancelled when its future is dropped before it took
/// the harness's answer.
struct CancelOnDrop(Arc<Pending>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let mut state = self.0.state.lock();
        if !state.taken {
            state.cancelled = true;
        }
    }
}

/// One recorded call of [`GuestStreamDial::dial`], and the harness's handle
/// on the stream it returned.
#[derive(Debug, Clone)]
pub struct SimDial {
    /// The address `dial` was called with.
    pub target: SocketAddrV4,
    /// The purpose `dial` was called with.
    pub purpose: DialPurpose,
    /// The script that answered the call ([`SimDialScript::Pending`] for a
    /// dial the harness answers later; [`Self::answer`] is its answer).
    pub answered: SimDialScript,
    shared: Arc<Shared>,
    pending: Option<Arc<Pending>>,
}

impl SimDial {
    /// The bytes the dialer wrote into the stream this call returned (empty
    /// for a refused dial).
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        self.shared.state.lock().written.clone()
    }

    /// The guest application sends `bytes` (an open guest direction only;
    /// bytes after the guest's end are dropped, as a closed socket drops
    /// them). Returns whether they were delivered.
    pub fn guest_send(&self, bytes: &[u8]) -> bool {
        let delivered = {
            let mut state = self.shared.state.lock();
            let open = state.guest_end.is_none() && !state.owner_aborted;
            if open {
                state.unread.extend_from_slice(bytes);
            }
            open
        };
        self.shared.changed.notify_waiters();
        delivered
    }

    /// The guest ends its direction: `HalfClose` (vsock SHUTDOWN 0x2, the
    /// guest application half-closed) or `Abort` (a guest reset, `0x3` with
    /// a direction open, or the VM's end). The first end is fixed; a later
    /// one changes nothing. Returns whether this call ended the direction.
    pub fn guest_end(&self, end: GuestStreamEnd) -> bool {
        let ended = {
            let mut state = self.shared.state.lock();
            let first = state.guest_end.is_none() && !state.owner_aborted;
            if first {
                state.guest_end = Some(end);
            }
            first
        };
        self.shared.changed.notify_waiters();
        ended
    }

    /// The owner ends the stream (EVMOD-STREAM-ABORT): every pending and
    /// later call returns `Err(ConnectionAborted)` at once.
    pub fn owner_abort(&self) {
        self.shared.state.lock().owner_aborted = true;
        self.shared.changed.notify_waiters();
    }

    /// The guest has received the host's SHUTDOWN 0x2.
    #[must_use]
    pub fn host_half_closed(&self) -> bool {
        self.shared.state.lock().host_half_closed
    }

    /// The guest has received the host's close `0x3` (the stream dropped).
    #[must_use]
    pub fn host_closed(&self) -> bool {
        self.shared.state.lock().host_closed
    }

    /// A [`SimDialScript::Pending`] dial that is still waiting for the
    /// harness's answer, its future not dropped.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            let state = pending.state.lock();
            state.answer.is_none() && !state.cancelled
        })
    }

    /// A [`SimDialScript::Pending`] dial whose future the dialer dropped
    /// before it took an answer (the cancellation the owner ends like a
    /// pairing deadline). `false` for every other dial.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.pending.as_ref().is_some_and(|pending| pending.state.lock().cancelled)
    }

    /// The harness's answer to a [`SimDialScript::Pending`] dial, once given.
    #[must_use]
    pub fn answer(&self) -> Option<SimDialScript> {
        self.pending.as_ref().and_then(|pending| pending.state.lock().answer.clone())
    }
}

/// Script-driven [`GuestStreamDial`] sim binding.
pub struct SimGuestStreamDial {
    by_purpose: Mutex<BTreeMap<(SocketAddrV4, u8), SimDialScript>>,
    by_target: Mutex<BTreeMap<SocketAddrV4, SimDialScript>>,
    default: Mutex<SimDialScript>,
    dials: Mutex<Vec<SimDial>>,
}

impl Default for SimGuestStreamDial {
    fn default() -> Self {
        Self::new()
    }
}

impl SimGuestStreamDial {
    /// A dial port with no scripts: every target is answered
    /// [`GuestDialError::NotServing`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_purpose: Mutex::new(BTreeMap::new()),
            by_target: Mutex::new(BTreeMap::new()),
            default: Mutex::new(SimDialScript::Refuse(GuestDialError::NotServing)),
            dials: Mutex::new(Vec::new()),
        }
    }

    /// Script every later dial of `target` (sticky until re-scripted).
    pub fn script(&self, target: SocketAddrV4, script: SimDialScript) {
        self.by_target.lock().insert(target, script);
    }

    /// Script every later dial of `target` made with `purpose` (sticky until
    /// re-scripted); it takes precedence over the target's script.
    pub fn script_purpose(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
        script: SimDialScript,
    ) {
        self.by_purpose.lock().insert((target, purpose_key(purpose)), script);
    }

    /// Script every later dial of a target that has no script of its own.
    pub fn script_default(&self, script: SimDialScript) {
        *self.default.lock() = script;
    }

    /// Answer the oldest [`SimDialScript::Pending`] dial of `target` that is
    /// still waiting with `script`. `false` when no such dial waits, or when
    /// `script` is itself [`SimDialScript::Pending`] (nothing is answered).
    pub fn answer_pending(&self, target: SocketAddrV4, script: SimDialScript) -> bool {
        if matches!(script, SimDialScript::Pending) {
            return false;
        }
        let waiting = self
            .dials
            .lock()
            .iter()
            .filter(|dial| dial.target == target && dial.is_pending())
            .find_map(|dial| dial.pending.clone());
        let Some(pending) = waiting else { return false };
        pending.state.lock().answer = Some(script);
        pending.answered.notify_waiters();
        true
    }

    /// The dials of `target` still waiting for the harness's answer.
    #[must_use]
    pub fn pending_dials(&self, target: SocketAddrV4) -> usize {
        self.dials.lock().iter().filter(|dial| dial.target == target && dial.is_pending()).count()
    }

    fn lookup(&self, target: SocketAddrV4, purpose: DialPurpose) -> SimDialScript {
        if let Some(script) = self.by_purpose.lock().get(&(target, purpose_key(purpose))) {
            return script.clone();
        }
        self.by_target.lock().get(&target).cloned().unwrap_or_else(|| self.default.lock().clone())
    }

    /// Every dial so far, in call order.
    #[must_use]
    pub fn dials(&self) -> Vec<SimDial> {
        self.dials.lock().clone()
    }

    /// Number of dials so far.
    #[must_use]
    pub fn dial_count(&self) -> usize {
        self.dials.lock().len()
    }

    /// The dials of `target` so far, in call order.
    #[must_use]
    pub fn dials_of(&self, target: SocketAddrV4) -> Vec<SimDial> {
        self.dials.lock().iter().filter(|d| d.target == target).cloned().collect()
    }
}

#[async_trait]
impl GuestStreamDial for SimGuestStreamDial {
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        let script = self.lookup(target, purpose);
        let (dial, io) = open_stream(target, purpose, script);
        self.dials.lock().push(dial.clone());
        let Some(pending) = dial.pending.clone() else {
            return match (dial.answered, io) {
                (SimDialScript::Refuse(error), _) => Err(error),
                (_, Some(io)) => Ok(GuestStream::new(target, Box::new(io))),
                (_, None) => unreachable!("open_stream returns an I/O for every accepting script"),
            };
        };
        let guard = CancelOnDrop(Arc::clone(&pending));
        let answer = loop {
            let answered = pending.answered.notified();
            tokio::pin!(answered);
            answered.as_mut().enable();
            let ready = {
                let mut state = pending.state.lock();
                let answer = state.answer.clone();
                if answer.is_some() {
                    state.taken = true;
                }
                answer
            };
            if let Some(answer) = ready {
                break answer;
            }
            answered.await;
        };
        drop(guard);
        resolve_pending(target, &dial.shared, answer)
    }
}

/// The `BTreeMap` key of a purpose (`DialPurpose` is not `Ord`).
const fn purpose_key(purpose: DialPurpose) -> u8 {
    match purpose {
        DialPurpose::Relay => 0,
        DialPurpose::Probe => 1,
    }
}

/// The result of a pending dial the harness answered with `answer`.
fn resolve_pending(
    target: SocketAddrV4,
    shared: &Arc<Shared>,
    answer: SimDialScript,
) -> Result<GuestStream, GuestDialError> {
    let (unread, guest_end) = match answer {
        SimDialScript::Refuse(error) => return Err(error),
        SimDialScript::Accept { reply } => (reply, Some(GuestStreamEnd::HalfClose)),
        SimDialScript::AcceptEnding { reply, end } => (reply, Some(end)),
        SimDialScript::AcceptOpen { reply } => (reply, None),
        SimDialScript::Pending => {
            unreachable!("answer_pending never answers a dial with Pending")
        }
    };
    {
        let mut state = shared.state.lock();
        state.unread = unread;
        state.guest_end = guest_end;
    }
    Ok(GuestStream::new(
        target,
        Box::new(SimGuestStreamIo { shared: Arc::clone(shared), write_shut: false }),
    ))
}

/// The recorded dial and, for an accepting script, the stream's I/O.
fn open_stream(
    target: SocketAddrV4,
    purpose: DialPurpose,
    script: SimDialScript,
) -> (SimDial, Option<SimGuestStreamIo>) {
    let (unread, guest_end, accepts) = match &script {
        SimDialScript::Accept { reply } => (reply.clone(), Some(GuestStreamEnd::HalfClose), true),
        SimDialScript::AcceptEnding { reply, end } => (reply.clone(), Some(*end), true),
        SimDialScript::AcceptOpen { reply } => (reply.clone(), None, true),
        SimDialScript::Refuse(_) | SimDialScript::Pending => (Vec::new(), None, false),
    };
    let shared = Arc::new(Shared {
        state: Mutex::new(StreamState { unread, guest_end, ..StreamState::default() }),
        changed: Notify::new(),
    });
    let io = accepts.then(|| SimGuestStreamIo { shared: Arc::clone(&shared), write_shut: false });
    let pending = matches!(script, SimDialScript::Pending).then(|| Arc::new(Pending::default()));
    (SimDial { target, purpose, answered: script, shared, pending }, io)
}

fn aborted() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "the owner ended the stream")
}

fn reset() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionReset, "the guest aborted its direction")
}

/// In-memory I/O behind a sim [`GuestStream`]: reads drain the guest's
/// bytes and then report its end; writes are recorded for the harness.
#[derive(Debug)]
pub struct SimGuestStreamIo {
    shared: Arc<Shared>,
    write_shut: bool,
}

#[async_trait]
impl GuestStreamIo for SimGuestStreamIo {
    async fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            // Register for the wake before reading the state, so a change
            // between the check and the park is not lost.
            let changed = self.shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.shared.state.lock();
                if state.owner_aborted {
                    return Err(aborted());
                }
                if !state.unread.is_empty() {
                    let n = state.unread.len().min(buf.len());
                    buf[..n].copy_from_slice(&state.unread[..n]);
                    state.unread.drain(..n);
                    return Ok(n);
                }
                match state.guest_end {
                    Some(GuestStreamEnd::HalfClose) => return Ok(0),
                    Some(GuestStreamEnd::Abort) => return Err(reset()),
                    None => {}
                }
            }
            changed.await;
        }
    }

    async fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.shared.state.lock();
        if state.owner_aborted {
            return Err(aborted());
        }
        if self.write_shut {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "host→guest direction already shut down",
            ));
        }
        if state.guest_end == Some(GuestStreamEnd::Abort) {
            return Err(reset());
        }
        state.written.extend_from_slice(buf);
        drop(state);
        Ok(buf.len())
    }

    async fn shutdown_write(&mut self) -> io::Result<()> {
        let mut state = self.shared.state.lock();
        if state.owner_aborted {
            return Err(aborted());
        }
        if !self.write_shut {
            self.write_shut = true;
            state.host_half_closed = true;
        }
        drop(state);
        Ok(())
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        let state = self.shared.state.lock();
        if state.owner_aborted {
            return Some(GuestStreamEnd::Abort);
        }
        match state.guest_end {
            Some(GuestStreamEnd::HalfClose) if state.unread.is_empty() => {
                Some(GuestStreamEnd::HalfClose)
            }
            Some(GuestStreamEnd::Abort) => Some(GuestStreamEnd::Abort),
            _ => None,
        }
    }
}

impl Drop for SimGuestStreamIo {
    /// The stream's drop closes `V_h`: the guest receives `0x3`.
    fn drop(&mut self) {
        self.shared.state.lock().host_closed = true;
        self.shared.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    //! The sim stream's end vocabulary matches the production `GuestStream`
    //! contract (EVMOD-STREAM-END, EVMOD-STREAM-ABORT), so a consumer tested
    //! over the sim sees the outcomes the owner's adapter delivers.

    #![allow(clippy::expect_used, reason = "test preconditions panic with a message")]

    use std::net::Ipv4Addr;
    use std::time::Duration;

    use super::*;

    const TARGET: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(100, 96, 0, 2), 8080);

    fn open(script: SimDialScript) -> (SimDial, SimGuestStreamIo) {
        let (dial, io) = open_stream(TARGET, DialPurpose::Relay, script);
        (dial, io.expect("an accepting script opens a stream"))
    }

    async fn read_all(io: &mut SimGuestStreamIo) -> (Vec<u8>, io::Result<usize>) {
        let mut bytes = Vec::new();
        let mut buf = [0_u8; 4];
        loop {
            match io.read(&mut buf).await {
                Ok(0) => return (bytes, Ok(0)),
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                Err(error) => return (bytes, Err(error)),
            }
        }
    }

    /// A guest half-close (SHUTDOWN 0x2) reads as `Ok(0)` after every byte,
    /// and the host may still write; `end` is `HalfClose`.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn a_guest_half_close_reads_zero_after_its_bytes_and_the_host_may_still_write() {
        let (dial, mut io) = open(SimDialScript::AcceptEnding {
            reply: b"pong-reply".to_vec(),
            end: GuestStreamEnd::HalfClose,
        });
        assert_eq!(io.end(), None, "unread bytes remain: the end is not yet resolved");
        let (bytes, last) = read_all(&mut io).await;
        assert_eq!(bytes, b"pong-reply");
        assert_eq!(last.expect("a half-close is a clean end"), 0);
        assert_eq!(io.end(), Some(GuestStreamEnd::HalfClose));
        assert_eq!(io.write(b"late").await.expect("host direction still open"), 4);
        assert_eq!(dial.written(), b"late");
    }

    /// A guest abort (reset, `0x3` with a direction open, the VM's end)
    /// reads `Err(ConnectionReset)` after the bytes; writes fail the same.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn a_guest_abort_reads_and_writes_connection_reset() {
        let (_dial, mut io) =
            open(SimDialScript::AcceptEnding { reply: b"ab".to_vec(), end: GuestStreamEnd::Abort });
        let (bytes, last) = read_all(&mut io).await;
        assert_eq!(bytes, b"ab");
        assert_eq!(
            last.expect_err("an abort is not a clean end").kind(),
            io::ErrorKind::ConnectionReset
        );
        assert_eq!(io.end(), Some(GuestStreamEnd::Abort));
        let write = io.write(b"x").await.expect_err("an aborted guest takes no bytes");
        assert_eq!(write.kind(), io::ErrorKind::ConnectionReset);
    }

    /// An owner abort wakes a read parked on an open guest direction with
    /// `Err(ConnectionAborted)`; every later call returns the same at once and
    /// `end` reports `Abort` (EVMOD-STREAM-ABORT).
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn an_owner_abort_wakes_a_parked_read_and_ends_every_later_call() {
        let (dial, mut io) = open(SimDialScript::AcceptOpen { reply: Vec::new() });
        let parked = tokio::spawn(async move {
            let mut buf = [0_u8; 8];
            let result = io.read(&mut buf).await;
            (io, result)
        });
        tokio::task::yield_now().await;
        assert!(!parked.is_finished(), "an open guest direction parks the read");
        dial.owner_abort();
        let (mut io, result) = tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the owner's abort wakes the parked read")
            .expect("the read task joins");
        assert_eq!(result.expect_err("owner abort").kind(), io::ErrorKind::ConnectionAborted);
        assert_eq!(
            io.write(b"x").await.expect_err("owner abort").kind(),
            io::ErrorKind::ConnectionAborted
        );
        assert_eq!(
            io.shutdown_write().await.expect_err("owner abort").kind(),
            io::ErrorKind::ConnectionAborted
        );
        assert_eq!(io.end(), Some(GuestStreamEnd::Abort));
        assert!(!dial.guest_send(b"late"), "no guest byte is delivered after the owner's end");
    }

    /// `shutdown_write` delivers one SHUTDOWN 0x2 to the guest; a second call
    /// sends nothing; a write after it is `BrokenPipe`; the drop delivers the
    /// close (`0x3`).
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn shutdown_write_sends_one_half_close_and_the_drop_closes() {
        let (dial, mut io) = open(SimDialScript::AcceptOpen { reply: Vec::new() });
        assert!(!dial.host_half_closed());
        io.shutdown_write().await.expect("first shutdown_write");
        io.shutdown_write().await.expect("a second shutdown_write is Ok and sends nothing");
        assert!(dial.host_half_closed(), "the guest received 0x2");
        assert_eq!(
            io.write(b"x").await.expect_err("after shutdown_write").kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(!dial.host_closed(), "the stream is still held");
        drop(io);
        assert!(dial.host_closed(), "the drop closes V_h: the guest receives 0x3");
    }

    /// A pending dial parks until the harness answers it; the answer decides
    /// the result, and the dial is no longer pending.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn a_pending_dial_returns_the_harness_answer() {
        let port = SimGuestStreamDial::new();
        port.script(TARGET, SimDialScript::Pending);
        let dialer = Arc::new(port);
        let task = {
            let dialer = Arc::clone(&dialer);
            tokio::spawn(async move { dialer.dial(TARGET, DialPurpose::Relay).await })
        };
        tokio::task::yield_now().await;
        assert!(!task.is_finished(), "a pending dial parks");
        assert_eq!(dialer.pending_dials(TARGET), 1);
        assert!(!dialer.answer_pending(TARGET, SimDialScript::Pending), "Pending is not an answer");
        assert!(
            dialer.answer_pending(TARGET, SimDialScript::Refuse(GuestDialError::PairingDeadline))
        );
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the answer wakes the dial")
            .expect("the dial task joins");
        assert_eq!(result.expect_err("answered with a refusal"), GuestDialError::PairingDeadline);
        assert_eq!(dialer.pending_dials(TARGET), 0);
        let dial = dialer.dials_of(TARGET).pop().expect("one recorded dial");
        assert_eq!(dial.answer(), Some(SimDialScript::Refuse(GuestDialError::PairingDeadline)));
        assert!(!dial.cancelled());
    }

    /// A pending dial whose future is dropped before an answer is recorded as
    /// cancelled, and a later answer reaches nothing.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn a_dropped_pending_dial_is_recorded_cancelled() {
        let dialer = Arc::new(SimGuestStreamDial::new());
        dialer.script(TARGET, SimDialScript::Pending);
        let task = {
            let dialer = Arc::clone(&dialer);
            tokio::spawn(async move { dialer.dial(TARGET, DialPurpose::Probe).await })
        };
        tokio::task::yield_now().await;
        task.abort();
        let _cancelled = task.await;
        let dial = dialer.dials_of(TARGET).pop().expect("one recorded dial");
        assert!(dial.cancelled(), "the dropped future is the cancellation");
        assert!(!dial.is_pending());
        assert!(!dialer.answer_pending(TARGET, SimDialScript::accept()), "nothing waits");
    }

    /// A per-purpose script takes precedence over the target's script for
    /// that purpose only.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn a_purpose_script_answers_only_its_purpose() {
        let dialer = SimGuestStreamDial::new();
        dialer.script(TARGET, SimDialScript::Refuse(GuestDialError::Capacity));
        dialer.script_purpose(TARGET, DialPurpose::Probe, SimDialScript::accept());
        assert_eq!(
            dialer.dial(TARGET, DialPurpose::Relay).await.expect_err("relay is at quota"),
            GuestDialError::Capacity
        );
        let stream = dialer.dial(TARGET, DialPurpose::Probe).await.expect("the probe dial pairs");
        assert_eq!(stream.target(), TARGET);
        let purposes: Vec<DialPurpose> = dialer.dials().iter().map(|d| d.purpose).collect();
        assert_eq!(purposes, vec![DialPurpose::Relay, DialPurpose::Probe]);
    }

    /// Bytes the guest sends later on an open direction are read in order,
    /// then its scripted end resolves the read.
    /// `CONTRACT_SHAPE`: bounded-change.
    #[tokio::test]
    async fn bytes_and_the_end_the_guest_delivers_later_are_read_in_order() {
        let (dial, mut io) = open(SimDialScript::AcceptOpen { reply: b"a".to_vec() });
        assert!(dial.guest_send(b"bc"));
        assert!(dial.guest_end(GuestStreamEnd::HalfClose));
        assert!(!dial.guest_end(GuestStreamEnd::Abort), "the first end is fixed");
        let (bytes, last) = read_all(&mut io).await;
        assert_eq!(bytes, b"abc");
        assert_eq!(last.expect("half-close"), 0);
    }
}
