//! DST invariants for `backend-discovery-bridge-service-reachability` (joint #174 + #175).
//!
//! Per `docs/feature/backend-discovery-bridge-service-reachability/distill/test-scenarios.md`
//! S-BDB-02..S-BDB-10 and Atlas Q2 (S-BDB-06). Tier 1 — pure-Rust under sim
//! adapters; runs via `cargo dst` on every PR.
//!
//! RED scaffold per `.claude/rules/testing.md` § "RED scaffolds and
//! intentionally-failing commits" and per `crates/overdrive-sim/src/harness.rs`
//! § 838 (this crate's convention): the invariant evaluator bodies are
//! `todo!("RED scaffold: ...")`. The DST harness's invariant-walk loop
//! is expected to call every registered invariant's `evaluate` method;
//! the `todo!` panic propagates to the harness's test bodies which carry
//! `#[should_panic(expected = "RED scaffold")]` (DST harness convention —
//! see `evaluators.rs` for the existing SCAFFOLD-pattern variants).
//!
//! GREEN transition: DELIVER Slice 1 (closes #174) replaces each `todo!`
//! with the real evaluator body. The `#[expect(clippy::todo, ...)]`
//! attribute self-removes when the lint stops firing — the natural moment
//! the scaffold goes GREEN.
//!
//! Production code these invariants guard (per Atlas Q3 mutation-scope
//! mapping in `docs/feature/backend-discovery-bridge-service-reachability/distill/wave-decisions.md`):
//!
//! - `BackendDiscoveryBridge::reconcile` body — main loop
//! - `BackendDiscoveryBridge::reconcile` dedup branch
//! - `BackendDiscoveryBridge::reconcile` View GC `retain` clause
//! - `fingerprint(&vip, &backends)` call site
//! - `hydrate_desired` allocator-lookup arm
//! - `hydrate_actual` Running-filter arm
//! - `Action::WriteServiceBackendRow` action shim dispatch
//! - `ViewStore` crash-recovery semantics (fsync-then-memory ordering)

#![expect(
    clippy::todo,
    reason = "RED scaffold for backend-discovery-bridge-service-reachability invariants; \
              lands GREEN in DELIVER Slice 1 (closes #174)"
)]

// ----------------------------------------------------------------------------
// Invariant: BridgeEventuallyWritesBackendRow
//
// Spec (S-BDB-02, S-BDB-03, S-BDB-04, S-BDB-10):
//   For every Service workload with >= 1 listener AND an allocator-issued
//   VIP for its spec_digest AND >= 1 Running alloc, a ServiceBackendRow is
//   eventually written whose `backends` contains exactly the Running allocs'
//   endpoints, whose `vip` equals the allocator-issued VIP, and whose
//   `updated_at.{counter, writer}` reflect `tick.tick + 1` and AppState.node_id.
//
// Property class: eventually.
//
// Fault catalogue (the harness drives all of these):
//   - Single Running alloc -> single backend entry.
//   - Pending -> Running -> Failed -> second Pending -> Running: steady state
//     is the second Running alloc only.
//   - Multiple concurrent Running allocs -> backend set is the union.
//   - SimViewStore::inject_fsync_failure between tick and observation read.
//   - SimObservationStore::write transient Busy -> bridge retries next tick.
// ----------------------------------------------------------------------------

/// `BridgeEventuallyWritesBackendRow` — see module-level docs for the spec
/// and fault catalogue. Replace the `todo!` body in DELIVER Slice 1.
pub struct BridgeEventuallyWritesBackendRow;

impl BridgeEventuallyWritesBackendRow {
    /// Construct the invariant. Zero arguments; the evaluator reads state
    /// from the harness via the standard Invariant trait protocol.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeEventuallyWritesBackendRow {
    fn default() -> Self {
        Self::new()
    }
}

// NOTE: the actual `impl Invariant for BridgeEventuallyWritesBackendRow`
// block lands in DELIVER Slice 1 alongside the production reconciler.
// At DISTILL stage we cannot bind to the `Invariant` trait body without
// also pulling in not-yet-existent production types (`BackendDiscoveryBridgeView`,
// `ServiceListenerSet`, `RunningAllocSet`, the `assigned_vip` projection),
// which would force production scaffolds we are not supposed to create
// (DISTILL writes test scaffolds, NOT production scaffolds; production
// scaffolds land in DELIVER step 01 per ADR-0035 contract).
//
// The RED signal at DISTILL stage IS the documented `todo!`-stubbed
// evaluator below, which DELIVER Slice 1 lifts into the real
// `Invariant::evaluate` body in this file. Until then the struct exists
// (allowing the `Invariant` enum variant in `mod.rs` to compile), and
// the placeholder `evaluate_red_scaffold` fn captures the intent of the
// real evaluator for DELIVER to lift verbatim.

impl BridgeEventuallyWritesBackendRow {
    /// Placeholder for the real `Invariant::evaluate` body. DELIVER Slice 1
    /// renames this to `evaluate(&self, harness: &Harness) -> Verdict`
    /// once the `Invariant` trait wiring lands. The panicking `todo!`
    /// preserves the RED signal until then.
    pub fn evaluate_red_scaffold(&self) -> ! {
        todo!(
            "RED scaffold: BridgeEventuallyWritesBackendRow.evaluate — \
             assert that for every Service workload with >= 1 listener AND \
             allocator-issued VIP AND >= 1 Running alloc, the harness's \
             SimObservationStore eventually reports \
             service_backends_rows(service_id).backends == expected within \
             the fault catalogue (see module docs); see S-BDB-02/03/04/10 \
             in distill/test-scenarios.md"
        )
    }
}

// ----------------------------------------------------------------------------
// Invariant: BridgeIdempotentSteadyState
//
// Spec (S-BDB-05, S-BDB-07):
//   Once observation matches desired for every Service workload, the bridge
//   emits zero Action::WriteServiceBackendRow actions per tick. The View's
//   `last_written_fingerprint` map is stable under unchanged inputs.
//   Additionally (S-BDB-07): when intent listeners shrink, the View's GC
//   retain clause drops the removed ServiceId entries.
//
// Property class: always.
// ----------------------------------------------------------------------------

/// `BridgeIdempotentSteadyState` — see module-level docs. Replace `todo!`
/// in DELIVER Slice 1.
pub struct BridgeIdempotentSteadyState;

impl BridgeIdempotentSteadyState {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeIdempotentSteadyState {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeIdempotentSteadyState {
    /// Placeholder for the real `Invariant::evaluate` body. DELIVER Slice 1
    /// lifts this into the trait impl.
    pub fn evaluate_red_scaffold(&self) -> ! {
        todo!(
            "RED scaffold: BridgeIdempotentSteadyState.evaluate — assert \
             that once obs.service_backends_rows(...).backends == expected \
             for every Service workload, K subsequent ticks (K >= 1) with \
             unchanged inputs produce zero Action::WriteServiceBackendRow \
             actions AND the View's last_written_fingerprint map is \
             unchanged AND no new service_backends row write is observable. \
             Also exercises the View GC retain clause (S-BDB-07) by \
             shrinking the listener set and asserting the View shrinks \
             correspondingly. See S-BDB-05/07 in distill/test-scenarios.md"
        )
    }
}

// ----------------------------------------------------------------------------
// Invariant: BridgeRecomputesFingerprintOnReplay (Atlas Q2)
//
// Spec (S-BDB-06):
//   The harness injects a crash between SimViewStore::write_through fsync
//   and the runtime's in-memory BTreeMap::insert. After restart + bulk_load,
//   the bridge's first tick re-projects from inputs, recomputes the
//   per-service fingerprint, and either (a) emits zero actions if the
//   fingerprint matches the persisted view (idempotent) or (b) emits
//   Action::WriteServiceBackendRow if it differs (no silent skip).
//
// Property class: always (under the crash-recovery scenario family).
//
// Why this matters: the fsync-then-memory ordering rule in
// `.claude/rules/development.md` § "Reconciler I/O" is structurally
// load-bearing for crash recovery; this invariant proves the bridge's
// reconcile body honors it.
// ----------------------------------------------------------------------------

/// `BridgeRecomputesFingerprintOnReplay` — addresses Atlas non-blocking Q2
/// from DESIGN review. See module-level docs and S-BDB-06.
pub struct BridgeRecomputesFingerprintOnReplay;

impl BridgeRecomputesFingerprintOnReplay {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BridgeRecomputesFingerprintOnReplay {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeRecomputesFingerprintOnReplay {
    /// Placeholder for the real `Invariant::evaluate` body. DELIVER Slice 1
    /// lifts this into the trait impl.
    pub fn evaluate_red_scaffold(&self) -> ! {
        todo!(
            "RED scaffold: BridgeRecomputesFingerprintOnReplay.evaluate — \
             with steady state reached and View persisted with fingerprint \
             FP_old, inject a crash via SimViewStore between fsync and the \
             in-memory BTreeMap::insert step, restart, bulk_load every \
             View, tick once. Assert that the bridge's first post-restart \
             tick re-projects from fresh inputs; if inputs yield the same \
             fingerprint -> zero actions emitted (idempotent); if different \
             -> Action::WriteServiceBackendRow emitted with the new \
             fingerprint. Eventually steady state is reached again. See \
             S-BDB-06 in distill/test-scenarios.md and Atlas Q2 in \
             DESIGN wave-decisions.md."
        )
    }
}
