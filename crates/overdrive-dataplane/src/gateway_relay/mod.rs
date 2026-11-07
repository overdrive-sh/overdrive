//! The host backend of the `GatewayRelay` driven port.
//!
//! `HostGatewayRelay` (G1, ADR-0244/0246/0249/0250/0253) completes a userspace
//! rustls TLS 1.3 handshake one record at a time, arms kernel TLS from the
//! resulting `KernelConnection`, reads heads and framing lines through the
//! terminator only, moves bodies by `splice(2)` through an empty private pipe of
//! at least 64 KiB, answers a peer KeyUpdate (never originating one), and ends
//! close-delimited bodies on the peer's close_notify. The one `unsafe` module is
//! [`sys`]; this module stays safe code.
//!
//! Kernel claims (`vendors/linux` @ v7.0): a peek stops at a non-DATA record
//! (`net/tls/tls_sw.c:1849-1862, 2089-2090`); a splice read of a non-DATA record is
//! `-EINVAL` and leaves it readable (`tls_sw.c:2310-2314`); after a KeyUpdate reads
//! fail `-EKEYEXPIRED` until `TLS_RX` is re-armed (`tls_sw.c:1369-1371`); a control
//! record flushes an open record first (`tls_main.c:228-268`); splice to a socket
//! honours only the socket's own `O_NONBLOCK` and returns partial counts
//! (`fs/splice.c:874-888`).
//!
//! The transmit lock of a leg is held for one whole `write_all` or one drain of
//! the pipe into the socket, never for a whole body, so a KeyUpdate answer is
//! sent between write steps (assumption A-K5).

#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::significant_drop_tightening,
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::trivially_copy_pass_by_ref,
    clippy::needless_pass_by_value,
    reason = "kernel-glue module: long state-machine bodies and kernel identifiers in docs"
)]

mod sys;

use std::future::Future;
use std::io;
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use overdrive_core::public_ingress::PublicHostname;
use overdrive_core::traits::clock::Clock;
use overdrive_gateway::ports::{
    AcceptedPublicLeg, GatewayRelay, HeadTerminator, KeyUpdateError, KtlsArmError, KtlsLeg,
    KtlsReadHalf, KtlsWriteHalf, LegReadError, LegWriteError, PublicHandshakeError, ReadDeadlines,
    RelayDeadlines, RelayError, RelayPipeError, RelayProbeError, ThroughBytes,
};
use parking_lot::Mutex;
use rustls::kernel::KernelConnection;
use rustls::server::{Acceptor, UnbufferedServerConnection};
use rustls::unbuffered::{ConnectionState, EncodeError};
use rustls::{ConnectionTrafficSecrets, Error as TlsError, ExtractedSecrets};
use tokio::io::unix::AsyncFd;

use self::sys::{CryptoInfoFault, Received};

/// The relay pipe's capacity floor (ADR-0246): the only runtime outcomes of the
/// pipe typestate are a failed creation or a capacity that reads back below it.
pub(crate) const PIPE_FLOOR: usize = 65_536;

/// A capacity read back from `F_GETPIPE_SZ`; it cannot be below [`PIPE_FLOOR`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct PipeCapacity(usize);

impl PipeCapacity {
    /// `Err(CapacityBelowFloor { got })` below the floor.
    pub(crate) fn new(read_back: usize) -> Result<Self, RelayPipeError> {
        if read_back < PIPE_FLOOR {
            return Err(RelayPipeError::CapacityBelowFloor { got: read_back });
        }
        Ok(Self(read_back))
    }

    pub(crate) fn get(&self) -> usize {
        self.0
    }
}

/// `min(octets owed, capacity)`: the length of the next splice into an empty
/// pipe (never above either).
pub(crate) fn splice_in_len(owed: NonZeroU64, capacity: PipeCapacity) -> NonZeroU64 {
    let held = u64::try_from(capacity.get()).unwrap_or(u64::MAX);
    NonZeroU64::new(owed.get().min(held))
        .unwrap_or_else(|| unreachable!("owed is nonzero and the capacity is at least the floor"))
}

// ---------------------------------------------------------------------------
// Deadlines
// ---------------------------------------------------------------------------

/// Runs `fut` until `limit` on `clock`; `Err(())` when the limit came first.
async fn bounded_opt<F: Future>(
    clock: &dyn Clock,
    limit: Option<Instant>,
    fut: F,
) -> Result<F::Output, ()> {
    let Some(limit) = limit else {
        return Ok(fut.await);
    };
    let wait = limit.saturating_duration_since(clock.now());
    tokio::select! {
        biased;
        out = fut => Ok(out),
        () = clock.sleep(wait) => Err(()),
    }
}

#[derive(Debug, Clone, Copy)]
enum Stall {
    Deadline,
    NoProgress,
}

/// Absolute and no-progress bounds of one relay call.
struct Pace<'a> {
    clock: &'a dyn Clock,
    absolute: Instant,
    no_progress: Duration,
    last: Instant,
}

impl<'a> Pace<'a> {
    fn new(clock: &'a dyn Clock, deadlines: RelayDeadlines) -> Self {
        Self {
            clock,
            absolute: deadlines.absolute,
            no_progress: deadlines.no_progress,
            last: clock.now(),
        }
    }

    fn note(&mut self) {
        self.last = self.clock.now();
    }

    async fn bounded<F: Future>(&self, fut: F) -> Result<F::Output, Stall> {
        let window = self.last.checked_add(self.no_progress).unwrap_or(self.absolute);
        let limit = self.absolute.min(window);
        match bounded_opt(self.clock, Some(limit), fut).await {
            Ok(out) => Ok(out),
            Err(()) if self.clock.now() >= self.absolute => Err(Stall::Deadline),
            Err(()) => Err(Stall::NoProgress),
        }
    }
}

// ---------------------------------------------------------------------------
// Leg state
// ---------------------------------------------------------------------------

/// What rustls keeps after the handshake: the next traffic secrets.
trait Rekey: Send {
    fn next_tx(&mut self) -> Result<(u64, ConnectionTrafficSecrets), TlsError>;
    fn next_rx(&mut self) -> Result<(u64, ConnectionTrafficSecrets), TlsError>;
}

impl<D: Send> Rekey for KernelConnection<D> {
    fn next_tx(&mut self) -> Result<(u64, ConnectionTrafficSecrets), TlsError> {
        self.update_tx_secret()
    }

    fn next_rx(&mut self) -> Result<(u64, ConnectionTrafficSecrets), TlsError> {
        self.update_rx_secret()
    }
}

/// The socket and key state both halves of a leg share.
struct LegCore {
    fd: AsyncFd<OwnedFd>,
    rekey: Mutex<Box<dyn Rekey>>,
    /// The leg's transmit lock (Contract of the legs, item 5).
    tx: tokio::sync::Mutex<()>,
    /// Peer-requested KeyUpdates whose answers have not been sent yet. Incremented
    /// in the same synchronous segment that consumes the peer's record and re-arms
    /// RX, and decremented in the same poll that sends one answer and re-arms TX,
    /// so a dropped `read_through` can never leave the leg half re-keyed. Every
    /// entry point that reads or writes the leg drives it to zero first.
    answers_owed: Mutex<u32>,
}

fn errno_of(fault: CryptoInfoFault) -> i32 {
    match fault {
        CryptoInfoFault::CipherNotSupported => libc::EINVAL,
        CryptoInfoFault::Errno(errno) => errno,
    }
}

impl LegCore {
    fn raw(&self) -> RawFd {
        self.fd.get_ref().as_raw_fd()
    }

    async fn read_io<T>(&self, mut op: impl FnMut(RawFd) -> io::Result<T>) -> io::Result<T> {
        loop {
            let mut guard = self.fd.readable().await?;
            if let Ok(result) = guard.try_io(|inner| op(inner.get_ref().as_raw_fd())) {
                return result;
            }
        }
    }

    async fn write_io<T>(&self, mut op: impl FnMut(RawFd) -> io::Result<T>) -> io::Result<T> {
        loop {
            let mut guard = self.fd.writable().await?;
            if let Ok(result) = guard.try_io(|inner| op(inner.get_ref().as_raw_fd())) {
                return result;
            }
        }
    }

    /// The receive side of a peer KeyUpdate: re-arms RX from the next secret. It has
    /// no await point, so the caller runs it in one poll with the record's
    /// consumption (RX reads fail `-EKEYEXPIRED` until this has run).
    fn rearm_rx(&self) -> Result<(), KeyUpdateError> {
        let rx = self.rekey.lock().next_rx();
        let (sequence, secrets) = rx.map_err(|_| KeyUpdateError::RustlsRefused)?;
        sys::set_gcm_crypto_info(self.raw(), sys::TLS_RX, sequence, &secrets)
            .map_err(|fault| KeyUpdateError::RearmRx { errno: errno_of(fault) })
    }

    /// Records that the peer asked for an update, so our own update record is owed.
    /// The gateway never originates one (ADR-0250).
    fn owe_answer(&self) {
        *self.answers_owed.lock() += 1;
    }

    /// Sends the owed answer under the old key, then re-arms TX, with the transmit
    /// lock already held (`_held`). The record's send is the only await; the TX
    /// re-arm runs in the same poll as its completion, so cancelling at any await
    /// leaves the answer owed and the TX key unchanged, and no writer can send
    /// under the old key after the peer has switched. Re-entry is idempotent.
    async fn answer_owed_locked(
        &self,
        _held: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), KeyUpdateError> {
        let update = [24u8, 0, 0, 1, 0];
        while *self.answers_owed.lock() > 0 {
            let sent = self
                .write_io(|fd| sys::send_record(fd, sys::RECORD_HANDSHAKE, &update))
                .await
                .map_err(|err| KeyUpdateError::Io { kind: err.kind() })?;
            if sent != update.len() {
                return Err(KeyUpdateError::Io { kind: io::ErrorKind::WriteZero });
            }
            *self.answers_owed.lock() -= 1;
            let tx = self.rekey.lock().next_tx();
            let (sequence, secrets) = tx.map_err(|_| KeyUpdateError::RustlsRefused)?;
            sys::set_gcm_crypto_info(self.raw(), sys::TLS_TX, sequence, &secrets)
                .map_err(|fault| KeyUpdateError::RearmTx { errno: errno_of(fault) })?;
        }
        Ok(())
    }

    /// Drives an owed answer to completion (a no-op, with no await, when none is owed).
    async fn settle(&self) -> Result<(), KeyUpdateError> {
        if *self.answers_owed.lock() == 0 {
            return Ok(());
        }
        let held = self.tx.lock().await;
        self.answer_owed_locked(&held).await
    }
}

/// A private relay pipe whose capacity read back at or above the floor.
struct RelayPipe {
    read: OwnedFd,
    write: OwnedFd,
    capacity: PipeCapacity,
}

impl RelayPipe {
    fn create() -> Result<Self, RelayPipeError> {
        let (read, write, got) = sys::make_pipe(PIPE_FLOOR)
            .map_err(|err| RelayPipeError::Create { kind: err.kind() })?;
        Ok(Self { read, write, capacity: PipeCapacity::new(got)? })
    }
}

struct ReadHalf {
    leg: Arc<LegCore>,
    /// Octets of a partial head consumed from the kernel (retained across a
    /// dropped `read_through`).
    head: Vec<u8>,
    pipe: Option<RelayPipe>,
    ended: bool,
}

struct WriteHalf {
    leg: Arc<LegCore>,
}

enum Control {
    Handled,
    CloseNotify,
}

enum Spliced {
    Moved(usize),
    CloseNotify,
    Eof,
}

enum SpliceFail {
    Read(LegReadError),
    Stall(Stall),
}

fn read_io_err(err: &io::Error) -> LegReadError {
    LegReadError::Io { kind: err.kind() }
}

fn write_err(err: &io::Error) -> LegWriteError {
    match err.kind() {
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset => LegWriteError::PeerClosed,
        kind => LegWriteError::Io { kind },
    }
}

fn relay_stall(stall: Stall) -> RelayError {
    match stall {
        Stall::Deadline => RelayError::Deadline,
        Stall::NoProgress => RelayError::NoProgress,
    }
}

impl ReadHalf {
    fn new(leg: Arc<LegCore>) -> Self {
        Self { leg, head: Vec::new(), pipe: None, ended: false }
    }

    fn take_or_create_pipe(&mut self) -> Result<RelayPipe, RelayPipeError> {
        match self.pipe.take() {
            Some(pipe) => Ok(pipe),
            None => RelayPipe::create(),
        }
    }

    /// Consumes the control record that a prior peek showed is next, into `buf`.
    /// No await: the record is present, so the receive cannot wait.
    fn take_record(&self, buf: &mut [u8]) -> Result<(), LegReadError> {
        match sys::recv_record(self.leg.raw(), buf, false) {
            Ok(Received::Control { .. }) => Ok(()),
            Ok(Received::Data(_)) => Err(LegReadError::Io { kind: io::ErrorKind::InvalidData }),
            Err(err) => Err(read_io_err(&err)),
        }
    }

    /// A control record is next in the socket: handle it (Contract of the legs,
    /// item 4). The only await is the peek; everything after it, including a
    /// KeyUpdate's owed-answer record, consumption and RX re-arm, runs in one poll,
    /// so a dropped call either changed nothing or finished the receive side.
    async fn next_control(&mut self) -> Result<Control, LegReadError> {
        let mut buf = vec![0u8; sys::CONTROL_RECORD_BUFFER];
        let peeked = self
            .leg
            .read_io(|fd| sys::recv_record(fd, &mut buf, true))
            .await
            .map_err(|err| read_io_err(&err))?;
        let Received::Control { record, len } = peeked else {
            return Err(LegReadError::Io { kind: io::ErrorKind::InvalidData });
        };
        if let (sys::RECORD_HANDSHAKE, [24, 0, 0, 1, flag @ 0..=1]) = (record, &buf[..len]) {
            if *flag == 1 {
                self.leg.owe_answer();
            }
            self.take_record(&mut buf)?;
            self.leg.rearm_rx().map_err(LegReadError::KeyUpdate)?;
            return Ok(Control::Handled);
        }
        self.take_record(&mut buf)?;
        self.handle_control(record, &buf[..len])
    }

    /// Handles one consumed non-KeyUpdate control record.
    fn handle_control(&mut self, record: u8, payload: &[u8]) -> Result<Control, LegReadError> {
        match (record, payload) {
            (sys::RECORD_ALERT, [1, 0, ..]) => {
                self.ended = true;
                Ok(Control::CloseNotify)
            }
            (sys::RECORD_ALERT, [_, description, ..]) => {
                Err(LegReadError::Alert { description: *description })
            }
            (sys::RECORD_HANDSHAKE, [4, ..]) => Ok(Control::Handled),
            (content_type, _) => Err(LegReadError::UnexpectedRecord { content_type }),
        }
    }

    /// Splices up to `len` octets into the (empty) pipe, handling control records.
    async fn splice_in(
        &mut self,
        pipe_write: RawFd,
        len: usize,
        pace: &Pace<'_>,
    ) -> Result<Spliced, SpliceFail> {
        loop {
            pace.bounded(self.leg.settle())
                .await
                .map_err(SpliceFail::Stall)?
                .map_err(|err| SpliceFail::Read(LegReadError::KeyUpdate(err)))?;
            let spliced = pace
                .bounded(
                    self.leg.read_io(|fd| sys::splice(fd, pipe_write, len, sys::SPLICE_F_NONBLOCK)),
                )
                .await
                .map_err(SpliceFail::Stall)?;
            match spliced {
                Ok(0) => return Ok(Spliced::Eof),
                Ok(moved) => return Ok(Spliced::Moved(moved)),
                Err(err) if err.raw_os_error() == Some(libc::EINVAL) => {
                    match self.next_control().await.map_err(SpliceFail::Read)? {
                        Control::Handled => {}
                        Control::CloseNotify => return Ok(Spliced::CloseNotify),
                    }
                }
                Err(err) => return Err(SpliceFail::Read(read_io_err(&err))),
            }
        }
    }

    /// Consumes exactly `count` octets that a peek already showed are present.
    fn consume(&self, count: usize) -> Result<(), LegReadError> {
        let mut buf = vec![0u8; count];
        let mut at = 0;
        while at < count {
            match sys::recv_record(self.leg.raw(), &mut buf[at..], false) {
                Ok(Received::Data(0) | Received::Control { .. }) => {
                    return Err(LegReadError::Io { kind: io::ErrorKind::UnexpectedEof });
                }
                Ok(Received::Data(n)) => at += n,
                Err(err) => return Err(read_io_err(&err)),
            }
        }
        Ok(())
    }

    async fn read_head(
        &mut self,
        clock: &dyn Clock,
        terminator: HeadTerminator,
        limit: usize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        let mut first_at = (!self.head.is_empty()).then(|| clock.now());
        loop {
            if self.ended {
                return Err(if self.head.is_empty() {
                    LegReadError::CleanEof
                } else {
                    LegReadError::Truncated
                });
            }
            let limit_at = match first_at {
                None => Some(
                    deadlines
                        .complete_by
                        .map_or(deadlines.first_octet, |by| by.min(deadlines.first_octet)),
                ),
                Some(first) => {
                    let within = deadlines.complete_within.and_then(|d| first.checked_add(d));
                    match (deadlines.complete_by, within) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    }
                }
            };
            match bounded_opt(clock, limit_at, self.leg.settle()).await {
                Err(()) => return Err(LegReadError::Deadline),
                Ok(Err(err)) => return Err(LegReadError::KeyUpdate(err)),
                Ok(Ok(())) => {}
            }
            let want = limit.saturating_sub(self.head.len()).clamp(1, 65_536);
            let mut peeked = vec![0u8; want];
            let received = match bounded_opt(
                clock,
                limit_at,
                self.leg.read_io(|fd| sys::recv_record(fd, &mut peeked, true)),
            )
            .await
            {
                Err(()) => return Err(LegReadError::Deadline),
                Ok(Err(err)) => return Err(read_io_err(&err)),
                Ok(Ok(received)) => received,
            };
            match received {
                Received::Data(0) => return Err(LegReadError::Truncated),
                Received::Data(n) => {
                    let prior = self.head.len();
                    self.head.extend_from_slice(&peeked[..n]);
                    if let Some(end) = find_end(&self.head, terminator, prior) {
                        self.head.truncate(end);
                        self.consume(end - prior)?;
                        return Ok(ThroughBytes::new(std::mem::take(&mut self.head)));
                    }
                    if self.head.len() >= limit {
                        self.head.truncate(prior);
                        return Err(LegReadError::LimitExceeded { limit });
                    }
                    self.consume(n)?;
                    if first_at.is_none() {
                        first_at = Some(clock.now());
                    }
                }
                Received::Control { .. } => {
                    self.next_control().await?;
                }
            }
        }
    }
}

/// Index just past the first terminator in `head`, searching from `from`.
fn find_end(head: &[u8], terminator: HeadTerminator, from: usize) -> Option<usize> {
    let pattern: &[u8] = match terminator {
        HeadTerminator::HeaderBlockEnd => b"\r\n\r\n",
        HeadTerminator::LineEnd => b"\r\n",
    };
    let start = from.saturating_sub(pattern.len() - 1);
    head.get(start..)?
        .windows(pattern.len())
        .position(|w| w == pattern)
        .map(|i| start + i + pattern.len())
}

/// Moves `count` octets from the pipe into `dst`, holding its transmit lock.
async fn drain_pipe(
    dst: &WriteHalf,
    pipe_read: RawFd,
    mut count: usize,
    pace: &mut Pace<'_>,
) -> Result<(), RelayError> {
    let transmit = pace.bounded(dst.leg.tx.lock()).await.map_err(relay_stall)?;
    pace.bounded(dst.leg.answer_owed_locked(&transmit))
        .await
        .map_err(relay_stall)?
        .map_err(|err| RelayError::Write(LegWriteError::KeyUpdate(err)))?;
    while count > 0 {
        let moved = pace
            .bounded(dst.leg.write_io(|fd| sys::splice(pipe_read, fd, count, 0)))
            .await
            .map_err(relay_stall)?
            .map_err(|err| RelayError::Write(write_err(&err)))?;
        if moved == 0 {
            return Err(RelayError::Write(LegWriteError::Io { kind: io::ErrorKind::WriteZero }));
        }
        count = count.saturating_sub(moved);
        pace.note();
    }
    Ok(())
}

fn splice_fail(fail: SpliceFail) -> RelayError {
    match fail {
        SpliceFail::Read(err) => RelayError::Read(err),
        SpliceFail::Stall(stall) => relay_stall(stall),
    }
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

fn handshake_io(err: &io::Error) -> PublicHandshakeError {
    PublicHandshakeError::Io { kind: err.kind() }
}

fn handshake_tls(err: &TlsError) -> PublicHandshakeError {
    let alert = match err {
        TlsError::AlertReceived(description) => Some(u8::from(*description)),
        _ => None,
    };
    PublicHandshakeError::Tls { alert }
}

async fn read_exact_plain(fd: &AsyncFd<OwnedFd>, buf: &mut [u8]) -> io::Result<()> {
    let mut at = 0;
    while at < buf.len() {
        let mut guard = fd.readable().await?;
        match guard.try_io(|inner| sys::read_plain(inner.get_ref().as_raw_fd(), &mut buf[at..])) {
            Ok(Ok(0)) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(Ok(n)) => at += n,
            Ok(Err(err)) => return Err(err),
            Err(_would_block) => {}
        }
    }
    Ok(())
}

async fn write_all_plain(fd: &AsyncFd<OwnedFd>, buf: &[u8]) -> io::Result<()> {
    let mut at = 0;
    while at < buf.len() {
        let mut guard = fd.writable().await?;
        match guard.try_io(|inner| sys::write_plain(inner.get_ref().as_raw_fd(), &buf[at..])) {
            Ok(Ok(n)) => at += n,
            Ok(Err(err)) => return Err(err),
            Err(_would_block) => {}
        }
    }
    Ok(())
}

/// Appends exactly one TLS record from `fd` to `out`.
async fn read_record(fd: &AsyncFd<OwnedFd>, out: &mut Vec<u8>) -> io::Result<()> {
    let mut header = [0u8; 5];
    read_exact_plain(fd, &mut header).await?;
    let len = usize::from(u16::from_be_bytes([header[3], header[4]]));
    out.extend_from_slice(&header);
    let at = out.len();
    out.resize(at + len, 0);
    read_exact_plain(fd, &mut out[at..]).await
}

struct Handshaken {
    sni: PublicHostname,
    secrets: ExtractedSecrets,
    rekey: Box<dyn Rekey>,
}

enum Next {
    Read,
    Continue,
    Transmit,
    Traffic,
}

/// The public TLS 1.3 server handshake, one record at a time so no application
/// octet is consumed before arming.
async fn server_handshake(
    fd: &AsyncFd<OwnedFd>,
    tls: Arc<rustls::ServerConfig>,
) -> Result<Handshaken, PublicHandshakeError> {
    let mut incoming: Vec<u8> = Vec::new();
    let mut acceptor = Acceptor::default();
    let accepted = loop {
        let before = incoming.len();
        read_record(fd, &mut incoming).await.map_err(|err| handshake_io(&err))?;
        acceptor.read_tls(&mut &incoming[before..]).map_err(|err| handshake_io(&err))?;
        match acceptor.accept() {
            Ok(Some(accepted)) => break accepted,
            Ok(None) => {}
            Err((err, _alert)) => return Err(handshake_tls(&err)),
        }
    };
    let hello = accepted.client_hello();
    let sni = hello
        .server_name()
        .and_then(|name| PublicHostname::new(name).ok())
        .ok_or(PublicHandshakeError::ServerNameMissing)?;
    if !hello.alpn().is_some_and(|mut offered| offered.any(|p| p == b"http/1.1")) {
        return Err(PublicHandshakeError::AlpnMismatch);
    }
    drop(accepted);

    let mut conn = UnbufferedServerConnection::new(tls).map_err(|err| handshake_tls(&err))?;
    let mut outgoing = vec![0u8; 4096];
    let mut used = 0usize;
    loop {
        let discard;
        let next;
        {
            let status = conn.process_tls_records(&mut incoming);
            discard = status.discard;
            next = match status.state.map_err(|err| handshake_tls(&err))? {
                ConnectionState::BlockedHandshake => Next::Read,
                ConnectionState::EncodeTlsData(mut state) => {
                    loop {
                        match state.encode(&mut outgoing[used..]) {
                            Ok(n) => {
                                used += n;
                                break;
                            }
                            Err(EncodeError::InsufficientSize(e)) => {
                                outgoing.resize(used + e.required_size, 0);
                            }
                            Err(EncodeError::AlreadyEncoded) => break,
                        }
                    }
                    Next::Continue
                }
                ConnectionState::TransmitTlsData(state) => {
                    state.done();
                    Next::Transmit
                }
                ConnectionState::WriteTraffic(_) => Next::Traffic,
                _ => return Err(PublicHandshakeError::Tls { alert: None }),
            };
        }
        incoming.drain(..discard);
        match next {
            Next::Read => {
                read_record(fd, &mut incoming).await.map_err(|err| handshake_io(&err))?;
            }
            Next::Continue => {}
            Next::Transmit => {
                write_all_plain(fd, &outgoing[..used]).await.map_err(|err| handshake_io(&err))?;
                used = 0;
            }
            Next::Traffic if conn.is_handshaking() => {
                read_record(fd, &mut incoming).await.map_err(|err| handshake_io(&err))?;
            }
            Next::Traffic => break,
        }
    }
    if !incoming.is_empty() || used > 0 {
        return Err(PublicHandshakeError::Tls { alert: None });
    }
    let (secrets, kernel) =
        conn.dangerous_into_kernel_connection().map_err(|err| handshake_tls(&err))?;
    let rekey: Box<dyn Rekey> = Box::new(kernel);
    Ok(Handshaken { sni, secrets, rekey })
}

/// Installs the TLS ULP and arms TX and RX from the handshake's secrets.
fn arm(fd: RawFd, secrets: &ExtractedSecrets) -> Result<(), KtlsArmError> {
    let gcm = |t: &ConnectionTrafficSecrets| {
        matches!(
            t,
            ConnectionTrafficSecrets::Aes128Gcm { .. } | ConnectionTrafficSecrets::Aes256Gcm { .. }
        )
    };
    if !gcm(&secrets.tx.1) || !gcm(&secrets.rx.1) {
        return Err(KtlsArmError::CipherNotSupported);
    }
    sys::install_tls_ulp(fd).map_err(|errno| KtlsArmError::UlpInstall { errno })?;
    for (direction, (sequence, traffic)) in [(sys::TLS_TX, &secrets.tx), (sys::TLS_RX, &secrets.rx)]
    {
        sys::set_gcm_crypto_info(fd, direction, *sequence, traffic).map_err(
            |fault| match fault {
                CryptoInfoFault::CipherNotSupported => KtlsArmError::CipherNotSupported,
                CryptoInfoFault::Errno(errno) => KtlsArmError::CryptoInfo { errno },
            },
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Earned-Trust probe
// ---------------------------------------------------------------------------

/// Server name of the probe's throwaway certificate (never on a real wire).
const PROBE_SERVER_NAME: &str = "relay-probe.overdrive.invalid";

/// The probe's relayed length: one octet past the pipe floor, so the splice
/// must cross a full pipe and a one-octet remainder.
const PROBE_RELAY_LEN: usize = PIPE_FLOOR + 1;

/// Octets the client sends right after the relayed span; the server leg must read
/// them first and intact (`relay_exact` "reads nothing beyond `len`",
/// feature-delta `GatewayRelay::relay_exact`). One line, so `read_through(LineEnd)`
/// returns exactly it.
const PROBE_TRAILER: &[u8] = b"relay-probe-trailer\r\n";

/// Octets the client sends after the trailer; the server relays them through the
/// pipe `relay_exact` left behind, so a stale octet in that pipe corrupts the echo.
const PROBE_TAIL: &[u8] = b"relay-probe-tail";

/// What the client expects echoed: the relayed span, then the tail.
fn probe_echo() -> Vec<u8> {
    let mut echo = probe_pattern();
    echo.extend_from_slice(PROBE_TAIL);
    echo
}

fn probe_io(kind: io::ErrorKind) -> RelayProbeError {
    RelayProbeError::Io { kind }
}

fn probe_pattern() -> Vec<u8> {
    (0..PROBE_RELAY_LEN).map(|i| (i % 251) as u8).collect()
}

/// An ephemeral self-signed certificate: server and client configs trusting it,
/// both TLS 1.3 with AES-128-GCM only, secret extraction on the server.
fn probe_tls() -> Result<(Arc<rustls::ServerConfig>, Arc<rustls::ClientConfig>), RelayProbeError> {
    let invalid = |_: &dyn std::fmt::Display| probe_io(io::ErrorKind::InvalidData);
    let params = rcgen::CertificateParams::new(vec![PROBE_SERVER_NAME.to_string()])
        .map_err(|e| invalid(&e))?;
    let key =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| invalid(&e))?;
    let cert = params.self_signed(&key).map_err(|e| invalid(&e))?;
    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::try_from(key.serialize_der()).map_err(|e| invalid(&e))?;

    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = vec![rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256];
    let provider = Arc::new(provider);

    let mut server = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| invalid(&e))?
        .with_no_client_auth()
        .with_single_cert(vec![cert_der.clone()], key_der)
        .map_err(|e| invalid(&e))?;
    server.enable_secret_extraction = true;
    server.send_tls13_tickets = 0;
    server.alpn_protocols = vec![b"http/1.1".to_vec()];

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).map_err(|e| invalid(&e))?;
    let mut client = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| invalid(&e))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((Arc::new(server), Arc::new(client)))
}

/// A connected loopback pair `(client, server)`; the listener is closed on return.
fn loopback_pair() -> io::Result<(std::net::TcpStream, std::net::TcpStream)> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let client = std::net::TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    Ok((client, server))
}

/// What the probe's client saw echoed: how many octets arrived, and how many
/// leading ones matched the expected echo.
struct Echoed {
    received: usize,
    matched: usize,
}

/// A TCP stream whose every operation is bounded by one absolute `end`.
struct DeadlineStream {
    stream: std::net::TcpStream,
    end: Instant,
}

impl DeadlineStream {
    fn remaining(&self) -> io::Result<Duration> {
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() { Err(io::ErrorKind::TimedOut.into()) } else { Ok(left) }
    }
}

impl io::Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(buf)
    }
}

impl io::Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// Shuts the client's socket down on drop, so a client still blocked in the
/// blocking pool when the probe returns or is dropped fails at once and releases
/// its descriptor.
struct ShutdownOnDrop(std::net::TcpStream);

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        // The peer may already be gone; shutdown only has to wake a blocked client.
        drop(self.0.shutdown(std::net::Shutdown::Both));
    }
}

/// The probe's userspace TLS client: handshakes, writes pattern, trailer and
/// tail, then reads the echo to the server's close_notify. Every socket
/// operation is bounded by `limit` from the call.
fn probe_client(
    stream: std::net::TcpStream,
    config: Arc<rustls::ClientConfig>,
    pattern: Vec<u8>,
    limit: Duration,
) -> io::Result<Echoed> {
    use std::io::{Read, Write};
    let mut stream = DeadlineStream { stream, end: Instant::now() + limit };
    let name = rustls::pki_types::ServerName::try_from(PROBE_SERVER_NAME)
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut conn = rustls::ClientConnection::new(config, name)
        .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let mut tls = rustls::Stream::new(&mut conn, &mut stream);
    tls.write_all(&pattern)?;
    tls.write_all(PROBE_TRAILER)?;
    tls.write_all(PROBE_TAIL)?;
    // End of the client's data: a server that consumed the trailer early sees
    // the close_notify instead of waiting out the deadline.
    tls.conn.send_close_notify();
    tls.flush()?;
    let expected = probe_echo();
    // One octet of headroom: an octet past the expected echo is an extra.
    let mut echoed = vec![0u8; expected.len() + 1];
    let mut received = 0;
    while received < echoed.len() {
        match tls.read(&mut echoed[received..]) {
            Ok(0) => break,
            Ok(n) => received += n,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        }
    }
    let matched = echoed[..received].iter().zip(&expected).take_while(|(a, b)| a == b).count();
    Ok(Echoed { received, matched })
}

fn probe_handshake_error(err: PublicHandshakeError) -> RelayProbeError {
    match err {
        PublicHandshakeError::Arm(kind) => RelayProbeError::Arm(kind),
        PublicHandshakeError::Timeout => RelayProbeError::Timeout,
        PublicHandshakeError::Io { kind } => probe_io(kind),
        PublicHandshakeError::Tls { .. }
        | PublicHandshakeError::AlpnMismatch
        | PublicHandshakeError::ServerNameMissing => probe_io(io::ErrorKind::InvalidData),
    }
}

fn probe_read_error(err: LegReadError) -> RelayProbeError {
    match err {
        LegReadError::Deadline => RelayProbeError::Timeout,
        LegReadError::Io { kind } => probe_io(kind),
        _ => probe_io(io::ErrorKind::InvalidData),
    }
}

fn probe_write_error(err: LegWriteError) -> RelayProbeError {
    match err {
        LegWriteError::Deadline => RelayProbeError::Timeout,
        LegWriteError::Io { kind } => probe_io(kind),
        _ => probe_io(io::ErrorKind::InvalidData),
    }
}

/// `before` octets of the echo precede the `span` this `relay_exact` call moves.
fn probe_relay_error(err: RelayError, before: usize, span: usize) -> RelayProbeError {
    match err {
        RelayError::Pipe(err) => RelayProbeError::Pipe(err),
        RelayError::Deadline | RelayError::NoProgress => RelayProbeError::Timeout,
        RelayError::SourceEndedMidBody { remaining } => RelayProbeError::SpliceExactness {
            moved: (before + span).saturating_sub(usize::try_from(remaining).unwrap_or(usize::MAX))
                as u64,
            expected: probe_echo().len() as u64,
        },
        RelayError::Read(err) => probe_read_error(err),
        RelayError::Write(err) => probe_write_error(err),
        RelayError::LimitExceeded { .. } => probe_io(io::ErrorKind::InvalidData),
    }
}

fn probe_join<T>(
    joined: Result<io::Result<T>, tokio::task::JoinError>,
) -> Result<T, RelayProbeError> {
    joined.map_err(|_| probe_io(io::ErrorKind::Other))?.map_err(|err| match err.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => RelayProbeError::Timeout,
        kind => probe_io(kind),
    })
}

impl HostGatewayRelay {
    /// What the relay contract needs of the node, proven on the node: a rustls
    /// handshake over a loopback pair that arms kernel TLS TX and RX, and
    /// `PIPE_FLOOR + 1` octets spliced through the relay's own pipe (whose floor
    /// `relay_exact` proves on creation) and returned pattern-exactly, with
    /// nothing read beyond them and the pipe left empty. The server leg is closed
    /// before this returns. The client socket is shut down on every path (return
    /// or drop), which wakes a client still blocked in the blocking pool; that
    /// client then fails at once and closes its own descriptor, so on a timeout
    /// the client's descriptor is released just after this returns, never later
    /// than the deadline.
    async fn run_probe(&self, deadline: Instant) -> Result<(), RelayProbeError> {
        let (server_tls, client_tls) = probe_tls()?;
        let (client, server) = probe_join(tokio::task::spawn_blocking(loopback_pair).await)?;
        let _wake_client = ShutdownOnDrop(client.try_clone().map_err(|e| probe_io(e.kind()))?);
        let pattern = probe_pattern();
        let limit = deadline.saturating_duration_since(self.clock.now());
        let client_task = tokio::task::spawn_blocking({
            let pattern = pattern.clone();
            move || probe_client(client, client_tls, pattern, limit)
        });
        let outcome = self.relay_probe_leg(OwnedFd::from(server), server_tls, deadline).await;
        // A failed leg is closed, so the client is failing or finished: do not wait on it.
        outcome?;
        let echoed = probe_join(client_task.await)?;
        let expected = probe_echo().len();
        if echoed.received == expected && echoed.matched == expected {
            return Ok(());
        }
        // Short or corrupt: the octets that were right; extra: everything received.
        let moved = if echoed.matched < expected { echoed.matched } else { echoed.received };
        Err(RelayProbeError::SpliceExactness { moved: moved as u64, expected: expected as u64 })
    }

    /// Accepts the probe's server leg and echoes the relayed span and the tail
    /// through the relay pipe, reading the trailer between them with
    /// `read_through`; the leg is closed on return.
    async fn relay_probe_leg(
        &self,
        server: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<(), RelayProbeError> {
        let accepted =
            self.accept_public(server, tls, deadline).await.map_err(probe_handshake_error)?;
        let (mut read, mut write) = accepted.leg.into_halves();
        let deadlines = || RelayDeadlines {
            absolute: deadline,
            no_progress: deadline.saturating_duration_since(self.clock.now()),
        };
        let nonzero = |len: usize| {
            NonZeroU64::new(len as u64)
                .unwrap_or_else(|| unreachable!("the probe lengths are nonzero constants"))
        };
        self.relay_exact(&mut read, &mut write, nonzero(PROBE_RELAY_LEN), deadlines())
            .await
            .map_err(|err| probe_relay_error(err, 0, PROBE_RELAY_LEN))?;
        self.expect_trailer(&mut read, deadline).await?;
        self.relay_exact(&mut read, &mut write, nonzero(PROBE_TAIL.len()), deadlines())
            .await
            .map_err(|err| probe_relay_error(err, PROBE_RELAY_LEN, PROBE_TAIL.len()))?;
        self.close_write(&mut write, deadline).await.map_err(probe_write_error)
    }

    /// `relay_exact` "reads nothing beyond `len`": the next octets on the leg are
    /// the client's trailer, first and intact. A trailer that never arrives, or
    /// arrives short or altered, means `relay_exact` moved octets past `len`.
    async fn expect_trailer(
        &self,
        read: &mut KtlsReadHalf,
        deadline: Instant,
    ) -> Result<(), RelayProbeError> {
        let violation = |got: usize| RelayProbeError::SpliceExactness {
            moved: (PROBE_RELAY_LEN + got) as u64,
            expected: (PROBE_RELAY_LEN + PROBE_TRAILER.len()) as u64,
        };
        // Room for the trailer and then some: an extra octet read is a violation too.
        let limit = NonZeroUsize::new(4 * PROBE_TRAILER.len())
            .unwrap_or_else(|| unreachable!("the probe trailer is a nonempty constant"));
        let deadlines = ReadDeadlines {
            first_octet: deadline,
            complete_within: None,
            complete_by: Some(deadline),
        };
        match self.read_through(read, HeadTerminator::LineEnd, limit, deadlines).await {
            Ok(bytes) if bytes.as_slice() == PROBE_TRAILER => Ok(()),
            Ok(bytes) => Err(violation(bytes.as_slice().len())),
            Err(LegReadError::CleanEof | LegReadError::Truncated) => Err(violation(0)),
            Err(err) => Err(probe_read_error(err)),
        }
    }
}

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// Host kernel-TLS relay. One instance serves both gateway legs: the public
/// legs it accepts and the authenticated upstream legs the gateway-client mTLS
/// adapter arms through it.
pub struct HostGatewayRelay {
    /// The injected clock all `ReadDeadlines` / `RelayDeadlines` are evaluated on.
    clock: Arc<dyn Clock>,
}

impl HostGatewayRelay {
    /// Infallible and kernel-silent: it opens no socket and touches no ULP, so
    /// construction claims no capability. Every kernel fact is proven by
    /// [`GatewayRelay::probe`] (Earned Trust), never assumed at construction.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }
}

#[async_trait]
impl GatewayRelay for HostGatewayRelay {
    async fn probe(&self, deadline: Instant) -> Result<(), RelayProbeError> {
        bounded_opt(&*self.clock, Some(deadline), self.run_probe(deadline))
            .await
            .map_err(|()| RelayProbeError::Timeout)?
    }

    async fn accept_public(
        &self,
        accepted: OwnedFd,
        tls: Arc<rustls::ServerConfig>,
        deadline: Instant,
    ) -> Result<AcceptedPublicLeg, PublicHandshakeError> {
        sys::set_nonblocking(accepted.as_raw_fd()).map_err(|err| handshake_io(&err))?;
        let raw = accepted.as_raw_fd();
        let fd = AsyncFd::new(accepted).map_err(|err| handshake_io(&err))?;
        let shaken = bounded_opt(&*self.clock, Some(deadline), server_handshake(&fd, tls))
            .await
            .map_err(|()| PublicHandshakeError::Timeout)??;
        arm(raw, &shaken.secrets).map_err(PublicHandshakeError::Arm)?;
        let core = Arc::new(LegCore {
            fd,
            rekey: Mutex::new(shaken.rekey),
            tx: tokio::sync::Mutex::new(()),
            answers_owed: Mutex::new(0),
        });
        let leg = KtlsLeg::from_armed(
            Box::new(ReadHalf::new(Arc::clone(&core))),
            Box::new(WriteHalf { leg: core }),
        );
        Ok(AcceptedPublicLeg { leg, sni: shaken.sni })
    }

    async fn read_through(
        &self,
        from: &mut KtlsReadHalf,
        terminator: HeadTerminator,
        limit: NonZeroUsize,
        deadlines: ReadDeadlines,
    ) -> Result<ThroughBytes, LegReadError> {
        let half = from.backend_mut::<ReadHalf>();
        if terminator == HeadTerminator::HeaderBlockEnd {
            half.pipe = None;
        }
        half.read_head(&*self.clock, terminator, limit.get(), deadlines).await
    }

    async fn write_all(
        &self,
        to: &mut KtlsWriteHalf,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        let leg = &to.backend_mut::<WriteHalf>().leg;
        let write = async {
            let transmit = leg.tx.lock().await;
            leg.answer_owed_locked(&transmit).await.map_err(LegWriteError::KeyUpdate)?;
            let mut sent = 0;
            while sent < bytes.len() {
                sent += leg
                    .write_io(|fd| sys::send_data(fd, &bytes[sent..]))
                    .await
                    .map_err(|err| write_err(&err))?;
            }
            Ok(())
        };
        match bounded_opt(&*self.clock, Some(deadline), write).await {
            Ok(result) => result,
            Err(()) => Err(LegWriteError::Deadline),
        }
    }

    async fn relay_exact(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        len: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), RelayError> {
        let src = from.backend_mut::<ReadHalf>();
        let dst = to.backend_mut::<WriteHalf>();
        let mut pace = Pace::new(&*self.clock, deadlines);
        let pipe = src.take_or_create_pipe().map_err(RelayError::Pipe)?;
        let mut remaining = len.get();
        while let Some(owed) = NonZeroU64::new(remaining) {
            let want =
                usize::try_from(splice_in_len(owed, pipe.capacity).get()).unwrap_or(usize::MAX);
            match src.splice_in(pipe.write.as_raw_fd(), want, &pace).await {
                Ok(Spliced::Moved(moved)) => {
                    pace.note();
                    drain_pipe(dst, pipe.read.as_raw_fd(), moved, &mut pace).await?;
                    remaining = remaining.saturating_sub(u64::try_from(moved).unwrap_or(u64::MAX));
                }
                Ok(Spliced::CloseNotify | Spliced::Eof) => {
                    return Err(RelayError::SourceEndedMidBody { remaining });
                }
                Err(fail) => return Err(splice_fail(fail)),
            }
        }
        src.pipe = Some(pipe);
        Ok(())
    }

    async fn relay_to_end(
        &self,
        from: &mut KtlsReadHalf,
        to: &mut KtlsWriteHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<u64, RelayError> {
        let src = from.backend_mut::<ReadHalf>();
        let dst = to.backend_mut::<WriteHalf>();
        let mut pace = Pace::new(&*self.clock, deadlines);
        let pipe = src.take_or_create_pipe().map_err(RelayError::Pipe)?;
        let mut moved_total = 0u64;
        loop {
            pace.bounded(src.leg.settle())
                .await
                .map_err(relay_stall)?
                .map_err(|err| RelayError::Read(LegReadError::KeyUpdate(err)))?;
            let Some(owed) = NonZeroU64::new(max.get() - moved_total) else {
                // At the ceiling: one more octet is a breach, a close_notify the end.
                let mut one = [0u8; 1];
                let seen = pace
                    .bounded(src.leg.read_io(|fd| sys::recv_record(fd, &mut one, true)))
                    .await
                    .map_err(relay_stall)?
                    .map_err(|err| RelayError::Read(read_io_err(&err)))?;
                match seen {
                    Received::Data(0) => return Err(RelayError::Read(LegReadError::Truncated)),
                    Received::Data(_) => return Err(RelayError::LimitExceeded { max: max.get() }),
                    Received::Control { .. } => {
                        match src.next_control().await.map_err(RelayError::Read)? {
                            Control::Handled => continue,
                            Control::CloseNotify => {
                                src.pipe = Some(pipe);
                                return Ok(moved_total);
                            }
                        }
                    }
                }
            };
            let want =
                usize::try_from(splice_in_len(owed, pipe.capacity).get()).unwrap_or(usize::MAX);
            match src.splice_in(pipe.write.as_raw_fd(), want, &pace).await {
                Ok(Spliced::Moved(moved)) => {
                    pace.note();
                    drain_pipe(dst, pipe.read.as_raw_fd(), moved, &mut pace).await?;
                    moved_total += u64::try_from(moved).unwrap_or(u64::MAX);
                }
                Ok(Spliced::CloseNotify) => {
                    src.pipe = Some(pipe);
                    return Ok(moved_total);
                }
                Ok(Spliced::Eof) => return Err(RelayError::Read(LegReadError::Truncated)),
                Err(fail) => return Err(splice_fail(fail)),
            }
        }
    }

    async fn close_write(
        &self,
        to: &mut KtlsWriteHalf,
        deadline: Instant,
    ) -> Result<(), LegWriteError> {
        let leg = &to.backend_mut::<WriteHalf>().leg;
        let close = async {
            let transmit = leg.tx.lock().await;
            leg.answer_owed_locked(&transmit).await.map_err(LegWriteError::KeyUpdate)?;
            let close_notify = [1u8, 0];
            leg.write_io(|fd| sys::send_record(fd, sys::RECORD_ALERT, &close_notify))
                .await
                .map_err(|err| write_err(&err))?;
            sys::shutdown_write(leg.raw()).map_err(|err| write_err(&err))
        };
        match bounded_opt(&*self.clock, Some(deadline), close).await {
            Ok(result) => result,
            Err(()) => Err(LegWriteError::Deadline),
        }
    }

    async fn discard_remaining(
        &self,
        from: &mut KtlsReadHalf,
        max: NonZeroU64,
        deadlines: RelayDeadlines,
    ) -> Result<(), LegReadError> {
        let src = from.backend_mut::<ReadHalf>();
        let mut pace = Pace::new(&*self.clock, deadlines);
        let mut dropped = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        while !src.ended && dropped < max.get() {
            pace.bounded(src.leg.settle())
                .await
                .map_err(|_| LegReadError::Deadline)?
                .map_err(LegReadError::KeyUpdate)?;
            // `max` bounds DATA octets only: peek at most `room` of them; a control
            // record is then taken whole by `next_control`, never truncated.
            let room = usize::try_from(max.get() - dropped).unwrap_or(usize::MAX).min(buf.len());
            let peeked = pace
                .bounded(src.leg.read_io(|fd| sys::recv_record(fd, &mut buf[..room], true)))
                .await
                .map_err(|_| LegReadError::Deadline)?
                .map_err(|err| read_io_err(&err))?;
            match peeked {
                Received::Data(0) => return Ok(()),
                Received::Data(n) => {
                    src.consume(n)?;
                    dropped += u64::try_from(n).unwrap_or(u64::MAX);
                    pace.note();
                }
                Received::Control { .. } => {
                    src.next_control().await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod pipe_floor_laws {
    #![allow(
        clippy::doc_markdown,
        clippy::expect_used,
        reason = "CONTRACT_SHAPE is required metadata; expect names the precondition of a fixture"
    )]

    use proptest::prelude::*;

    use super::*;

    /// S-PIG-86 (the constant): the pipe floor is 65 536 octets, the capacity
    /// the relay's record-boundary and backpressure rules are sized for
    /// (ADR-0246). The DESIGN pins the value, so this row is expected-GREEN on
    /// the scaffold.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn the_pipe_floor_is_sixty_five_thousand_five_hundred_and_thirty_six_octets() {
        assert_eq!(PIPE_FLOOR, 65_536);
    }

    /// S-PIG-86 (the read-back): a capacity read back at the floor or above is a
    /// `PipeCapacity` holding exactly that value; one octet below the floor, and
    /// every smaller value, is `CapacityBelowFloor` carrying what was read.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_read_back_below_the_floor_is_refused_with_what_was_read() {
        for at_or_above in [PIPE_FLOOR, PIPE_FLOOR + 1, 2 * PIPE_FLOOR, 1 << 20] {
            let capacity =
                PipeCapacity::new(at_or_above).expect("a capacity at or above the floor");
            assert_eq!(capacity.get(), at_or_above);
        }
        for below in [0, 1, 4096, PIPE_FLOOR - 1] {
            assert_eq!(
                PipeCapacity::new(below).err(),
                Some(RelayPipeError::CapacityBelowFloor { got: below }),
                "{below} is below the floor",
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-86 (the splice-in length): for every octet count owed and every
        /// capacity at or above the floor, the next splice into an empty pipe is
        /// exactly `min(owed, capacity)`, never above either and never zero.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn the_splice_in_length_is_the_smaller_of_what_is_owed_and_the_capacity(
            owed in 1u64..=u64::MAX,
            capacity in PIPE_FLOOR..=(64usize << 20),
        ) {
            let owed = NonZeroU64::new(owed).expect("owed is nonzero");
            let capacity = PipeCapacity::new(capacity).expect("a capacity at or above the floor");
            let held = capacity.get() as u64;
            let length = splice_in_len(owed, capacity).get();
            prop_assert_eq!(length, owed.get().min(held));
            prop_assert!(length <= owed.get());
            prop_assert!(length <= held);
        }

        /// S-PIG-86 (the boundary around the floor): a body one octet below, at
        /// and one octet above the pipe capacity splices in whole, whole, and
        /// capacity octets respectively (the remainder is the next splice).
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
        fn bodies_around_the_capacity_splice_in_whole_or_capacity_octets(
            capacity in PIPE_FLOOR..=(1usize << 20),
        ) {
            let held = capacity as u64;
            for (owed, expected) in [(held - 1, held - 1), (held, held), (held + 1, held)] {
                let owed = NonZeroU64::new(owed).expect("owed is nonzero");
                let pipe = PipeCapacity::new(capacity).expect("a capacity at or above the floor");
                prop_assert_eq!(splice_in_len(owed, pipe).get(), expected);
            }
        }
    }
}
