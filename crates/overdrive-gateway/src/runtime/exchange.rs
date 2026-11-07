//! The exchange driver of one public connection: two concurrent halves per
//! request over the `GatewayRelay` port, driven by `ExchangeMachine`.
//!
//! The DESIGN pins the driver's behaviour (Kernel relay data path, "Message-
//! framing contract", "Exchange state machine") but no public or cross-module
//! item: the driver, its deadline composition and its status mapping are
//! crate-private structure. Its behaviour is specified through the public
//! listener by `tests/integration/public_ingress_gateway_relay.rs` and replayed
//! against the pure owner by `http1::quint_exchange`.
//!
//! [`serve_connection`] owns one accepted public connection from the TLS
//! handshake to its close. Per request it reads the head, admits the request,
//! validates the head, matches the Route, connects one upstream leg and writes
//! the upstream head. It then runs the request half (public read to upstream
//! write) and the response half (upstream read to public write) concurrently.
//! Every deadline is composed by `http1::deadlines` on the injected clock; no
//! function here reads a wall clock or sleeps.

use std::net::Ipv4Addr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use http::header::{CONNECTION, CONTENT_LENGTH, HeaderValue, TRANSFER_ENCODING};
use http::{HeaderMap, Method, StatusCode};
use overdrive_core::public_ingress::PublicHostname;
use overdrive_core::traits::clock::Clock;
use tokio::sync::watch;

use crate::application::GatewayConnectionLease;
use crate::connector::{GatewayUpstreamConnectError, GatewayUpstreamConnector};
use crate::ports::{
    GatewayRelay, HeadTerminator, KtlsReadHalf, KtlsWriteHalf, LegReadError, ReadDeadlines,
    RelayDeadlines, RelayError, ThroughBytes,
};
use crate::runtime::GatewayLimits;
use crate::runtime::http1::deadlines::{ReadPhase, compose_read_deadlines, request_deadline};
use crate::runtime::http1::exchange::{
    BodyFraming, Disposition, ExchangeMachine, FramingViolation, Remaining, Side,
};
use crate::runtime::http1::head::{
    ParsedResponseHead, check_chunk_terminator, is_trailer_end, parse_chunk_size_line,
    parse_request_head, parse_response_head,
};
use crate::runtime::proxy_headers::{rewrite_request_headers, rewrite_response_headers};

/// What every connection needs and none owns: the relay, the upstream
/// connector, the injected clock and the fixed limits.
pub struct ConnectionServices {
    pub relay: Arc<dyn GatewayRelay>,
    pub connector: Arc<GatewayUpstreamConnector>,
    pub clock: Arc<dyn Clock>,
    pub limits: Arc<GatewayLimits>,
}

/// The gateway's drain as a connection sees it: `Some(deadline)` once the
/// drain began, carrying the one drain deadline the connections owner
/// enforces by aborting what is still running.
#[derive(Clone)]
pub struct DrainSignal(watch::Receiver<Option<Instant>>);

impl DrainSignal {
    pub const fn new(receiver: watch::Receiver<Option<Instant>>) -> Self {
        Self(receiver)
    }

    fn deadline(&self) -> Option<Instant> {
        *self.0.borrow()
    }

    /// Resolves with the drain deadline when the drain begins; never resolves
    /// if the owner is gone without draining.
    async fn begun(&self) -> Instant {
        let mut receiver = self.0.clone();
        loop {
            let current = *receiver.borrow_and_update();
            if let Some(deadline) = current {
                return deadline;
            }
            if receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

/// Serve one accepted public connection until it closes.
///
/// An idle keep-alive connection closes at once when the drain begins, an
/// in-flight exchange runs on (the owner force-aborts it at the one drain
/// deadline), and no later request is parsed.
pub async fn serve_connection(
    services: Arc<ConnectionServices>,
    accepted: OwnedFd,
    peer: Ipv4Addr,
    lease: GatewayConnectionLease,
    drain: DrainSignal,
) {
    let tls = lease.snapshot().certified_key().server_config();
    let handshake_deadline = services.clock.now() + services.limits.tls_handshake_deadline();
    let public = match services.relay.accept_public(accepted, tls, handshake_deadline).await {
        Ok(public) => public,
        Err(error) => {
            tracing::debug!(
                name: "gateway.public.handshake_refused",
                error = %error,
                "public handshake refused; the socket is closed",
            );
            return;
        }
    };
    let handshake_at = services.clock.now();
    let (read, write) = public.leg.into_halves();
    let connection = Connection {
        services,
        peer,
        lease,
        drain,
        sni: public.sni,
        read,
        write,
        machine: Mutex::new(ExchangeMachine::new()),
    };
    connection.serve(handshake_at).await;
}

// ---------------------------------------------------------------------------
// Connection loop
// ---------------------------------------------------------------------------

struct Connection {
    services: Arc<ConnectionServices>,
    peer: Ipv4Addr,
    lease: GatewayConnectionLease,
    drain: DrainSignal,
    sni: PublicHostname,
    read: KtlsReadHalf,
    write: KtlsWriteHalf,
    machine: Mutex<ExchangeMachine>,
}

/// How a connection ends.
enum End {
    /// Nothing more to say: close_notify, then close.
    Close,
    /// A final response was delivered while request octets may remain unread:
    /// half-close, then discard what the peer still sends until `until`.
    Linger { until: Instant },
    /// A gateway-authored final response, then close. `linger` is the discard
    /// bound, `None` for a response that is never lingered (408).
    Reject { status: StatusCode, linger: Option<Instant> },
    /// Close both legs, no further octet.
    Abort,
}

enum Step {
    Next,
    End(End),
}

/// Why one half of an exchange failed.
enum Fault {
    /// Answer with `status` if no final response octet is out, then close.
    Reject { status: StatusCode, linger: bool },
    /// Close both legs with no further octet.
    Abort,
    /// The driver's own call sequence contradicted the machine.
    Violation(FramingViolation),
}

impl Fault {
    const fn reject(status: StatusCode) -> Self {
        Self::Reject { status, linger: true }
    }

    const fn bad_gateway() -> Self {
        Self::reject(StatusCode::BAD_GATEWAY)
    }
}

enum Outcome {
    /// The response completed: the machine decides reuse.
    Concluded,
    Fault(Fault),
}

impl Connection {
    async fn serve(mut self, handshake_at: Instant) {
        let mut first = true;
        loop {
            match self.one_request(first, handshake_at).await {
                Step::Next => first = false,
                Step::End(end) => {
                    self.end(end).await;
                    return;
                }
            }
        }
    }

    fn lock_machine(&self) -> MutexGuard<'_, ExchangeMachine> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn one_request(&mut self, first: bool, handshake_at: Instant) -> Step {
        let services = Arc::clone(&self.services);
        let head = match self.read_head(first, handshake_at).await {
            Ok(head) => head,
            Err(end) => return Step::End(end),
        };
        let anchor = if first { handshake_at } else { services.clock.now() };
        let deadline = request_deadline(anchor, &services.limits);
        let reject = |status| Step::End(End::Reject { status, linger: Some(deadline) });

        let Ok(request_lease) = self.lease.try_request() else {
            return Step::End(End::Close);
        };
        let parsed = match parse_request_head(head.as_slice(), &self.sni, &services.limits) {
            Ok(parsed) => parsed,
            Err(error) => return reject(error.status()),
        };
        let route = self.lease.snapshot().route();
        let raw_path = parsed.target().split('?').next().unwrap_or_default();
        if route.public_hostname() != parsed.authority()
            || !route.path_match().matches_raw_path(raw_path)
        {
            return reject(StatusCode::NOT_FOUND);
        }
        let connect_deadline =
            deadline.min(services.clock.now() + services.limits.upstream_connect_deadline());
        let frontend = self.lease.snapshot().frontend();
        let upstream = match services.connector.connect(frontend, connect_deadline).await {
            Ok(upstream) => upstream,
            Err(GatewayUpstreamConnectError::NoBackend) => {
                return reject(StatusCode::SERVICE_UNAVAILABLE);
            }
            Err(error) => {
                tracing::debug!(
                    name: "gateway.upstream.connect_failed",
                    error = %error,
                    "upstream connect failed; the request is a 502",
                );
                return reject(StatusCode::BAD_GATEWAY);
            }
        };
        let (mut upstream_read, mut upstream_write) = upstream.into_halves();

        let method = parsed.method().clone();
        let target = parsed.target().to_owned();
        let authority = parsed.authority().clone();
        let framing = parsed.framing();
        let head_request = parsed.method_is_head();
        let headers = rewrite_request_headers(parsed.into_headers(), self.peer, &authority);
        let forwarded_head = upstream_request_head(&method, &target, headers, framing);
        if services.relay.write_all(&mut upstream_write, &forwarded_head, deadline).await.is_err() {
            return reject(StatusCode::BAD_GATEWAY);
        }
        let begun = self.lock_machine().begin_request(head_request, framing);
        if let Err(violation) = begun {
            return violation_step(violation, false, deadline);
        }

        let close_after_response = request_lease.close_after_response();
        let completed_now = matches!(framing, BodyFraming::NoBody | BodyFraming::Length(0));
        let shared = Shared {
            services: &services,
            machine: &self.machine,
            request_deadline: deadline,
            head_request,
            close_after_response,
            final_head_written: AtomicBool::new(false),
            request_complete: watch::channel(completed_now.then(|| services.clock.now())).0,
        };
        let outcome = run_halves(
            &shared,
            &self.drain,
            (&mut self.read, &mut upstream_write),
            (&mut upstream_read, &mut self.write),
            framing,
        )
        .await;
        let final_head_written = shared.final_head_written.load(Ordering::Acquire);
        let step = match outcome {
            Outcome::Concluded => {
                self.conclude(
                    &mut upstream_write,
                    close_after_response,
                    final_head_written,
                    deadline,
                )
                .await
            }
            Outcome::Fault(fault) => {
                self.lock_machine().force_abort();
                fault_step(fault, final_head_written, deadline)
            }
        };
        drop(upstream_read);
        drop(upstream_write);
        drop(request_lease);
        step
    }

    /// The exchange ran both halves to their ends: decide whether the
    /// connection is reused, closed or lingers for the unread request octets.
    async fn conclude(
        &mut self,
        upstream_write: &mut KtlsWriteHalf,
        close_after_response: bool,
        final_head_written: bool,
        deadline: Instant,
    ) -> Step {
        let request_unfinished = self.lock_machine().request_remaining() != Remaining::None;
        let disposition = self.lock_machine().finish();
        match disposition {
            Ok(Disposition::Close) if request_unfinished => {
                Step::End(End::Linger { until: deadline })
            }
            Ok(_) if close_after_response => Step::End(End::Close),
            Ok(Disposition::Reuse) => {
                let _ = self.services.relay.close_write(upstream_write, deadline).await;
                Step::Next
            }
            Ok(Disposition::Close) => Step::End(End::Close),
            Err(violation) => violation_step(violation, final_head_written, deadline),
        }
    }

    // -- request head ---------------------------------------------------------

    /// Read the next request head. A later request waits idle: the drain
    /// closes it at once. The first request is already in flight: the drain
    /// leaves it to its deadlines and to the owner's force-abort at the one
    /// drain deadline.
    async fn read_head(&mut self, first: bool, handshake_at: Instant) -> Result<ThroughBytes, End> {
        let services = Arc::clone(&self.services);
        let limits = &services.limits;
        let waited_from = services.clock.now();
        if !first && self.drain.deadline().is_some() {
            return Err(End::Close);
        }
        let phase = if first { ReadPhase::FirstRequestHead } else { ReadPhase::LaterRequestHead };
        let deadlines = compose_read_deadlines(
            phase,
            waited_from,
            request_deadline(handshake_at, limits),
            limits,
        );
        let read = services.relay.read_through(
            &mut self.read,
            HeadTerminator::HeaderBlockEnd,
            limits.max_request_head_bytes(),
            deadlines,
        );
        let result = if first {
            read.await
        } else {
            tokio::select! {
                biased;
                _ = self.drain.begun() => return Err(End::Close),
                result = read => result,
            }
        };
        result.map_err(|error| self.head_read_end(&error, first, waited_from))
    }

    fn head_read_end(&self, error: &LegReadError, first: bool, waited_from: Instant) -> End {
        let limits = &self.services.limits;
        match error {
            LegReadError::CleanEof => End::Close,
            LegReadError::Deadline => {
                let idle = waited_from + limits.keep_alive_idle_deadline();
                if first || self.services.clock.now() < idle {
                    End::Reject { status: StatusCode::REQUEST_TIMEOUT, linger: None }
                } else {
                    End::Close
                }
            }
            LegReadError::LimitExceeded { .. } => End::Reject {
                status: StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                linger: Some(self.services.clock.now() + limits.whole_request_deadline()),
            },
            LegReadError::Truncated
            | LegReadError::Alert { .. }
            | LegReadError::UnexpectedRecord { .. }
            | LegReadError::KeyUpdate(_)
            | LegReadError::Io { .. } => End::Abort,
        }
    }

    // -- the end of a connection ----------------------------------------------

    async fn end(&mut self, end: End) {
        let services = Arc::clone(&self.services);
        let window = services.clock.now() + services.limits.body_no_progress_deadline();
        match end {
            End::Abort => {}
            End::Close => {
                let _ = services.relay.close_write(&mut self.write, window).await;
            }
            End::Linger { until } => self.linger(until).await,
            End::Reject { status, linger } => {
                let response = authored_response(status);
                if services.relay.write_all(&mut self.write, &response, window).await.is_err() {
                    return;
                }
                match linger {
                    Some(until) => self.linger(until).await,
                    None => {
                        let _ = services.relay.close_write(&mut self.write, window).await;
                    }
                }
            }
        }
    }

    /// Half-close, then discard what the peer still sends so closing the socket
    /// cannot reset the response it has not yet read.
    async fn linger(&mut self, until: Instant) {
        let services = Arc::clone(&self.services);
        let now = services.clock.now();
        let no_progress = services.limits.body_no_progress_deadline();
        let _ = services.relay.close_write(&mut self.write, now + no_progress).await;
        if now >= until {
            return;
        }
        let _ = services
            .relay
            .discard_remaining(
                &mut self.read,
                services.limits.max_request_body_bytes(),
                RelayDeadlines { absolute: until, no_progress },
            )
            .await;
    }
}

/// Every `FramingViolation` is the driver's own error: close both legs, answer
/// 502 when no public octet was written, and say so once.
fn violation_step(violation: FramingViolation, final_head_written: bool, until: Instant) -> Step {
    tracing::error!(
        name: "gateway.exchange.framing_violation",
        violation = ?violation,
        "the exchange driver's call sequence contradicted the exchange machine",
    );
    Step::End(if final_head_written {
        End::Abort
    } else {
        End::Reject { status: StatusCode::BAD_GATEWAY, linger: Some(until) }
    })
}

fn fault_step(fault: Fault, final_head_written: bool, until: Instant) -> Step {
    match fault {
        Fault::Violation(violation) => violation_step(violation, final_head_written, until),
        Fault::Abort => Step::End(End::Abort),
        Fault::Reject { .. } if final_head_written => Step::End(End::Abort),
        Fault::Reject { status, linger } => {
            Step::End(End::Reject { status, linger: linger.then_some(until) })
        }
    }
}

// ---------------------------------------------------------------------------
// The two concurrent halves
// ---------------------------------------------------------------------------

/// What both halves of one exchange share.
struct Shared<'a> {
    services: &'a ConnectionServices,
    machine: &'a Mutex<ExchangeMachine>,
    request_deadline: Instant,
    head_request: bool,
    close_after_response: bool,
    /// The final response head has begun to be written.
    final_head_written: AtomicBool,
    /// The instant the upstream request was completely written.
    request_complete: watch::Sender<Option<Instant>>,
}

impl Shared<'_> {
    fn machine(&self) -> MutexGuard<'_, ExchangeMachine> {
        self.machine.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn with_machine<T>(
        &self,
        call: impl FnOnce(&mut ExchangeMachine) -> Result<T, FramingViolation>,
    ) -> Result<T, Fault> {
        call(&mut self.machine()).map_err(Fault::Violation)
    }

    fn now(&self) -> Instant {
        self.services.clock.now()
    }

    fn limits(&self) -> &GatewayLimits {
        &self.services.limits
    }

    fn relay(&self) -> &dyn GatewayRelay {
        self.services.relay.as_ref()
    }

    fn body_deadlines(&self) -> RelayDeadlines {
        RelayDeadlines {
            absolute: self.request_deadline,
            no_progress: self.limits().body_no_progress_deadline(),
        }
    }

    fn framing_line_deadlines(&self) -> ReadDeadlines {
        compose_read_deadlines(
            ReadPhase::FramingLine,
            self.now(),
            self.request_deadline,
            self.limits(),
        )
    }
}

async fn run_halves(
    shared: &Shared<'_>,
    drain: &DrainSignal,
    (public_read, upstream_write): (&mut KtlsReadHalf, &mut KtlsWriteHalf),
    (upstream_read, public_write): (&mut KtlsReadHalf, &mut KtlsWriteHalf),
    framing: BodyFraming,
) -> Outcome {
    let mut drain_noted = drain.deadline().is_some();
    if drain_noted {
        shared.machine().drain_begin();
    }
    let request = request_half(shared, public_read, upstream_write, framing);
    let response = response_half(shared, upstream_read, public_write);
    tokio::pin!(request, response);
    let mut request_done = false;
    loop {
        tokio::select! {
            biased;
            _ = drain.begun(), if !drain_noted => {
                drain_noted = true;
                shared.machine().drain_begin();
            }
            result = &mut request, if !request_done => match result {
                Ok(()) => request_done = true,
                Err(fault) => return Outcome::Fault(fault),
            },
            result = &mut response => return match result {
                Ok(()) => Outcome::Concluded,
                Err(fault) => Outcome::Fault(fault),
            },
        }
    }
}

// -- request half -------------------------------------------------------------

async fn request_half(
    shared: &Shared<'_>,
    public_read: &mut KtlsReadHalf,
    upstream_write: &mut KtlsWriteHalf,
    framing: BodyFraming,
) -> Result<(), Fault> {
    match framing {
        BodyFraming::NoBody | BodyFraming::Length(0) => {}
        BodyFraming::Length(length) => {
            relay_request_exact(shared, public_read, upstream_write, length).await?;
        }
        BodyFraming::Chunked => relay_request_chunks(shared, public_read, upstream_write).await?,
        BodyFraming::UntilClose => {
            return Err(Fault::Violation(FramingViolation::UntilCloseRequest));
        }
    }
    shared.request_complete.send_replace(Some(shared.now()));
    Ok(())
}

async fn relay_request_exact(
    shared: &Shared<'_>,
    public_read: &mut KtlsReadHalf,
    upstream_write: &mut KtlsWriteHalf,
    length: u64,
) -> Result<(), Fault> {
    let Some(owed) = NonZeroU64::new(length) else {
        return Ok(());
    };
    shared
        .relay()
        .relay_exact(public_read, upstream_write, owed, shared.body_deadlines())
        .await
        .map_err(request_relay_fault)?;
    shared.with_machine(|machine| machine.request_octets_moved(length))
}

fn request_relay_fault(error: RelayError) -> Fault {
    match error {
        RelayError::NoProgress => {
            Fault::Reject { status: StatusCode::REQUEST_TIMEOUT, linger: false }
        }
        RelayError::Deadline | RelayError::Write(_) | RelayError::Pipe(_) => Fault::bad_gateway(),
        RelayError::Read(_)
        | RelayError::SourceEndedMidBody { .. }
        | RelayError::LimitExceeded { .. } => Fault::Abort,
    }
}

async fn relay_request_chunks(
    shared: &Shared<'_>,
    public_read: &mut KtlsReadHalf,
    upstream_write: &mut KtlsWriteHalf,
) -> Result<(), Fault> {
    let line_limit = shared.limits().max_request_head_bytes();
    let ceiling = shared.limits().max_request_body_bytes().get();
    let mut total: u64 = 0;
    loop {
        let line = read_request_line(shared, public_read, line_limit).await?;
        let size = parse_chunk_size_line(line.as_slice())
            .map_err(|_| Fault::reject(StatusCode::BAD_REQUEST))?;
        let Some(owed) = NonZeroU64::new(size) else {
            break;
        };
        total = total.saturating_add(size);
        if total > ceiling {
            return Err(Fault::reject(StatusCode::PAYLOAD_TOO_LARGE));
        }
        write_upstream(shared, upstream_write, format!("{size:x}\r\n").as_bytes()).await?;
        shared
            .relay()
            .relay_exact(public_read, upstream_write, owed, shared.body_deadlines())
            .await
            .map_err(request_relay_fault)?;
        shared.with_machine(|machine| machine.request_octets_moved(size))?;
        let terminator = read_request_line(shared, public_read, line_limit).await?;
        check_chunk_terminator(terminator.as_slice())
            .map_err(|_| Fault::reject(StatusCode::BAD_REQUEST))?;
        write_upstream(shared, upstream_write, b"\r\n").await?;
    }
    let mut trailer_octets = 0_usize;
    loop {
        let line = read_request_line(shared, public_read, line_limit).await?;
        trailer_octets = trailer_octets.saturating_add(line.as_slice().len());
        if trailer_octets > line_limit.get() {
            return Err(Fault::reject(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE));
        }
        let ends =
            is_trailer_end(line.as_slice()).map_err(|_| Fault::reject(StatusCode::BAD_REQUEST))?;
        if ends {
            break;
        }
    }
    write_upstream(shared, upstream_write, b"0\r\n\r\n").await?;
    shared.with_machine(ExchangeMachine::request_chunked_end)
}

async fn write_upstream(
    shared: &Shared<'_>,
    upstream_write: &mut KtlsWriteHalf,
    bytes: &[u8],
) -> Result<(), Fault> {
    shared
        .relay()
        .write_all(upstream_write, bytes, shared.request_deadline)
        .await
        .map_err(|_| Fault::bad_gateway())
}

/// One chunk-size, chunk-terminator or trailer line of a request body.
async fn read_request_line(
    shared: &Shared<'_>,
    public_read: &mut KtlsReadHalf,
    limit: NonZeroUsize,
) -> Result<ThroughBytes, Fault> {
    shared
        .relay()
        .read_through(public_read, HeadTerminator::LineEnd, limit, shared.framing_line_deadlines())
        .await
        .map_err(|error| match error {
            LegReadError::Deadline if shared.now() >= shared.request_deadline => {
                Fault::bad_gateway()
            }
            LegReadError::Deadline => {
                Fault::Reject { status: StatusCode::REQUEST_TIMEOUT, linger: false }
            }
            LegReadError::LimitExceeded { .. } => Fault::reject(StatusCode::BAD_REQUEST),
            _ => Fault::Abort,
        })
}

// -- response half ------------------------------------------------------------

async fn response_half(
    shared: &Shared<'_>,
    upstream_read: &mut KtlsReadHalf,
    public_write: &mut KtlsWriteHalf,
) -> Result<(), Fault> {
    let limits = shared.limits();
    let mut cumulative = 0_usize;
    let mut request_complete = shared.request_complete.subscribe();
    let parsed = loop {
        let remaining = limits.max_response_head_bytes().get().saturating_sub(cumulative);
        let limit = NonZeroUsize::new(remaining).ok_or_else(Fault::bad_gateway)?;
        let head = read_response_head(shared, upstream_read, limit, &mut request_complete)
            .await
            .map_err(|_| Fault::bad_gateway())?;
        cumulative = cumulative.saturating_add(head.as_slice().len());
        match parse_response_head(head.as_slice(), shared.head_request, limits)
            .map_err(|_| Fault::bad_gateway())?
        {
            ParsedResponseHead::Interim { status, forward } => {
                if forward {
                    let interim = format!("HTTP/1.1 {}\r\n\r\n", status_text(status));
                    shared
                        .relay()
                        .write_all(public_write, interim.as_bytes(), shared.request_deadline)
                        .await
                        .map_err(|_| Fault::Abort)?;
                }
            }
            ParsedResponseHead::Final(parsed) => break parsed,
        }
    };
    let status = parsed.status();
    let framing = parsed.framing();
    shared.with_machine(|machine| machine.response_head(framing))?;
    let draining = shared.machine().is_draining();
    let close = shared.close_after_response || draining || framing == BodyFraming::UntilClose;
    let head = public_response_head(
        status,
        rewrite_response_headers(parsed.into_headers()),
        framing,
        close,
    );
    shared.final_head_written.store(true, Ordering::Release);
    shared
        .relay()
        .write_all(public_write, &head, shared.request_deadline)
        .await
        .map_err(|_| Fault::Abort)?;
    match framing {
        BodyFraming::NoBody | BodyFraming::Length(0) => Ok(()),
        BodyFraming::Length(length) => {
            relay_response_exact(shared, upstream_read, public_write, length).await
        }
        BodyFraming::Chunked => relay_response_chunks(shared, upstream_read, public_write).await,
        BodyFraming::UntilClose => {
            let moved = shared
                .relay()
                .relay_to_end(
                    upstream_read,
                    public_write,
                    shared.limits().max_response_body_bytes(),
                    shared.body_deadlines(),
                )
                .await
                .map_err(|_| Fault::Abort)?;
            shared.with_machine(|machine| machine.response_octets_moved(moved))?;
            shared.with_machine(|machine| {
                machine.peer_closed(Side::Upstream, true);
                Ok(())
            })
        }
    }
}

/// One response head read, re-issued at `t0` so the response-head budget starts
/// when the upstream request is completely written.
async fn read_response_head(
    shared: &Shared<'_>,
    upstream_read: &mut KtlsReadHalf,
    limit: NonZeroUsize,
    request_complete: &mut watch::Receiver<Option<Instant>>,
) -> Result<ThroughBytes, LegReadError> {
    loop {
        let completed_at = *request_complete.borrow_and_update();
        let deadlines = compose_read_deadlines(
            ReadPhase::ResponseHead { request_complete_at: completed_at },
            shared.now(),
            shared.request_deadline,
            shared.limits(),
        );
        let read = shared.relay().read_through(
            upstream_read,
            HeadTerminator::HeaderBlockEnd,
            limit,
            deadlines,
        );
        tokio::select! {
            result = read => return result,
            changed = request_complete.changed(), if completed_at.is_none() => {
                if changed.is_err() {
                    return Err(LegReadError::Deadline);
                }
            }
        }
    }
}

async fn relay_response_exact(
    shared: &Shared<'_>,
    upstream_read: &mut KtlsReadHalf,
    public_write: &mut KtlsWriteHalf,
    length: u64,
) -> Result<(), Fault> {
    let Some(owed) = NonZeroU64::new(length) else {
        return Ok(());
    };
    shared
        .relay()
        .relay_exact(upstream_read, public_write, owed, shared.body_deadlines())
        .await
        .map_err(|_| Fault::Abort)?;
    shared.with_machine(|machine| machine.response_octets_moved(length))
}

async fn relay_response_chunks(
    shared: &Shared<'_>,
    upstream_read: &mut KtlsReadHalf,
    public_write: &mut KtlsWriteHalf,
) -> Result<(), Fault> {
    let line_limit = shared.limits().max_response_head_bytes();
    let ceiling = shared.limits().max_response_body_bytes().get();
    let mut total: u64 = 0;
    loop {
        let line = read_response_line(shared, upstream_read, line_limit).await?;
        let size = parse_chunk_size_line(line.as_slice()).map_err(|_| Fault::Abort)?;
        let Some(owed) = NonZeroU64::new(size) else {
            break;
        };
        total = total.saturating_add(size);
        if total > ceiling {
            return Err(Fault::Abort);
        }
        write_public(shared, public_write, format!("{size:x}\r\n").as_bytes()).await?;
        shared
            .relay()
            .relay_exact(upstream_read, public_write, owed, shared.body_deadlines())
            .await
            .map_err(|_| Fault::Abort)?;
        shared.with_machine(|machine| machine.response_octets_moved(size))?;
        let terminator = read_response_line(shared, upstream_read, line_limit).await?;
        check_chunk_terminator(terminator.as_slice()).map_err(|_| Fault::Abort)?;
        write_public(shared, public_write, b"\r\n").await?;
    }
    let mut trailer_octets = 0_usize;
    loop {
        let line = read_response_line(shared, upstream_read, line_limit).await?;
        trailer_octets = trailer_octets.saturating_add(line.as_slice().len());
        if trailer_octets > line_limit.get() {
            return Err(Fault::Abort);
        }
        if is_trailer_end(line.as_slice()).map_err(|_| Fault::Abort)? {
            break;
        }
    }
    write_public(shared, public_write, b"0\r\n\r\n").await?;
    shared.with_machine(ExchangeMachine::response_chunked_end)
}

async fn write_public(
    shared: &Shared<'_>,
    public_write: &mut KtlsWriteHalf,
    bytes: &[u8],
) -> Result<(), Fault> {
    shared
        .relay()
        .write_all(public_write, bytes, shared.request_deadline)
        .await
        .map_err(|_| Fault::Abort)
}

/// One chunk-size, chunk-terminator or trailer line of a response body; any
/// failure is after the public head, so it closes both legs.
async fn read_response_line(
    shared: &Shared<'_>,
    upstream_read: &mut KtlsReadHalf,
    limit: NonZeroUsize,
) -> Result<ThroughBytes, Fault> {
    shared
        .relay()
        .read_through(
            upstream_read,
            HeadTerminator::LineEnd,
            limit,
            shared.framing_line_deadlines(),
        )
        .await
        .map_err(|_| Fault::Abort)
}

// ---------------------------------------------------------------------------
// Wire heads
// ---------------------------------------------------------------------------

fn status_text(status: StatusCode) -> String {
    format!("{} {}", status.as_str(), status.canonical_reason().unwrap_or_default())
}

/// A gateway-authored final response: empty body, connection closing.
fn authored_response(status: StatusCode) -> Vec<u8> {
    format!("HTTP/1.1 {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", status_text(status))
        .into_bytes()
}

fn append_headers(head: &mut Vec<u8>, headers: &HeaderMap) {
    for (name, value) in headers {
        head.extend_from_slice(name.as_str().as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
}

/// The gateway alone emits framing headers: the original ones are dropped and
/// the one the framing decision derived is written.
fn replace_framing_headers(headers: &mut HeaderMap, framing: BodyFraming) {
    if framing == BodyFraming::NoBody {
        return;
    }
    headers.remove(CONTENT_LENGTH);
    headers.remove(TRANSFER_ENCODING);
    match framing {
        BodyFraming::Length(length) => {
            headers.insert(CONTENT_LENGTH, HeaderValue::from(length));
        }
        BodyFraming::Chunked => {
            headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        }
        BodyFraming::NoBody | BodyFraming::UntilClose => {}
    }
}

/// The upstream request head: the rewritten headers, `Connection: close` (one
/// request per upstream leg) and the gateway-authored framing header.
fn upstream_request_head(
    method: &Method,
    target: &str,
    mut headers: HeaderMap,
    framing: BodyFraming,
) -> Vec<u8> {
    replace_framing_headers(&mut headers, framing);
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    let mut head = format!("{method} {target} HTTP/1.1\r\n").into_bytes();
    append_headers(&mut head, &headers);
    head.extend_from_slice(b"\r\n");
    head
}

/// The public response head: the rewritten headers, the gateway-authored
/// framing header and `Connection: close` when the connection ends with it.
fn public_response_head(
    status: StatusCode,
    mut headers: HeaderMap,
    framing: BodyFraming,
    close: bool,
) -> Vec<u8> {
    replace_framing_headers(&mut headers, framing);
    if close {
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
    }
    let mut head = format!("HTTP/1.1 {}\r\n", status_text(status)).into_bytes();
    append_headers(&mut head, &headers);
    head.extend_from_slice(b"\r\n");
    head
}
