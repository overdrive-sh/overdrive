//! `SimObservationStore` — in-memory observation peer for deterministic
//! simulation.
//!
//! This module realises step 04-01 of the Phase 1 foundation roadmap:
//! the **single-peer happy path**. Multi-peer gossip and partition
//! injection arrive in step 04-02; LWW convergence is step 04-03.
//!
//! # Shape
//!
//! A peer owns two pieces of state behind a single mutex:
//!
//! * a `Vec<ObservationRow>` of every row written to this peer so far,
//!   ordered by insertion, and
//! * a `tokio::sync::broadcast::Sender<ObservationRow>` used to fan
//!   writes out to any active subscriptions on this peer.
//!
//! Writes push onto the vector *and* publish on the broadcast; the
//! vector is what multi-peer gossip in step 04-02 will reconcile over,
//! and the broadcast is what subscribers listen on.
//!
//! # Why `broadcast` rather than a `watch` channel
//!
//! `tokio::sync::watch` holds only the latest value — it would silently
//! drop a second write before the subscriber polled. `broadcast` keeps
//! each row until every subscriber has seen it (modulo capacity). For
//! the Phase 1 sim we care about observing *every* row, not just the
//! latest, so `broadcast` is the correct primitive.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use overdrive_core::id::NodeId;
use overdrive_core::traits::observation_store::{
    ObservationRow, ObservationStore, ObservationStoreError, ObservationSubscription,
};

/// Default capacity for the fan-out broadcast channel. Writes beyond
/// this count before a subscriber polls cause that subscriber to miss
/// rows — deliberate: it surfaces a sim back-pressure bug rather than
/// letting it hide behind an unbounded buffer.
const DEFAULT_FANOUT_CAPACITY: usize = 1024;

/// In-memory observation store peer.
///
/// Construct with [`SimObservationStore::single_peer`] for step 04-01
/// scenarios. Later steps will add multi-peer constructors that wire
/// several peers together under an injectable gossip delay.
pub struct SimObservationStore {
    node_id: NodeId,
    #[allow(dead_code)]
    seed: u64,
    inner: Arc<PeerState>,
}

struct PeerState {
    rows: Mutex<Vec<ObservationRow>>,
    fan_out: broadcast::Sender<ObservationRow>,
}

impl SimObservationStore {
    /// Construct a single-peer store for the given node identity and
    /// seed. Step 04-01 only exercises one peer; the seed is carried on
    /// the peer so step 04-03's deterministic LWW proptest has a
    /// reproducible source of "random" timing already threaded through.
    #[must_use]
    pub fn single_peer(node_id: NodeId, seed: u64) -> Self {
        let (fan_out, _rx) = broadcast::channel(DEFAULT_FANOUT_CAPACITY);
        Self { node_id, seed, inner: Arc::new(PeerState { rows: Mutex::new(Vec::new()), fan_out }) }
    }

    /// The node identity this peer reports to gossip. Exposed for the
    /// DST harness and for step 04-02's multi-peer wiring.
    #[must_use]
    pub const fn node_id(&self) -> &NodeId {
        &self.node_id
    }
}

#[async_trait]
impl ObservationStore for SimObservationStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        // Full-row writes only — §4 guardrail. We push a clone onto the
        // durable vector *before* fan-out so a subscriber racing with a
        // restart sees a consistent row set.
        self.inner.rows.lock().push(row.clone());

        // `send` only errors when there are zero receivers; that is a
        // valid steady state (no subscriptions yet) and must not fail
        // the write. Rows are still durable on this peer via the vector
        // above, which step 04-02's gossip will replay on subscribe.
        let _ = self.inner.fan_out.send(row);
        Ok(())
    }

    async fn subscribe_all(&self) -> Result<ObservationSubscription, ObservationStoreError> {
        let rx = self.inner.fan_out.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(ok_or_skip);
        Ok(Box::new(Box::pin(stream)) as ObservationSubscription)
    }
}

/// Helper for [`SimObservationStore::subscribe_all`]'s stream: drops any
/// `Lagged` signal emitted by `BroadcastStream` when the subscriber has
/// fallen behind the `DEFAULT_FANOUT_CAPACITY` window. A lagged
/// subscriber in a DST run is a test-author bug (capacity should be
/// sized for the workload); surfacing it as a stream value would force
/// every caller to handle a variant they cannot do anything about.
fn ok_or_skip<T, E>(item: Result<T, E>) -> futures::future::Ready<Option<T>> {
    futures::future::ready(item.ok())
}

// The `Stream` trait bound on `ObservationSubscription` needs `Unpin`,
// which `BroadcastStream` is *not*. We satisfy the bound by boxing
// into a `Pin<Box<_>>` above, but `Box<dyn Stream + Unpin>` still
// requires `Unpin` on the inner stream. Pinning a `BroadcastStream`
// into a `Box` gives us `Pin<Box<BroadcastStream>>`, which is `Unpin`
// because any `Pin<Box<T>>` is unconditionally `Unpin`.

// Typecheck: the `filter_map` adapter above produces a `FilterMap<
// BroadcastStream, fn(Result<T, E>) -> Ready<Option<T>>>` whose `Unpin`
// status depends on the inner stream. The surrounding `Box::pin` plus
// the outer `Box<_> as _` coerce into `Box<dyn Stream + Send + Unpin>`
// through `Pin<Box<_>>::Unpin`.

// Small sanity check that the public types line up. Not a replacement
// for the acceptance test; exists so that renaming
// `ObservationSubscription` fails the compile here first.
#[cfg(test)]
mod static_wiring_check {
    use super::*;
    use futures::Stream;
    #[allow(dead_code)]
    fn _assert_observation_subscription_is_stream(
        s: &ObservationSubscription,
    ) -> &(dyn Stream<Item = ObservationRow> + Send + Unpin) {
        &**s
    }
}
