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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::SpiffeId;
use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use crate::id::{
    AllocationId, ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip, WorkloadId,
};
use crate::traits::dataplane::Backend;
use crate::traits::observation_store::{LogicalTimestamp, ServiceBackendRow};

use super::{Action, Reconciler, ReconcilerName, TargetResource, TickContext};

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

/// The bridge reconciler — step 01-02 lands the full struct +
/// `impl Reconciler` body per architecture.md § 4.2.
///
/// Both `host_ipv4` and `writer_node_id` are MANDATORY constructor
/// parameters per `.claude/rules/development.md` § "Port-trait
/// dependencies" — required, not defaulted. The host IPv4 is
/// resolved once at boot via `getifaddrs` on the configured
/// `client_iface` (Phase 2.2 is single-node, so every Running alloc's
/// backend endpoint uses this IP); the writer node id is the local
/// node's identity, stamped onto every emitted `LogicalTimestamp`
/// for LWW tiebreaking.
pub struct BackendDiscoveryBridge {
    /// Canonical reconciler name — `Self::NAME`. Constructed via
    /// the validating [`ReconcilerName::new`] in
    /// [`BackendDiscoveryBridge::new`].
    name: ReconcilerName,
    /// Host IPv4 for backend endpoint construction. Phase 2.2
    /// single-node: every Running alloc resolves to this IP.
    host_ipv4: Ipv4Addr,
    /// Local node id, stamped onto every emitted
    /// [`LogicalTimestamp`] for LWW tiebreaking.
    writer_node_id: NodeId,
}

/// Canonical name of the `service-map-hydrator` reconciler — the
/// downstream sibling the bridge re-enqueues on every
/// `WriteServiceBackendRow` emission (UI-05 cross-reconciler handoff).
///
/// Compile-time alias to `<ServiceMapHydrator as Reconciler>::NAME` —
/// a rename of the hydrator's `NAME` constant without updating this
/// reference is a compile error, not a silent handoff failure.
const SERVICE_MAP_HYDRATOR_NAME: &str =
    <super::service_map_hydrator::ServiceMapHydrator as Reconciler>::NAME;

impl BackendDiscoveryBridge {
    /// Canonical kebab-case name; single compile-time anchor per
    /// the project's `Reconciler::NAME` convention.
    pub const NAME: &'static str = "backend-discovery-bridge";

    /// Construct a bridge bound to a host IPv4 + writer node id.
    /// Both parameters are MANDATORY (no defaulted constructor) per
    /// `.claude/rules/development.md` § "Port-trait dependencies"
    /// — the runtime composes them at boot.
    ///
    /// # Panics
    ///
    /// Never — `Self::NAME` is a compile-time string literal
    /// satisfying every `ReconcilerName` validation rule.
    #[must_use]
    pub fn new(host_ipv4: Ipv4Addr, writer_node_id: NodeId) -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(Self::NAME)
            .expect("'backend-discovery-bridge' is a valid ReconcilerName by construction");
        Self { name, host_ipv4, writer_node_id }
    }
}

impl Reconciler for BackendDiscoveryBridge {
    const NAME: &'static str = "backend-discovery-bridge";

    type State = BackendDiscoveryBridgeState;
    type View = BackendDiscoveryBridgeView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    /// Pure-sync per ADR-0035 — NO `.await`, NO `Instant::now()` /
    /// `SystemTime::now()`, NO direct IntentStore / ObservationStore
    /// / ViewStore writes, NO DB handle.
    ///
    /// Per architecture.md § 4.2:
    ///
    /// 1. Loop over `desired.desired.listeners`.
    /// 2. Build `backends: Vec<Backend>` from `actual.actual.running`
    ///    (one `Backend` per Running alloc; every alloc resolves to
    ///    `self.host_ipv4` in Phase 2.2 single-node).
    /// 3. Compute `new_fp = fingerprint(&listener.vip, &backends)`.
    /// 4. Dedup against `view.last_written_fingerprint.get(service_id)`;
    ///    emit `Action::WriteServiceBackendRow` if different.
    /// 5. GC: shrink `next_view.last_written_fingerprint` to the
    ///    service-ids still present in `desired.listeners`.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();
        let mut next_view = view.clone();

        for (service_id, listener) in &desired.desired.listeners {
            // Build backend set — one `Backend` per Running alloc.
            // Phase 2.2 single-node: every alloc resolves to
            // `self.host_ipv4`. The SpiffeId derives from the
            // canonical `mint_identity(workload_id, alloc_id)` shape
            // used everywhere else in the reconciler module.
            let backends: Vec<Backend> = actual
                .actual
                .running
                .iter()
                .map(|alloc_id| Backend {
                    alloc: mint_alloc_identity(&actual.actual.workload_id, alloc_id),
                    addr: SocketAddr::new(IpAddr::V4(self.host_ipv4), listener.port.get()),
                    weight: 1,
                    healthy: true, // GH #170 ships real health
                })
                .collect();

            let new_fp = fingerprint(&listener.vip, &backends);
            let prev_fp = view.last_written_fingerprint.get(service_id).copied();

            if Some(new_fp) == prev_fp {
                // Dedup: no change since last successful write.
                continue;
            }

            let vip_v4 = vip_to_ipv4(&listener.vip);
            let target = format!("backend-discovery-bridge/{service_id}");
            let spec_hash = ContentHash::of(new_fp.to_le_bytes().as_slice());
            let correlation =
                CorrelationKey::derive(&target, &spec_hash, "write-service-backend-row");

            actions.push(Action::WriteServiceBackendRow {
                row: ServiceBackendRow {
                    service_id: *service_id,
                    vip: vip_v4,
                    backends,
                    updated_at: LogicalTimestamp {
                        counter: tick.tick.saturating_add(1),
                        writer: self.writer_node_id.clone(),
                    },
                },
                correlation,
            });
            // UI-05 — cross-reconciler handoff at the action boundary.
            // The bridge wrote a row the `service-map-hydrator` needs
            // to re-tick against. Emitting `EnqueueEvaluation` here
            // (rather than the action-shim auto-enqueueing on
            // `WriteServiceBackendRow`) keeps the handoff explicit at
            // the reconciler surface: any reader of the bridge's
            // reconcile body sees the bridge → hydrator handoff
            // without having to read the action-shim dispatch source.
            // The broker is LWW at
            // `(ReconcilerName, TargetResource)` per ADR-0013 §8 / §18,
            // so duplicate enqueues collapse to one dispatch per drain
            // cycle.
            //
            // `expect`: both `ReconcilerName::new("service-map-hydrator")`
            // and `TargetResource::new("service/<u64>")` are
            // constructor-time validated against compile-time-known
            // patterns; failure would indicate a constructor regression,
            // not a runtime concern.
            #[allow(clippy::expect_used)]
            {
                let hydrator_name = ReconcilerName::new(SERVICE_MAP_HYDRATOR_NAME)
                    .expect("'service-map-hydrator' is a valid ReconcilerName by construction");
                let hydrator_target = TargetResource::new(&format!("service/{service_id}"))
                    .expect("'service/<u64>' is a valid TargetResource by construction");
                actions.push(Action::EnqueueEvaluation {
                    reconciler: hydrator_name,
                    target: hydrator_target,
                });
            }
            next_view.last_written_fingerprint.insert(*service_id, new_fp);
        }

        // GC: drop dedup entries for services no longer in desired.
        next_view
            .last_written_fingerprint
            .retain(|sid, _| desired.desired.listeners.contains_key(sid));

        (actions, next_view)
    }
}

/// Derive a SpiffeId for a Running alloc — pure function over
/// `(workload_id, alloc_id)`. Matches the project-wide shape used by
/// `mint_identity` in the reconciler module.
fn mint_alloc_identity(workload_id: &WorkloadId, alloc_id: &AllocationId) -> SpiffeId {
    let raw = format!(
        "spiffe://overdrive.local/job/{}/alloc/{}",
        workload_id.as_str(),
        alloc_id.as_str()
    );
    #[allow(clippy::expect_used)]
    SpiffeId::new(&raw).expect("derived SpiffeId is valid by construction")
}

/// Project a [`ServiceVip`] to the IPv4 wire shape used by
/// `ServiceBackendRow`. Phase 2.2 ships IPv4-only per ADR-0049 § 5;
/// the `None` arm is structurally unreachable. The newtype's own
/// docstring on [`ServiceVip::try_as_ipv4`] documents the contract.
fn vip_to_ipv4(vip: &ServiceVip) -> Ipv4Addr {
    // mutants: skip — the unwrap_or branch is structurally
    // unreachable in Phase 1: the allocator's `VipRange` is IPv4-only
    // per ADR-0049 § 5. IPv6 admission is tracked in GH #155.
    vip.try_as_ipv4().unwrap_or(Ipv4Addr::UNSPECIFIED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wall_clock::UnixInstant;
    use std::time::{Duration, Instant};

    fn workload_id() -> WorkloadId {
        WorkloadId::new("payments").expect("'payments' is a valid WorkloadId")
    }

    fn node_id() -> NodeId {
        NodeId::new("node-1").expect("'node-1' is a valid NodeId")
    }

    fn host_ip() -> Ipv4Addr {
        Ipv4Addr::new(10, 0, 0, 5)
    }

    fn alloc_id(suffix: &str) -> AllocationId {
        AllocationId::new(suffix).expect("alloc id is valid")
    }

    fn service_id(value: u64) -> ServiceId {
        ServiceId::new(value).expect("ServiceId accepts any u64")
    }

    fn service_vip(addr: Ipv4Addr) -> ServiceVip {
        ServiceVip::new(IpAddr::V4(addr)).expect("ServiceVip accepts IPv4")
    }

    fn listener(addr: Ipv4Addr, port: u16) -> ProjectedListener {
        ProjectedListener {
            vip: service_vip(addr),
            port: NonZeroU16::new(port).expect("port must be non-zero"),
            protocol: Proto::Tcp,
        }
    }

    fn tick(counter: u64) -> TickContext {
        TickContext {
            now: Instant::now(),
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(counter)),
            tick: counter,
            deadline: Instant::now() + Duration::from_secs(1),
        }
    }

    fn empty_state() -> BackendDiscoveryBridgeState {
        BackendDiscoveryBridgeState::empty_for_workload(workload_id())
    }

    fn bridge() -> BackendDiscoveryBridge {
        BackendDiscoveryBridge::new(host_ip(), node_id())
    }

    /// S-BDB-09 unit-level proxy — empty desired set emits zero
    /// actions and leaves the view unchanged.
    #[test]
    fn reconcile_empty_listeners_emits_zero_actions() {
        let bridge = bridge();
        let state = empty_state();
        let view = BackendDiscoveryBridgeView::default();

        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick(1));

        assert!(actions.is_empty(), "empty listener set must emit zero actions");
        assert_eq!(view, next_view, "view must be unchanged when no listeners exist");
    }

    /// S-BDB-02 unit-level proxy + UI-05 dual-emit assertion —
    /// single listener + single Running alloc + empty view emits
    /// exactly two actions: one `WriteServiceBackendRow` carrying
    /// one backend, plus one `EnqueueEvaluation` for the
    /// `service-map-hydrator` keyed by `service/<sid>`. The next-view
    /// records the fingerprint. The dual emission is the UI-05
    /// architectural fix that makes the bridge → hydrator handoff
    /// explicit at the action boundary.
    #[test]
    fn reconcile_single_alloc_emits_write_and_enqueue() {
        let bridge = bridge();
        let sid = service_id(1);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 1), 8080));
        state.actual.running.insert(alloc_id("alloc-a"));
        let view = BackendDiscoveryBridgeView::default();

        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick(7));

        assert_eq!(
            actions.len(),
            2,
            "exactly two actions expected: WriteServiceBackendRow + EnqueueEvaluation \
             (UI-05 cross-reconciler handoff)"
        );
        let Action::WriteServiceBackendRow { row, .. } = &actions[0] else {
            panic!("expected WriteServiceBackendRow at index 0, got {:?}", actions[0]);
        };
        let Action::EnqueueEvaluation { reconciler, target } = &actions[1] else {
            panic!("expected EnqueueEvaluation at index 1, got {:?}", actions[1]);
        };
        assert_eq!(
            reconciler.as_str(),
            "service-map-hydrator",
            "enqueue must target the service-map-hydrator per UI-05"
        );
        assert_eq!(
            target.as_str(),
            &format!("service/{sid}"),
            "enqueue target must be service/<service_id>"
        );
        assert_eq!(row.service_id, sid);
        assert_eq!(row.vip, Ipv4Addr::new(10, 1, 0, 1));
        assert_eq!(row.backends.len(), 1, "single Running alloc yields one backend");
        assert_eq!(
            row.backends[0].addr,
            SocketAddr::new(IpAddr::V4(host_ip()), 8080),
            "backend addr must be host_ipv4:listener.port"
        );
        assert_eq!(row.updated_at.counter, 8, "counter = tick.tick + 1");
        assert_eq!(row.updated_at.writer, node_id());
        assert!(
            next_view.last_written_fingerprint.contains_key(&sid),
            "next_view must record fingerprint for the written service"
        );
    }

    /// S-BDB-05 unit-level proxy — dedup branch. Same inputs + a
    /// view-from-prior-call emits zero actions.
    #[test]
    fn reconcile_dedup_branch_emits_zero_actions_on_unchanged_inputs() {
        let bridge = bridge();
        let sid = service_id(2);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 2), 9000));
        state.actual.running.insert(alloc_id("alloc-b"));

        // First tick — write happens, next_view records fingerprint.
        // UI-05: dual emit — one WriteServiceBackendRow + one
        // EnqueueEvaluation per drifted service.
        let (actions_first, view_after_first) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));
        assert_eq!(actions_first.len(), 2, "first call must emit two actions (UI-05 dual emit)");

        // Second tick — feed prior next_view back in; expect zero
        // emissions (dedup).
        let (actions_second, view_after_second) =
            bridge.reconcile(&state, &state, &view_after_first, &tick(2));

        assert!(
            actions_second.is_empty(),
            "second call with unchanged inputs + prior view must emit zero actions"
        );
        assert_eq!(view_after_first, view_after_second, "dedup must not mutate the view");
    }

    /// S-BDB-07 unit-level proxy — GC branch. Removing a service
    /// from `desired.listeners` shrinks `next_view.last_written_fingerprint`.
    #[test]
    fn reconcile_gc_branch_drops_removed_service_id() {
        let bridge = bridge();
        let stale_sid = service_id(99);

        // Seed the view with a fingerprint for a service no longer in
        // desired.
        let mut view = BackendDiscoveryBridgeView::default();
        view.last_written_fingerprint.insert(stale_sid, 0xdead_beef_u64);

        let state = empty_state(); // no listeners

        let (actions, next_view) = bridge.reconcile(&state, &state, &view, &tick(1));

        assert!(actions.is_empty(), "no listeners means no actions");
        assert!(
            !next_view.last_written_fingerprint.contains_key(&stale_sid),
            "GC must drop fingerprint entries for services no longer in desired"
        );
    }

    /// S-BDB-04 unit-level proxy — N Running allocs produce
    /// `backends.len() == N`.
    #[test]
    fn reconcile_multi_replica_emits_all_backends() {
        let bridge = bridge();
        let sid = service_id(3);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 3), 8080));
        state.actual.running.insert(alloc_id("alloc-x"));
        state.actual.running.insert(alloc_id("alloc-y"));
        state.actual.running.insert(alloc_id("alloc-z"));
        let view = BackendDiscoveryBridgeView::default();

        let (actions, _) = bridge.reconcile(&state, &state, &view, &tick(1));

        // UI-05 dual emit: WriteServiceBackendRow + EnqueueEvaluation.
        assert_eq!(actions.len(), 2, "one row + one enqueue regardless of backend count");
        let Action::WriteServiceBackendRow { row, .. } = &actions[0] else {
            panic!("expected WriteServiceBackendRow at index 0");
        };
        assert_eq!(row.backends.len(), 3, "three Running allocs yield three backends");
        assert!(
            matches!(&actions[1], Action::EnqueueEvaluation { .. }),
            "second action must be EnqueueEvaluation for hydrator handoff"
        );
    }

    /// S-BDB-03 unit-level proxy — terminated alloc. After
    /// converging on a Running set, dropping a Running alloc on the
    /// next tick emits a fresh row with the remaining backend(s)
    /// only.
    #[test]
    fn reconcile_terminated_alloc_drops_backend() {
        let bridge = bridge();
        let sid = service_id(4);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 4), 8080));
        state.actual.running.insert(alloc_id("alloc-m"));
        state.actual.running.insert(alloc_id("alloc-n"));

        // First tick — write with two backends. UI-05 dual emit.
        let (actions_first, view_after_first) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));
        assert_eq!(actions_first.len(), 2, "first tick emits write + enqueue");

        // Second tick — one alloc terminated; expect a fresh row
        // with one backend plus the paired enqueue.
        state.actual.running.remove(&alloc_id("alloc-n"));
        let (actions_second, _) = bridge.reconcile(&state, &state, &view_after_first, &tick(2));

        assert_eq!(actions_second.len(), 2, "removed alloc must trigger a fresh write + enqueue");
        let Action::WriteServiceBackendRow { row, .. } = &actions_second[0] else {
            panic!("expected WriteServiceBackendRow at index 0");
        };
        assert_eq!(row.backends.len(), 1, "after termination, only one backend remains");
        assert!(
            matches!(&actions_second[1], Action::EnqueueEvaluation { .. }),
            "second action must be EnqueueEvaluation for hydrator handoff"
        );
    }

    /// Fingerprint determinism — same inputs across multiple
    /// invocations produce the same fingerprint. Proxy for the
    /// architecture-mandated "deterministic across runs" property.
    #[test]
    fn fingerprint_deterministic_across_runs() {
        let bridge = bridge();
        let sid = service_id(5);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 5), 8080));
        state.actual.running.insert(alloc_id("alloc-determ"));
        let view = BackendDiscoveryBridgeView::default();

        let (_, view_a) = bridge.reconcile(&state, &state, &view, &tick(1));
        let (_, view_b) = bridge.reconcile(&state, &state, &view, &tick(1));

        assert_eq!(
            view_a.last_written_fingerprint.get(&sid),
            view_b.last_written_fingerprint.get(&sid),
            "fingerprint MUST be deterministic across reconcile invocations"
        );
    }
}
