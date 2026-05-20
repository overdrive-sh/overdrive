//! `BackendDiscoveryBridge` reconciler — type surface (step 01-01 of
//! `backend-discovery-bridge-service-reachability`).
//!
//! This module lands the pure type surface for the bridge reconciler
//! per `docs/feature/backend-discovery-bridge-service-reachability/
//! design/architecture.md` § 4.2:
//!
//! - [`BackendDiscoveryBridgeState`] — merged `(desired, actual)`
//!   stitched by the runtime before `reconcile` per ADR-0036.
//! - [`ServiceListenerSet`] — desired-side projection of every
//!   listener the workload's intent declares, paired with the
//!   allocator-issued `ServiceVip` resolved at hydrate time.
//! - [`ProjectedListener`] — single allocator-issued
//!   `(vip, port, protocol)` triple. The VIP is NOT carried by intent
//!   (`ServiceV1` has no `vip` field per ADR-0050 § 2); the runtime's
//!   hydrate path looks it up via `ServiceVipAllocator::get(&spec_digest)`
//!   per ADR-0049 § 5a.
//! - [`RunningAllocSet`] — actual-side projection of the Running
//!   alloc set for the workload, sourced from
//!   `ObservationStore::alloc_status_rows_for_workload`.
//! - [`BackendDiscoveryBridgeView`] — runtime-persisted typed memory
//!   per ADR-0035 § 1. Persists *inputs* per
//!   `.claude/rules/development.md` § "Persist inputs, not derived
//!   state": the per-service fingerprint of the last row the bridge
//!   successfully wrote. The dedup decision is recomputed every tick
//!   from this input + the freshly-computed current fingerprint —
//!   never persisted as a derived "needs write" boolean.
//! - [`BackendDiscoveryBridge`] — empty struct placeholder so the
//!   `AnyReconciler::BackendDiscoveryBridge(_)` variant has a
//!   concrete inner type to carry. The `Reconciler` trait impl,
//!   the `reconcile` body, and the `host_ipv4` constructor parameter
//!   land in step 01-02 alongside the dedup loop.
//!
//! Per ADR-0035 § 1 the View derives the four mandatory bounds
//! (`Serialize + Deserialize + Default + Clone`) plus `PartialEq + Eq`
//! for the runtime's Eq-diff skip and for DST equality assertions.
//! The CBOR codec is the runtime's choice (ADR-0035 § 3); the test
//! surface at `crates/overdrive-core/tests/backend_discovery_bridge_types.rs`
//! pins the round-trip property.
//!
//! `BTreeMap` / `BTreeSet` per `.claude/rules/development.md` §
//! "Ordered-collection choice" — every keyed map in this module is
//! iterated by the bridge's reconcile loop (lands in 01-02) and DST
//! invariants assert on observed iteration order, so the per-process
//! random hash-seed of `HashMap` is structurally banned.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::BackendSetFingerprint;
use crate::id::{AllocationId, ServiceId, ServiceVip, WorkloadId};

use super::{Reconciler, ReconcilerName};

/// Desired-side projection: the workload's declared listener set,
/// keyed by `ServiceId`, with each entry's VIP sourced from the
/// allocator (NOT the intent aggregate).
///
/// Sourced by the runtime's `hydrate_desired` arm (lands in step
/// 01-03) from two reads:
///
/// 1. `IntentStore::get(IntentKey::for_workload(&workload_id))` →
///    `WorkloadIntent::Service(ServiceV1)`, which carries the
///    per-listener `(port, protocol)` pairs.
/// 2. `ServiceVipAllocator::get(&spec_digest)` per ADR-0049 § 5a,
///    where `spec_digest = WorkloadIntent::spec_digest(&intent)?`.
///
/// Phase 1 invariant: the allocator memo is populated synchronously
/// at admission (ADR-0049 § 4) before the intent is persisted, so
/// the allocator lookup at hydrate time is always `Some(_)` for a
/// Service workload that reached IntentStore. A `None` here would
/// be a structural bug and surfaces as a debug event; the bridge
/// returns an empty desired state and defers convergence to a
/// subsequent tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceListenerSet {
    /// Workload that owns this listener set. Carried so the
    /// reconcile body (lands 01-02) can correlate dispatched
    /// `WriteServiceBackendRow` actions back to the workload
    /// without re-deriving from the `TargetResource`.
    pub workload_id: WorkloadId,
    /// Per-listener projection keyed by `ServiceId`. The
    /// allocator-issued VIP is the same across every entry (one VIP
    /// per Service per ADR-0049 § 5a); the `ServiceId` key
    /// distinguishes per-port instances within the workload.
    pub listeners: BTreeMap<ServiceId, ProjectedListener>,
}

/// Single allocator-issued `(vip, port, protocol)` triple. Carried
/// in the per-`ServiceId` entries of [`ServiceListenerSet`].
///
/// The VIP is allocator-issued at hydrate time per ADR-0049 § 5a;
/// `ServiceV1` carries no `vip` field per ADR-0050 § 2. Per
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state" the VIP is hydration input, NOT a value persisted
/// anywhere on the bridge's `View`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedListener {
    /// Allocator-issued VIP for the workload. Sourced from
    /// `ServiceVipAllocator::get(&spec_digest)` at hydrate time per
    /// ADR-0049 § 5a; NOT from the intent aggregate (`ServiceV1`
    /// carries no VIP field — ADR-0050 § 2).
    pub vip: ServiceVip,
    /// TCP / UDP port the listener accepts traffic on. `NonZeroU16`
    /// because zero is rejected by the parser at the intent
    /// boundary (`crate::aggregate::workload_spec`) and the bridge
    /// is downstream of that validation — preserving the type-level
    /// "non-zero" property keeps the bridge's reconcile body free
    /// of redundant runtime checks.
    pub port: NonZeroU16,
    /// Transport protocol. Phase 2.2 ships `Tcp` only; `Udp` is the
    /// natural Phase 2.3+ extension. Wired through unchanged from
    /// the intent's listener block.
    pub protocol: Proto,
}

/// Actual-side projection: the set of Running allocs for the
/// workload.
///
/// Sourced by the runtime's `hydrate_actual` arm (lands in step
/// 01-03) from
/// `ObservationStore::alloc_status_rows_for_workload(&workload_id)`
/// filtered to `state == Running`. The bridge's reconcile loop
/// reads this set to drive backend-row writes against the configured
/// `host_ipv4` (single-node Phase 2.2 — every Running alloc
/// terminates on the same host's interface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAllocSet {
    /// Workload that owns this Running alloc set. Carried for
    /// symmetry with [`ServiceListenerSet::workload_id`] and so
    /// downstream consumers do not need to thread the workload id
    /// through a separate channel.
    pub workload_id: WorkloadId,
    /// Running alloc identifiers. `BTreeSet` per
    /// `.claude/rules/development.md` § "Ordered-collection choice"
    /// — the bridge's reconcile body iterates this set to assemble
    /// the `Vec<Backend>` it fingerprints, and the fingerprint MUST
    /// be deterministic across DST seeds.
    pub running: BTreeSet<AllocationId>,
}

/// Merged state per ADR-0036 — the runtime stitches the desired and
/// actual projections into one struct before calling `reconcile`.
///
/// The fields' shapes mirror the desired / actual hydration sites in
/// the runtime: [`ServiceListenerSet`] is the desired-side output of
/// `hydrate_desired`, [`RunningAllocSet`] is the actual-side output
/// of `hydrate_actual`. The bridge's `reconcile` body (lands in
/// step 01-02) reads `desired.listeners` cross-producted with
/// `actual.running` to emit one `Action::WriteServiceBackendRow` per
/// drift-detected `(service_id, backend-set)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDiscoveryBridgeState {
    /// Desired-side projection — declared listener set.
    pub desired: ServiceListenerSet,
    /// Actual-side projection — Running alloc set.
    pub actual: RunningAllocSet,
}

impl BackendDiscoveryBridgeState {
    /// Construct an empty state scoped to `workload_id`. Used by the
    /// runtime's hydrate path (lands in step 01-03) as the starting
    /// shape before populating `desired.listeners` and
    /// `actual.running` from `IntentStore` + `ObservationStore`
    /// reads.
    ///
    /// A free function rather than `Default` because the contained
    /// [`WorkloadId`] does NOT implement `Default` (every newtype
    /// in the project rejects empty input per
    /// `.claude/rules/development.md` § "Newtypes — STRICT by
    /// default"); the caller MUST supply a real
    /// `WorkloadId`. This mirrors the construction shape every
    /// other reconciler state struct uses today.
    #[must_use]
    pub fn empty_for_workload(workload_id: WorkloadId) -> Self {
        Self {
            desired: ServiceListenerSet {
                workload_id: workload_id.clone(),
                listeners: BTreeMap::new(),
            },
            actual: RunningAllocSet { workload_id, running: BTreeSet::new() },
        }
    }
}

/// Runtime-persisted typed memory for the bridge per ADR-0035 § 1.
///
/// Carries the per-service fingerprint of the last row the bridge
/// successfully wrote — the canonical *input* per
/// `.claude/rules/development.md` § "Persist inputs, not derived
/// state". The dedup decision ("do we need to write a row this
/// tick?") is recomputed on every tick from this input + the
/// freshly-computed current fingerprint; the bridge never persists
/// a derived "needs write" / "next-write-due-at" boolean.
///
/// # Derives
///
/// `Serialize + Deserialize + Default + Clone` are the four
/// mandatory bounds per ADR-0035 § 1 — the runtime owns CBOR
/// persistence end-to-end and cannot construct the per-target
/// `BTreeMap<TargetResource, View>` snapshot without them.
///
/// `PartialEq + Eq` are additional to the mandatory four:
///
/// - The runtime's Eq-diff skip elides the per-tick `write_through`
///   fsync when the returned `next_view` is equal to the in-memory
///   view — saves one fsync per converged tick.
/// - DST equality assertions (twin-invocation purity checks per
///   ADR-0017 / the `ReconcilerIsPure` invariant) compare returned
///   views directly.
///
/// `#[serde(default)]` on the field is the load-bearing escape hatch
/// for additive schema evolution per ADR-0035 § 6: a V1 reader of a
/// V2-written file (where V2 added a new optional field) MUST
/// tolerate the missing field without error. The CBOR-roundtrip
/// test at `crates/overdrive-core/tests/backend_discovery_bridge_types.rs`
/// pins both properties.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendDiscoveryBridgeView {
    /// Per-service fingerprint of the last `ServiceBackendRow` the
    /// bridge successfully wrote. The bridge's reconcile body
    /// (lands 01-02) compares this against the freshly-computed
    /// fingerprint for the current `(desired, actual)` pair and
    /// emits `Action::WriteServiceBackendRow` only on drift.
    ///
    /// `BTreeMap` per `.claude/rules/development.md` §
    /// "Ordered-collection choice" — iterated by the reconcile
    /// body's GC sweep at the end of each tick (stale `ServiceId`
    /// entries — listeners removed from intent — are dropped) and
    /// observed by DST invariants on every tick.
    #[serde(default)]
    pub last_written_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>,
}

/// The bridge reconciler. Phase 1 lands an empty marker struct so
/// the `AnyReconciler::BackendDiscoveryBridge(_)` variant has a
/// concrete inner type to carry — the `Reconciler` trait impl,
/// the `reconcile` body, and the `host_ipv4: Ipv4Addr` constructor
/// parameter land in step 01-02 alongside the dedup loop.
///
/// Holds only the canonical reconciler name today; the
/// `host_ipv4` field lands in step 01-02 when the boot composition
/// resolves the configured `client_iface` once via `getifaddrs` and
/// hands the resolved IPv4 in at construction time. Phase 2.2 is
/// single-node, so every Running alloc's backend endpoint uses this
/// single IP.
pub struct BackendDiscoveryBridge {
    /// Canonical reconciler name — `Self::NAME`. Constructed via
    /// the validating [`ReconcilerName::new`] in
    /// [`BackendDiscoveryBridge::canonical`].
    #[allow(dead_code, reason = "consumed by the Reconciler trait impl in step 01-02")]
    name: ReconcilerName,
}

impl BackendDiscoveryBridge {
    /// Canonical kebab-case name; single compile-time anchor per
    /// the project's `Reconciler::NAME` convention.
    ///
    /// Exposed as a `pub const` so the runtime-side `static_name()`
    /// dispatch (lands when this variant joins `AnyReconciler`)
    /// can return `&'static str` matching the value the trait impl
    /// (step 01-02) declares via `const NAME: &'static str`.
    pub const NAME: &'static str = "backend-discovery-bridge";

    /// Construct the canonical `backend-discovery-bridge` instance.
    /// Named constructor rather than `Default` because the name is
    /// not defaultable — it carries the canonical string literal.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule. Failure
    /// would indicate a bug in the newtype constructor.
    #[must_use]
    pub fn canonical() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(Self::NAME)
            .expect("'backend-discovery-bridge' is a valid ReconcilerName by construction");
        Self { name }
    }

    /// Accessor used by the `AnyReconciler::name()` dispatch arm
    /// landed in step 01-01. The `Reconciler::name(&self)` trait
    /// method lands in step 01-02 alongside the `impl Reconciler
    /// for BackendDiscoveryBridge` block; until then the dispatch
    /// arm reads the field through this dedicated accessor so the
    /// `AnyReconciler` match stays exhaustive without prematurely
    /// committing the trait surface. Crate-visible so only the
    /// `super` reconciler module dispatch reads it.
    #[must_use]
    pub(crate) const fn canonical_name_for_dispatch(&self) -> &ReconcilerName {
        &self.name
    }
}

impl Default for BackendDiscoveryBridge {
    fn default() -> Self {
        Self::canonical()
    }
}

// NOTE: `impl Reconciler for BackendDiscoveryBridge` lands in step
// 01-02 alongside the `reconcile` body and the `host_ipv4`
// constructor parameter. Landing it here would force the type to
// nominate a `type State = ...` / `type View = ...` pair that the
// reconcile body has nothing to do with yet, and the
// `AnyReconciler::BackendDiscoveryBridge` dispatch arm would either
// `todo!()` (RED-on-greenbar) or land an empty pass-through body
// (forward-incompatible with the 01-02 implementation). The struct
// is a marker today; the trait impl arrives with the dispatch arm
// in step 01-02.
#[doc(hidden)]
const _: fn() = || {
    // Compile-time documentation: the [`Reconciler`] re-export is
    // present so 01-02's `impl Reconciler for BackendDiscoveryBridge`
    // can `use super::Reconciler` (or `use crate::reconciler::Reconciler`)
    // without restructuring the imports landed in this commit.
    const fn _assert_reconciler_in_scope<R: Reconciler + ?Sized>() {}
};
