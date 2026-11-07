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

    /// Answers a peer KeyUpdate: re-arms RX, and when the peer asked, sends our
    /// own update record under the old key and re-arms TX. The gateway never
    /// originates one (ADR-0250).
    async fn answer_key_update(&self, requested: bool) -> Result<(), KeyUpdateError> {
        let rx = self.rekey.lock().next_rx();
        let (sequence, secrets) = rx.map_err(|_| KeyUpdateError::RustlsRefused)?;
        sys::set_gcm_crypto_info(self.raw(), sys::TLS_RX, sequence, &secrets)
            .map_err(|fault| KeyUpdateError::RearmRx { errno: errno_of(fault) })?;
        if !requested {
            return Ok(());
        }
        let _transmit = self.tx.lock().await;
        let update = [24u8, 0, 0, 1, 0];
        self.write_io(|fd| sys::send_record(fd, sys::RECORD_HANDSHAKE, &update))
            .await
            .map_err(|err| KeyUpdateError::Io { kind: err.kind() })?;
        let tx = self.rekey.lock().next_tx();
        let (sequence, secrets) = tx.map_err(|_| KeyUpdateError::RustlsRefused)?;
        sys::set_gcm_crypto_info(self.raw(), sys::TLS_TX, sequence, &secrets)
            .map_err(|fault| KeyUpdateError::RearmTx { errno: errno_of(fault) })
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

    /// Consumes the pending control record and returns its type and payload.
    async fn consume_control(&self) -> Result<(u8, Vec<u8>), LegReadError> {
        let mut buf = vec![0u8; sys::CONTROL_RECORD_BUFFER];
        let received = self
            .leg
            .read_io(|fd| sys::recv_record(fd, &mut buf, false))
            .await
            .map_err(|err| read_io_err(&err))?;
        match received {
            Received::Control { record, len } => {
                buf.truncate(len);
                Ok((record, buf))
            }
            Received::Data(_) => Err(LegReadError::Io { kind: io::ErrorKind::InvalidData }),
        }
    }

    /// Handles one control record (Contract of the legs, item 4).
    async fn control(&mut self, record: u8, payload: &[u8]) -> Result<Control, LegReadError> {
        match (record, payload) {
            (sys::RECORD_ALERT, [1, 0, ..]) => {
                self.ended = true;
                Ok(Control::CloseNotify)
            }
            (sys::RECORD_ALERT, [_, description, ..]) => {
                Err(LegReadError::Alert { description: *description })
            }
            (sys::RECORD_HANDSHAKE, [24, 0, 0, 1, flag]) if *flag <= 1 => {
                self.leg.answer_key_update(*flag == 1).await.map_err(LegReadError::KeyUpdate)?;
                Ok(Control::Handled)
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
                    match self.control_pending().await.map_err(SpliceFail::Read)? {
                        Control::Handled => {}
                        Control::CloseNotify => return Ok(Spliced::CloseNotify),
                    }
                }
                Err(err) => return Err(SpliceFail::Read(read_io_err(&err))),
            }
        }
    }

    /// A control record is next in the socket: consume and handle it.
    async fn control_pending(&mut self) -> Result<Control, LegReadError> {
        let mut one = [0u8; 1];
        match sys::recv_record(self.leg.raw(), &mut one, true) {
            Ok(Received::Control { .. }) => {}
            Ok(Received::Data(_)) => {
                return Err(LegReadError::Io { kind: io::ErrorKind::InvalidInput });
            }
            Err(err) => return Err(read_io_err(&err)),
        }
        let (record, payload) = self.consume_control().await?;
        self.control(record, &payload).await
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
                    let (record, payload) = self.consume_control().await?;
                    self.control(record, &payload).await?;
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
    let _transmit = pace.bounded(dst.leg.tx.lock()).await.map_err(relay_stall)?;
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
    #[allow(clippy::todo, reason = "the Earned-Trust probe is a later roadmap step")]
    async fn probe(&self, _deadline: Instant) -> Result<(), RelayProbeError> {
        todo!("SCAFFOLD: HostGatewayRelay::probe")
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
            let _transmit = leg.tx.lock().await;
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
                        match src.control_pending().await.map_err(RelayError::Read)? {
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
            let _transmit = leg.tx.lock().await;
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
            let room = usize::try_from(max.get() - dropped).unwrap_or(usize::MAX).min(buf.len());
            let received = pace
                .bounded(src.leg.read_io(|fd| sys::recv_record(fd, &mut buf[..room], false)))
                .await
                .map_err(|_| LegReadError::Deadline)?
                .map_err(|err| read_io_err(&err))?;
            match received {
                Received::Data(0) => return Ok(()),
                Received::Data(n) => {
                    dropped += u64::try_from(n).unwrap_or(u64::MAX);
                    pace.note();
                }
                Received::Control { record, len } => {
                    let payload = buf[..len].to_vec();
                    src.control(record, &payload).await?;
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
