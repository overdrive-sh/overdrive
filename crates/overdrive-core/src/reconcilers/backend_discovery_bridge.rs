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
//!   per ADR-0035 § 1. **Deliberately field-less** since ADR-0079
//!   § D3: the bridge converges by diffing `desired` against the
//!   `service_backends` rows it manages, so it holds no per-tick
//!   memory. The former `last_written_fingerprint` was an emit-time
//!   marker consulted as the diff — the
//!   `.claude/rules/reconcilers.md` § "Symptoms during review"
//!   anti-pattern — and is deleted, not relocated.
//! - [`BackendDiscoveryBridge`] — the reconciler itself. Both
//!   `host_ipv4` and `writer_node_id` are mandatory constructor
//!   parameters.
//!
//! Per ADR-0035 § 1 the View derives the four mandatory bounds
//! (`Serialize + Deserialize + Default + Clone`) plus `PartialEq + Eq`
//! for the runtime's Eq-diff skip and for DST equality assertions.
//! The CBOR codec is the runtime's choice (ADR-0035 § 3); the test
//! surface at `crates/overdrive-core/tests/backend_discovery_bridge_types.rs`
//! pins that a legacy blob carrying the removed field still decodes.
//!
//! `BTreeMap` / `BTreeSet` per `.claude/rules/development.md` §
//! "Ordered-collection choice" — every keyed map in this module is
//! iterated by the bridge's reconcile loop (lands in 01-02) and DST
//! invariants assert on observed iteration order, so the per-process
//! random hash-seed of `HashMap` is structurally banned.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use crate::SpiffeId;
use crate::dataplane::backend_key::Proto;
use crate::dataplane::fingerprint::fingerprint;
use crate::id::{
    AllocationId, ContentHash, CorrelationKey, NodeId, ServiceId, ServiceVip, WorkloadId,
};
use crate::traits::dataplane::Backend;
use crate::traits::observation_store::{LogicalTimestamp, ObservationRowKind, ServiceBackendRow};

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
    /// Running allocs keyed by id, each carrying the per-alloc
    /// canonical `workload_addr` materialized at provision time
    /// (D-BLOCKER2, GH #241): `Some(addr)` for a Path-A mesh alloc,
    /// `None` for a host-netns / non-Path-A alloc. The bridge selects
    /// the advertised `Backend.addr` source per-alloc from this value
    /// (D-B2) — `Some` advertises `workload_addr:port`, `None` falls
    /// back to `host_ipv4:port`.
    ///
    /// `BTreeMap` (NOT `HashMap`) per
    /// `.claude/rules/development.md` § "Ordered-collection choice"
    /// — the bridge's reconcile body iterates this map to assemble
    /// the `Vec<Backend>` it fingerprints, and the fingerprint MUST
    /// be deterministic across DST seeds.
    pub running: BTreeMap<AllocationId, Option<Ipv4Addr>>,
}

/// Merged state per ADR-0036 — the runtime stitches the desired and
/// actual projections into one struct before calling `reconcile`.
///
/// The bridge's `reconcile` body reads `desired.listeners`
/// cross-producted with `actual.running` to compute the row each
/// service should carry, then diffs that against the OBSERVED row in
/// [`Self::service_backends`] — the resource the bridge actually
/// manages (ADR-0079 § D1 / `.claude/rules/reconcilers.md` Bar 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDiscoveryBridgeState {
    /// Desired-side projection — declared listener set.
    pub desired: ServiceListenerSet,
    /// Second desired-side INPUT — the Running alloc set. Despite the
    /// name this is not the resource the bridge manages; it is one of
    /// the two inputs from which `desired` is computed. Retained
    /// unchanged so the field's meaning is not silently redefined.
    pub actual: RunningAllocSet,
    /// The `service_backends` rows this bridge manages, keyed by
    /// `ServiceId` — the genuine `actual` per
    /// `.claude/rules/reconcilers.md` Bar 1.
    ///
    /// Populated ONLY in the `actual` projection (ADR-0021 uses one
    /// type for both halves; the `desired` projection leaves this
    /// empty, exactly as `hydrate_actual` leaves `desired.listeners`
    /// empty).
    ///
    /// `BTreeMap` per `.claude/rules/development.md` §
    /// "Ordered-collection choice".
    pub service_backends: BTreeMap<ServiceId, ServiceBackendRow>,
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
            actual: RunningAllocSet { workload_id, running: BTreeMap::new() },
            service_backends: BTreeMap::new(),
        }
    }
}

/// Runtime-persisted typed memory for the bridge per ADR-0035 § 1.
///
/// **Deliberately empty.** The bridge converges by diffing `desired`
/// against the `service_backends` rows it manages (ADR-0079 § D2), so
/// it holds no per-tick memory. The former
/// `last_written_fingerprint` was an emit-time marker consulted as the
/// diff — the `.claude/rules/reconcilers.md` § "Symptoms during
/// review" anti-pattern — and is deleted, not relocated.
///
/// The type is retained rather than collapsed to `()` so a future
/// bridge-side retry or backoff policy lands without re-wiring the
/// runtime's `AnyViewMap` / `AnyReconcilerView` variants. Precedent:
/// `WorkflowLifecycleView`, which is likewise zero-sized and fully
/// wired.
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
///   view. With a field-less View that gate now always
///   short-circuits, so the bridge never writes through again;
///   legacy rows stay on disk, inert.
/// - DST equality assertions (twin-invocation purity checks per
///   ADR-0017 / the `ReconcilerIsPure` invariant) compare returned
///   views directly.
///
/// Removing the field is the *removal* direction of serde's
/// unknown-field tolerance (ciborium encodes a struct as a
/// string-keyed map and this type does not set `deny_unknown_fields`),
/// so a persisted `{"last_written_fingerprint": {...}}` blob still
/// decodes. Pinned by `legacy_bridge_view_blob_decodes_to_empty_view`
/// in `crates/overdrive-core/tests/backend_discovery_bridge_types.rs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendDiscoveryBridgeView {}

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

    /// Row-backed: the bridge converges `service_backends` rows for a
    /// workload's listeners against the running alloc set, so an accepted
    /// `alloc_status` transition (a Running → Failed that drops a backend, or
    /// a fresh Running that adds one) must wake it (ADR-0081 §5 single-cut
    /// migration — the declarative replacement for the deleted `exit_observer`
    /// producer-push). It authors no `alloc_status` rows and converges by
    /// reading the row it manages back (ADR-0079), so the interest is
    /// loop-free.
    fn interests(&self) -> &'static [ObservationRowKind] {
        &[ObservationRowKind::AllocStatus]
    }

    /// Pure-sync per ADR-0035 — NO `.await`, NO `Instant::now()` /
    /// `SystemTime::now()`, NO direct IntentStore / ObservationStore
    /// / ViewStore writes, NO DB handle.
    ///
    /// Per ADR-0079 § D2 the bridge CONVERGES — it diffs against the
    /// row it manages, never against a memo of what it emitted:
    ///
    /// 1. Loop over `desired.desired.listeners`.
    /// 2. Look up the OBSERVED row at `actual.service_backends`.
    /// 3. Build `backends: Vec<Backend>` from `actual.actual.running`,
    ///    carrying each backend's `healthy` through from the observed
    ///    row (the bridge does not author that field).
    /// 4. `continue` when the observed row already equals
    ///    `(vip, backends)`; otherwise emit
    ///    `Action::WriteServiceBackendRow` + the UI-05
    ///    `EnqueueEvaluation` handoff.
    ///
    /// The View is field-less and returned unchanged (§ D3); retry
    /// after a dropped write is carried by the runtime's `has_work`
    /// self-re-enqueue, which is true on exactly the ticks that
    /// emitted a write.
    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        view: &Self::View,
        tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        let mut actions: Vec<Action> = Vec::new();

        for (service_id, listener) in &desired.desired.listeners {
            // The genuine `actual`: the row this bridge manages, or None.
            let observed: Option<&ServiceBackendRow> = actual.service_backends.get(service_id);

            // Build backend set — one `Backend` per Running alloc. The
            // SpiffeId derives from the canonical
            // `SpiffeId::for_allocation(workload, alloc)` constructor
            // (ADR-0067 D5) — the single derivation the reconciler module
            // routes through.
            let backends: Vec<Backend> = actual
                .actual
                .running
                .iter()
                .map(|(alloc_id, workload_addr)| {
                    let alloc = SpiffeId::for_allocation(&actual.actual.workload_id, alloc_id);
                    // `healthy` is authored by ServiceLifecycle's readiness
                    // branch (`service_lifecycle.rs`), NOT by this bridge.
                    // Carry the observed value through so convergence does
                    // not erase it; default `true` for an alloc with no
                    // observed entry, preserving the backward-compat value
                    // for a newly-Running alloc (ADR-0079 § D2; ownership
                    // of the field is recorded in § D9).
                    let healthy = observed
                        .and_then(|row| row.backends.iter().find(|b| b.alloc == alloc))
                        .is_none_or(|b| b.healthy);
                    Backend {
                        // D-B2 (GH #241): advertise the canonical per-alloc
                        // `workload_addr` when present (Path-A mesh alloc),
                        // else fall back to `host_ipv4` (host-netns /
                        // non-Path-A alloc). The addr is the materialized
                        // value read off the observation row at hydrate
                        // time (D-BLOCKER2); the bridge never recomputes it
                        // from `NetSlot`.
                        //
                        // ADR-0079 § D8 standing constraint: this
                        // expression is mirrored byte-for-byte by
                        // `hydrate_service_alloc_facts` in
                        // `reconciler_runtime.rs`. The two writers agree on
                        // `addr` only while they stay identical — change
                        // one and you MUST change the other.
                        alloc,
                        addr: SocketAddr::new(
                            IpAddr::V4(workload_addr.unwrap_or(self.host_ipv4)),
                            listener.port.get(),
                        ),
                        weight: 1,
                        healthy,
                    }
                })
                .collect();

            let vip_v4 = vip_to_ipv4(&listener.vip);

            // Converge: diff desired against the OBSERVED row. `updated_at`
            // is excluded — it is the LWW stamp, not part of desired.
            if let Some(row) = observed
                && row.vip == vip_v4
                && row.backends == backends
            {
                continue;
            }

            // `fingerprint` survives as the correlation content-address
            // (its documented role), NOT as the diff.
            let fp = fingerprint(&listener.vip, &backends);
            let target = format!("backend-discovery-bridge/{service_id}");
            let spec_hash = ContentHash::of(fp.to_le_bytes().as_slice());
            let correlation =
                CorrelationKey::derive(&target, &spec_hash, "write-service-backend-row");

            actions.push(Action::WriteServiceBackendRow {
                row: ServiceBackendRow {
                    service_id: *service_id,
                    vip: vip_v4,
                    backends,
                    // ADR-0077 § D2 site 9: derive the LWW counter from the
                    // prior row, not from the tick, so a post-restart write
                    // dominates whatever survived.
                    updated_at: LogicalTimestamp::dominating(
                        tick.tick,
                        self.writer_node_id.clone(),
                        observed.map(|r| &r.updated_at),
                    ),
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
        }

        (actions, view.clone())
    }
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
        state.actual.running.insert(alloc_id("alloc-a"), None);
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
        assert_eq!(row.updated_at.counter, 8, "counter = tick.tick + 1 with no prior row");
        assert_eq!(row.updated_at.writer, node_id());
        assert_eq!(
            next_view,
            BackendDiscoveryBridgeView::default(),
            "the field-less View is returned unchanged (ADR-0079 § D3)"
        );
    }

    /// T-BDB-CONV-1 — **the whole ADR's falsifiable claim.** A write
    /// the store discarded is retried on the next tick.
    ///
    /// The dropped write is modelled exactly as it appears to the
    /// reconciler: `actual.service_backends` still shows no row. Under
    /// the deleted emit-fingerprint design the second call emitted
    /// ZERO actions and the drop was permanently forgotten
    /// (ADR-0079 § Context / RCA § 4.3).
    #[test]
    fn bridge_reemits_when_observed_row_does_not_match_desired() {
        let bridge = bridge();
        let sid = service_id(2);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 2), 9000));
        state.actual.running.insert(alloc_id("alloc-b"), None);

        let view = BackendDiscoveryBridgeView::default();

        let (actions_first, view_after_first) = bridge.reconcile(&state, &state, &view, &tick(1));
        assert_eq!(actions_first.len(), 2, "first call must emit two actions (UI-05 dual emit)");

        // The store discarded the write — `actual.service_backends` is
        // STILL empty. Identical state, identical (empty) view.
        let (actions_second, _) = bridge.reconcile(&state, &state, &view_after_first, &tick(2));

        assert_eq!(
            actions_second.len(),
            2,
            "a dropped write MUST be retried on the next tick; got {} action(s): {:?}",
            actions_second.len(),
            actions_second
        );
    }

    /// T-BDB-CONV-2 — convergence terminates. With the observed row
    /// equal to what the bridge would emit, zero actions are emitted.
    /// Pins the absence of a busy loop (the runtime's `has_work`
    /// self-re-enqueue is driven by action emission).
    #[test]
    fn bridge_emits_nothing_when_observed_row_matches_desired() {
        let bridge = bridge();
        let sid = service_id(2);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 2), 9000));
        state.actual.running.insert(alloc_id("alloc-b"), None);

        // Seed `actual` with the row the bridge computed on tick 1.
        let (first, _) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));
        let Action::WriteServiceBackendRow { row, .. } = &first[0] else {
            panic!("expected WriteServiceBackendRow at index 0, got {:?}", first[0]);
        };
        state.service_backends.insert(sid, row.clone());

        let (actions, _) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(2));

        assert!(
            actions.is_empty(),
            "an observed row matching desired must emit zero actions; got {actions:?}"
        );
    }

    /// T-BDB-CONV-3 — the § D2 regression guard. The bridge must not
    /// clobber the `healthy` value it does not author.
    ///
    /// The observed row carries `healthy: false` at an `addr` of
    /// `host_ipv4` — the shape `ServiceLifecycle`'s readiness branch
    /// writes. The bridge rewrites (the addr drifted), and the
    /// rewritten row MUST still carry `healthy: false`.
    #[test]
    fn bridge_carries_observed_healthy_through_on_rewrite() {
        let bridge = bridge();
        let sid = service_id(6);
        let alloc = alloc_id("alloc-h");
        let mesh_addr = Ipv4Addr::new(10, 99, 0, 6);
        let mut state = empty_state();
        state.desired.listeners.insert(sid, listener(Ipv4Addr::new(10, 1, 0, 6), 8080));
        state.actual.running.insert(alloc.clone(), Some(mesh_addr));

        state.service_backends.insert(
            sid,
            ServiceBackendRow {
                service_id: sid,
                vip: Ipv4Addr::new(10, 1, 0, 6),
                backends: vec![Backend {
                    alloc: SpiffeId::for_allocation(&workload_id(), &alloc),
                    // `host_ipv4`, not the mesh addr — the drift trigger.
                    addr: SocketAddr::new(IpAddr::V4(host_ip()), 8080),
                    weight: 1,
                    healthy: false,
                }],
                updated_at: LogicalTimestamp::dominating(0, node_id(), None),
            },
        );

        let (actions, _) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(5));

        assert_eq!(actions.len(), 2, "addr drift must trigger a rewrite; got {actions:?}");
        let Action::WriteServiceBackendRow { row, .. } = &actions[0] else {
            panic!("expected WriteServiceBackendRow at index 0, got {:?}", actions[0]);
        };
        assert_eq!(
            row.backends[0].addr,
            SocketAddr::new(IpAddr::V4(mesh_addr), 8080),
            "the rewrite must advertise the canonical workload_addr"
        );
        assert!(
            !row.backends[0].healthy,
            "the observed `healthy: false` MUST be carried through, not clobbered with `true`"
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
        state.actual.running.insert(alloc_id("alloc-x"), None);
        state.actual.running.insert(alloc_id("alloc-y"), None);
        state.actual.running.insert(alloc_id("alloc-z"), None);
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
        state.actual.running.insert(alloc_id("alloc-m"), None);
        state.actual.running.insert(alloc_id("alloc-n"), None);

        // First tick — write with two backends. UI-05 dual emit.
        let (actions_first, _) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(1));
        assert_eq!(actions_first.len(), 2, "first tick emits write + enqueue");
        let Action::WriteServiceBackendRow { row: first_row, .. } = &actions_first[0] else {
            panic!("expected WriteServiceBackendRow at index 0");
        };
        // The write landed — the bridge now observes its own row.
        state.service_backends.insert(sid, first_row.clone());

        // Second tick — one alloc terminated; expect a fresh row
        // with one backend plus the paired enqueue.
        state.actual.running.remove(&alloc_id("alloc-n"));
        let (actions_second, _) =
            bridge.reconcile(&state, &state, &BackendDiscoveryBridgeView::default(), &tick(2));

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
}
