//! INB-1 — the guest stream dial vocabulary in `overdrive-core` (GH #295,
//! vsock attachment replacement).
//!
//! Contract: `docs/feature/netns-density-295/feature-delta.md` § "[REF]
//! Driving port — guest stream dial `GuestStreamDial`" (INB-1, ADR-0173,
//! ADR-0174; `DialPurpose` is PROBE-NOSAMPLE, ADR-0187). The dial's own
//! behaviour — admission order, pairing, deadlines, aborts, capacity — is the
//! owner's and is specified against the production owner in
//! `overdrive-control-plane`; this file pins the core vocabulary every
//! implementation and every consumer (leg-S, the VM probe runner) shares:
//!
//! - `GuestStream` is a move-only holder over a `GuestStreamIo`: it reports
//!   the target it was dialled for and delegates `read`, `write`,
//!   `shutdown_write` and `end` unchanged — the I/O contract (every end
//!   resolved from the owner's forwarder watch, EVMOD-STREAM-END; an owner
//!   abort answered `ConnectionAborted` without waiting on the socket,
//!   EVMOD-STREAM-ABORT) lives in the `GuestStreamIo` the owner supplies, and
//!   is specified against the production owner;
//! - `GuestStreamEnd`, `DialPurpose` and `GuestDialError` carry distinct
//!   labels (the `guest_dial.failed { error }` telemetry), and every
//!   `GuestDialError` variant renders its pinned operator text (C6b).
//!
//! Driving port: the public `overdrive_core::guest_transport::dial` items
//! (layer 1). The `GuestStreamIo` below is test-local support: a scripted
//! in-memory pipe that records what the stream forwarded to it.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::doc_markdown,
    reason = "test bodies unwrap fixtures they construct; the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
)]

use std::collections::{BTreeSet, VecDeque};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial, GuestStreamEnd, GuestStreamIo,
};
use overdrive_core::guest_transport::wire::KernelErrno;
use parking_lot::Mutex;

// ---------------------------------------------------------------------------
// Test-local GuestStreamIo: a scripted pipe that records every forwarded call
// ---------------------------------------------------------------------------

/// What the stream forwarded to its I/O, observed after the fact.
#[derive(Debug, Default)]
struct PipeLog {
    /// Guest application bytes still to be read, in order; `None` marks the
    /// guest direction's end.
    to_read: VecDeque<Option<Vec<u8>>>,
    /// Bytes the stream wrote toward the guest, in order.
    written: Vec<u8>,
    /// How many times the stream asked for a write-side shutdown.
    shutdowns: u32,
    /// The guest direction's classified end, once ended.
    end: Option<GuestStreamEnd>,
    /// The owner ended the stream (EVMOD-STREAM-ABORT).
    owner_aborted: bool,
}

#[derive(Debug, Clone, Default)]
struct ScriptedPipe(Arc<Mutex<PipeLog>>);

#[async_trait::async_trait]
impl GuestStreamIo for ScriptedPipe {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut log = self.0.lock();
        if log.owner_aborted {
            return Err(ErrorKind::ConnectionAborted.into());
        }
        let next = log.to_read.pop_front();
        let end = log.end;
        if let Some(Some(chunk)) = next {
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            if n < chunk.len() {
                log.to_read.push_front(Some(chunk[n..].to_vec()));
            }
            drop(log);
            return Ok(n);
        }
        drop(log);
        match end {
            Some(GuestStreamEnd::Abort) => Err(ErrorKind::ConnectionReset.into()),
            Some(GuestStreamEnd::HalfClose) | None => Ok(0),
        }
    }

    async fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut log = self.0.lock();
        if log.owner_aborted {
            return Err(ErrorKind::ConnectionAborted.into());
        }
        if log.shutdowns > 0 {
            return Err(ErrorKind::BrokenPipe.into());
        }
        log.written.extend_from_slice(buf);
        drop(log);
        Ok(buf.len())
    }

    async fn shutdown_write(&mut self) -> std::io::Result<()> {
        let mut log = self.0.lock();
        if log.owner_aborted {
            return Err(ErrorKind::ConnectionAborted.into());
        }
        log.shutdowns += 1;
        drop(log);
        Ok(())
    }

    fn end(&self) -> Option<GuestStreamEnd> {
        self.0.lock().end
    }
}

const fn target() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 7), 8080)
}

/// A stream over a pipe scripted with `chunks` then the guest's `end`.
fn stream_over(chunks: &[&[u8]], end: Option<GuestStreamEnd>) -> (GuestStream, ScriptedPipe) {
    let pipe = ScriptedPipe::default();
    {
        let mut log = pipe.0.lock();
        log.to_read = chunks.iter().map(|chunk| Some(chunk.to_vec())).collect();
        if end.is_some() {
            log.to_read.push_back(None);
        }
        log.end = end;
    }
    (GuestStream::new(target(), Box::new(pipe.clone())), pipe)
}

const ALL_DIAL_ERRORS: [GuestDialError; 8] = [
    GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED },
    GuestDialError::TimedOut { errno: KernelErrno::ETIMEDOUT },
    GuestDialError::PairingDeadline,
    GuestDialError::NotServing,
    GuestDialError::NoSession,
    GuestDialError::Quiesced,
    GuestDialError::Unreachable { errno: KernelErrno::ENODEV },
    GuestDialError::Capacity,
];

/// The port is a required constructor argument of leg-S and the VM probe
/// runner: it must be usable as a shared trait object.
const fn assert_dial_is_a_shareable_trait_object(_: Option<&Arc<dyn GuestStreamDial>>) {}
const _: () = assert_dial_is_a_shareable_trait_object(None);

// ===========================================================================
// GuestStream — the holder the dial returns
// ===========================================================================

/// The stream reports the address `dial` was called with.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn a_stream_reports_the_target_it_was_dialled_for() {
    let (stream, _pipe) = stream_over(&[], None);
    assert_eq!(stream.target(), target());
}

/// Guest application bytes arrive through the stream in order, and the
/// half-closed guest direction reads `Ok(0)` with `end` reporting
/// `HalfClose` (R2-END).
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 2"]
async fn a_stream_delivers_guest_bytes_in_order_then_the_half_close() {
    let (mut stream, pipe) =
        stream_over(&[&b"HTTP/1.1 "[..], &b"200 OK"[..]], Some(GuestStreamEnd::HalfClose));
    let mut received = Vec::new();
    let mut buf = [0u8; 4];
    loop {
        let n = stream.read(&mut buf).await.expect("a half-closed direction reads Ok");
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }
    assert_eq!(received, b"HTTP/1.1 200 OK");
    assert_eq!(stream.end(), Some(GuestStreamEnd::HalfClose));
    assert!(pipe.0.lock().to_read.is_empty(), "every scripted byte was consumed");
}

/// A guest abort reaches the reader as `ConnectionReset`, and `end` reports
/// `Abort`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 2"]
async fn a_stream_reports_the_guest_abort() {
    let (mut stream, _pipe) = stream_over(&[], Some(GuestStreamEnd::Abort));
    let mut buf = [0u8; 8];
    let err = stream.read(&mut buf).await.expect_err("an aborted direction errs");
    assert_eq!(err.kind(), ErrorKind::ConnectionReset);
    assert_eq!(stream.end(), Some(GuestStreamEnd::Abort));
}

/// Writes reach the I/O unchanged and in order; `shutdown_write` is forwarded
/// once per call, and a write after it is refused `BrokenPipe` with nothing
/// forwarded. The guest direction stays open (`end` is `None`) throughout.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 2"]
async fn a_stream_forwards_writes_and_the_write_side_shutdown() {
    let (mut stream, pipe) = stream_over(&[], None);
    assert_eq!(stream.write(b"GET /healthz ").await.expect("open direction"), 13);
    assert_eq!(stream.write(b"HTTP/1.1\r\n\r\n").await.expect("open direction"), 12);
    assert_eq!(stream.end(), None, "the guest direction has not ended");

    let before = pipe.0.lock().written.clone();
    stream.shutdown_write().await.expect("the first shutdown_write succeeds");
    let err = stream.write(b"late").await.expect_err("a write after shutdown_write errs");
    assert_eq!(err.kind(), ErrorKind::BrokenPipe);

    let log = pipe.0.lock();
    assert_eq!(before, b"GET /healthz HTTP/1.1\r\n\r\n");
    assert_eq!(log.written, before, "nothing forwarded after the shutdown");
    assert_eq!(log.shutdowns, 1);
    drop(log);
    assert_eq!(stream.end(), None);
}

/// EVMOD-STREAM-ABORT: an owner abort the I/O reports reaches the holder
/// unchanged on every call — `read`, `write` and `shutdown_write` each return
/// `ConnectionAborted`, nothing is forwarded, and `end` reports `Abort`.
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
#[ignore = "pending DELIVER step 2"]
async fn an_owner_abort_reaches_every_stream_call_as_connection_aborted() {
    let (mut stream, pipe) = stream_over(&[&b"unread"[..]], None);
    {
        let mut log = pipe.0.lock();
        log.owner_aborted = true;
        log.end = Some(GuestStreamEnd::Abort);
    }
    let mut buf = [0u8; 8];
    assert_eq!(
        stream.read(&mut buf).await.expect_err("aborted").kind(),
        ErrorKind::ConnectionAborted
    );
    assert_eq!(
        stream.write(b"late").await.expect_err("aborted").kind(),
        ErrorKind::ConnectionAborted
    );
    assert_eq!(
        stream.shutdown_write().await.expect_err("aborted").kind(),
        ErrorKind::ConnectionAborted
    );
    assert_eq!(stream.end(), Some(GuestStreamEnd::Abort));
    let (written, shutdowns) = {
        let log = pipe.0.lock();
        (log.written.clone(), log.shutdowns)
    };
    assert!(written.is_empty(), "nothing forwarded after the owner's abort");
    assert_eq!(shutdowns, 0);
}

// ===========================================================================
// Labels and operator texts (C6b)
// ===========================================================================

/// Every purpose, end class and dial error carries a distinct, non-empty
/// label; an errno-carrying error's label names the class, not the errno.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 2"]
fn purposes_end_classes_and_dial_errors_have_distinct_labels() {
    let purposes: BTreeSet<&str> =
        [DialPurpose::Relay, DialPurpose::Probe].map(DialPurpose::as_str).into_iter().collect();
    assert_eq!(purposes.len(), 2);
    assert!(purposes.iter().all(|label| !label.is_empty()));

    let ends: BTreeSet<&str> = [GuestStreamEnd::HalfClose, GuestStreamEnd::Abort]
        .map(GuestStreamEnd::as_str)
        .into_iter()
        .collect();
    assert_eq!(ends.len(), 2);
    assert!(ends.iter().all(|label| !label.is_empty()));

    let errors: BTreeSet<&str> = ALL_DIAL_ERRORS.iter().map(GuestDialError::as_str).collect();
    assert_eq!(errors.len(), ALL_DIAL_ERRORS.len());
    assert!(errors.iter().all(|label| !label.is_empty()));
    assert_eq!(
        GuestDialError::Unreachable { errno: KernelErrno::ENODEV }.as_str(),
        GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }.as_str(),
    );
}

/// The variants that carry no errno render their pinned operator text.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn dial_errors_without_an_errno_render_their_pinned_text() {
    let cases = [
        (
            GuestDialError::PairingDeadline,
            "no pairing reply from the guest within the pairing deadline",
        ),
        (GuestDialError::NotServing, "no Active VM allocation serves this address and port"),
        (GuestDialError::NoSession, "the VM's control session is not live"),
        (GuestDialError::Quiesced, "node forwarding is quiesced"),
        (GuestDialError::Capacity, "guest flow capacity exhausted"),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text, "{error:?}");
    }
}

/// The errno-carrying variants render their pinned text with the errno in
/// decimal (`KernelErrno`'s `Display`).
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 2"]
fn dial_errors_with_an_errno_render_their_pinned_text_and_the_errno() {
    assert_eq!(
        GuestDialError::Refused { errno: KernelErrno::ECONNREFUSED }.to_string(),
        "the guest refused the connection: errno 111"
    );
    assert_eq!(
        GuestDialError::TimedOut { errno: KernelErrno::ETIMEDOUT }.to_string(),
        "the guest's connect to the application timed out: errno 110"
    );
    assert_eq!(
        GuestDialError::Unreachable { errno: KernelErrno::ENODEV }.to_string(),
        "the VM is unreachable over vsock: errno 19"
    );
    assert_eq!(
        GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }.to_string(),
        "the VM is unreachable over vsock: errno 104"
    );
}
