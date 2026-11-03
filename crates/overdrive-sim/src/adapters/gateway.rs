//! Sim adapters of the public-ingress gateway driven ports.
//!
//! SCAFFOLD: true. This file currently holds the kernel-relay share (G1):
//! [`SimGatewayRelay`] — the `GatewayRelay` port backed by in-memory record
//! queues on the injected clock — and its test-support surface. The connect
//! dataplane, exact-peer client mTLS and identity doubles land beside it in
//! the same file. Port bodies fail with SCAFFOLD text until DELIVER; the
//! scripting data types are plain data and are real.
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
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::unused_async,
    clippy::unused_self,
    clippy::needless_pass_by_ref_mut,
    dead_code,
    unused_variables,
    reason = "public-ingress DISTILL scaffold: the Sim port bodies are DELIVER's"
)]

use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::id::SpiffeId;
use overdrive_core::public_ingress::PublicHostname;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayRelay, HeadTerminator, KeyUpdateError, KtlsArmError, KtlsLeg,
    KtlsReadHalf, KtlsWriteHalf, LegReadError, LegWriteError, PublicHandshakeError, ReadDeadlines,
    RelayDeadlines, RelayError, RelayPipeError, RelayProbeError, ThroughBytes,
};

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
    KeyUpdate { requested: bool, answer_fails: Option<KeyUpdateError> },
    NewSessionTicket,
    /// The peer's close_notify.
    CloseNotify,
    /// A fatal alert with this description.
    Alert { description: u8 },
    /// A record of an unexpected content type.
    UnexpectedRecord { content_type: u8 },
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

/// The Sim `GatewayRelay`: in-memory leg state on the injected clock.
pub struct SimGatewayRelay {
    clock: Arc<dyn Clock>,
}

impl SimGatewayRelay {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        todo!("SCAFFOLD: SimGatewayRelay::new")
    }

    /// Show `lie` to the next probes; `None` is the honest substrate.
    pub fn present_substrate(&self, lie: Option<SubstrateLie>) {
        todo!("SCAFFOLD: SimGatewayRelay::present_substrate")
    }

    /// Queue one public connection for the next `accept_public` (FIFO).
    pub fn script_public_connection(&self, script: PublicLegScript) {
        todo!("SCAFFOLD: SimGatewayRelay::script_public_connection")
    }

    /// Queue the script the next minted upstream leg plays (FIFO).
    pub fn script_upstream_leg(&self, script: LegScript) {
        todo!("SCAFFOLD: SimGatewayRelay::script_upstream_leg")
    }

    /// The next `accept_public` arms nothing and fails `Arm(kind)`.
    pub fn fail_next_public_arm(&self, kind: KtlsArmError) {
        todo!("SCAFFOLD: SimGatewayRelay::fail_next_public_arm")
    }

    /// The next upstream arm (after the exact-peer equality) fails with `kind`.
    pub fn fail_next_upstream_arm(&self, kind: KtlsArmError) {
        todo!("SCAFFOLD: SimGatewayRelay::fail_next_upstream_arm")
    }

    /// The same-backend arm of an authenticated upstream socket: what
    /// `SimGatewayClientMtls::authenticate` calls once the selected-peer
    /// equality held. Plays the next upstream script, or fails with the queued
    /// arm refusal. The socket is kept inside the leg.
    pub fn arm_upstream_leg(
        &self,
        connected: OwnedFd,
        expected_peer: &SpiffeId,
    ) -> Result<KtlsLeg, KtlsArmError> {
        todo!("SCAFFOLD: SimGatewayRelay::arm_upstream_leg")
    }

    /// Number of `probe` calls received.
    pub fn probe_calls(&self) -> u32 {
        todo!("SCAFFOLD: SimGatewayRelay::probe_calls")
    }

    /// Transcript of every leg handed out, in creation order.
    pub fn leg_transcripts(&self) -> Vec<LegTranscript> {
        todo!("SCAFFOLD: SimGatewayRelay::leg_transcripts")
    }

    /// Legs not yet closed (residue check).
    pub fn live_leg_count(&self) -> usize {
        todo!("SCAFFOLD: SimGatewayRelay::live_leg_count")
    }

    /// Pipes created and not yet released (residue check).
    pub fn live_pipe_count(&self) -> usize {
        todo!("SCAFFOLD: SimGatewayRelay::live_pipe_count")
    }
}

#[async_trait]
impl GatewayRelay for SimGatewayRelay {
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError> {
        todo!("SCAFFOLD: SimGatewayRelay::probe")
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        todo!("SCAFFOLD: SimGatewayRelay::accept_public")
    }

    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        todo!("SCAFFOLD: SimGatewayRelay::read_through")
    }

    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        todo!("SCAFFOLD: SimGatewayRelay::write_all")
    }

    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        todo!("SCAFFOLD: SimGatewayRelay::relay_exact")
    }

    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        todo!("SCAFFOLD: SimGatewayRelay::relay_to_end")
    }

    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        todo!("SCAFFOLD: SimGatewayRelay::close_write")
    }

    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, LegReadError> {
        todo!("SCAFFOLD: SimGatewayRelay::discard_remaining")
    }
}
