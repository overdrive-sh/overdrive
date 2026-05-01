//! `SimObservationStore` — in-memory observation peer for DST.
//!
//! Plus the multi-peer `SimObservationCluster` that wires several peers
//! together with an injectable gossip delay and partition matrix.
//!
//! # Shape
//!
//! A peer owns four pieces of state behind a single mutex each:
//!
//! * a `Vec<ObservationRow>` of every LWW-winning row observed by this
//!   peer (local writes + gossip receives), ordered by receive time,
//! * an `AllocIndex` (`HashMap<AllocationId, AllocStatusRow>`) that
//!   lets queries answer "latest LWW winner for this alloc" in O(1),
//! * a parallel `NodeIndex` (`HashMap<NodeId, NodeHealthRow>`) that
//!   does the same for `node_health` rows, and
//! * a `tokio::sync::broadcast::Sender<ObservationRow>` used to fan
//!   rows out to any active subscriptions on this peer — only rows
//!   that WIN the LWW comparison (or are new) are broadcast; losers
//!   are dropped so subscribers see a convergent view.
//!
//! The peer itself is agnostic to whether it is a lone peer or part of
//! a cluster. The cluster wraps peers with a gossip router (per-peer
//! FIFO of pending deliveries) and a partition matrix; `advance(Duration)`
//! on the cluster drains the router subject to gossip delay and
//! partition rules.
//!
//! # LWW merge
//!
//! On receive (whether a local write or a gossip delivery), the peer
//! compares the incoming row's `updated_at` (alloc-status) or
//! `last_heartbeat` (node-health) against the current index entry via
//! the shared comparator [`LogicalTimestamp::dominates`] in
//! `overdrive-core`. Full rows only: losers are dropped wholesale and
//! NEVER fan out on the broadcast channel; winners replace the prior
//! row wholesale. No field-diff merge is ever applied (§4 guardrail).
//! See the `ObservationStore::write` trait docstring in `overdrive-core`
//! for the trait-level contract this implementation honours.
//!
//! # Why `broadcast` rather than a `watch` channel
//!
//! `tokio::sync::watch` holds only the latest value — it would silently
//! drop a second write before the subscriber polled. `broadcast` keeps
//! each row until every subscriber has seen it (modulo capacity). For
//! the Phase 1 sim we care about observing *every convergent row*, not
//! just the latest, so `broadcast` is the correct primitive.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use overdrive_core::id::{AllocationId, NodeId};
use overdrive_core::traits::observation_store::{
    AllocStatusRow, NodeHealthRow, ObservationRow, ObservationStore, ObservationStoreError,
    ObservationSubscription,
};

/// Default capacity for the fan-out broadcast channel. Writes beyond
/// this count before a subscriber polls cause that subscriber to miss
/// rows — deliberate: it surfaces a sim back-pressure bug rather than
/// letting it hide behind an unbounded buffer.
const DEFAULT_FANOUT_CAPACITY: usize = 1024;

// ---------------------------------------------------------------------------
// Peer
// ---------------------------------------------------------------------------

/// In-memory observation store peer.
///
/// Construct with [`SimObservationStore::single_peer`] for a lone peer,
/// or via [`SimObservationStore::cluster_builder`] for a multi-peer
/// cluster with injectable gossip delay and partitions.
pub struct SimObservationStore {
    node_id: NodeId,
    #[allow(dead_code)]
    seed: u64,
    inner: Arc<PeerState>,
    /// `Some` when this peer belongs to a cluster; `None` when it is a
    /// lone peer constructed via [`SimObservationStore::single_peer`].
    router: Option<Arc<GossipRouter>>,
}

/// Per-peer state. Rows are stored thrice: once in the ordered `rows`
/// vector (for subscription fan-out and debug inspection), once in
/// `by_alloc` (for O(1) latest-LWW queries on alloc-status), and once
/// in `by_node` (for the same on node-health). Both indexes track LWW
/// winners only — losing rows are dropped wholesale before reaching
/// the `rows` vector or the broadcast channel, per the
/// `ObservationStore::write` LWW contract.
struct PeerState {
    rows: Mutex<Vec<ObservationRow>>,
    by_alloc: Mutex<HashMap<AllocationId, AllocStatusRow>>,
    by_node: Mutex<HashMap<NodeId, NodeHealthRow>>,
    fan_out: broadcast::Sender<ObservationRow>,
}

impl PeerState {
    fn new(_seed: u64) -> Arc<Self> {
        let (fan_out, _rx) = broadcast::channel(DEFAULT_FANOUT_CAPACITY);
        Arc::new(Self {
            rows: Mutex::new(Vec::new()),
            by_alloc: Mutex::new(HashMap::new()),
            by_node: Mutex::new(HashMap::new()),
            fan_out,
        })
    }

    /// Apply an incoming row (whether a local write or a gossip
    /// delivery) under LWW semantics.
    fn apply(&self, row: ObservationRow) -> bool {
        let accepted = match &row {
            ObservationRow::AllocStatus(incoming) => self.apply_alloc_status(incoming),
            ObservationRow::NodeHealth(incoming) => self.apply_node_health(incoming),
        };

        if accepted {
            self.rows.lock().push(row.clone());
            // `send` only errors when there are zero receivers; that is
            // a valid steady state (no subscriptions yet) and must not
            // fail the write.
            let _ = self.fan_out.send(row);
        }
        accepted
    }

    /// LWW merge for `alloc_status`. Returns `true` when the incoming
    /// row dominates or is new; `false` when it loses to an existing
    /// entry.
    fn apply_alloc_status(&self, incoming: &AllocStatusRow) -> bool {
        let mut by_alloc = self.by_alloc.lock();
        match by_alloc.get(&incoming.alloc_id) {
            None => {
                by_alloc.insert(incoming.alloc_id.clone(), incoming.clone());
                true
            }
            Some(existing) if incoming.updated_at.dominates(&existing.updated_at) => {
                by_alloc.insert(incoming.alloc_id.clone(), incoming.clone());
                true
            }
            Some(_) => false,
        }
    }

    /// LWW merge for `node_health`. Mirrors [`apply_alloc_status`] —
    /// keyed by [`NodeHealthRow::node_id`], compares
    /// [`NodeHealthRow::last_heartbeat`].
    fn apply_node_health(&self, incoming: &NodeHealthRow) -> bool {
        let mut by_node = self.by_node.lock();
        match by_node.get(&incoming.node_id) {
            None => {
                by_node.insert(incoming.node_id.clone(), incoming.clone());
                true
            }
            Some(existing) if incoming.last_heartbeat.dominates(&existing.last_heartbeat) => {
                by_node.insert(incoming.node_id.clone(), incoming.clone());
                true
            }
            Some(_) => false,
        }
    }

    fn latest_alloc_status(&self, alloc_id: &AllocationId) -> Option<AllocStatusRow> {
        self.by_alloc.lock().get(alloc_id).cloned()
    }

    /// Snapshot every alloc-status row this peer holds as LWW winner,
    /// keyed by `AllocationId` under `BTreeMap` (deterministic iteration
    /// is load-bearing for the bit-identical comparison in
    /// [`check_lww_convergence`]).
    fn alloc_status_snapshot(&self) -> BTreeMap<AllocationId, AllocStatusRow> {
        self.by_alloc.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Snapshot every node-health row this peer holds as LWW winner,
    /// keyed by [`NodeId`] under `BTreeMap`. Same determinism rationale
    /// as [`alloc_status_snapshot`].
    fn node_health_snapshot(&self) -> BTreeMap<NodeId, NodeHealthRow> {
        self.by_node.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

impl SimObservationStore {
    /// Construct a single-peer store for the given node identity and
    /// seed. Backwards-compatible with step 04-01's acceptance test.
    #[must_use]
    pub fn single_peer(node_id: NodeId, seed: u64) -> Self {
        Self { node_id, seed, inner: PeerState::new(seed), router: None }
    }

    /// Start building a multi-peer cluster. See
    /// [`SimObservationClusterBuilder`] for the configurable knobs.
    #[must_use]
    pub const fn cluster_builder() -> SimObservationClusterBuilder {
        SimObservationClusterBuilder::new()
    }

    /// The node identity this peer reports to gossip.
    #[must_use]
    pub const fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Latest LWW winner for an `alloc_status` on this peer, if any.
    /// Reads are local — never cross-peer.
    #[must_use]
    pub fn latest_alloc_status(&self, alloc_id: &AllocationId) -> Option<AllocStatusRow> {
        self.inner.latest_alloc_status(alloc_id)
    }

    /// Snapshot every alloc-status row this peer currently holds as its
    /// LWW winner. Deterministic iteration is guaranteed by the
    /// `BTreeMap` key ordering on [`AllocationId`].
    ///
    /// Used by [`check_lww_convergence`] to build a cross-peer report;
    /// also useful for debugging a single peer's view of the cluster
    /// from test code.
    #[must_use]
    pub fn alloc_status_snapshot(&self) -> BTreeMap<AllocationId, AllocStatusRow> {
        self.inner.alloc_status_snapshot()
    }
}

#[async_trait]
impl ObservationStore for SimObservationStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        // Full-row writes only — §4 guardrail. Apply locally first so
        // the writing peer's own subscribers see the write immediately;
        // the LWW check at `apply` drops a losing local write wholesale.
        let accepted = self.inner.apply(row.clone());

        // If we belong to a cluster, enqueue the row for gossip to every
        // non-partitioned peer. Losers are also enqueued: a peer that
        // received a losing write locally still has to tell the other
        // peers that this timestamped row exists — remote peers may not
        // have seen the dominant row yet and need to run their own LWW
        // merge. (In practice, for Phase 1 where the only writer is the
        // alloc's owning node, a losing write is rare; we still model
        // it correctly.)
        if let Some(router) = &self.router {
            let _ = accepted; // accepted is used above; no gating here
            router.enqueue_from(&self.node_id, &row);
        }
        Ok(())
    }

    async fn subscribe_all(&self) -> Result<ObservationSubscription, ObservationStoreError> {
        let rx = self.inner.fan_out.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(ok_or_skip);
        Ok(Box::new(Box::pin(stream)) as ObservationSubscription)
    }

    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        // Deterministic iteration via the BTreeMap ordering on
        // AllocationId — every call on the same state returns rows in
        // the same order, so byte-identical responses across runs are
        // preserved (K3 reproducibility).
        Ok(self.inner.alloc_status_snapshot().into_values().collect())
    }

    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        // LWW winners only — keyed by `NodeId` under `BTreeMap` for
        // deterministic iteration. The `ObservationStore::write` LWW
        // contract requires losers be dropped before reaching either
        // the broadcast channel or the read snapshot; the index is
        // already maintained that way by `apply_node_health`.
        Ok(self.inner.node_health_snapshot().into_values().collect())
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

// ---------------------------------------------------------------------------
// Cluster
// ---------------------------------------------------------------------------

/// A multi-peer cluster of [`SimObservationStore`]s wired together with
/// a gossip router. Construct via [`SimObservationStore::cluster_builder`].
///
/// Handing out peers is `Arc`-shared — every call to [`peer`] returns the
/// *same* peer for a given `NodeId`. Subscriptions and writes therefore
/// compose as expected.
///
/// [`peer`]: SimObservationCluster::peer
pub struct SimObservationCluster {
    peers: HashMap<NodeId, Arc<SimObservationStore>>,
    router: Arc<GossipRouter>,
}

impl SimObservationCluster {
    /// The peer with the given node id, shared with every other caller.
    /// Panics if `node_id` is not part of this cluster — construction
    /// time is where unknown peers must be caught.
    #[must_use]
    pub fn peer(&self, node_id: &NodeId) -> Arc<SimObservationStore> {
        self.peers
            .get(node_id)
            .cloned()
            .unwrap_or_else(|| panic!("peer {node_id} not part of this cluster"))
    }

    /// Every peer in this cluster, paired with its [`NodeId`]. Order is
    /// **not** guaranteed — callers that need deterministic iteration
    /// must sort by [`NodeId`] themselves. [`check_lww_convergence`] does
    /// so via `BTreeMap` to keep its output byte-comparable across runs.
    pub fn peers(&self) -> impl Iterator<Item = (&NodeId, &Arc<SimObservationStore>)> {
        self.peers.iter()
    }

    /// Advance simulated time by `duration`. Drains any gossip messages
    /// whose delay has elapsed, subject to the partition matrix.
    ///
    /// The router carries its own logical clock (`RouterState::now`);
    /// this call bumps that clock and then drains every message whose
    /// `deliver_at` is now `<= now`. No `tokio::time::sleep` happens
    /// here — the router's clock is explicit state, not wall time, and
    /// gossip delivery is a synchronous in-memory fan-out via the
    /// broadcast channel. This keeps the cluster fully deterministic
    /// under DST regardless of whether callers use a paused runtime.
    ///
    /// The `async` signature is kept for forward-compat with step 05-01
    /// when `SimClock` lands and `advance` coordinates with the shared
    /// logical clock.
    #[allow(clippy::unused_async)]
    pub async fn advance(&self, duration: Duration) {
        self.router.advance_clock(duration);
        self.router.drain_pending(&self.peers);
        // Yield once so any task awaiting a subscription stream is
        // polled after the fan-out has published winners. Without
        // this, a `next().await` started synchronously before the
        // advance could miss the freshly-published row on the first
        // wake. `yield_now` is wall-clock-free.
        tokio::task::yield_now().await;
    }

    /// Install a bidirectional partition between `a` and `b`. While the
    /// partition is in place, gossip neither delivers A→B nor B→A.
    /// Multiple overlapping partition calls are idempotent.
    ///
    /// `async` is preserved for forward-compat with step 05-01 when
    /// `SimClock` lands — partition and repair will coordinate with the
    /// harness's logical clock at that point.
    #[allow(clippy::unused_async)]
    pub async fn partition(&self, a: &NodeId, b: &NodeId) {
        self.router.partition(a.clone(), b.clone());
    }

    /// Remove a partition between `a` and `b`. Idempotent: repairing an
    /// unpartitioned pair is a no-op. See [`partition`] for the `async`
    /// rationale.
    ///
    /// [`partition`]: Self::partition
    #[allow(clippy::unused_async)]
    pub async fn repair(&self, a: &NodeId, b: &NodeId) {
        self.router.repair(a, b);
    }
}

// ---------------------------------------------------------------------------
// Convergence helper — shared between proptest and US-06 invariant
// ---------------------------------------------------------------------------

/// Snapshot of the LWW state of every peer in a cluster.
///
/// Keyed by [`NodeId`] under `BTreeMap` so that iteration order — and
/// therefore equality comparison — is deterministic across runs.
/// See [`SimObservationCluster`].
///
/// Produced by [`check_lww_convergence`]. Step 04-03's proptest compares
/// two reports across seeded runs for bit-identity; step 06-02 re-uses
/// the same helper inside the `SimObservationLwwConverges`
/// `assert_always!` invariant body. The assertion logic lives in one
/// place — this type — and never duplicated across the two call sites.
///
/// # Equality
///
/// `ConvergenceReport::eq` compares the full nested map structure. Two
/// reports are equal when and only when every peer holds the same set
/// of `(alloc_id → AllocStatusRow)` entries. The `AllocStatusRow` fields
/// are compared wholesale (no field-diff semantics — the §4 guardrail
/// applies to the report just as it applies to the stored rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvergenceReport {
    peer_views: BTreeMap<NodeId, BTreeMap<AllocationId, AllocStatusRow>>,
}

impl ConvergenceReport {
    /// Construct an empty report — a cluster with no writes at all.
    /// Kept `pub(crate)` so tests in this crate can stub a report
    /// without going through the cluster path when they want to assert
    /// on the `is_converged` logic in isolation.
    pub(crate) const fn new() -> Self {
        Self { peer_views: BTreeMap::new() }
    }

    /// Borrow the per-peer views. The outer `BTreeMap` is keyed by
    /// [`NodeId`]; the inner `BTreeMap` is keyed by [`AllocationId`].
    /// Iteration order is deterministic on both axes.
    #[must_use]
    pub const fn peer_views(&self) -> &BTreeMap<NodeId, BTreeMap<AllocationId, AllocStatusRow>> {
        &self.peer_views
    }

    /// True when every peer that has observed an allocation holds the
    /// same `AllocStatusRow` for it as every other peer that has
    /// observed it.
    ///
    /// A peer that has never seen a given allocation is not a
    /// violation — the allocation's winning row may simply not have
    /// reached that peer yet (partition, gossip still draining).
    /// Convergence means *there is no disagreement among peers that
    /// have seen a row*, not *every peer has seen every row*.
    ///
    /// This definition matches the §5.1 scenario 3 AC: "every peer's
    /// final row set is bit-identical across two runs" is the stronger
    /// cross-run property, enforced by `ConvergenceReport::eq`.
    /// `is_converged` is the within-run property: for each `alloc_id`,
    /// every peer that has a row for it has the **same** row.
    #[must_use]
    pub fn is_converged(&self) -> bool {
        // Collect the union of alloc_ids seen across all peers; for
        // each, verify every peer that has an entry agrees on the row.
        let mut per_alloc: BTreeMap<&AllocationId, &AllocStatusRow> = BTreeMap::new();
        for view in self.peer_views.values() {
            for (alloc_id, row) in view {
                match per_alloc.get(alloc_id) {
                    None => {
                        per_alloc.insert(alloc_id, row);
                    }
                    Some(existing) if *existing == row => {
                        // Agreement so far — continue.
                    }
                    Some(_) => return false,
                }
            }
        }
        true
    }
}

/// Snapshot every peer's LWW state in `cluster` and return a
/// deterministic [`ConvergenceReport`].
///
/// The returned report is read-only: it captures the cluster state at
/// the moment of the call. Call after `cluster.advance(...)` has
/// drained the gossip window — otherwise the report reflects an
/// in-flight state, which is valid but usually not what a convergence
/// check wants to assert on.
///
/// # Used by
///
/// * The §5.1 scenario 3 proptest in
///   `tests/acceptance/sim_observation_lww_converges.rs` — two reports
///   across two seeded runs must be `==` (bit-identical).
/// * The `SimObservationLwwConverges` `assert_always!` invariant
///   evaluator added in step 06-02 — the invariant body checks
///   `check_lww_convergence(cluster).is_converged()` on every tick.
#[must_use]
pub fn check_lww_convergence(cluster: &SimObservationCluster) -> ConvergenceReport {
    let mut peer_views = BTreeMap::new();
    for (node_id, peer) in cluster.peers() {
        peer_views.insert(node_id.clone(), peer.alloc_status_snapshot());
    }
    ConvergenceReport { peer_views }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for [`SimObservationCluster`]. Every knob is explicit; there
/// are no hidden defaults for seed or gossip delay — forgetting either
/// would silently yield a non-deterministic test.
pub struct SimObservationClusterBuilder {
    peers: Vec<NodeId>,
    gossip_delay: Option<Duration>,
    partitions: Vec<(NodeId, NodeId)>,
    seed: Option<u64>,
}

impl SimObservationClusterBuilder {
    const fn new() -> Self {
        Self { peers: Vec::new(), gossip_delay: None, partitions: Vec::new(), seed: None }
    }

    /// Declare the peers in this cluster. Panics at [`build`] if called
    /// with fewer than two peers — single-peer tests should use
    /// [`SimObservationStore::single_peer`] directly.
    ///
    /// [`build`]: Self::build
    #[must_use]
    pub fn peers<I>(mut self, peers: I) -> Self
    where
        I: IntoIterator<Item = NodeId>,
    {
        self.peers = peers.into_iter().collect();
        self
    }

    /// Fixed gossip propagation delay. Every inter-peer message waits
    /// this long in the router's FIFO before becoming eligible for
    /// delivery on the next `advance` pass.
    #[must_use]
    pub const fn gossip_delay(mut self, delay: Duration) -> Self {
        self.gossip_delay = Some(delay);
        self
    }

    /// Install a bidirectional partition between `a` and `b` at build
    /// time. Equivalent to calling
    /// [`SimObservationCluster::partition`] immediately after `build`.
    #[must_use]
    pub fn partition(mut self, a: NodeId, b: NodeId) -> Self {
        self.partitions.push((a, b));
        self
    }

    /// Seed carried on each peer. Required; omitting it means callers
    /// are writing a test that DST cannot reproduce.
    #[must_use]
    pub const fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Finalise the cluster. Panics when required fields are missing or
    /// the peer set is degenerate — these are test-author bugs.
    #[must_use]
    pub fn build(self) -> SimObservationCluster {
        assert!(
            self.peers.len() >= 2,
            "SimObservationCluster requires at least 2 peers; use \
             SimObservationStore::single_peer for a lone peer"
        );
        let Some(seed) = self.seed else {
            panic!("cluster builder requires .seed(_)");
        };
        let Some(gossip_delay) = self.gossip_delay else {
            panic!("cluster builder requires .gossip_delay(_)");
        };

        let router = Arc::new(GossipRouter::new(gossip_delay));
        router.set_peers(self.peers.clone());
        for (a, b) in self.partitions {
            router.partition(a, b);
        }

        let mut peers = HashMap::with_capacity(self.peers.len());
        for node_id in self.peers {
            let store = Arc::new(SimObservationStore {
                node_id: node_id.clone(),
                seed,
                inner: PeerState::new(seed),
                router: Some(Arc::clone(&router)),
            });
            peers.insert(node_id, store);
        }

        SimObservationCluster { peers, router }
    }
}

// ---------------------------------------------------------------------------
// Gossip router — per-peer FIFO + partition matrix + logical clock
// ---------------------------------------------------------------------------

/// Router for inter-peer gossip. Owns a logical clock, a per-peer FIFO
/// of `(deliver_at, row)` entries, and a symmetric partition matrix.
/// All state lives behind a single mutex — the cluster is not trying to
/// be a performance-optimal gossip impl, it is trying to be correct and
/// deterministic under DST.
struct GossipRouter {
    state: Mutex<RouterState>,
    gossip_delay: Duration,
}

struct RouterState {
    /// Simulated "now" — advances monotonically via
    /// [`GossipRouter::advance_clock`].
    now: Duration,
    /// The full peer set, populated at cluster build time via
    /// [`GossipRouter::set_peers`]. Required so `enqueue_from` can fan
    /// a source's write out across per-recipient queues at enqueue
    /// time.
    known_peers: Vec<NodeId>,
    /// Pending deliveries keyed by `(source, recipient)`. Each queue
    /// is FIFO per source-recipient pair, matching gossip semantics:
    /// writes from one source arrive at each peer in the order they
    /// were made. Different source-recipient pairs are independent.
    ///
    /// Keyed this way so we can leave partition-blocked entries in
    /// place until the partition heals: a head-of-line entry that
    /// cannot be delivered to a specific recipient does not prevent
    /// later entries from different sources reaching a different
    /// recipient.
    ///
    /// `BTreeMap` rather than `HashMap` so `drain_pending` iterates
    /// pairs in a deterministic `(source, recipient)` order. A
    /// `HashMap` iteration order varies per-run via Rust's default
    /// `RandomState` hasher — under Phase 2 multi-writer scenarios,
    /// that randomness would change the order in which peers observe
    /// each row and therefore the LWW winner, breaking K3 bit-for-bit
    /// reproducibility at the router level.
    queues: BTreeMap<(NodeId, NodeId), VecDeque<Pending>>,
    /// Unordered pairs of partitioned nodes. Stored canonicalised so
    /// insertion and lookup are symmetric regardless of argument order.
    partitions: HashSet<PartitionPair>,
}

struct Pending {
    deliver_at: Duration,
    row: ObservationRow,
}

/// Unordered pair of `NodeId`s. Two `PartitionPair`s are equal iff they
/// contain the same two ids, regardless of which argument was `a` and
/// which was `b` at construction time.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PartitionPair(NodeId, NodeId);

impl PartitionPair {
    fn new(a: NodeId, b: NodeId) -> Self {
        // Canonicalise by `Display` string ordering so that
        // `PartitionPair::new(a, b) == PartitionPair::new(b, a)`.
        if a.to_string() <= b.to_string() { Self(a, b) } else { Self(b, a) }
    }
}

impl GossipRouter {
    fn new(gossip_delay: Duration) -> Self {
        Self {
            state: Mutex::new(RouterState {
                now: Duration::from_secs(0),
                known_peers: Vec::new(),
                queues: BTreeMap::new(),
                partitions: HashSet::new(),
            }),
            gossip_delay,
        }
    }

    /// Register the full peer set. Called once by
    /// [`SimObservationClusterBuilder::build`] after all peers have
    /// been instantiated — the router needs the full list to fan out
    /// writes across per-recipient queues at enqueue time.
    fn set_peers(&self, peers: Vec<NodeId>) {
        self.state.lock().known_peers = peers;
    }

    fn advance_clock(&self, duration: Duration) {
        let mut s = self.state.lock();
        s.now = s.now.saturating_add(duration);
    }

    fn partition(&self, a: NodeId, b: NodeId) {
        self.state.lock().partitions.insert(PartitionPair::new(a, b));
    }

    fn repair(&self, a: &NodeId, b: &NodeId) {
        // `HashSet::remove` returns `bool` — we don't care, repair is
        // idempotent and a no-op on an unpartitioned pair is correct
        // behaviour (not a panic).
        self.state.lock().partitions.remove(&PartitionPair::new(a.clone(), b.clone()));
    }

    /// Enqueue a row emitted by `source` for delivery to every other
    /// known peer. Fans out at enqueue time: one pending entry per
    /// `(source, recipient)` queue. This shape lets `drain_pending`
    /// leave partition-blocked entries in their own queue without
    /// head-of-line-blocking deliveries to other recipients.
    fn enqueue_from(&self, source: &NodeId, row: &ObservationRow) {
        let mut s = self.state.lock();
        let deliver_at = s.now.saturating_add(self.gossip_delay);
        // Take a snapshot of known peers under the lock; cloning the
        // `Vec<NodeId>` is cheap and lets us drop the lock before the
        // `entry()` calls below would otherwise hold it twice.
        let recipients: Vec<NodeId> =
            s.known_peers.iter().filter(|p| *p != source).cloned().collect();
        for recipient in recipients {
            let pending = Pending { deliver_at, row: row.clone() };
            s.queues.entry((source.clone(), recipient)).or_default().push_back(pending);
        }
    }

    /// Drain any pending gossip whose `deliver_at` has been reached by
    /// the router's logical clock, subject to the partition matrix.
    ///
    /// Partition-blocked entries are LEFT in their queue — they do
    /// not block other queues, and they become deliverable once the
    /// partition is repaired. This is what makes the §5.3 "partition
    /// heals, B and C see A's row" scenario work without re-enqueueing
    /// at repair time.
    fn drain_pending(&self, peers: &HashMap<NodeId, Arc<SimObservationStore>>) {
        // We drain under a lock, but must not call `peer.apply` under
        // the router lock — `apply` takes the peer's own mutex and
        // could deadlock against a concurrent subscribe. So we copy
        // out the eligible deliveries first, drop the lock, then apply.
        let mut s = self.state.lock();
        let now = s.now;
        let partitions = s.partitions.clone();
        let mut eligible: Vec<(NodeId, ObservationRow)> = Vec::new();

        for ((source, recipient), queue) in &mut s.queues {
            // Skip the whole queue if the partition is in place —
            // entries stay in place for delivery after repair.
            if partitions.contains(&PartitionPair::new(source.clone(), recipient.clone())) {
                continue;
            }
            while let Some(front) = queue.front() {
                if front.deliver_at <= now {
                    let Some(pending) = queue.pop_front() else { break };
                    eligible.push((recipient.clone(), pending.row));
                } else {
                    break;
                }
            }
        }
        drop(s);

        // Apply each eligible delivery to its recipient.
        for (recipient_id, row) in eligible {
            if let Some(recipient) = peers.get(&recipient_id) {
                recipient.inner.apply(row);
            }
        }
    }
}

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
