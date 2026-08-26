//! [`HydrationContext`] and [`HydrateError`] — the hydration boundary types
//! (ADR-0086 D1/D3/D5).
//!
//! ADR-0086 reverses the ADR-0036 "runtime owns all hydration" ruling for the
//! intent + observation half: reconcilers own their hydration as the async
//! `Reconciler::hydrate_desired` / `hydrate_actual` trait methods. Those methods
//! read every fact they need through [`HydrationContext`] — a borrow-bundle of
//! the injected read-ports + already-core stores/registry + plain data — instead
//! of reaching concrete `AppState` fields. The control-plane composition root
//! builds one `HydrationContext` per tick and passes it to
//! `AnyReconciler::hydrate_*` (wired in step 03; at 02-01 nothing calls the
//! hydrate methods yet — they are `todo!` scaffolds).
//!
//! **S1 acceptance invariant (ADR-0086 D5):** `HydrationContext` carries a
//! handle for EVERY surface any `hydrate_*` body reads, and NO more — post
//! ADR-0087 there is NO restart-budget surface (the cross-read was removed at
//! its root). The "read every `hydrate_*` body" audit is the evidence; the
//! S-ROH-B-09 structural test enumerates the field set against the ADR-0086 D5
//! read-surface list.

use std::net::Ipv4Addr;
use std::path::Path;

use crate::id::NodeId;
use crate::traits::driver::DriverRegistry;
use crate::traits::intent_store::IntentStore;
use crate::traits::observation_store::ObservationStore;
use crate::traits::vm_host_state::VmHostState;
use crate::traits::{HeldSvidView, ListenerFacts, ServiceVipView, WorkflowLiveSet};

/// The borrow-bundle a reconciler's `hydrate_desired` / `hydrate_actual` reads
/// its facts through (ADR-0086 D5).
///
/// Every field is a surface some moved `hydrate_*` body reads:
///
/// * [`IntentStore`] / [`ObservationStore`] — already-core stores (`get` /
///   `scan_prefix` / the `*_rows` observation reads).
/// * [`DriverRegistry`] — already-core; `Driver::live_allocations` for VM
///   supervision (`state.drivers.get(Vm).live_allocations()`).
/// * [`VmHostState`] — already-core; the vm-reclamation host observation
///   (`state.vm_host_state.observe()`).
/// * [`ListenerFacts`] / [`ServiceVipView`] / [`WorkflowLiveSet`] /
///   [`HeldSvidView`] — the four NEW narrow read-ports (ADR-0086 D5) that make
///   the previously-concrete hydration surfaces DST-injectable.
/// * `node_id` / `host_ipv4` / `intent_redb_path` — plain data threaded in (not
///   traits).
///
/// It is a **borrow bundle**: no owned allocation, no clone of the underlying
/// state — the composition root lends each handle for the duration of one tick.
/// There is deliberately NO restart-budget surface — ADR-0087 (single restart
/// authority) eliminated the cross-reconciler restart-budget read at its root,
/// so no `RestartBudgetView` port exists.
pub struct HydrationContext<'a> {
    /// Intent SSOT reads (`get` / `scan_prefix`).
    pub intent_store: &'a dyn IntentStore,
    /// Observation reads (the `*_rows` surfaces).
    pub observation_store: &'a dyn ObservationStore,
    /// Driver registry — `Driver::live_allocations` for VM supervision.
    pub drivers: &'a DriverRegistry,
    /// Host observation for vm-reclamation (`observe()`).
    pub vm_host_state: &'a dyn VmHostState,
    /// Per-`ServiceId` listener-fact read port (ADR-0086 D5).
    pub listener_facts: &'a dyn ListenerFacts,
    /// Assigned-VIP read port over the allocator memo (ADR-0086 D5).
    pub service_vip_view: &'a dyn ServiceVipView,
    /// Live-workflow-instance snapshot read port (ADR-0086 D5).
    pub workflow_live_set: &'a dyn WorkflowLiveSet,
    /// Global node-held SVID snapshot read port (ADR-0086 D5).
    pub held_svid_view: &'a dyn HeldSvidView,
    /// The local node id (plain data).
    pub node_id: &'a NodeId,
    /// The local host IPv4 (plain data).
    pub host_ipv4: Ipv4Addr,
    /// Path to the intent redb file (plain data — some hydrate bodies need the
    /// on-disk path, not just the store handle).
    pub intent_redb_path: &'a Path,
}

/// The hydration-boundary error (ADR-0086 D3), returned by
/// `Reconciler::hydrate_desired` / `hydrate_actual` and forwarded by
/// `AnyReconciler::hydrate_*`.
///
/// Variants mirror the failure modes the current central hydrate bodies already
/// produce (`ConvergenceError::{IntentRead, ObservationRead, IntentDecode}` in
/// `reconciler_runtime.rs`). The runtime converts at the call site via a
/// `ConvergenceError: From<HydrateError>` `#[from]` variant — added in step 03
/// where the runtime call site is wired, so the tick loop keeps consuming
/// `ConvergenceError`. (At 02-01 nothing calls hydrate, so no `From` impl is
/// wired yet.)
///
/// **Name note:** ADR-0036 §Consequences retired an *older* `HydrateError` (the
/// per-reconciler libSQL read error). This is a **new, distinct** type reusing
/// the name — the hydration-boundary error, not the libSQL read error; the two
/// never coexist.
#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    /// An `IntentStore` read failed during hydration.
    #[error("intent read failed: {0}")]
    IntentRead(String),
    /// An `ObservationStore` read failed during hydration.
    #[error("observation read failed: {0}")]
    ObservationRead(String),
    /// A persisted intent failed to decode through its rkyv-envelope codec.
    /// Intent is the load-bearing SSOT (ADR-0048 §3 asymmetry): an undecodable
    /// intent REFUSES — it is NOT log-and-skipped like an observation row.
    #[error("intent decode failed: {0}")]
    IntentDecode(String),
    /// The [`TargetResource`](crate::reconcilers::TargetResource) a hydrate body
    /// was asked to project did not carry a well-formed id for that reconciler's
    /// shape (e.g. `workload/<invalid>` / `service/<invalid>`). This mirrors the
    /// pre-move central `ConvergenceError::TargetShape` the free-fn hydrate
    /// bodies produced from `workload_id_from_target` / `service_id_from_target`;
    /// it is a distinct failure mode from a store read, so it carries its own
    /// variant (per `.claude/rules/development.md` § "Errors").
    #[error("invalid target resource: {0}")]
    TargetShape(String),
}
