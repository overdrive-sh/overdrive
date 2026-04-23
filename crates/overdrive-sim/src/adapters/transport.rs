//! `SimTransport` — in-process [`Transport`] implementation for DST.
//!
//! The real transport opens TCP / QUIC sockets. The sim transport
//! routes every "packet" through in-memory channels with an
//! injectable partition matrix so that the DST harness can model
//! "partition A from B", "repair", "drop this datagram" without any
//! kernel involvement.
//!
//! For Phase 1 we implement the *datagram* surface end-to-end — that
//! is what §6.1's acceptance test exercises and what Corrosion gossip
//! rides on. The trait's `connect` surface is kept as an
//! `unimplemented!` stub because no Phase-1 call site uses it;
//! stream-based transport arrives in Phase 2 together with the first
//! real node-agent / gateway code.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use overdrive_core::traits::transport::{Connection, Transport, TransportError};

/// In-process transport with injectable partition matrix.
///
/// `SimTransport` is cheap to clone — the router state lives behind an
/// [`Arc`] so tests that hand the transport to several tasks all see
/// the same partition set and the same inbox routing.
#[derive(Clone)]
pub struct SimTransport {
    router: Arc<RouterState>,
}

/// A datagram as delivered to a bound inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Datagram {
    /// Source address the datagram was sent from.
    pub from: SocketAddr,
    /// Payload bytes. The sim never mutates them.
    pub payload: Bytes,
}

/// Receiver handle returned by [`SimTransport::bind_inbox`]. Calling
/// [`SimInbox::recv`] returns the next datagram delivered to the bound
/// address, or `None` once every sender has been dropped.
pub struct SimInbox {
    rx: UnboundedReceiver<Datagram>,
}

impl SimInbox {
    /// Await the next datagram. Returns `None` once every sender has
    /// been dropped (i.e. the transport itself has been dropped).
    pub async fn recv(&mut self) -> Option<Datagram> {
        self.rx.recv().await
    }
}

struct RouterState {
    /// Registered inboxes — sender side of the mpsc channel keyed by
    /// the bound address.
    inboxes: Mutex<HashMap<SocketAddr, UnboundedSender<Datagram>>>,
    /// Unordered pairs of partitioned addresses. Stored canonicalised
    /// so that `partition(a, b)` and `partition(b, a)` share one entry.
    partitions: Mutex<HashSet<PartitionPair>>,
    /// Queue of datagrams that were sent while partitioned; they are
    /// retained in case a caller chooses to repair and re-send. Phase
    /// 1 does not replay this queue automatically — partitioned sends
    /// are dropped at delivery time; the queue exists as a debugging
    /// aid and keeps the shape ready for the auto-replay variant
    /// Phase 2 may want.
    #[allow(dead_code)]
    blackholed: Mutex<VecDeque<(SocketAddr, SocketAddr, Bytes)>>,
}

impl RouterState {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inboxes: Mutex::new(HashMap::new()),
            partitions: Mutex::new(HashSet::new()),
            blackholed: Mutex::new(VecDeque::new()),
        })
    }
}

/// Unordered pair of [`SocketAddr`]s. Two `PartitionPair`s are equal
/// iff they contain the same addresses regardless of argument order.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PartitionPair(SocketAddr, SocketAddr);

impl PartitionPair {
    fn new(a: SocketAddr, b: SocketAddr) -> Self {
        if a <= b { Self(a, b) } else { Self(b, a) }
    }
}

impl SimTransport {
    /// Construct a fresh sim transport.
    #[must_use]
    pub fn new() -> Self {
        Self { router: RouterState::new() }
    }

    /// Bind a datagram inbox at `addr`. Subsequent datagrams sent to
    /// `addr` are delivered to the returned [`SimInbox`], subject to
    /// the partition matrix. Re-binding the same `addr` replaces the
    /// prior inbox — there is only ever one sink per address.
    ///
    /// The `async` signature is preserved even though the sim path is
    /// synchronous — Phase-2 stream transport will need the await
    /// point, and keeping the `.await` in test code now means it does
    /// not grow a compile break later.
    #[allow(clippy::unused_async)]
    pub async fn bind_inbox(&self, addr: SocketAddr) -> Result<SimInbox, TransportError> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.router.inboxes.lock().insert(addr, tx);
        Ok(SimInbox { rx })
    }

    /// Send a datagram from `source` to `dest`. The sim never needs a
    /// listening socket on the source side — sources are identified by
    /// address only so that partition rules can be checked without
    /// requiring the sender to bind.
    ///
    /// Returns `Ok(payload.len())` whether the destination is bound
    /// or not; the datagram is silently dropped when no inbox is
    /// registered. This matches real UDP semantics (the sender never
    /// knows if the receiver was listening) and keeps the partition
    /// test honest — the sender's return value is identical whether
    /// the link is partitioned or not.
    #[allow(clippy::unused_async)]
    pub async fn send_datagram_from(
        &self,
        source: SocketAddr,
        dest: SocketAddr,
        payload: Bytes,
    ) -> Result<usize, TransportError> {
        let len = payload.len();
        let partitioned = self.router.partitions.lock().contains(&PartitionPair::new(source, dest));
        if partitioned {
            // Record for observability; the datagram is not delivered.
            self.router.blackholed.lock().push_back((source, dest, payload));
            return Ok(len);
        }

        let inbox = self.router.inboxes.lock().get(&dest).cloned();
        if let Some(tx) = inbox {
            // Dropped receiver == inbox torn down; treat like an
            // unbound destination (silent drop).
            let _ = tx.send(Datagram { from: source, payload });
        }
        Ok(len)
    }

    /// Install a bidirectional partition between `a` and `b`. While
    /// in place, datagrams flowing in either direction are dropped.
    pub fn partition(&self, a: SocketAddr, b: SocketAddr) {
        self.router.partitions.lock().insert(PartitionPair::new(a, b));
    }

    /// Remove a partition between `a` and `b`. Idempotent: repairing
    /// an unpartitioned pair is a no-op.
    pub fn repair(&self, a: SocketAddr, b: SocketAddr) {
        self.router.partitions.lock().remove(&PartitionPair::new(a, b));
    }
}

impl Default for SimTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for SimTransport {
    async fn connect(&self, addr: SocketAddr) -> Result<Box<dyn Connection>, TransportError> {
        // Phase 1 has no stream-based call sites; implementation lands
        // in Phase 2 with the first gateway/node-agent code. Surfacing
        // the gap as a typed error (rather than silently succeeding)
        // ensures a future caller hits a compile-time TODO instead of
        // a phantom-pass at runtime.
        Err(TransportError::Connect {
            addr,
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "SimTransport::connect — stream transport lands in Phase 2",
            ),
        })
    }

    async fn send_datagram(
        &self,
        addr: SocketAddr,
        payload: Bytes,
    ) -> Result<usize, TransportError> {
        // Default-source the "unspecified" address so call sites that
        // don't care about a source (broadcast, one-shot probes) still
        // work. Partition rules against UNSPECIFIED match nothing —
        // tests that want partition semantics use `send_datagram_from`.
        let source = SocketAddr::from((std::net::Ipv4Addr::UNSPECIFIED, 0));
        self.send_datagram_from(source, addr, payload).await
    }
}
