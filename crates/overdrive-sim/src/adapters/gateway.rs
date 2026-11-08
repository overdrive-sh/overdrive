//! Sim adapters of the public-ingress gateway driven ports.
//!
//! This file holds the kernel-relay share (G1):
//! [`SimGatewayRelay`] — the `GatewayRelay` port backed by in-memory record
//! queues on the injected clock — and its test-support surface; and the
//! dataplane-side share: [`SimGatewayConnectDataplane`] (over the shared
//! `SimDataplane` state), [`SimGatewayClientMtls`] (the scripted exact-peer
//! client, arming through the Sim relay), [`SimConnectHold`] (the gate hold
//! the connect dataplane exposes and the identity control parks on), and the gateway identity doubles
//! ([`SimGatewayIdentityActionTarget`], [`SimGatewayIdentityLifecycleControl`]).
//! [`SimGatewayClientMtlsFactory`] and
//! [`SimGatewayAdapters`] (R-20) assemble the Sim relay and the Sim exact-peer
//! client behind the pinned `GatewayClientMtlsFactory` port.
//!
//! # Test-support surface (shaped by DISTILL; capabilities, not production API)
//!
//! `SimGatewayRelay` implements the pinned `GatewayRelay` contract over
//! in-memory leg state. The DESIGN pins only the port and `SimGatewayRelay::new`;
//! everything below exists for tests and lives only in this crate:
//!
//! * [`SubstrateLie`] — what the Earned-Trust `probe` is shown (S-PIG-99, 100).
//! * [`LegScript`] / [`LegEvent`] — each leg's inbound application octets with
//!   control events placed at octet offsets by their position in the event
//!   list (S-PIG-83, 87–95, 102, 103, 119–122). `Quiet` makes the next event
//!   deliverable only after a span of the injected clock, measured from the
//!   previous event's delivery; for an upstream leg the first span is measured
//!   from the completion of the first `write_all` on that leg (the instant `t0`
//!   of S-PIG-120), for a public leg from `accept_public`. A read that has no
//!   deliverable event waits on the clock and ends in `Deadline` / `NoProgress`
//!   when the passed deadline elapses (no sleeping: the test advances the clock).
//!   A script whose last event is a `Quiet` ends with it: once the span has
//!   passed, the peer closes cleanly. Any other script that has played out stays
//!   silent.
//! * [`PublicLegScript`] — one scripted public connection, consumed by the next
//!   `accept_public`. The accepted socket is kept inside the leg, so closing the
//!   leg closes the test client's real socket.
//! * `script_upstream_leg` — the script the next minted upstream leg plays.
//! * `fail_next_public_arm` / `fail_next_upstream_arm` — an arm refusal of a
//!   chosen `KtlsArmError` kind (S-PIG-98).
//! * The `answer_fails` field of [`LegEvent::KeyUpdate`] — a failed answer to a
//!   peer KeyUpdate (S-PIG-90). [`LegScript::with_pipe_failure`] — the pipe fails at
//!   the leg's first `relay_*` call (S-PIG-95). [`LegScript::with_send_window`]
//!   — the peer accepts only that many octets, then stalls (short counts and
//!   backpressure, S-PIG-104's Sim twin of S-PIG-111).
//! * Observation: [`LegTranscript`] per leg (octets written, port calls in
//!   order, close_notify issued, closed), `live_leg_count`, `live_pipe_count`,
//!   `probe_calls`.
//!
//! The Sim never produces a fault the host cannot produce except the substrate
//! lies (probe only) and the two scripted failure kinds above (a failed
//! KeyUpdate answer and an arm refusal); those are excluded from the
//! host/Sim equivalence comparison.

#![allow(
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    clippy::needless_pass_by_ref_mut,
    reason = "Sim port bodies implement async trait signatures that need not use every argument; doc comments name pinned identifiers"
)]
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::ca::issue_failure::SvidIssueFailure;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::eval_broker::{Evaluation, EvaluationBroker, EvaluationEligibility};
use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityIssueRecord,
    GatewayIdentityView,
};
use overdrive_core::id::{BackendId, SpiffeId};
use overdrive_core::public_ingress::{PublicHostname, ServiceKey, SocketCookie};
use overdrive_core::traits::ca::SvidMaterial;
use overdrive_core::traits::clock::Clock;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayClientMtls, GatewayClientMtlsDeps, GatewayClientMtlsError,
    GatewayClientMtlsFactory, GatewayConnectCleanup, GatewayConnectCleanupSweep,
    GatewayConnectDataplane, GatewayConnectDataplaneError, GatewayConnectIntent,
    GatewayIdentityActionTarget, GatewayIdentityLifecycleControl, GatewayIdentityLifecycleError,
    GatewayIdentityLifecycleTarget, GatewayIdentityMutationOutcome, GatewayRelay,
    GatewaySelectionReceipt, GatewayUpstream, GatewayUpstreamSeal, HeadTerminator, KeyUpdateError,
    KtlsArmError, KtlsLeg, KtlsReadHalf, KtlsWriteHalf, LegReadError, LegWriteError,
    PreparedUpstream, PublicHandshakeError, ReadDeadlines, RelayDeadlines, RelayError,
    RelayPipeError, RelayProbeError, ThroughBytes,
};
use parking_lot::Mutex;
use tokio::sync::{Notify, watch};

use super::dataplane::SimDataplane;

// ---------------------------------------------------------------------------
// Test-support data (real)
// ---------------------------------------------------------------------------

/// What the Earned-Trust `probe` is shown.
///
/// `probe` returns the matching `RelayProbeError` variant of the DESIGN's
/// "Earned-Trust probe for the relay":
/// `UlpAbsent` -> `Arm(UlpInstall)`, `CipherRefused` -> `Arm(CipherNotSupported)`,
/// `SpliceInexact` -> `SpliceExactness`, `PipeClamped` -> `Pipe(CapacityBelowFloor)`,
/// `ProbeTimeout` -> `Timeout`, `ProbeIo` -> `Io`. No variant maps to two lies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstrateLie {
    UlpAbsent,
    CipherRefused,
    SpliceInexact { moved: u64, expected: u64 },
    PipeClamped { got: usize },
    ProbeTimeout,
    ProbeIo { kind: std::io::ErrorKind },
}

/// One scripted occurrence on a leg's inbound side, in delivery order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegEvent {
    /// Application octets, delivered as one record.
    Octets(Vec<u8>),
    /// A peer KeyUpdate. `answer_fails`: the relay's answer to it (a requested
    /// update) or its re-arm fails with this error.
    KeyUpdate {
        requested: bool,
        answer_fails: Option<KeyUpdateError>,
    },
    NewSessionTicket,
    /// The peer's close_notify.
    CloseNotify,
    /// A fatal alert with this description.
    Alert {
        description: u8,
    },
    /// A record of an unexpected content type.
    UnexpectedRecord {
        content_type: u8,
    },
    /// The peer resets the connection.
    Reset,
    /// The next event is deliverable only this long after the previous one
    /// (see the module documentation for the first span's anchor).
    Quiet(Duration),
}

/// The inbound behaviour of one leg plus its optional faults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegScript {
    events: Vec<LegEvent>,
    send_window: Option<usize>,
    pipe_failure: Option<RelayPipeError>,
}

impl LegScript {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub fn octets(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.events.push(LegEvent::Octets(bytes.into()));
        self
    }
    #[must_use]
    pub fn key_update(mut self, requested: bool) -> Self {
        self.events.push(LegEvent::KeyUpdate { requested, answer_fails: None });
        self
    }
    #[must_use]
    pub fn key_update_whose_answer_fails(mut self, requested: bool, error: KeyUpdateError) -> Self {
        self.events.push(LegEvent::KeyUpdate { requested, answer_fails: Some(error) });
        self
    }
    #[must_use]
    pub fn new_session_ticket(mut self) -> Self {
        self.events.push(LegEvent::NewSessionTicket);
        self
    }
    #[must_use]
    pub fn close_notify(mut self) -> Self {
        self.events.push(LegEvent::CloseNotify);
        self
    }
    #[must_use]
    pub fn alert(mut self, description: u8) -> Self {
        self.events.push(LegEvent::Alert { description });
        self
    }
    #[must_use]
    pub fn unexpected_record(mut self, content_type: u8) -> Self {
        self.events.push(LegEvent::UnexpectedRecord { content_type });
        self
    }
    #[must_use]
    pub fn reset(mut self) -> Self {
        self.events.push(LegEvent::Reset);
        self
    }
    #[must_use]
    pub fn quiet(mut self, span: Duration) -> Self {
        self.events.push(LegEvent::Quiet(span));
        self
    }
    /// The peer accepts only `octets` application octets, then stops reading.
    #[must_use]
    pub fn with_send_window(mut self, octets: usize) -> Self {
        self.send_window = Some(octets);
        self
    }
    /// The leg's pipe fails at its first `relay_*` call with `error`.
    #[must_use]
    pub fn with_pipe_failure(mut self, error: RelayPipeError) -> Self {
        self.pipe_failure = Some(error);
        self
    }
    #[must_use]
    pub fn events(&self) -> &[LegEvent] {
        &self.events
    }
}

/// One scripted public connection: the SNI the modelled handshake negotiated and
/// the client's inbound behaviour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicLegScript {
    pub sni: PublicHostname,
    pub inbound: LegScript,
}

/// Identity of one leg the Sim handed out, in creation order from zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SimLegId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegRole {
    Public,
    Upstream,
}

/// A port call the relay received for a leg, recorded in call order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCall {
    ReadThrough,
    WriteAll,
    RelayExact,
    RelayToEnd,
    CloseWrite,
    DiscardRemaining,
}

/// Everything the Sim observed on one leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegTranscript {
    pub id: SimLegId,
    pub role: LegRole,
    /// Application octets written to the peer (`write_all` and relay destination).
    pub written: Vec<u8>,
    pub calls: Vec<RelayCall>,
    /// A close_notify was sent to the peer (`close_write`).
    pub close_notify_sent: bool,
    /// Both halves of the leg were dropped (the socket is closed).
    pub closed: bool,
    /// The leg's private pipe was created.
    pub pipe_created: bool,
}

// ---------------------------------------------------------------------------
// SimGatewayRelay
// ---------------------------------------------------------------------------

/// Octets one relay step moves: the relay pipe's capacity floor. A step that
/// fills the pipe is a scheduling point, as the host's splice steps are, so the
/// other half of an exchange runs while a large body moves; smaller moves are
/// not.
const PIPE_STEP: usize = 65_536;

/// How a leg's inbound side ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LegEnd {
    CleanEof,
    Reset,
    Alert(u8),
    Unexpected(u8),
    KeyUpdate(KeyUpdateError),
}

impl LegEnd {
    /// The read error of a read that meets this end with `partial` octets of
    /// the item already received.
    fn read_error(&self, partial: bool) -> LegReadError {
        match self {
            Self::CleanEof if partial => LegReadError::Truncated,
            Self::CleanEof => LegReadError::CleanEof,
            Self::Reset => LegReadError::Io { kind: std::io::ErrorKind::ConnectionReset },
            Self::Alert(description) => LegReadError::Alert { description: *description },
            Self::Unexpected(content_type) => {
                LegReadError::UnexpectedRecord { content_type: *content_type }
            }
            Self::KeyUpdate(error) => LegReadError::KeyUpdate(*error),
        }
    }
}

/// What one attempt to take inbound octets found.
enum Pulled {
    /// Octets were appended to the leg's inbound buffer.
    Octets,
    /// The inbound side ended.
    Ended(LegEnd),
    /// No event is deliverable now; the next one is due at the instant, or only
    /// an anchoring write can unblock it.
    Blocked(Option<Instant>),
}

/// Whether the leg ever created its splice pipe, and whether it is still open.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PipeLife {
    Never,
    Live,
    Released,
}

impl PipeLife {
    const fn released(self) -> Self {
        match self {
            Self::Live => Self::Released,
            other => other,
        }
    }
}

struct LegState {
    events: VecDeque<LegEvent>,
    /// The virtual instant of the last consumed `Quiet` span; `None` until an
    /// upstream leg's first write completes.
    cursor: Option<Instant>,
    /// Inbound octets received and not yet consumed.
    buffer: Vec<u8>,
    /// When the first octet of the buffered item arrived.
    started_at: Option<Instant>,
    ended: Option<LegEnd>,
    /// The script ends in a `Quiet`: once its span passed the peer is gone.
    trailing_quiet: bool,
    send_window: Option<usize>,
    sent: usize,
    pipe_failure: Option<RelayPipeError>,
    pipe: PipeLife,
    written: Vec<u8>,
    calls: Vec<RelayCall>,
    close_notify_sent: bool,
    halves_alive: u8,
    closed: bool,
    /// The accepted or connected socket the leg owns; dropped when the leg
    /// closes, so closing the leg closes the test client's real socket.
    socket: Option<OwnedFd>,
}

impl LegState {
    fn pull(&mut self, now: Instant) -> Pulled {
        loop {
            if let Some(end) = &self.ended {
                return Pulled::Ended(end.clone());
            }
            match self.events.front() {
                None if self.trailing_quiet => match self.cursor {
                    Some(due) if now >= due => self.ended = Some(LegEnd::CleanEof),
                    Some(due) => return Pulled::Blocked(Some(due)),
                    None => return Pulled::Blocked(None),
                },
                None => return Pulled::Blocked(None),
                Some(LegEvent::Quiet(span)) => {
                    let span = *span;
                    let Some(cursor) = self.cursor else {
                        return Pulled::Blocked(None);
                    };
                    self.cursor = Some(cursor + span);
                    self.events.pop_front();
                    self.trailing_quiet = self.events.is_empty();
                }
                Some(_) => {
                    if let Some(cursor) = self.cursor
                        && now < cursor
                    {
                        return Pulled::Blocked(Some(cursor));
                    }
                    let delivered_at = self.cursor.unwrap_or(now);
                    let Some(event) = self.events.pop_front() else {
                        return Pulled::Blocked(None);
                    };
                    match event {
                        LegEvent::Octets(octets) => {
                            if self.buffer.is_empty() {
                                self.started_at = Some(delivered_at);
                            }
                            self.buffer.extend_from_slice(&octets);
                            return Pulled::Octets;
                        }
                        LegEvent::KeyUpdate { answer_fails: Some(error), .. } => {
                            self.ended = Some(LegEnd::KeyUpdate(error));
                        }
                        LegEvent::KeyUpdate { answer_fails: None, .. }
                        | LegEvent::NewSessionTicket
                        | LegEvent::Quiet(_) => {}
                        LegEvent::CloseNotify => self.ended = Some(LegEnd::CleanEof),
                        LegEvent::Alert { description } => {
                            self.ended = Some(LegEnd::Alert(description));
                        }
                        LegEvent::UnexpectedRecord { content_type } => {
                            self.ended = Some(LegEnd::Unexpected(content_type));
                        }
                        LegEvent::Reset => self.ended = Some(LegEnd::Reset),
                    }
                }
            }
        }
    }

    fn release_pipe(&mut self) {
        self.pipe = self.pipe.released();
    }
}

struct LegShared {
    id: SimLegId,
    role: LegRole,
    clock: Arc<dyn Clock>,
    wake: Notify,
    state: Mutex<LegState>,
}

/// Aborts the helper task when the waiting future is dropped.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What an attempt to take octets from a leg produced.
enum Taken {
    Octets(Vec<u8>),
    Ended(LegEnd),
    Blocked(Option<Instant>),
}

impl LegShared {
    fn new(
        id: SimLegId,
        role: LegRole,
        clock: Arc<dyn Clock>,
        script: LegScript,
        cursor: Option<Instant>,
        socket: Option<OwnedFd>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            role,
            clock,
            wake: Notify::new(),
            state: Mutex::new(LegState {
                events: script.events.into(),
                cursor,
                buffer: Vec::new(),
                started_at: None,
                ended: None,
                trailing_quiet: false,
                send_window: script.send_window,
                sent: 0,
                pipe_failure: script.pipe_failure,
                pipe: PipeLife::Never,
                written: Vec::new(),
                calls: Vec::new(),
                close_notify_sent: false,
                halves_alive: 2,
                closed: false,
                socket,
            }),
        })
    }

    fn transcript(&self) -> LegTranscript {
        let state = self.state.lock();
        LegTranscript {
            id: self.id,
            role: self.role,
            written: state.written.clone(),
            calls: state.calls.clone(),
            close_notify_sent: state.close_notify_sent,
            closed: state.closed,
            pipe_created: state.pipe != PipeLife::Never,
        }
    }

    fn record(&self, call: RelayCall) {
        self.state.lock().calls.push(call);
    }

    /// One half was dropped; the leg closes with its second half.
    fn half_dropped(&self, releases_pipe: bool) {
        let mut state = self.state.lock();
        if releases_pipe {
            state.release_pipe();
        }
        state.halves_alive = state.halves_alive.saturating_sub(1);
        if state.halves_alive == 0 {
            state.closed = true;
            state.pipe = state.pipe.released();
            drop(state.socket.take());
        }
    }

    /// Park until `until` on the injected clock or until the leg is woken. The
    /// clock wait runs in its own task so concurrent waits of one task never
    /// share a clock waker.
    async fn park(&self, until: Instant) {
        if until.saturating_duration_since(self.clock.now()).is_zero() {
            return;
        }
        let clock = Arc::clone(&self.clock);
        let mut sleeper = tokio::spawn(async move {
            let remaining = until.saturating_duration_since(clock.now());
            clock.sleep(remaining).await;
        });
        let _abort = AbortOnDrop(sleeper.abort_handle());
        tokio::select! {
            () = self.wake.notified() => {}
            _ = &mut sleeper => {}
        }
    }

    /// Wait on the injected clock alone until `until`.
    async fn sleep_until(&self, until: Instant) {
        let clock = Arc::clone(&self.clock);
        let remaining = until.saturating_duration_since(clock.now());
        let _ = tokio::spawn(async move { clock.sleep(remaining).await }).await;
    }

    /// Take up to `max` inbound octets, buffered ones first.
    fn take(&self, max: usize) -> Taken {
        let now = self.clock.now();
        let mut state = self.state.lock();
        loop {
            if !state.buffer.is_empty() {
                let count = max.min(state.buffer.len());
                let octets: Vec<u8> = state.buffer.drain(..count).collect();
                state.started_at = (!state.buffer.is_empty()).then_some(now);
                return Taken::Octets(octets);
            }
            match state.pull(now) {
                Pulled::Octets => {}
                Pulled::Ended(end) => return Taken::Ended(end),
                Pulled::Blocked(at) => return Taken::Blocked(at),
            }
        }
    }

    /// Put octets back at the front of the inbound buffer.
    fn unread(&self, octets: &[u8]) {
        let mut state = self.state.lock();
        if state.buffer.is_empty() {
            state.started_at = Some(state.cursor.unwrap_or_else(|| self.clock.now()));
        }
        state.buffer.splice(0..0, octets.iter().copied());
    }

    /// Append what the send window admits; returns the admitted count.
    fn write_some(&self, bytes: &[u8]) -> Result<usize, LegWriteError> {
        let mut state = self.state.lock();
        match &state.ended {
            Some(LegEnd::Reset) => return Err(LegWriteError::PeerClosed),
            Some(LegEnd::Alert(description)) => {
                return Err(LegWriteError::Alert { description: *description });
            }
            _ => {}
        }
        let room = state.send_window.map_or(usize::MAX, |window| window.saturating_sub(state.sent));
        let admitted = bytes.len().min(room);
        state.written.extend_from_slice(&bytes[..admitted]);
        state.sent = state.sent.saturating_add(admitted);
        drop(state);
        Ok(admitted)
    }

    /// The first completed write anchors an upstream leg's script clock.
    fn anchor(&self) {
        let mut state = self.state.lock();
        if state.cursor.is_none() {
            state.cursor = Some(self.clock.now());
            drop(state);
            self.wake.notify_one();
        }
    }

    fn create_pipe(&self) -> Result<(), RelayPipeError> {
        let mut state = self.state.lock();
        if let Some(error) = state.pipe_failure.clone() {
            return Err(error);
        }
        state.pipe = PipeLife::Live;
        drop(state);
        Ok(())
    }

    async fn read_through(
        &self,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        let marker: &[u8] = match terminator {
            HeadTerminator::HeaderBlockEnd => b"\r\n\r\n",
            HeadTerminator::LineEnd => b"\r\n",
        };
        loop {
            let now = self.clock.now();
            let wake_at = {
                let mut state = self.state.lock();
                loop {
                    let window = limit.get().min(state.buffer.len());
                    if let Some(at) =
                        state.buffer[..window].windows(marker.len()).position(|w| w == marker)
                    {
                        let end = at + marker.len();
                        let taken: Vec<u8> = state.buffer.drain(..end).collect();
                        state.started_at = (!state.buffer.is_empty()).then_some(now);
                        return Ok(ThroughBytes::new(taken));
                    }
                    if state.buffer.len() >= limit.get() {
                        return Err(LegReadError::LimitExceeded { limit: limit.get() });
                    }
                    match state.pull(now) {
                        Pulled::Octets => {}
                        Pulled::Ended(end) => {
                            return Err(end.read_error(!state.buffer.is_empty()));
                        }
                        Pulled::Blocked(at) => break at,
                    }
                }
            };
            let deadline = {
                let state = self.state.lock();
                let by_item = match (state.started_at, deadlines.complete_within) {
                    (Some(started), Some(within)) => Some(started + within),
                    (Some(_), None) => None,
                    (None, _) => Some(deadlines.first_octet),
                };
                drop(state);
                [by_item, deadlines.complete_by].into_iter().flatten().min()
            };
            if let Some(deadline) = deadline
                && now >= deadline
            {
                return Err(LegReadError::Deadline);
            }
            let until = [wake_at, deadline].into_iter().flatten().min();
            match until {
                Some(until) => self.park(until).await,
                None => self.wake.notified().await,
            }
        }
    }

    async fn write_all(&self, bytes: &[u8], deadline: Instant) -> Result<(), LegWriteError> {
        let mut rest = bytes;
        loop {
            let admitted = self.write_some(rest)?;
            rest = &rest[admitted..];
            if rest.is_empty() {
                self.anchor();
                return Ok(());
            }
            if self.clock.now() >= deadline {
                return Err(LegWriteError::Deadline);
            }
            self.sleep_until(deadline).await;
        }
    }

    /// The deadline check of a relay or discard loop: `Err(true)` when the
    /// absolute deadline elapsed, `Err(false)` when the no-progress window did.
    fn relay_expiry(
        &self,
        deadlines: &RelayDeadlines,
        last_progress: Instant,
    ) -> Result<Instant, bool> {
        let now = self.clock.now();
        if now >= deadlines.absolute {
            return Err(true);
        }
        let idle_until = last_progress + deadlines.no_progress;
        if now >= idle_until {
            return Err(false);
        }
        Ok(deadlines.absolute.min(idle_until))
    }
}

/// The receive half of a Sim leg.
struct SimReadHalf {
    leg: Arc<LegShared>,
}

impl Drop for SimReadHalf {
    fn drop(&mut self) {
        self.leg.half_dropped(true);
    }
}

/// The transmit half of a Sim leg.
struct SimWriteHalf {
    leg: Arc<LegShared>,
}

impl Drop for SimWriteHalf {
    fn drop(&mut self) {
        self.leg.half_dropped(false);
    }
}

#[derive(Default)]
struct RelayState {
    lie: Option<SubstrateLie>,
    probe_calls: u32,
    public_queue: VecDeque<PublicLegScript>,
    upstream_queue: VecDeque<LegScript>,
    public_arm_failure: Option<KtlsArmError>,
    upstream_arm_failure: Option<KtlsArmError>,
    legs: Vec<Arc<LegShared>>,
}

/// The Sim `GatewayRelay`: in-memory leg state on the injected clock.
pub struct SimGatewayRelay {
    clock: Arc<dyn Clock>,
    state: Mutex<RelayState>,
}

impl SimGatewayRelay {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, state: Mutex::new(RelayState::default()) }
    }

    /// Show `lie` to the next probes; `None` is the honest substrate.
    pub fn present_substrate(&self, lie: Option<SubstrateLie>) {
        self.state.lock().lie = lie;
    }

    /// Queue one public connection for the next `accept_public` (FIFO).
    pub fn script_public_connection(&self, script: PublicLegScript) {
        self.state.lock().public_queue.push_back(script);
    }

    /// Queue the script the next minted upstream leg plays (FIFO).
    pub fn script_upstream_leg(&self, script: LegScript) {
        self.state.lock().upstream_queue.push_back(script);
    }

    /// The next `accept_public` arms nothing and fails `Arm(kind)`.
    pub fn fail_next_public_arm(&self, kind: KtlsArmError) {
        self.state.lock().public_arm_failure = Some(kind);
    }

    /// The next upstream arm (after the exact-peer equality) fails with `kind`.
    pub fn fail_next_upstream_arm(&self, kind: KtlsArmError) {
        self.state.lock().upstream_arm_failure = Some(kind);
    }

    /// The same-backend arm of an authenticated upstream socket: what
    /// `SimGatewayClientMtls::authenticate` calls once the selected-peer
    /// equality held. Plays the next upstream script, or fails with the queued
    /// arm refusal. The socket is kept inside the leg.
    pub fn arm_upstream_leg(
        &self,
        connected: OwnedFd,
        _expected_peer: &SpiffeId,
    ) -> Result<KtlsLeg, KtlsArmError> {
        let mut state = self.state.lock();
        if let Some(kind) = state.upstream_arm_failure.take() {
            return Err(kind);
        }
        // An unscripted origin ends the connection at once.
        let script =
            state.upstream_queue.pop_front().unwrap_or_else(|| LegScript::new().close_notify());
        let leg = LegShared::new(
            SimLegId(state.legs.len() as u64),
            LegRole::Upstream,
            Arc::clone(&self.clock),
            script,
            None,
            Some(connected),
        );
        state.legs.push(Arc::clone(&leg));
        drop(state);
        Ok(Self::armed(&leg))
    }

    fn armed(leg: &Arc<LegShared>) -> KtlsLeg {
        KtlsLeg::from_armed(
            Box::new(SimReadHalf { leg: Arc::clone(leg) }),
            Box::new(SimWriteHalf { leg: Arc::clone(leg) }),
        )
    }

    /// Number of `probe` calls received.
    pub fn probe_calls(&self) -> u32 {
        self.state.lock().probe_calls
    }

    /// Transcript of every leg handed out, in creation order.
    pub fn leg_transcripts(&self) -> Vec<LegTranscript> {
        let legs: Vec<Arc<LegShared>> = self.state.lock().legs.clone();
        legs.iter().map(|leg| leg.transcript()).collect()
    }

    /// Legs not yet closed (residue check).
    pub fn live_leg_count(&self) -> usize {
        let legs: Vec<Arc<LegShared>> = self.state.lock().legs.clone();
        legs.iter().filter(|leg| !leg.state.lock().closed).count()
    }

    /// Pipes created and not yet released (residue check).
    pub fn live_pipe_count(&self) -> usize {
        let legs: Vec<Arc<LegShared>> = self.state.lock().legs.clone();
        legs.iter().filter(|leg| leg.state.lock().pipe == PipeLife::Live).count()
    }
}

fn leg_of_read(half: &mut KtlsReadHalf) -> Arc<LegShared> {
    Arc::clone(&half.backend_mut::<SimReadHalf>().leg)
}

fn leg_of_write(half: &mut KtlsWriteHalf) -> Arc<LegShared> {
    Arc::clone(&half.backend_mut::<SimWriteHalf>().leg)
}

/// The relay error of a source that ended while `owed` octets were still due.
fn source_ended(end: &LegEnd, owed: u64) -> RelayError {
    match end {
        LegEnd::CleanEof => RelayError::SourceEndedMidBody { remaining: owed },
        other => RelayError::Read(other.read_error(false)),
    }
}

#[async_trait]
impl GatewayRelay for SimGatewayRelay {
    async fn probe(&self, _deadline: Instant) -> Result<(), RelayProbeError> {
        let mut state = self.state.lock();
        state.probe_calls += 1;
        match state.lie.clone() {
            None => Ok(()),
            Some(SubstrateLie::UlpAbsent) => {
                Err(RelayProbeError::Arm(KtlsArmError::UlpInstall { errno: libc::ENOENT }))
            }
            Some(SubstrateLie::CipherRefused) => {
                Err(RelayProbeError::Arm(KtlsArmError::CipherNotSupported))
            }
            Some(SubstrateLie::SpliceInexact { moved, expected }) => {
                Err(RelayProbeError::SpliceExactness { moved, expected })
            }
            Some(SubstrateLie::PipeClamped { got }) => {
                Err(RelayProbeError::Pipe(RelayPipeError::CapacityBelowFloor { got }))
            }
            Some(SubstrateLie::ProbeTimeout) => Err(RelayProbeError::Timeout),
            Some(SubstrateLie::ProbeIo { kind }) => Err(RelayProbeError::Io { kind }),
        }
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        _tls: Arc<rustls::ServerConfig>,
        _deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        let mut state = self.state.lock();
        if let Some(kind) = state.public_arm_failure.take() {
            return Err(PublicHandshakeError::Arm(kind));
        }
        let Some(script) = state.public_queue.pop_front() else {
            return Err(PublicHandshakeError::Tls { alert: None });
        };
        let leg = LegShared::new(
            SimLegId(state.legs.len() as u64),
            LegRole::Public,
            Arc::clone(&self.clock),
            script.inbound,
            Some(self.clock.now()),
            Some(accepted),
        );
        state.legs.push(Arc::clone(&leg));
        drop(state);
        Ok(AcceptedPublicLeg { leg: Self::armed(&leg), sni: script.sni })
    }

    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        let leg = leg_of_read(from);
        leg.record(RelayCall::ReadThrough);
        if terminator == HeadTerminator::HeaderBlockEnd {
            leg.state.lock().release_pipe();
        }
        leg.read_through(terminator, limit, deadlines).await
    }

    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        let leg = leg_of_write(to);
        leg.record(RelayCall::WriteAll);
        leg.write_all(bytes, deadline).await
    }

    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        let source = leg_of_read(from);
        let sink = leg_of_write(to);
        source.record(RelayCall::RelayExact);
        source.create_pipe().map_err(RelayError::Pipe)?;
        let mut owed = len.get();
        let mut last_progress = self.clock.now();
        loop {
            let max = usize::try_from(owed).unwrap_or(usize::MAX).min(PIPE_STEP);
            let wake_at = match source.take(max) {
                Taken::Octets(octets) => {
                    let admitted = sink.write_some(&octets).map_err(RelayError::Write)?;
                    if admitted < octets.len() {
                        source.unread(&octets[admitted..]);
                    }
                    if admitted > 0 {
                        owed -= admitted as u64;
                        last_progress = self.clock.now();
                        if owed == 0 {
                            return Ok(());
                        }
                        if admitted == PIPE_STEP {
                            tokio::task::yield_now().await;
                        }
                        continue;
                    }
                    None
                }
                Taken::Ended(end) => return Err(source_ended(&end, owed)),
                Taken::Blocked(at) => at,
            };
            let limit = source.relay_expiry(&deadlines, last_progress).map_err(|absolute| {
                if absolute { RelayError::Deadline } else { RelayError::NoProgress }
            })?;
            source.park(wake_at.map_or(limit, |at| at.min(limit))).await;
        }
    }

    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        let source = leg_of_read(from);
        let sink = leg_of_write(to);
        source.record(RelayCall::RelayToEnd);
        source.create_pipe().map_err(RelayError::Pipe)?;
        let mut moved: u64 = 0;
        let mut last_progress = self.clock.now();
        loop {
            let room = max.get() - moved;
            let step = usize::try_from(room.max(1)).unwrap_or(usize::MAX).min(PIPE_STEP);
            let wake_at = match source.take(step) {
                Taken::Octets(octets) => {
                    if room == 0 {
                        return Err(RelayError::LimitExceeded { max: max.get() });
                    }
                    let admitted = sink.write_some(&octets).map_err(RelayError::Write)?;
                    if admitted < octets.len() {
                        source.unread(&octets[admitted..]);
                    }
                    if admitted > 0 {
                        moved += admitted as u64;
                        last_progress = self.clock.now();
                        if admitted == PIPE_STEP {
                            tokio::task::yield_now().await;
                        }
                        continue;
                    }
                    None
                }
                Taken::Ended(LegEnd::CleanEof) => return Ok(moved),
                Taken::Ended(end) => return Err(RelayError::Read(end.read_error(false))),
                Taken::Blocked(at) => at,
            };
            let limit = source.relay_expiry(&deadlines, last_progress).map_err(|absolute| {
                if absolute { RelayError::Deadline } else { RelayError::NoProgress }
            })?;
            source.park(wake_at.map_or(limit, |at| at.min(limit))).await;
        }
    }

    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        _deadline: Instant,
    ) -> Result<(), LegWriteError> {
        let leg = leg_of_write(to);
        leg.record(RelayCall::CloseWrite);
        leg.state.lock().close_notify_sent = true;
        Ok(())
    }

    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), LegReadError> {
        let leg = leg_of_read(from);
        leg.record(RelayCall::DiscardRemaining);
        let mut discarded: u64 = 0;
        let mut last_progress = self.clock.now();
        loop {
            let wake_at = match leg.take(usize::MAX) {
                Taken::Octets(octets) => {
                    discarded += octets.len() as u64;
                    last_progress = self.clock.now();
                    if discarded >= max.get() {
                        return Ok(());
                    }
                    continue;
                }
                Taken::Ended(LegEnd::CleanEof) => return Ok(()),
                Taken::Ended(end) => return Err(end.read_error(false)),
                Taken::Blocked(at) => at,
            };
            let limit =
                leg.relay_expiry(&deadlines, last_progress).map_err(|_| LegReadError::Deadline)?;
            leg.park(wake_at.map_or(limit, |at| at.min(limit))).await;
        }
    }
}

// ---------------------------------------------------------------------------
// SimGatewayConnectDataplane (the dataplane-side share)
// ---------------------------------------------------------------------------

/// A `GatewayConnectDataplane` operation a fault can be armed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConnectOperation {
    Probe,
    OpenUpstream,
    TakeReceipt,
    Cleanup,
    CleanupAllGatewayIntents,
    SelectedBackendIdentity,
    LiveIntentCount,
    LiveReceipts,
}

/// The gate hold shared by the Sim connect dataplane and the Sim identity
/// control.
///
/// Route activation does not depend on backend health or authentication
/// (R-65): the gate never reaches `GatewayClientMtls::authenticate`. A held gate
/// parks at an awaited operation the gate owns (DG-9), and the park point
/// depends on the gate's place in the boot:
///
/// - the owner latches the once-per-boot proof
///   (`GatewayConnectDataplane::probe`) on its first success, so only the
///   first candidate of a boot reaches it before the latch is set, and a hold
///   armed before that candidate parks it (and any candidate that follows it
///   before the latch) inside [`SimGatewayConnectDataplane::probe`]. That wait
///   cannot be cancelled: a gate the owner supersedes stays parked until the
///   hold is released and then completes a real late result, which the gate
///   discards;
/// - every gate that reaches the identity wait
///   ([`SimGatewayIdentityLifecycleControl::ensure_current`]) while the hold is
///   armed parks there. The owner cancels that wait when it supersedes the gate,
///   so only the live staged gate stays parked.
///
/// Neither port operation carries a frontend, so a hold armed for a frontend
/// parks every gate that reaches either point until that frontend is released;
/// the rigs that use it compose one Service frontend. Cloning shares the state.
#[derive(Clone)]
pub struct SimConnectHold {
    shared: Arc<HoldShared>,
}

struct HoldShared {
    state: Mutex<HoldState>,
    /// Bumped on every `release`; parked gates re-check their frontends.
    changed: watch::Sender<u64>,
}

#[derive(Default)]
struct HoldState {
    /// Frontends whose gates park.
    held: Vec<ServiceFrontend>,
    parked: usize,
}

impl Default for SimConnectHold {
    fn default() -> Self {
        let (changed, _) = watch::channel(0);
        Self { shared: Arc::new(HoldShared { state: Mutex::new(HoldState::default()), changed }) }
    }
}

/// Counts a parked gate for as long as its waiter exists, so a waiter that is
/// dropped stops counting.
struct ParkedEntry(Arc<HoldShared>);

impl Drop for ParkedEntry {
    fn drop(&mut self) {
        let mut state = self.0.state.lock();
        state.parked = state.parked.saturating_sub(1);
    }
}

impl SimConnectHold {
    /// Park the next gate(s) at the held operation they reach while `frontend`
    /// is held.
    pub fn hold(&self, frontend: ServiceFrontend) {
        let mut state = self.shared.state.lock();
        if !state.held.contains(&frontend) {
            state.held.push(frontend);
        }
    }

    /// Release every gate held for `frontend`. A later gate never parks on a
    /// hold that was released before it arrived.
    pub fn release(&self, frontend: ServiceFrontend) {
        self.shared.state.lock().held.retain(|held| *held != frontend);
        self.shared.changed.send_modify(|epoch| *epoch += 1);
    }

    /// Number of gates currently parked.
    pub fn held_entries(&self) -> usize {
        self.shared.state.lock().parked
    }

    /// Park the caller while any frontend that was held when it arrived is
    /// still held; returns at once when no frontend is held.
    async fn park_if_held(&self) {
        let (frontends, mut changed) = {
            let mut state = self.shared.state.lock();
            if state.held.is_empty() {
                return;
            }
            let frontends = state.held.clone();
            state.parked += 1;
            let changed = self.shared.changed.subscribe();
            drop(state);
            (frontends, changed)
        };
        let _entry = ParkedEntry(Arc::clone(&self.shared));
        while self.shared.state.lock().held.iter().any(|held| frontends.contains(held)) {
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}

/// The Sim `GatewayConnectDataplane`.
///
/// The same observable contract as the real hook, over the `SimDataplane` state
/// it shares (D-PASS7-4). No outer entry for the key or an empty slot ->
/// `NoBackend`; otherwise `Selected` with the lowest `BackendId` of the key's
/// current generation. Cookies are deterministic nonzero values. The Sim does
/// not evaluate `Backend.healthy`.
///
/// The Sim sees no `connect(2)`, so a registered intent's receipt is decided
/// from the shared Service state when it is read (`take_receipt`,
/// `live_receipts`) and is live from registration until taken or cleaned up.
pub struct SimGatewayConnectDataplane {
    dataplane: Arc<SimDataplane>,
    hold: SimConnectHold,
    state: Mutex<ConnectState>,
}

#[derive(Default)]
struct ConnectState {
    last_cookie: u64,
    /// Registered intents by cookie.
    intents: BTreeMap<SocketCookie, ServiceKey>,
    /// Cookies whose receipt is live (registered, not yet taken).
    receipts: BTreeSet<SocketCookie>,
    /// Armed one-shot faults, FIFO per operation.
    faults: BTreeMap<ConnectOperation, VecDeque<GatewayConnectDataplaneError>>,
}

impl ConnectState {
    fn fault(&mut self, operation: ConnectOperation) -> Result<(), GatewayConnectDataplaneError> {
        self.faults.get_mut(&operation).and_then(VecDeque::pop_front).map_or(Ok(()), Err)
    }
}

impl SimGatewayConnectDataplane {
    /// Share `dataplane`'s Service state.
    pub fn new(dataplane: Arc<SimDataplane>) -> Self {
        Self {
            dataplane,
            hold: SimConnectHold::default(),
            state: Mutex::new(ConnectState::default()),
        }
    }

    /// The next call of `operation` returns `error` (FIFO; typed-fault
    /// configuration at the existing port operations).
    pub fn fail_next(&self, operation: ConnectOperation, error: GatewayConnectDataplaneError) {
        self.state.lock().faults.entry(operation).or_default().push_back(error);
    }

    /// The gate hold this adapter parks `probe` on; arm the same hold on a
    /// [`SimGatewayIdentityLifecycleControl`] with `with_hold` to park the
    /// identity wait as well.
    pub fn connect_hold(&self) -> SimConnectHold {
        self.hold.clone()
    }

    /// The receipt the Sim records for `cookie` registered for `key`.
    fn receipt_for(&self, cookie: SocketCookie, key: ServiceKey) -> GatewaySelectionReceipt {
        let selected = key.vip().try_as_ipv4().and_then(|vip| {
            self.dataplane.selected_backend_for(vip, key.port().get(), key.protocol())
        });
        selected.map_or(
            GatewaySelectionReceipt::NoBackend { socket_cookie: cookie, service_key: key },
            |backend_id| GatewaySelectionReceipt::Selected {
                socket_cookie: cookie,
                service_key: key,
                backend_id,
            },
        )
    }
}

impl GatewayConnectCleanup for SimGatewayConnectDataplane {
    fn cleanup(&self, intent: &GatewayConnectIntent) -> Result<(), GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::Cleanup)?;
        state.intents.remove(&intent.socket_cookie);
        state.receipts.remove(&intent.socket_cookie);
        drop(state);
        Ok(())
    }

    fn cleanup_all_gateway_intents(
        &self,
    ) -> Result<GatewayConnectCleanupSweep, GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::CleanupAllGatewayIntents)?;
        let removed_intents = u32::try_from(std::mem::take(&mut state.intents).len())
            .map_err(|_| GatewayConnectDataplaneError::Cleanup)?;
        let removed_receipts = u32::try_from(std::mem::take(&mut state.receipts).len())
            .map_err(|_| GatewayConnectDataplaneError::Cleanup)?;
        drop(state);
        Ok(GatewayConnectCleanupSweep { removed_intents, removed_receipts })
    }

    fn live_intent_count(&self) -> Result<u32, GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::LiveIntentCount)?;
        u32::try_from(state.intents.len()).map_err(|_| GatewayConnectDataplaneError::Kernel)
    }

    #[allow(
        clippy::significant_drop_tightening,
        reason = "the state guard is held across the snapshot so receipts and intents agree"
    )]
    fn live_receipts(
        &self,
    ) -> Result<BTreeMap<SocketCookie, GatewaySelectionReceipt>, GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::LiveReceipts)?;
        Ok(state
            .receipts
            .iter()
            .filter_map(|cookie| {
                let key = *state.intents.get(cookie)?;
                Some((*cookie, self.receipt_for(*cookie, key)))
            })
            .collect())
    }
}

#[async_trait]
impl GatewayConnectDataplane for SimGatewayConnectDataplane {
    async fn probe(
        &self,
        deadline: Instant,
        clock: &dyn Clock,
    ) -> Result<(), GatewayConnectDataplaneError> {
        {
            let mut state = self.state.lock();
            state.fault(ConnectOperation::Probe)?;
            // The probe first sweeps residue, then runs a sentinel connect that
            // is denied and consumed: it leaves nothing registered.
            state.intents.clear();
            state.receipts.clear();
        }
        if clock.now() >= deadline {
            return Err(GatewayConnectDataplaneError::ProbeTimeout);
        }
        // The probe's completion is the held operation: the sweep above has
        // run, and the result is not delivered until the hold is released.
        self.hold.park_if_held().await;
        Ok(())
    }

    fn open_upstream(
        &self,
        key: ServiceKey,
    ) -> Result<PreparedUpstream, GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::OpenUpstream)?;
        let socket = tokio::net::TcpSocket::new_v4()
            .map_err(|error| GatewayConnectDataplaneError::SocketCreate(error.kind()))?;
        state.last_cookie = state.last_cookie.saturating_add(1);
        let socket_cookie = SocketCookie::new(state.last_cookie).map_err(|_| {
            GatewayConnectDataplaneError::CookieRead(std::io::ErrorKind::InvalidData)
        })?;
        state.intents.insert(socket_cookie, key);
        state.receipts.insert(socket_cookie);
        drop(state);
        Ok(PreparedUpstream::new(socket, GatewayConnectIntent { socket_cookie, service_key: key }))
    }

    fn take_receipt(
        &self,
        intent: &GatewayConnectIntent,
    ) -> Result<GatewaySelectionReceipt, GatewayConnectDataplaneError> {
        let mut state = self.state.lock();
        state.fault(ConnectOperation::TakeReceipt)?;
        let cookie = intent.socket_cookie;
        if !state.receipts.contains(&cookie) {
            return Err(GatewayConnectDataplaneError::ReceiptMissing);
        }
        if state.intents.get(&cookie) != Some(&intent.service_key) {
            return Err(GatewayConnectDataplaneError::ReceiptMismatch);
        }
        state.receipts.remove(&cookie);
        drop(state);
        Ok(self.receipt_for(cookie, intent.service_key))
    }

    fn selected_backend_identity(
        &self,
        backend_id: BackendId,
    ) -> Result<SpiffeId, GatewayConnectDataplaneError> {
        self.state.lock().fault(ConnectOperation::SelectedBackendIdentity)?;
        self.dataplane
            .applied_identity(backend_id)
            .ok_or(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id })
    }
}

// ---------------------------------------------------------------------------
// SimGatewayClientMtls
// ---------------------------------------------------------------------------

/// The Sim exact-peer client: a scripted peer, one seal, the Sim relay backend.
///
/// `authenticate` returns a `GatewayUpstream` only after the scripted peer
/// equals `expected_peer`; otherwise the exact mismatch error.
pub struct SimGatewayClientMtls {
    seal: GatewayUpstreamSeal,
    relay: Arc<SimGatewayRelay>,
    clock: Arc<dyn Clock>,
    peer: Mutex<Option<SpiffeId>>,
}

impl SimGatewayClientMtls {
    pub fn new(
        seal: GatewayUpstreamSeal,
        relay: Arc<SimGatewayRelay>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { seal, relay, clock, peer: Mutex::new(None) }
    }

    /// The identity the scripted peer presents.
    pub fn set_peer(&self, peer: SpiffeId) {
        *self.peer.lock() = Some(peer);
    }
}

#[async_trait]
impl GatewayClientMtls for SimGatewayClientMtls {
    async fn authenticate(
        &self,
        connected: OwnedFd,
        expected_peer: SpiffeId,
        _deadline: Instant,
    ) -> Result<GatewayUpstream, GatewayClientMtlsError> {
        let presented = self.peer.lock().clone();
        let Some(actual) = presented else {
            return Err(GatewayClientMtlsError::PeerCertificateMissing);
        };
        if actual != expected_peer {
            return Err(GatewayClientMtlsError::PeerSpiffeMismatch {
                expected: expected_peer,
                actual,
            });
        }
        let leg = self
            .relay
            .arm_upstream_leg(connected, &expected_peer)
            .map_err(GatewayClientMtlsError::Arm)?;
        Ok(self.seal.seal_authenticated(leg))
    }
}

// ---------------------------------------------------------------------------
// Gateway identity doubles
// ---------------------------------------------------------------------------

struct IdentityState {
    desired: GatewayIdentityDesired,
    held: Option<SvidMaterial>,
    issue: Option<GatewayIdentityIssueRecord>,
}

/// The Sim gateway identity target: epoch and current facts only, under one
/// mutex-held deterministic state. Its current projection never contains key
/// material.
pub struct SimGatewayIdentityActionTarget {
    state: Mutex<IdentityState>,
    facts: watch::Sender<Option<GatewayIdentityFacts>>,
}

impl SimGatewayIdentityActionTarget {
    pub fn new(desired: GatewayIdentityDesired) -> Self {
        let (facts, _initial) = watch::channel(None);
        Self { state: Mutex::new(IdentityState { desired, held: None, issue: None }), facts }
    }

    pub fn current(&self) -> Option<GatewayIdentityFacts> {
        self.facts.borrow().clone()
    }

    pub fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        self.state.lock().issue
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.facts.subscribe()
    }

    pub fn begin_disable(&self) -> GatewayIdentityEpoch {
        let mut state = self.state.lock();
        let next = state
            .desired
            .epoch
            .get()
            .checked_add(1)
            .unwrap_or_else(|| unreachable!("a u64 epoch cannot be exhausted by disables"));
        let epoch = GatewayIdentityEpoch::new(next.get())
            .unwrap_or_else(|_| unreachable!("a checked add of a nonzero epoch is nonzero"));
        state.desired = GatewayIdentityDesired { epoch, spiffe_id: None };
        state.held = None;
        self.facts.send_replace(None);
        drop(state);
        epoch
    }
}

impl GatewayIdentityActionTarget for SimGatewayIdentityActionTarget {
    fn hold_audited(
        &self,
        epoch: GatewayIdentityEpoch,
        material: SvidMaterial,
    ) -> GatewayIdentityMutationOutcome {
        let mut state = self.state.lock();
        if epoch != state.desired.epoch {
            return GatewayIdentityMutationOutcome::StaleEpoch;
        }
        let facts = GatewayIdentityFacts {
            epoch,
            spiffe_id: material.spiffe_id().clone(),
            serial: material.serial().clone(),
            not_after: material.not_after(),
        };
        state.held = Some(material);
        self.facts.send_replace(Some(facts));
        drop(state);
        GatewayIdentityMutationOutcome::Applied
    }

    fn record_issue_failure(&self, failure: SvidIssueFailure) {
        let mut state = self.state.lock();
        let count = state.issue.map_or(NonZeroU64::MIN, |record| record.count.saturating_add(1));
        state.issue = Some(GatewayIdentityIssueRecord { count, last: failure });
        self.facts.send_modify(|_| {});
        drop(state);
    }
}

impl GatewayIdentityLifecycleTarget for SimGatewayIdentityActionTarget {
    fn current(&self) -> Option<GatewayIdentityFacts> {
        Self::current(self)
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        Self::issue_record(self)
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        Self::subscribe(self)
    }

    fn begin_disable(&self) -> GatewayIdentityEpoch {
        Self::begin_disable(self)
    }
}

impl GatewayIdentityView for SimGatewayIdentityActionTarget {
    fn desired(&self) -> GatewayIdentityDesired {
        self.state.lock().desired.clone()
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        Self::current(self)
    }
}

/// The Sim identity lifecycle control.
///
/// Submits its retained evaluation to the same real `EvaluationBroker` and uses
/// the injected Sim clock plus the target watch for the same ensure
/// absolute-deadline race as production (the Sim clock is read for the
/// `ensure_current` deadline race only; `disable_after_drain` takes no
/// deadline); it never sets Current directly. Disable is the synchronous
/// `begin_disable` clear.
pub struct SimGatewayIdentityLifecycleControl {
    target: Arc<SimGatewayIdentityActionTarget>,
    broker: Arc<parking_lot::Mutex<EvaluationBroker>>,
    evaluation: Evaluation,
    clock: Arc<dyn Clock>,
    hold: Option<SimConnectHold>,
}

impl SimGatewayIdentityLifecycleControl {
    pub fn new(
        target: Arc<SimGatewayIdentityActionTarget>,
        broker: Arc<parking_lot::Mutex<EvaluationBroker>>,
        evaluation: Evaluation,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { target, broker, evaluation, clock, hold: None }
    }

    /// Park `ensure_current` while the gate hold is armed (test support, DG-9,
    /// R-65): the staged gate's identity wait is the awaited operation a held
    /// gate parks at.
    #[must_use]
    pub fn with_hold(mut self, hold: SimConnectHold) -> Self {
        self.hold = Some(hold);
        self
    }

    /// The held facts when they are still time-usable at the injected clock.
    fn usable_current(&self) -> Option<GatewayIdentityFacts> {
        let now = UnixInstant::from_clock(self.clock.as_ref());
        self.target.current().filter(|facts| facts.not_after > now)
    }
}

#[async_trait]
impl GatewayIdentityLifecycleControl for SimGatewayIdentityLifecycleControl {
    async fn ensure_current(
        &self,
        deadline: Instant,
    ) -> Result<GatewayIdentityFacts, GatewayIdentityLifecycleError> {
        if let Some(hold) = &self.hold {
            hold.park_if_held().await;
        }
        let mut receiver = self.target.subscribe();
        self.broker.lock().submit(
            self.evaluation.clone(),
            self.clock.now(),
            EvaluationEligibility::Immediate,
        );
        if let Some(facts) = self.usable_current() {
            return Ok(facts);
        }
        let remaining = deadline.saturating_duration_since(self.clock.now());
        let timeout = self.clock.sleep(remaining);
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                changed = receiver.changed() => {
                    changed.map_err(|_| GatewayIdentityLifecycleError::Closed)?;
                    if let Some(facts) = self.usable_current() {
                        return Ok(facts);
                    }
                }
                () = &mut timeout => return Err(GatewayIdentityLifecycleError::Timeout),
            }
        }
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.target.current()
    }

    fn issue_record(&self) -> Option<GatewayIdentityIssueRecord> {
        self.target.issue_record()
    }

    fn subscribe(&self) -> watch::Receiver<Option<GatewayIdentityFacts>> {
        self.target.subscribe()
    }

    fn disable_after_drain(&self) {
        let _disabled_epoch = self.target.begin_disable();
    }
}

// ---------------------------------------------------------------------------
// Client-mTLS factory and the assembled Sim adapters (R-20)
// ---------------------------------------------------------------------------

/// Builds [`SimGatewayClientMtls`] over the one [`SimGatewayRelay`] it shares
/// with [`SimGatewayAdapters::relay`]. It ignores `Deps.identity`.
pub struct SimGatewayClientMtlsFactory {
    relay: Arc<SimGatewayRelay>,
    clock: Arc<dyn Clock>,
}

impl SimGatewayClientMtlsFactory {
    pub fn new(relay: Arc<SimGatewayRelay>, clock: Arc<dyn Clock>) -> Self {
        Self { relay, clock }
    }
}

impl GatewayClientMtlsFactory for SimGatewayClientMtlsFactory {
    fn client_mtls(&self, deps: GatewayClientMtlsDeps) -> Arc<dyn GatewayClientMtls> {
        Arc::new(SimGatewayClientMtls::new(
            deps.seal,
            Arc::clone(&self.relay),
            Arc::clone(&self.clock),
        ))
    }
}

/// The Sim counterpart of the Host relay and client pair: one backend, so no
/// mixing.
///
/// Tests assemble `GatewayAdapters { certified_key_codec, client_mtls, relay }`
/// from its two accessors; the Sim crate does not name the control-plane type.
pub struct SimGatewayAdapters {
    relay: Arc<SimGatewayRelay>,
    client_mtls: Arc<SimGatewayClientMtlsFactory>,
}

impl SimGatewayAdapters {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let relay = Arc::new(SimGatewayRelay::new(Arc::clone(&clock)));
        let client_mtls = Arc::new(SimGatewayClientMtlsFactory::new(Arc::clone(&relay), clock));
        Self { relay, client_mtls }
    }

    pub fn relay(&self) -> Arc<dyn GatewayRelay> {
        Arc::clone(&self.relay) as Arc<dyn GatewayRelay>
    }

    pub fn client_mtls(&self) -> Arc<dyn GatewayClientMtlsFactory> {
        Arc::clone(&self.client_mtls) as Arc<dyn GatewayClientMtlsFactory>
    }

    /// The concrete Sim relay, for the test-support surface (scripting,
    /// transcripts).
    pub fn sim_relay(&self) -> &Arc<SimGatewayRelay> {
        &self.relay
    }
}
