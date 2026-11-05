//! The exchange state machine of one public connection (ADR-0222, ADR-0238,
//! ADR-0240, ADR-0242, ADR-0247, ADR-0254).
//!
//! Source-local properties implement S-PIG-84; the Quint conformance driver
//! (`quint_exchange`, QC-4) replays `specs/quint/public-ingress-gateway/
//! relay_exchange.qnt` against this owner.
//!
//! One machine per public connection; it is reused across keep-alive
//! exchanges, never shared. The two concurrent halves of an exchange are not
//! phases: each half is read through its own accessor.

/// Connection-level phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExchangePhase {
    Idle,
    Relay,
    Closed,
}

/// Progress of the response half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseState {
    Pending,
    Body,
    Complete,
}

/// Octets still owed by one half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Remaining {
    None,
    Exactly(u64),
    ChunkedOpen,
    UntilClose,
}

/// What `finish` decided for the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    Reuse,
    Close,
}

/// A leg of the exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Public,
    Upstream,
}

/// The body delimitation the pure framing decision derived from a head
/// (ADR-0245). It is the machine's only framing input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyFraming {
    /// No body: a request without Content-Length or Transfer-Encoding; a
    /// response to HEAD, or 204, 205, 304.
    NoBody,
    /// `Content-Length: n`. `Length(0)` is equivalent to `NoBody`.
    Length(u64),
    /// `Transfer-Encoding: chunked`.
    Chunked,
    /// Close-delimited response body (R-12). Illegal as a request framing.
    UntilClose,
}

/// A call that contradicts the machine's state. Peer framing faults are
/// rejected by the pure head parser before the machine is called, so every
/// violation is an internal-consistency failure of the exchange driver's own
/// call sequence, never a client or origin fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramingViolation {
    /// The call is not legal in the current phase.
    WrongPhase { phase: ExchangePhase },
    /// `response_head` when `response_state()` is not `Pending`.
    ResponseHeadRepeated,
    /// `UntilClose` passed to `begin_request`.
    UntilCloseRequest,
    /// `*_octets_moved(n)` with `n > 0` above the side's `Remaining::Exactly(k)`,
    /// or with `n > 0` when that side's remaining is `None`.
    OctetsBeyondFraming { side: Side, n: u64 },
    /// `request_chunked_end` / `response_chunked_end` when that side's
    /// remaining is not `ChunkedOpen`.
    ChunkedEndNotOpen { side: Side },
    /// `finish()` while `response_state()` is not `Complete`.
    ResponseNotComplete,
}

/// The pure exchange state machine (the only owner of the Exchange table).
pub(crate) struct ExchangeMachine {
    phase: ExchangePhase,
    request_remaining: Remaining,
    response_state: ResponseState,
    response_remaining: Remaining,
    head_request: bool,
    draining: bool,
    public_half_closed: bool,
}

impl ExchangeMachine {
    /// Fresh machine: `Idle`, `ResponseState::Pending`, both remainings
    /// `Remaining::None`, not draining, public peer not half-closed.
    pub(crate) const fn new() -> Self {
        Self {
            phase: ExchangePhase::Idle,
            request_remaining: Remaining::None,
            response_state: ResponseState::Pending,
            response_remaining: Remaining::None,
            head_request: false,
            draining: false,
            public_half_closed: false,
        }
    }

    /// Idle -> Relay (the upstream leg is connected by the exchange driver
    /// before this call).
    pub(crate) fn begin_request(
        &mut self,
        method_is_head: bool,
        framing: BodyFraming,
    ) -> Result<(), FramingViolation> {
        if self.phase != ExchangePhase::Idle {
            return Err(FramingViolation::WrongPhase { phase: self.phase });
        }
        if framing == BodyFraming::UntilClose {
            return Err(FramingViolation::UntilCloseRequest);
        }
        self.phase = ExchangePhase::Relay;
        self.head_request = method_is_head;
        self.request_remaining = Remaining::from_framing(framing);
        self.response_state = ResponseState::Pending;
        self.response_remaining = Remaining::None;
        Ok(())
    }

    pub(crate) fn request_octets_moved(&mut self, n: u64) -> Result<(), FramingViolation> {
        self.require_relay()?;
        self.request_remaining = self.request_remaining.after_moving(n, Side::Public)?;
        Ok(())
    }

    pub(crate) fn response_head(&mut self, framing: BodyFraming) -> Result<(), FramingViolation> {
        self.require_relay()?;
        if self.response_state != ResponseState::Pending {
            return Err(FramingViolation::ResponseHeadRepeated);
        }
        self.response_remaining =
            if self.head_request { Remaining::None } else { Remaining::from_framing(framing) };
        self.response_state = self.state_for_remaining();
        Ok(())
    }

    pub(crate) fn response_octets_moved(&mut self, n: u64) -> Result<(), FramingViolation> {
        self.require_relay()?;
        self.response_remaining = self.response_remaining.after_moving(n, Side::Upstream)?;
        if self.response_state == ResponseState::Body {
            self.response_state = self.state_for_remaining();
        }
        Ok(())
    }

    /// Ends a `ChunkedOpen` request body (last chunk and trailers consumed).
    pub(crate) fn request_chunked_end(&mut self) -> Result<(), FramingViolation> {
        self.require_relay()?;
        if self.request_remaining != Remaining::ChunkedOpen {
            return Err(FramingViolation::ChunkedEndNotOpen { side: Side::Public });
        }
        self.request_remaining = Remaining::None;
        Ok(())
    }

    /// Ends a `ChunkedOpen` response body.
    pub(crate) fn response_chunked_end(&mut self) -> Result<(), FramingViolation> {
        self.require_relay()?;
        if self.response_remaining != Remaining::ChunkedOpen {
            return Err(FramingViolation::ChunkedEndNotOpen { side: Side::Upstream });
        }
        self.response_remaining = Remaining::None;
        self.response_state = ResponseState::Complete;
        Ok(())
    }

    pub(crate) fn peer_closed(&mut self, side: Side, clean: bool) {
        match (self.phase, side) {
            (ExchangePhase::Closed, _) => {}
            (_, Side::Upstream) => self.upstream_closed(clean),
            (_, Side::Public) => self.public_closed(clean),
        }
    }

    pub(crate) fn drain_begin(&mut self) {
        if self.phase == ExchangePhase::Closed || self.draining {
            return;
        }
        self.draining = true;
        if self.phase == ExchangePhase::Idle {
            self.close();
        }
    }

    pub(crate) fn force_abort(&mut self) {
        self.close();
    }

    /// Fails `FramingViolation` unless `response_state() == Complete`.
    /// Relay -> Idle and `Reuse` when `reusable()`, else Relay -> Closed and
    /// `Close`. A reused `Idle` keeps the completed message's state
    /// (`response_state() == Complete`, both remainings `None`) until the next
    /// `begin_request`.
    pub(crate) fn finish(&mut self) -> Result<Disposition, FramingViolation> {
        self.require_relay()?;
        if self.response_state != ResponseState::Complete {
            return Err(FramingViolation::ResponseNotComplete);
        }
        if !self.reusable() {
            self.close();
            return Ok(Disposition::Close);
        }
        self.phase = ExchangePhase::Idle;
        Ok(Disposition::Reuse)
    }

    pub(crate) const fn phase(&self) -> ExchangePhase {
        self.phase
    }

    pub(crate) const fn request_remaining(&self) -> Remaining {
        self.request_remaining
    }

    pub(crate) const fn response_state(&self) -> ResponseState {
        self.response_state
    }

    pub(crate) const fn response_remaining(&self) -> Remaining {
        self.response_remaining
    }

    pub(crate) const fn public_half_closed(&self) -> bool {
        self.public_half_closed
    }

    pub(crate) const fn is_draining(&self) -> bool {
        self.draining
    }

    /// Derived: `finish()` returns `Reuse` exactly when this holds.
    pub(crate) fn reusable(&self) -> bool {
        self.request_remaining == Remaining::None
            && self.response_remaining == Remaining::None
            && self.response_state == ResponseState::Complete
            && !self.draining
            && !self.public_half_closed
    }

    fn require_relay(&self) -> Result<(), FramingViolation> {
        match self.phase {
            ExchangePhase::Relay => Ok(()),
            phase => Err(FramingViolation::WrongPhase { phase }),
        }
    }

    fn state_for_remaining(&self) -> ResponseState {
        match self.response_remaining {
            Remaining::None => ResponseState::Complete,
            _ => ResponseState::Body,
        }
    }

    /// Closed is absorbing and owes nothing; `draining` and the public
    /// half-close stay observable.
    fn close(&mut self) {
        self.phase = ExchangePhase::Closed;
        self.request_remaining = Remaining::None;
        self.response_state = ResponseState::Pending;
        self.response_remaining = Remaining::None;
    }

    fn upstream_closed(&mut self, clean: bool) {
        let ends_close_delimited_body = clean
            && self.response_state == ResponseState::Body
            && self.response_remaining == Remaining::UntilClose;
        if ends_close_delimited_body {
            self.response_state = ResponseState::Complete;
            return;
        }
        if self.response_state != ResponseState::Complete {
            self.close();
        }
    }

    fn public_closed(&mut self, clean: bool) {
        let request_unfinished = self.request_remaining != Remaining::None;
        let aborts = !clean || self.phase == ExchangePhase::Idle || request_unfinished;
        if aborts {
            self.close();
        } else {
            self.public_half_closed = true;
        }
    }
}

impl Remaining {
    /// The framing-to-remaining mapping (the machine's one rule).
    const fn from_framing(framing: BodyFraming) -> Self {
        match framing {
            BodyFraming::NoBody | BodyFraming::Length(0) => Self::None,
            BodyFraming::Length(n) => Self::Exactly(n),
            BodyFraming::Chunked => Self::ChunkedOpen,
            BodyFraming::UntilClose => Self::UntilClose,
        }
    }

    /// What is owed after `n` octets moved on `side`.
    const fn after_moving(self, n: u64, side: Side) -> Result<Self, FramingViolation> {
        match self {
            Self::Exactly(k) if n <= k => {
                Ok(if n == k { Self::None } else { Self::Exactly(k - n) })
            }
            Self::ChunkedOpen | Self::UntilClose => Ok(self),
            _ if n == 0 => Ok(self),
            _ => Err(FramingViolation::OctetsBeyondFraming { side, n }),
        }
    }
}

#[cfg(test)]
mod laws {
    #![allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required metadata")]

    use proptest::prelude::*;

    use super::*;

    // -----------------------------------------------------------------
    // Generators and the reference vocabulary
    // -----------------------------------------------------------------

    /// How a request body is delimited.
    #[derive(Debug, Clone, Copy)]
    enum RequestShape {
        NoBody,
        Length(u64),
        Chunked,
    }

    impl RequestShape {
        fn framing(self) -> BodyFraming {
            match self {
                Self::NoBody => BodyFraming::NoBody,
                Self::Length(n) => BodyFraming::Length(n),
                Self::Chunked => BodyFraming::Chunked,
            }
        }

        fn remaining(self) -> Remaining {
            match self {
                Self::NoBody | Self::Length(0) => Remaining::None,
                Self::Length(n) => Remaining::Exactly(n),
                Self::Chunked => Remaining::ChunkedOpen,
            }
        }
    }

    /// How a response body is delimited.
    #[derive(Debug, Clone, Copy)]
    enum ResponseShape {
        NoBody,
        Length(u64),
        Chunked,
        UntilClose,
    }

    impl ResponseShape {
        fn framing(self) -> BodyFraming {
            match self {
                Self::NoBody => BodyFraming::NoBody,
                Self::Length(n) => BodyFraming::Length(n),
                Self::Chunked => BodyFraming::Chunked,
                Self::UntilClose => BodyFraming::UntilClose,
            }
        }

        /// The remaining the machine must derive; a `HEAD` response describes
        /// a body that is not sent.
        fn remaining(self, head_request: bool) -> Remaining {
            if head_request {
                return Remaining::None;
            }
            match self {
                Self::NoBody | Self::Length(0) => Remaining::None,
                Self::Length(n) => Remaining::Exactly(n),
                Self::Chunked => Remaining::ChunkedOpen,
                Self::UntilClose => Remaining::UntilClose,
            }
        }
    }

    fn arb_request() -> impl Strategy<Value = RequestShape> {
        prop_oneof![
            Just(RequestShape::NoBody),
            (0u64..=4).prop_map(RequestShape::Length),
            Just(RequestShape::Chunked),
        ]
    }

    fn arb_response() -> impl Strategy<Value = ResponseShape> {
        prop_oneof![
            Just(ResponseShape::NoBody),
            (0u64..=4).prop_map(ResponseShape::Length),
            Just(ResponseShape::Chunked),
            Just(ResponseShape::UntilClose),
        ]
    }

    /// A response whose delimitation is by length (never close-delimited).
    fn arb_length_delimited_response() -> impl Strategy<Value = ResponseShape> {
        prop_oneof![
            Just(ResponseShape::NoBody),
            (0u64..=4).prop_map(ResponseShape::Length),
            Just(ResponseShape::Chunked),
        ]
    }

    /// Relays the whole request body of `shape`.
    fn complete_request(machine: &mut ExchangeMachine, shape: RequestShape) {
        match shape {
            RequestShape::NoBody | RequestShape::Length(0) => {}
            RequestShape::Length(n) => {
                let first = n / 2;
                machine.request_octets_moved(first).expect("request octets within the framing");
                machine
                    .request_octets_moved(n - first)
                    .expect("the rest of the request octets within the framing");
            }
            RequestShape::Chunked => {
                machine.request_octets_moved(3).expect("chunk data");
                machine.request_chunked_end().expect("the open chunked request body ends");
            }
        }
    }

    /// Relays the whole response body of `shape`, ending a close-delimited
    /// body with the origin's clean close.
    fn complete_response(machine: &mut ExchangeMachine, shape: ResponseShape, head_request: bool) {
        if head_request {
            return;
        }
        match shape {
            ResponseShape::NoBody | ResponseShape::Length(0) => {}
            ResponseShape::Length(n) => {
                let first = n / 2;
                machine.response_octets_moved(first).expect("response octets within the framing");
                machine
                    .response_octets_moved(n - first)
                    .expect("the rest of the response octets within the framing");
            }
            ResponseShape::Chunked => {
                machine.response_octets_moved(3).expect("chunk data");
                machine.response_chunked_end().expect("the open chunked response body ends");
            }
            ResponseShape::UntilClose => {
                machine.response_octets_moved(3).expect("close-delimited octets");
                machine.peer_closed(Side::Upstream, true);
            }
        }
    }

    fn close_delimited(shape: ResponseShape, head_request: bool) -> bool {
        !head_request && matches!(shape, ResponseShape::UntilClose)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-84 (reuse law): a finished exchange is reused exactly when
        /// the response completed, the request body was fully relayed, the
        /// response is length-delimited, the connection is not draining and the
        /// public peer has not half-closed; `reusable()` and the `Disposition`
        /// `finish()` returns agree, in every combination of framings, event
        /// order and `HEAD`.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn reuse_happens_only_at_a_clean_boundary(
            head_request in any::<bool>(),
            request in arb_request(),
            response in arb_response(),
            response_first in any::<bool>(),
            drain in any::<bool>(),
            public_close in any::<bool>(),
        ) {
            let mut machine = ExchangeMachine::new();
            machine.begin_request(head_request, request.framing()).expect("begin the exchange");
            prop_assert_eq!(machine.phase(), ExchangePhase::Relay);
            prop_assert_eq!(machine.request_remaining(), request.remaining());
            prop_assert_eq!(machine.response_state(), ResponseState::Pending);
            if drain {
                machine.drain_begin();
                prop_assert!(machine.is_draining());
                prop_assert_eq!(
                    machine.phase(),
                    ExchangePhase::Relay,
                    "an in-flight exchange keeps running while the connection drains"
                );
            }

            if response_first {
                machine.response_head(response.framing()).expect("response head");
                complete_response(&mut machine, response, head_request);
                complete_request(&mut machine, request);
            } else {
                complete_request(&mut machine, request);
                machine.response_head(response.framing()).expect("response head");
                complete_response(&mut machine, response, head_request);
            }
            if public_close {
                machine.peer_closed(Side::Public, true);
                prop_assert!(machine.public_half_closed());
                prop_assert_eq!(
                    machine.phase(),
                    ExchangePhase::Relay,
                    "the response is still delivered after the request was fully read"
                );
            }

            prop_assert_eq!(machine.response_state(), ResponseState::Complete);
            prop_assert_eq!(machine.request_remaining(), Remaining::None);
            let expected_reuse =
                !drain && !public_close && !close_delimited(response, head_request);
            prop_assert_eq!(machine.reusable(), expected_reuse);

            let disposition = machine.finish().expect("a completed exchange finishes");
            if expected_reuse {
                prop_assert_eq!(disposition, Disposition::Reuse);
                prop_assert_eq!(machine.phase(), ExchangePhase::Idle);
                prop_assert_eq!(machine.request_remaining(), Remaining::None);
                prop_assert_eq!(machine.response_remaining(), Remaining::None);
                prop_assert!(!machine.is_draining());
                prop_assert!(!machine.public_half_closed());
                machine
                    .begin_request(false, BodyFraming::NoBody)
                    .expect("a reused connection takes the next request");
                prop_assert_eq!(machine.phase(), ExchangePhase::Relay);
                prop_assert_eq!(machine.response_state(), ResponseState::Pending);
            } else {
                prop_assert_eq!(disposition, Disposition::Close);
                prop_assert_eq!(machine.phase(), ExchangePhase::Closed);
                prop_assert_eq!(machine.request_remaining(), Remaining::None);
                prop_assert_eq!(machine.response_remaining(), Remaining::None);
                prop_assert_eq!(machine.response_state(), ResponseState::Pending);
                prop_assert_eq!(machine.is_draining(), drain);
                prop_assert_eq!(machine.public_half_closed(), public_close);
            }
        }

        /// S-PIG-84 (early response): a response that completes while the
        /// request body is unfinished (a declared length not fully relayed, or
        /// a `ChunkedOpen` body) is never reused: `finish()` returns `Close`.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn an_early_response_never_reuses_the_connection(
            declared in 1u64..=4,
            already_moved in 0u64..4,
            chunked_open in any::<bool>(),
            chunk_octets in 0u64..=3,
            response in arb_length_delimited_response(),
        ) {
            let mut machine = ExchangeMachine::new();
            if chunked_open {
                machine.begin_request(false, BodyFraming::Chunked).expect("begin chunked");
                machine.request_octets_moved(chunk_octets).expect("chunk data");
                prop_assert_eq!(machine.request_remaining(), Remaining::ChunkedOpen);
            } else {
                let moved = already_moved % declared;
                machine.begin_request(false, BodyFraming::Length(declared)).expect("begin");
                machine.request_octets_moved(moved).expect("request octets");
                prop_assert_eq!(machine.request_remaining(), Remaining::Exactly(declared - moved));
            }
            machine.response_head(response.framing()).expect("response head");
            complete_response(&mut machine, response, false);

            prop_assert_eq!(machine.response_state(), ResponseState::Complete);
            prop_assert!(!machine.reusable());
            prop_assert_eq!(machine.finish().expect("finish"), Disposition::Close);
            prop_assert_eq!(machine.phase(), ExchangePhase::Closed);
            prop_assert_eq!(machine.request_remaining(), Remaining::None);
            prop_assert_eq!(machine.response_remaining(), Remaining::None);
        }

        /// S-PIG-84 (aborts): an origin close_notify before the response head,
        /// or during a length-delimited or chunked body, closes the connection;
        /// during a close-delimited body it is the clean end of the response
        /// and the connection is still not reused.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn an_upstream_close_aborts_unless_the_body_is_close_delimited(
            stage in 0u8..4,
            declared in 2u64..=4,
            octets in 0u64..=3,
        ) {
            let mut machine = ExchangeMachine::new();
            machine.begin_request(false, BodyFraming::NoBody).expect("begin");
            match stage {
                0 => {}
                1 => {
                    machine.response_head(BodyFraming::Length(declared)).expect("head");
                    machine.response_octets_moved(declared - 1).expect("part of the body");
                }
                2 => {
                    machine.response_head(BodyFraming::Chunked).expect("head");
                    machine.response_octets_moved(octets).expect("chunk data");
                }
                _ => {
                    machine.response_head(BodyFraming::UntilClose).expect("head");
                    machine.response_octets_moved(octets).expect("close-delimited octets");
                }
            }
            machine.peer_closed(Side::Upstream, true);
            if stage == 3 {
                prop_assert_eq!(machine.response_state(), ResponseState::Complete);
                prop_assert_eq!(machine.phase(), ExchangePhase::Relay);
                prop_assert!(!machine.reusable());
                prop_assert_eq!(machine.finish().expect("finish"), Disposition::Close);
            } else {
                prop_assert_eq!(machine.phase(), ExchangePhase::Closed);
                prop_assert_eq!(machine.response_state(), ResponseState::Pending);
            }
            prop_assert_eq!(machine.phase(), ExchangePhase::Closed);
        }

        /// S-PIG-84 (public close): a public close_notify while idle or while a
        /// request body is unfinished closes the connection; after the request
        /// was fully read it only marks the peer half-closed and the response
        /// is still delivered.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn a_public_close_aborts_only_before_the_request_is_fully_read(
            request in arb_request(),
            fully_read in any::<bool>(),
        ) {
            let mut idle = ExchangeMachine::new();
            idle.peer_closed(Side::Public, true);
            prop_assert_eq!(idle.phase(), ExchangePhase::Closed);
            prop_assert!(!idle.public_half_closed());

            let mut machine = ExchangeMachine::new();
            machine.begin_request(false, request.framing()).expect("begin");
            let unfinished = !matches!(request.remaining(), Remaining::None);
            if fully_read {
                complete_request(&mut machine, request);
            }
            machine.peer_closed(Side::Public, true);
            if unfinished && !fully_read {
                prop_assert_eq!(machine.phase(), ExchangePhase::Closed);
            } else {
                prop_assert_eq!(machine.phase(), ExchangePhase::Relay);
                prop_assert!(machine.public_half_closed());
                prop_assert!(!machine.reusable());
            }
        }

        /// S-PIG-84 (framing to remaining): `NoBody` and `Length(0)` give
        /// `None`; `Length(n>0)` gives `Exactly(n)`; `Chunked` gives
        /// `ChunkedOpen`; `UntilClose` gives `UntilClose`; a `HEAD` response is
        /// always `None`. `response_head` completes the response exactly when
        /// the remaining is `None`. Moves reduce `Exactly` and leave
        /// `ChunkedOpen` and `UntilClose` unchanged.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn framing_maps_to_remaining_and_moves_reduce_it(
            head_request in any::<bool>(),
            request in arb_request(),
            response in arb_response(),
            moved in 0u64..6,
        ) {
            let mut machine = ExchangeMachine::new();
            machine.begin_request(head_request, request.framing()).expect("begin");
            prop_assert_eq!(machine.request_remaining(), request.remaining());
            machine.response_head(response.framing()).expect("response head");
            let remaining = response.remaining(head_request);
            prop_assert_eq!(machine.response_remaining(), remaining);
            let expected_state = if remaining == Remaining::None {
                ResponseState::Complete
            } else {
                ResponseState::Body
            };
            prop_assert_eq!(machine.response_state(), expected_state);

            match remaining {
                Remaining::Exactly(k) if moved <= k => {
                    machine.response_octets_moved(moved).expect("within the declared remainder");
                    let left = k - moved;
                    if left == 0 {
                        prop_assert_eq!(machine.response_remaining(), Remaining::None);
                        prop_assert_eq!(machine.response_state(), ResponseState::Complete);
                    } else {
                        prop_assert_eq!(machine.response_remaining(), Remaining::Exactly(left));
                        prop_assert_eq!(machine.response_state(), ResponseState::Body);
                    }
                }
                Remaining::ChunkedOpen | Remaining::UntilClose => {
                    machine.response_octets_moved(moved).expect("open-ended framing accepts any count");
                    prop_assert_eq!(machine.response_remaining(), remaining);
                    prop_assert_eq!(machine.response_state(), ResponseState::Body);
                }
                _ => {}
            }
        }

        /// S-PIG-84 (octets beyond the framing): `*_octets_moved(n)` with
        /// `n > 0` above the declared remainder, or with nothing owed, is
        /// `OctetsBeyondFraming` carrying `n`; `ChunkedOpen` and `UntilClose`
        /// accept any count, zero is always accepted, and the two halves report
        /// distinct sides.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn octets_beyond_the_declared_remainder_are_refused(
            declared in 1u64..=4,
            excess in 1u64..=1000,
            open_count in any::<u64>(),
        ) {
            let mut request_half = ExchangeMachine::new();
            request_half.begin_request(false, BodyFraming::Length(declared)).expect("begin");
            request_half.request_octets_moved(0).expect("zero octets are always legal");
            let request_error = request_half
                .request_octets_moved(declared + excess)
                .expect_err("octets above the declared request remainder");
            let FramingViolation::OctetsBeyondFraming { side: request_side, n } = request_error
            else {
                return Err(TestCaseError::fail(format!("unexpected violation {request_error:?}")));
            };
            prop_assert_eq!(n, declared + excess);

            let mut response_half = ExchangeMachine::new();
            response_half.begin_request(false, BodyFraming::NoBody).expect("begin");
            response_half.response_head(BodyFraming::Length(declared)).expect("response head");
            let response_error = response_half
                .response_octets_moved(declared + excess)
                .expect_err("octets above the declared response remainder");
            let FramingViolation::OctetsBeyondFraming { side: response_side, n } = response_error
            else {
                return Err(TestCaseError::fail(format!("unexpected violation {response_error:?}")));
            };
            prop_assert_eq!(n, declared + excess);
            prop_assert_ne!(request_side, response_side, "the two halves are told apart");

            let mut open = ExchangeMachine::new();
            open.begin_request(false, BodyFraming::Chunked).expect("begin chunked");
            open.request_octets_moved(open_count).expect("ChunkedOpen accepts any count");
            open.response_head(BodyFraming::UntilClose).expect("response head");
            open.response_octets_moved(open_count).expect("UntilClose accepts any count");

            let mut none_owed = ExchangeMachine::new();
            none_owed.begin_request(false, BodyFraming::NoBody).expect("begin");
            let request_refused = matches!(
                none_owed.request_octets_moved(excess),
                Err(FramingViolation::OctetsBeyondFraming { n, .. }) if n == excess
            );
            prop_assert!(request_refused, "a request with nothing owed refuses octets");
            let mut before_head = ExchangeMachine::new();
            before_head.begin_request(false, BodyFraming::NoBody).expect("begin");
            let response_refused = matches!(
                before_head.response_octets_moved(excess),
                Err(FramingViolation::OctetsBeyondFraming { n, .. }) if n == excess
            );
            prop_assert!(response_refused, "a response before its head owes nothing");
        }
    }

    /// S-PIG-84 (fresh machine and phases): a new machine is `Idle` with
    /// nothing owed; every call other than `begin_request` is refused in
    /// `Idle`, `begin_request` is refused outside `Idle`, and `UntilClose` is
    /// not a request framing.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_fresh_machine_is_idle_and_refuses_calls_outside_their_phase() {
        let mut machine = ExchangeMachine::new();
        assert_eq!(machine.phase(), ExchangePhase::Idle);
        assert_eq!(machine.response_state(), ResponseState::Pending);
        assert_eq!(machine.request_remaining(), Remaining::None);
        assert_eq!(machine.response_remaining(), Remaining::None);
        assert!(!machine.is_draining());
        assert!(!machine.public_half_closed());

        let idle = FramingViolation::WrongPhase { phase: ExchangePhase::Idle };
        assert_eq!(machine.request_octets_moved(1), Err(idle));
        assert_eq!(machine.response_head(BodyFraming::NoBody), Err(idle));
        assert_eq!(machine.response_octets_moved(1), Err(idle));
        assert_eq!(machine.request_chunked_end(), Err(idle));
        assert_eq!(machine.response_chunked_end(), Err(idle));
        assert_eq!(machine.finish(), Err(idle));

        assert_eq!(
            machine.begin_request(false, BodyFraming::UntilClose),
            Err(FramingViolation::UntilCloseRequest),
            "a close-delimited framing is illegal for a request",
        );

        machine.begin_request(false, BodyFraming::NoBody).expect("begin");
        assert_eq!(
            machine.begin_request(false, BodyFraming::NoBody),
            Err(FramingViolation::WrongPhase { phase: ExchangePhase::Relay }),
            "one request is in flight per connection",
        );
    }

    /// S-PIG-84 (violations): the remaining violation variants, one per call
    /// that contradicts the state: a second response head, a chunk end on a
    /// body that is not `ChunkedOpen` (each half), and `finish` before the
    /// response completed.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_call_that_contradicts_the_state_is_a_distinct_violation() {
        let mut machine = ExchangeMachine::new();
        machine.begin_request(false, BodyFraming::Length(2)).expect("begin");

        assert_eq!(machine.finish(), Err(FramingViolation::ResponseNotComplete));
        assert!(matches!(
            machine.request_chunked_end(),
            Err(FramingViolation::ChunkedEndNotOpen { .. })
        ));
        assert!(matches!(
            machine.response_chunked_end(),
            Err(FramingViolation::ChunkedEndNotOpen { .. })
        ));
        let Err(FramingViolation::ChunkedEndNotOpen { side: request_side }) =
            machine.request_chunked_end()
        else {
            panic!("a Length request body is not ChunkedOpen");
        };
        let Err(FramingViolation::ChunkedEndNotOpen { side: response_side }) =
            machine.response_chunked_end()
        else {
            panic!("a response before its head is not ChunkedOpen");
        };
        assert_ne!(request_side, response_side, "the two halves are told apart");

        machine.response_head(BodyFraming::Length(3)).expect("response head");
        assert_eq!(
            machine.response_head(BodyFraming::NoBody),
            Err(FramingViolation::ResponseHeadRepeated),
        );
        assert_eq!(
            machine.finish(),
            Err(FramingViolation::ResponseNotComplete),
            "a response with octets owed is not complete",
        );
    }

    /// S-PIG-84 (absorbing Closed): after the connection closed, every
    /// fallible call is refused with `WrongPhase { Closed }` and every event is
    /// ignored: the phase stays `Closed` and nothing is owed.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_event_after_closed_is_refused_or_ignored() {
        let mut machine = ExchangeMachine::new();
        machine.begin_request(false, BodyFraming::Length(2)).expect("begin");
        machine.drain_begin();
        machine.force_abort();
        assert_eq!(machine.phase(), ExchangePhase::Closed);
        assert_eq!(machine.request_remaining(), Remaining::None);
        assert_eq!(machine.response_remaining(), Remaining::None);
        assert_eq!(machine.response_state(), ResponseState::Pending);

        let closed = FramingViolation::WrongPhase { phase: ExchangePhase::Closed };
        assert_eq!(machine.begin_request(false, BodyFraming::NoBody), Err(closed));
        assert_eq!(machine.request_octets_moved(1), Err(closed));
        assert_eq!(machine.response_head(BodyFraming::NoBody), Err(closed));
        assert_eq!(machine.response_octets_moved(1), Err(closed));
        assert_eq!(machine.request_chunked_end(), Err(closed));
        assert_eq!(machine.response_chunked_end(), Err(closed));
        assert_eq!(machine.finish(), Err(closed));

        machine.peer_closed(Side::Public, true);
        machine.peer_closed(Side::Upstream, true);
        machine.drain_begin();
        machine.force_abort();
        assert_eq!(machine.phase(), ExchangePhase::Closed);
        assert_eq!(machine.request_remaining(), Remaining::None);
        assert_eq!(machine.response_remaining(), Remaining::None);
        assert_eq!(machine.response_state(), ResponseState::Pending);
    }

    /// S-PIG-84 (drain): draining an idle connection closes it at once and
    /// marks it draining; an in-flight exchange is not interrupted until
    /// `force_abort`, which closes it.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn drain_closes_an_idle_connection_and_force_abort_ends_an_inflight_one() {
        let mut idle = ExchangeMachine::new();
        idle.drain_begin();
        assert_eq!(idle.phase(), ExchangePhase::Closed);
        assert!(idle.is_draining());

        let mut inflight = ExchangeMachine::new();
        inflight.begin_request(false, BodyFraming::NoBody).expect("begin");
        inflight.response_head(BodyFraming::Length(2)).expect("response head");
        inflight.drain_begin();
        assert_eq!(inflight.phase(), ExchangePhase::Relay);
        assert!(inflight.is_draining());
        inflight.response_octets_moved(1).expect("the exchange keeps running");
        inflight.force_abort();
        assert_eq!(inflight.phase(), ExchangePhase::Closed);
    }
}
