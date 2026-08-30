//! Action shim — the single async I/O boundary in the convergence
//! loop. Per ADR-0023.
//!
//! The shim consumes `Vec<Action>` emitted by the reconciler runtime
//! (after `reconcile` returns), dispatches allocation-management
//! actions to `&dyn Driver`, and writes resulting `AllocStatusRow`s
//! to `&dyn ObservationStore`. All `.await` points in the
//! post-reconcile pipeline live here — `reconcile` itself is
//! synchronous + pure per ADR-0013.
//!
//! # Module path
//!
//! Per ADR-0023 §1, the canonical module path is
//! `overdrive_control_plane::reconciler_runtime::action_shim`. The
//! existing `reconciler_runtime` is currently a single .rs file;
//! during DELIVER's first refactor pass, it becomes a directory and
//! this module is re-exported from inside it. For Phase 1 the shim
//! lives at the crate root as `action_shim` and is re-exported under
//! the canonical path via `pub mod` in lib.rs.

use std::sync::Arc;

use overdrive_core::TransitionReason;
use overdrive_core::eval_broker::EvaluationBroker;
use overdrive_core::id::{AllocationId, ContentHash, CorrelationKey, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::ca::Ca;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Dataplane;
use overdrive_core::traits::driver::{
    AllocationHandle, AllocationSpec, Driver, DriverError, DriverPayload, DriverRegistry,
    DriverStartClass, DriverStartFailure, DriverType, VmStartFailure,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, CrashFacts, LogicalTimestamp, ObservationRow, ObservationStore,
    ObservationStoreError,
};
use overdrive_core::traits::vm_host_state::VmHostState;
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_dataplane::allocators::{PersistentAllocatorError, PersistentServiceVipAllocator};
use tokio::sync::broadcast;

use crate::api::{AllocStateWire, TransitionSource};
use crate::identity_mgr::IdentityMgr;
use crate::journal::WorkflowId;
// transparent-mtls-enrollment (D-TME-12 G1/G2/G3 + JOIN; step 04-01) — the C3
// lifecycle wiring: per-host slot allocator, slot→plan derivation, the
// gateway-as-responder helper, and the netns provision/teardown executors.
use crate::veth_provisioner::{
    NetSlotAllocator, NetSlotExhausted, VethProvisionError, VmTapPlan, WorkloadNetnsPlan,
    derive_vm_tap_plan, derive_workload_netns_plan, provision_vm_tap, provision_workload_netns,
    responder_addr_for_slot, teardown_workload_netns,
};
use crate::workflow_runtime::WorkflowEngine;
// transparent-mtls-host-socket (D-MTLS-16/17, GH #26; step 06-03) — the
// (β) lifecycle component the shim fires alongside the driver hooks.
use overdrive_worker::mtls_intercept_worker::{MtlsInterceptInstallError, MtlsInterceptWorker};

const fn exec_release_permitted(
    running_committed: bool,
    intercept_required: bool,
    stable_exact_rule_baseline: bool,
) -> bool {
    running_committed && (!intercept_required || stable_exact_rule_baseline)
}

fn is_duplicate_vm_owner(failure: &DriverStartFailure, alloc: &AllocationId) -> bool {
    matches!(
        &failure.class,
        DriverStartClass::Vm(VmStartFailure::AllocationAlreadyOwned { alloc: owner })
            if owner == alloc
    )
}

fn is_start_cleanup_pending_row(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Pending
        && matches!(row.reason, Some(TransitionReason::DriverInternalError { .. }))
}

async fn ensure_intercept_identity(
    alloc_id: &AllocationId,
    workload_id: &WorkloadId,
    node_id: &NodeId,
    ca: &dyn Ca,
    observation: &dyn ObservationStore,
    clock: &dyn Clock,
    identity: &IdentityMgr,
) -> Result<(), ShimError> {
    if identity.held_snapshot().contains_key(alloc_id) {
        return Ok(());
    }
    let spiffe_id = SpiffeId::for_allocation(workload_id, alloc_id);
    let target = format!("svid-lifecycle/{alloc_id}");
    let spec_hash = ContentHash::of(spiffe_id.as_str().as_bytes());
    let action = Action::IssueSvid {
        alloc_id: alloc_id.clone(),
        spiffe_id,
        node_id: node_id.clone(),
        correlation: CorrelationKey::derive(&target, &spec_hash, "issue-svid"),
    };
    issue_svid::dispatch_issue(&action, ca, observation, clock, identity)
        .await
        .map_err(ShimError::from)
}

/// Per-arm dispatch for `Action::DataplaneUpdateService`. See
/// module docstring of [`dataplane_update_service`] for the
/// failure-surface contract per architecture.md § 7.
pub mod dataplane_update_service;

/// Per-arm dispatch for `Action::ReleaseServiceVip` per ADR-0049
/// (amended 2026-05-15). See module docstring of
/// [`release_service_vip`] for the lock discipline + idempotency
/// contract (service-vip-allocator step 03-02).
pub mod release_service_vip;

/// Per-arm dispatch for `Action::WriteServiceBackendRow` per
/// `docs/feature/backend-discovery-bridge-service-reachability/
/// design/architecture.md` § 4.4. The wrapper writes the row to the
/// ObservationStore; the bridge's next tick observes its own write
/// via the dedup fingerprint in [`BackendDiscoveryBridgeView`].
///
/// [`BackendDiscoveryBridgeView`]:
///     overdrive_reconcilers::backend_discovery_bridge::BackendDiscoveryBridgeView
pub mod write_service_backend_row;

/// Per-arm dispatch for `Action::EnqueueEvaluation` per UI-05 (the
/// `backend-discovery-bridge-service-reachability` step 02-04
/// architectural remediation). The wrapper submits an
/// `Evaluation { reconciler, target }` to the runtime's
/// [`EvaluationBroker`] so the named downstream reconciler ticks
/// against `target` on the next convergence cycle.
pub mod enqueue_evaluation;

/// Per-arm dispatch for `Action::RegisterLocalBackend` per ADR-0053
/// § 3. Invokes `Dataplane::register_local_backend` so the
/// cgroup_sock_addr program rewrites subsequent
/// `connect(vip:vip_port)` calls to the resolved backend address.
pub mod register_local_backend;

/// Per-arm dispatch for `Action::DeregisterLocalBackend` per ADR-0053
/// § 3. Invokes `Dataplane::deregister_local_backend` to remove the
/// LOCAL_BACKEND_MAP entry. Idempotent per the ADR-0053 § 2
/// trait contract.
pub mod deregister_local_backend;

/// Per-arm dispatch for `Action::IssueSvid` / `Action::DropSvid` per
/// ADR-0067 D3 — the ONE place workload-CA I/O happens. `IssueSvid`
/// mints the leaf + writes the `issued_certificates` audit row + holds
/// the material in `IdentityMgr` (audit-before-hold, K4 fail-closed);
/// `DropSvid` removes the held entry (O2/K2). See the module docstring
/// on [`issue_svid::dispatch_issue`] for the audit-before-hold contract.
pub mod issue_svid;

/// `VmReclamation` executor(s) (ADR-0083 §D7, brief.md §105a.5, GH #42).
/// See the module docstring on [`reclamation::execute_discard_stranded_artifacts`]
/// for the scope of what this step implements.
pub mod reclamation;

/// Reconcile-output invariant validator — rejects post-`reconcile`
/// `Vec<Action>` returns that contain two or more write-actions
/// targeting the same service-LB VIP (see the module docstring on
/// [`validate`] for the full conflict taxonomy and fail-safe
/// semantics).
pub mod validate;

/// SCAFFOLD marker.
pub const SCAFFOLD: bool = false;

// ---------------------------------------------------------------------------
// LifecycleEvent — broadcast-channel payload for slice 02 streaming
// ---------------------------------------------------------------------------

/// Internal broadcast-channel payload emitted by the action shim after
/// every successful `AllocStatusRow` write.
///
/// Per architecture.md §10 (cli-submit-vs-deploy-and-alloc-status DESIGN):
/// `LifecycleEvent` is a wire-shape projection of the row write event.
/// It does NOT carry the raw `AllocStatusRow` directly — a trybuild
/// compile-fail fixture (architecture.md §8) pins this invariant. The
/// fields are typed projections (`AllocStateWire` for from/to,
/// `TransitionReason` for the cause-class, `TransitionSource` for who
/// produced the row).
///
/// `LifecycleEvent` is the broadcast payload, NOT the wire type. The
/// per-kind streaming event (`JobSubmitEvent` / `ServiceSubmitEvent`)
/// is constructed FROM a `LifecycleEvent` by the streaming loop. For
/// that reason `LifecycleEvent` derives only `Debug + Clone` — NOT
/// `Serialize`/`Deserialize`/`ToSchema`. That property is what the
/// trybuild fixture defends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleEvent {
    /// Allocation this transition concerns.
    pub alloc_id: AllocationId,
    /// Job the allocation belongs to.
    pub workload_id: WorkloadId,
    /// Wire-shape state the alloc was in before this transition. The
    /// shim does not currently track this on every write (it's the
    /// downstream consumer's job to compute `from` from prior state);
    /// in step 02-01 this carries the row's *new* state in both `from`
    /// and `to` for the row-write events the shim emits without prior
    /// context. Step 02-03's streaming handler refines this against
    /// per-alloc prior-state tracking when it lands.
    pub from: AllocStateWire,
    /// Wire-shape state the alloc moved to.
    pub to: AllocStateWire,
    /// Structured cause-class for this transition.
    pub reason: TransitionReason,
    /// Verbatim driver text the cause-class payload does not capture
    /// (e.g. raw `errno`-decorated message). Audit trail per
    /// architecture.md §10.
    pub detail: Option<String>,
    /// Who/what produced the row write — `Reconciler` or
    /// `Driver(DriverType)` per ADR-0033 §1.
    pub source: TransitionSource,
    /// Logical-timestamp string (counter@writer) for this transition.
    pub at: String,
    /// Reconciler-decided terminal claim per ADR-0037 §4. Carries the
    /// SAME value the action shim wrote onto `AllocStatusRow.terminal`
    /// in the same dispatch call frame. Drift between the two surfaces
    /// is structurally impossible — both are populated from the
    /// originating `Action.terminal` field at one source site.
    /// `None` means "this transition is not terminal" (e.g. a
    /// Pending → Running success, a mid-budget Failed transition, an
    /// exit-observer-emitted exit event whose terminal classification
    /// is the reconciler's job to make on a subsequent tick).
    pub terminal: Option<TerminalCondition>,
}

// ---------------------------------------------------------------------------
// The driver-start text grammar is RETIRED (DWD-24 / ADR-0032 §4
// amendment 2026-08-16).
//
// `classify_driver_failure` and its `split_once_after_path` helper are
// DELETED, not retained behind a compatibility path: drivers now author a
// typed `DriverStartFailure`, and this layer applies the total, pure,
// core-owned `TransitionReason::from(&failure)` conversion. The shim
// performs no parsing, no prefix matching, no path extraction, and no
// `DriverType` dispatch to select a cause.
//
// The structural guard against regression is the `xtask::dst_lint` clause
// that rejects both a `reason: String` field on `StartRejected` and any
// function named `classify_driver_failure` in this crate.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// dispatch — single async I/O boundary, with broadcast emit
// ---------------------------------------------------------------------------

/// Build an `AllocStatusRow` for a state transition driven by the shim.
/// Used by every variant that writes observation: `StartAllocation`,
/// `RestartAllocation`, `StopAllocation`, and `FinalizeFailed` all funnel
/// through this helper so the row shape is constructed in exactly one
/// place. Pure over its inputs — does not touch the observation store.
///
/// Per ADR-0032 §3 (Amendment 2026-04-30) the row carries
/// `reason: Option<TransitionReason>` and `detail: Option<String>`
/// for cause-class attribution.
///
/// Per ADR-0037 §4 the row carries `terminal: Option<TerminalCondition>`
/// — the reconciler-emitted classification of *why* an allocation
/// reached a terminal lifecycle state. The dispatch arm passes the
/// `Action.terminal` value through here so the row's durable surface
/// and the broadcasted `LifecycleEvent.terminal` derived in
/// `build_lifecycle_event` BOTH come from the same Action-derived value
/// — drift between the two surfaces is structurally impossible.
//
// 9 row fields are intentional — the row is the durable wire shape and
// adding indirection would add noise without simplifying the call sites;
// ADR-0032 §3 + ADR-0037 §4 + slice 02-06 (stderr_tail propagation)
// each grew this list deliberately.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_alloc_status_row(
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: NodeId,
    state: AllocState,
    // The row's LWW stamp, a REQUIRED parameter so every writer must decide
    // it explicitly: `LogicalTimestamp::dominating(tick.tick, node_id, prior)`
    // where `prior` is the `updated_at` of the row currently stored at this
    // alloc's key (`None` only when genuinely no row exists). Deriving it
    // from the tick ALONE is what let a superseding `Failed` row silently
    // inherit the counter of the `Running` row it had to dominate (GH #250),
    // and what made every write for a surviving alloc lose the LWW merge for
    // the whole post-restart window (ADR-0077) — the same required-parameter
    // discipline `workload_addr` got for GH #248.
    updated_at: LogicalTimestamp,
    reason: Option<TransitionReason>,
    detail: Option<String>,
    terminal: Option<TerminalCondition>,
    stderr_tail: Option<String>,
    kind: overdrive_core::aggregate::WorkloadKind,
    // Per the subsidiary fix to GAP-1: wall-clock at the
    // Pending → Running transition. Captured ONCE when this row
    // records a `state == AllocState::Running` for the first time;
    // preserved verbatim by every subsequent arm by reading the
    // prior row and forwarding the value. Typed `UnixInstant` —
    // unit + origin are encoded in the type. See the
    // `AllocStatusRowV1::started_at` docstring on the
    // input-vs-derived discipline.
    started_at: Option<overdrive_core::UnixInstant>,
    // The canonical per-instance workload address (GH #241 /
    // AllocStatusRowV2), a REQUIRED parameter so every writer must decide
    // it explicitly: `Some(addr)` for a Running mesh alloc that owns a
    // per-instance address, `None` for a host-netns alloc or any
    // non-Running (Failed / Terminated) row. Making it a parameter rather
    // than a defaulted `None` field kills the forgot-to-forward-carry bug
    // class — a successor / terminal writer can no longer silently drop the
    // prior row's address (the dial-by-name walking-skeleton backend-drop;
    // the residual host_ipv4-fallback masking is tracked in #248).
    workload_addr: Option<std::net::Ipv4Addr>,
    // ADR-0078 § D2. REQUIRED: the row this write supersedes at this alloc's
    // LWW key (`None` ONLY when genuinely no row exists). The builder derives
    // `last_terminated` and `restart_count` from it via
    // `CrashFacts::advance(prior, state)` — using the SAME `state` it writes
    // onto the row, so the two cannot be computed against different states.
    // Callers never construct a `CrashFacts`. Same required-parameter
    // discipline `updated_at` / `started_at` / `workload_addr` already carry:
    // the compiler enumerates every writer, and the forget-to-forward bug
    // class (which has bitten twice) cannot recur silently.
    prior: Option<&AllocStatusRow>,
) -> AllocStatusRow {
    // ADR-0078 § D1 / § D2: the two crash-observability fields are computed
    // together, here, from the SAME `state` this builder writes onto the row.
    // No call site names a `CrashFacts`, so "the facts were derived against a
    // different state than the row carries" is not expressible.
    let facts = CrashFacts::advance(prior, state);
    if let Some(superseded) = prior
        && facts.restart_count > superseded.restart_count
    {
        // The single increment site in the system — the only place a restart
        // is observed landing. Emitted here so a crash-and-recover is
        // *alertable*, not merely pollable (ADR-0078 § D2).
        tracing::info!(
            name: "alloc.restart.observed",
            alloc = %alloc_id,
            workload = %workload_id,
            restart_count = facts.restart_count,
            prior_state = %superseded.state,
            "allocation recovered from a terminal observation",
        );
    }
    AllocStatusRow {
        alloc_id,
        workload_id,
        node_id,
        state,
        updated_at,
        reason,
        detail,
        terminal,
        // Per ADR-0033 Amendment 2026-05-10 / slice 02-05 the
        // observation row's `stderr_tail` field carries the workload's
        // stderr verbatim (the `ExitObserver` boundary populates it
        // for crashed exits). Per slice 02-06 the action shim's
        // `FinalizeFailed` arm propagates this forward so the typed
        // terminal row inherits the prior attempt's stderr — without
        // this, the streaming layer's terminal `Failed` projection
        // sees `stderr_tail: None` even when the workload wrote to
        // stderr before exiting. Other arms still pass `None` (no
        // stderr was observed at those write sites).
        stderr_tail,
        kind,
        listeners: Vec::new(),
        started_at,
        // canonical-workload-address-inbound-tproxy (GH #241 /
        // AllocStatusRowV2): supplied by the REQUIRED `workload_addr`
        // parameter (above) — every writer decides it explicitly rather
        // than the builder defaulting `None` and relying on callers to
        // remember a post-build copy. StartAllocation / RestartAllocation
        // pass `spec.workload_addr` on the Running write; FinalizeFailed's
        // Stable (still-Running) arm forward-carries `prior_row.workload_addr`;
        // genuine-terminal / Failed / Terminated rows pass `None` (a
        // non-Running alloc is not a live backend). See #248 for the
        // residual host_ipv4-fallback masking.
        workload_addr,
        // ADR-0078 § D1 / § D2: both derived by `CrashFacts::advance` above
        // from the `prior` row and THIS row's `state`. Destructured onto the
        // two fields as a unit so they cannot drift.
        last_terminated: facts.last_terminated,
        restart_count: facts.restart_count,
    }
}

/// Build a `LifecycleEvent` for the broadcast channel from a freshly
/// written `AllocStatusRow`. The wire-shape projection is mechanical —
/// `state → AllocStateWire`, `LogicalTimestamp → String`. `prior_state`
/// carries the actual allocation state before this transition; `from` is
/// set to `prior_state` so the event correctly reflects the transition
/// direction. Each call site reads the prior obs row and passes it here;
/// `StartAllocation` defaults to `Pending` for first-seen allocs.
///
/// Per ADR-0037 §4: the event's `terminal` field is byte-equal to the
/// row's `terminal` field — both are populated from the originating
/// `Action.terminal` value in the same dispatch call frame, so drift is
/// structurally impossible.
fn build_lifecycle_event(
    row: &AllocStatusRow,
    prior_state: AllocStateWire,
    source: TransitionSource,
) -> LifecycleEvent {
    let to_wire: AllocStateWire = row.state.into();
    LifecycleEvent {
        alloc_id: row.alloc_id.clone(),
        workload_id: row.workload_id.clone(),
        from: prior_state,
        to: to_wire,
        reason: row
            .reason
            .clone()
            .unwrap_or(TransitionReason::DriverInternalError { detail: String::new() }),
        detail: row.detail.clone(),
        source,
        at: format_logical_timestamp(&row.updated_at),
        terminal: row.terminal.clone(),
    }
}

/// Fail-closed handling for a per-alloc transparent-mTLS intercept-install
/// failure (D-MTLS-18 mechanism (a)). Shared by the `StartAllocation` and
/// `RestartAllocation` arms.
///
/// The alloc has just committed a `Running` `AllocStatusRow` and the driver
/// process is spawned, but `MtlsInterceptWorker::start_alloc` returned `Err`
/// — the alloc cannot run with cleartext egress/ingress, so it MUST be driven
/// terminal. This:
///
/// 1. Stops the just-spawned driver process (`driver.stop`, best-effort like
///    the `RestartAllocation` stop half — a `NotFound` is tolerated).
/// 2. Writes a superseding `Failed` `AllocStatusRow` carrying
///    [`TransitionReason::MtlsInterceptInstallFailed`] (`stage` = the install
///    step that failed; `detail` = the verbatim error `Display`) — mirroring
///    the existing `StartRejected → Failed` precedent. The superseding row's
///    LWW stamp comes from [`LogicalTimestamp::dominating`] against the
///    `Running` row it replaces: both rows are written in the SAME tick by
///    the SAME node, so a tick-derived counter would tie and the `Failed`
///    row would be silently dropped, leaving the alloc durably recorded
///    `Running` with no interception installed (GH #250 / ADR-0076
///    § Decision 7, generalised by ADR-0077 § D1).
/// 3. Emits the lifecycle event for the `Failed` transition.
///
/// It does NOT call `driver.release_for_exit_emission` — both call sites
/// invoke this and `return` BEFORE the release, so the Running-gate /
/// exit-observer watcher is never released for a now-`Failed` alloc (the
/// existing Failed-branch rule).
///
/// Returns `Ok(())`: the dispatch itself succeeded (the alloc is durably
/// recorded `Failed`), exactly as the `StartRejected → Failed` arm returns
/// `Ok(())` after writing its Failed row. The obs-store write is the one
/// fallible step propagated as `ShimError`.
#[allow(clippy::too_many_arguments)]
async fn fail_closed_on_mtls_install(
    driver: &dyn Driver,
    obs: &dyn ObservationStore,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    running_row: &AllocStatusRow,
    prior_state: AllocStateWire,
    handle: Option<&AllocationHandle>,
    cause: &MtlsInterceptInstallError,
) -> Result<(), ShimError> {
    // Stop the just-spawned driver process so the workload does not keep
    // running uninstrumented. Best-effort: a `NotFound` (already gone) is
    // tolerated, mirroring the `RestartAllocation` stop half.
    if let Some(handle) = handle {
        let _ = driver.stop(handle).await;
    }

    let reason = TransitionReason::MtlsInterceptInstallFailed {
        stage: cause.stage().to_owned(),
        detail: cause.to_string(),
    };
    // Supersede the `Running` row with a `Failed` row. Preserve the alloc's
    // identity + kind + `started_at` verbatim from the just-written row; only
    // the state + reason + detail change. `terminal: None` — like the
    // `StartRejected → Failed` arm, a single mid-budget install failure is not
    // a terminal claim (WorkloadLifecycle owns the BackoffExhausted terminal).
    let failed_row = build_alloc_status_row(
        running_row.alloc_id.clone(),
        running_row.workload_id.clone(),
        running_row.node_id.clone(),
        AllocState::Failed,
        LogicalTimestamp::dominating(
            tick.tick,
            running_row.node_id.clone(),
            Some(&running_row.updated_at),
        ),
        Some(reason),
        Some(cause.to_string()),
        None,
        None,
        running_row.kind,
        running_row.started_at,
        // Failed row supersedes the prior Running row — a failed alloc is
        // not a live backend (the bridge renders only `state == Running`),
        // so it carries no per-instance address.
        None,
        // ADR-0078 § D2 site 1: the row this write supersedes at the alloc's
        // LWW key is the `Running` row this same dispatch frame just wrote
        // (the fail-closed supersession shape). `advance` FORWARDS both crash
        // fields — the prior is `Running`, so no snapshot and no increment.
        Some(running_row),
    );
    obs.write(ObservationRow::AllocStatus(Box::new(failed_row.clone()))).await?;
    emit_event(bus, build_lifecycle_event(&failed_row, prior_state, TransitionSource::Reconciler));
    Ok(())
}

/// Classify a [`ShimError`] surfaced from the C3 PROVISION SEAM
/// ([`provision_and_inject_netns`]) into the
/// [`TransitionReason::WorkloadNetnsProvisionFailed`] cause-class, or `None`
/// when the error is NOT a provision-seam failure (a different `dispatch_single`
/// error that should propagate as `Err` unchanged).
///
/// The two provision-seam error variants map to the closed `stage` vocabulary:
/// - [`ShimError::NetSlotExhausted`] → `"net_slot_assign"` (no free slot)
/// - [`ShimError::WorkloadNetnsProvision`] → `"netns_provision"` (the typed
///   netns/veth/tap kernel provisioning boundary failed)
///
/// `detail` carries the verbatim `Display` of the underlying error so the
/// operator sees the privilege / capacity remediation. Mirrors
/// [`MtlsInterceptInstallError::stage`]'s closed-vocabulary shape.
fn netns_provision_cause(err: &ShimError) -> Option<TransitionReason> {
    let stage = match err {
        ShimError::NetSlotExhausted(_) => "net_slot_assign",
        ShimError::WorkloadNetnsProvision(_) => "netns_provision",
        _ => return None,
    };
    Some(TransitionReason::WorkloadNetnsProvisionFailed {
        stage: stage.to_owned(),
        detail: err.to_string(),
    })
}

/// Fail-closed handling for a per-alloc netns/veth PROVISION failure at the C3
/// seam (transparent-mtls-enrollment D-TME-12 / AC14, step 04-01). Shared by the
/// `StartAllocation` and `RestartAllocation` arms.
///
/// Unlike [`fail_closed_on_mtls_install`] — which fires AFTER a `Running` row is
/// committed and so must STOP the spawned driver and SUPERSEDE that row — this
/// fires at the PRE-`Running` provision seam: the provision precedes
/// `Driver::start`, so nothing was spawned and there is no `Running` row to
/// supersede. It writes a FRESH `Failed` `AllocStatusRow` carrying the
/// [`TransitionReason::WorkloadNetnsProvisionFailed`] cause-class (so a
/// persistent provision failure — slot exhaustion, EPERM creating the
/// netns/veth — reaches the `Failed` terminal instead of looping `Pending`
/// forever as the reconciler re-emits StartAllocation each tick) and emits the
/// lifecycle event for the transition.
///
/// `started_at` is `None`: the alloc never reached Running, mirroring the
/// `StartRejected → Failed` arm's `None`. `terminal: None`: a single provision
/// failure is not a terminal claim — the `WorkloadLifecycle` reconciler owns the
/// `BackoffExhausted` terminal across attempts (same rationale as the
/// `StartRejected → Failed` and mTLS-install fail-closed arms).
///
/// Returns `Ok(())`: like the `StartRejected → Failed` arm, the dispatch itself
/// succeeded (the alloc is durably recorded `Failed`); the obs-store write is
/// the one fallible step propagated as `ShimError`. Returning `Ok` (not the
/// provision `Err`) is the whole point — it stops the indefinite Pending retry
/// loop the bare `?` produced.
#[allow(clippy::too_many_arguments)]
async fn fail_closed_on_netns_provision(
    obs: &dyn ObservationStore,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: NodeId,
    kind: overdrive_core::aggregate::WorkloadKind,
    prior_state: AllocStateWire,
    cause: TransitionReason,
    // The row currently stored at this alloc's key, or `None` iff none
    // exists. REQUIRED (no default, no builder) so the caller must decide it
    // explicitly — the same discipline `build_alloc_status_row`'s
    // `updated_at` / `workload_addr` / `prior` carry. A tick-derived stamp
    // here loses the LWW merge for the entire post-restart window (ADR-0077
    // § D2 site 1, correcting ADR-0076 § 7c's "any prior row is from an
    // EARLIER tick" premise).
    //
    // ADR-0078 § D2 site 2 REPLACES ADR-0077 § D2 site 1's
    // `prior_updated_at: Option<&LogicalTimestamp>` with the whole row —
    // strictly more informative, and the stamp is derived from it internally
    // below. Carrying BOTH would be two parameters derived from one row, with
    // a standing risk they disagree.
    prior: Option<&AllocStatusRow>,
) -> Result<(), ShimError> {
    let detail = cause.human_readable();
    let updated_at =
        LogicalTimestamp::dominating(tick.tick, node_id.clone(), prior.map(|r| &r.updated_at));
    let failed_row = build_alloc_status_row(
        alloc_id,
        workload_id,
        node_id,
        AllocState::Failed,
        updated_at,
        Some(cause),
        Some(detail),
        None,
        None,
        kind,
        None,
        // Pre-Running provision failure — the alloc never reached Running
        // and owns no per-instance address.
        None,
        // ADR-0078 § D2 site 2: `advance` FORWARDS both crash fields — the
        // prior is `Running` / `Pending` / absent, never terminal, so this
        // pre-Running failure neither snapshots nor increments.
        prior,
    );
    obs.write(ObservationRow::AllocStatus(Box::new(failed_row.clone()))).await?;
    emit_event(bus, build_lifecycle_event(&failed_row, prior_state, TransitionSource::Reconciler));
    Ok(())
}

/// Render a `LogicalTimestamp` as `counter@writer` for the wire/event
/// surface. Phase 1 keeps it stringly-typed because the CLI renders it
/// verbatim and never round-trips through arithmetic.
fn format_logical_timestamp(ts: &LogicalTimestamp) -> String {
    format!("{}@{}", ts.counter, ts.writer.as_str())
}

/// Emit a `LifecycleEvent` on the broadcast channel. Per
/// architecture.md §10: broadcast-send error is logged and discarded —
/// the row was already committed, the snapshot will see it, and a
/// missing event signals a missing subscriber (not a missed write).
/// Per-variant error isolation is preserved: a broadcast send failure
/// does not abort subsequent action dispatch.
fn emit_event(bus: &broadcast::Sender<LifecycleEvent>, event: LifecycleEvent) {
    if let Err(err) = bus.send(event) {
        // No subscribers is the normal Phase 1 case (the streaming
        // handler in 02-03 may not be active yet); demote to debug so
        // the no-subscriber path does not spam the log.
        tracing::debug!(
            target: "overdrive::action_shim",
            err = %err,
            "lifecycle event broadcast send returned error (no subscribers?); ignored",
        );
    }
}

/// Alloc → driver-kind index (ADR-0083 §D2a(b), GH #42). `StopAllocation`
/// and `FinalizeFailed` carry no `spec` (and hence no `DriverPayload`), so
/// the shim cannot re-derive which driver started the allocation from the
/// action alone. Written on the `StartAllocation` / `RestartAllocation`
/// arms (where the payload IS in hand) and read on every stop/terminal
/// arm. `parking_lot::Mutex`, per `.claude/rules/development.md` §
/// "Concurrency & async": lock → clone the `DriverType` → drop the guard
/// → THEN `.await` the resolved driver call — the read sites all
/// immediately `.await` a driver method, the textbook "never hold a lock
/// across `.await`" trap.
pub type AllocDriverIndex =
    parking_lot::Mutex<std::collections::BTreeMap<AllocationId, DriverType>>;

/// Driven boundary for the C3 host-network mutation.
///
/// Production dispatch uses [`HostNetworkProvisioner`], which converges the
/// real netns/veth/TAP resources. Component compositions may substitute this
/// port while still exercising the same slot derivation, complete
/// [`AllocationSpec`] injection, driver selection, supervision claim, and
/// observation write. A substitute is therefore not permission to omit C3:
/// it acknowledges the exact derived plans and the shim still injects every
/// field before `Driver::start`.
pub trait WorkloadNetworkProvisioner: Send + Sync {
    /// Converge the derived workload plan and optional VM TAP plan.
    ///
    /// # Errors
    ///
    /// Returns the exact typed provision failure authored by the adapter.
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError>;

    /// Tear down the derived workload plan.
    ///
    /// # Errors
    ///
    /// Returns the exact typed teardown failure authored by the adapter.
    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError>;
}

#[derive(Debug, Default)]
struct HostNetworkProvisioner;

impl WorkloadNetworkProvisioner for HostNetworkProvisioner {
    fn provision(
        &self,
        workload: &WorkloadNetnsPlan,
        vm_tap: Option<&VmTapPlan>,
    ) -> Result<(), VethProvisionError> {
        provision_workload_netns(workload)?;
        if let Some(tap) = vm_tap {
            provision_vm_tap(workload, tap)?;
        }
        Ok(())
    }

    fn teardown(&self, workload: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
        teardown_workload_netns(workload)
    }
}

/// Resolve the driver(s) a stop/terminal arm should act on for `alloc`.
///
/// The common case is an index hit: exactly the one driver that started
/// this allocation. When the index has NO entry — an allocation whose
/// `Running`/lifecycle state was established without a corresponding
/// `StartAllocation`/`RestartAllocation` dispatch (e.g. a still-Running
/// alloc surviving a `serve` restart with a freshly-empty per-boot index,
/// or a driver's lifecycle hook exercised directly in a test) — this
/// falls back to EVERY composed driver rather than silently no-op'ing.
/// Every `Driver::stop`/`on_alloc_terminal`/`on_alloc_stable` call is
/// already documented best-effort / idempotent for an alloc the driver
/// does not track (`DriverError::NotFound` is tolerated; the lifecycle
/// hooks default to a no-op), so broadcasting is safe — and it is the
/// only way to guarantee the hook/stop still reaches the right driver
/// when the index cannot say which one that is.
fn resolve_drivers_for_alloc<'a>(
    drivers: &'a DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    alloc: &AllocationId,
) -> Vec<&'a Arc<dyn Driver>> {
    let known_kind = alloc_drivers.lock().get(alloc).copied();
    known_kind.and_then(|kind| drivers.get(kind)).map_or_else(
        || drivers.kinds().filter_map(|kind| drivers.get(kind)).collect(),
        |driver| vec![driver],
    )
}

/// Dispatch a reconciler's emitted `Vec<Action>` against the composed
/// driver registry and observation store. Called by the runtime's tick
/// loop after every `reconcile` call.
///
/// Per ADR-0023 §2 (amended by ADR-0083 §D1/§D2a for the registry, GH
/// #42):
/// - Takes `&DriverRegistry` (not a single `&dyn Driver`) and
///   `&dyn ObservationStore` (NOT Arc; the caller holds the Arcs).
/// - Each [`Action`] variant gets its own match arm; the compiler
///   enforces exhaustiveness across the [`Action`] enum.
/// - A driver `StartRejected` writes a `Failed` [`AllocStatusRow`]
///   (ADR-0032 §5: distinguishes "operator stopped" from "driver
///   could not start") and returns `Ok(())` — the failure is
///   *recorded*, not surfaced as [`ShimError`].
/// - [`ShimError`] is reserved for failures the shim cannot resolve
///   into an observation row (e.g. observation store itself broken).
///
/// Per architecture.md §10: every successful `obs.write(row)` is
/// followed by `bus.send(event)` against the broadcast channel. The
/// send error is logged and discarded; a failed send does not abort
/// subsequent action dispatch (per-variant error isolation).
///
/// # Errors
///
/// Returns [`ShimError::Driver`] only when the underlying error is not
/// representable as an [`AllocStatusRow`]. Returns
/// [`ShimError::Observation`] when the observation store rejects the
/// write itself.
#[allow(
    clippy::too_many_arguments,
    reason = "Action-shim ports (Driver, ObservationStore, Dataplane, LifecycleEvent bus, ServiceVipAllocator, NetSlotAllocator) are required at dispatch per .claude/rules/development.md § Port-trait dependencies; bundling into a struct would make individual deps optional and defeat the explicit-injection invariant."
)]
pub async fn dispatch(
    actions: Vec<Action>,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    ca: &dyn Ca,
    clock: &dyn Clock,
    identity: &IdentityMgr,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
    mtls_worker: Option<&Arc<MtlsInterceptWorker>>,
    net_slot_allocator: &NetSlotAllocator,
    host: &dyn VmHostState,
) -> Result<(), ShimError> {
    dispatch_with_network_provisioner(
        actions,
        drivers,
        alloc_drivers,
        obs,
        dataplane,
        ca,
        clock,
        identity,
        bus,
        tick,
        writer_node,
        allocator,
        broker,
        workflow_engine,
        mtls_worker,
        net_slot_allocator,
        &HostNetworkProvisioner,
        host,
    )
    .await
}

/// Dispatch through an explicitly supplied C3 network provisioner.
///
/// This is the supported component-composition form of [`dispatch`]. It keeps
/// C3 mandatory and changes only the driven host-mutation adapter, allowing a
/// non-root deterministic composition to acknowledge the complete derived
/// assignment before the VM-shaped driver is exercised.
///
/// # Errors
///
/// Identical to [`dispatch`], including typed network-provision failures.
#[allow(
    clippy::too_many_arguments,
    reason = "This explicit composition root adds the required C3 driven port to dispatch's existing required port set."
)]
pub async fn dispatch_with_network_provisioner(
    actions: Vec<Action>,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    ca: &dyn Ca,
    clock: &dyn Clock,
    identity: &IdentityMgr,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
    mtls_worker: Option<&Arc<MtlsInterceptWorker>>,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
    host: &dyn VmHostState,
) -> Result<(), ShimError> {
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        let result = dispatch_single(
            action,
            drivers,
            alloc_drivers,
            obs,
            dataplane,
            ca,
            clock,
            identity,
            bus,
            tick,
            writer_node,
            &allocator,
            broker,
            workflow_engine,
            mtls_worker,
            net_slot_allocator,
            network_provisioner,
            host,
        )
        .await;
        if let Err(err) = result {
            // Per-variant error isolation: record only the first error
            // and continue draining the rest of the actions.
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
    }

    first_error.map_or(Ok(()), Err)
}

/// Pre-flight: persist workflow-instance desired-intent for every
/// `Action::StartWorkflow` in `actions`, with **per-action isolation**.
///
/// For each action:
/// - `StartWorkflow`: attempt
///   `put(IntentKey::for_workflow_instance(correlation), spec.name bytes)`.
///   - `Ok`  → the action is pushed into the returned `dispatchable` set;
///     its desired-intent is now durable, so it is safe to drive the
///     engine for it.
///   - `Err` → the failure is recorded into `first_error` (only if no
///     earlier error exists, mirroring [`dispatch`]'s first-error
///     aggregation), and this `StartWorkflow` is **DROPPED** from the
///     batch. RATIONALE (load-bearing, ADR-0064 §5): dispatching
///     `engine.start` for a `StartWorkflow` whose intent did NOT persist
///     would leave a running instance that is NOT re-emittable on restart
///     — the exact invariant the intent-persist-before-engine-start
///     ordering protects. So drop it; the level-triggered
///     `WorkflowLifecycle` reconciler re-emits it next tick.
/// - any other action: always pushed into `dispatchable` unchanged.
///
/// Returns `(dispatchable, first_error)`. A failed intent write for one
/// `StartWorkflow` therefore does NOT discard the rest of the tick's
/// batch — every surviving action (including already-persisted earlier
/// `StartWorkflow`s and all non-workflow actions) still reaches
/// [`dispatch`]. The first intent error is surfaced to the caller.
pub(crate) async fn persist_workflow_intents(
    store: &dyn overdrive_core::traits::intent_store::IntentStore,
    actions: Vec<Action>,
) -> (Vec<Action>, Option<ShimError>) {
    use overdrive_core::aggregate::IntentKey;

    let mut dispatchable: Vec<Action> = Vec::with_capacity(actions.len());
    let mut first_error: Option<ShimError> = None;

    for action in actions {
        match &action {
            Action::StartWorkflow { start, correlation } => {
                let key = IntentKey::for_workflow_instance(correlation);
                // Persist the FULL `WorkflowStart` spec (name + opaque CBOR
                // input) via the co-located rkyv-envelope codec — NOT the bare
                // name bytes (the #217 bug). Per `development.md` § "Persist
                // inputs, not derived state" the persisted intent is the inputs;
                // `from_store_bytes` rehydrates the whole spec on every tick so
                // a restart re-emits with the original input intact (ADR-0048
                // §4b, ADR-0065 §5).
                //
                // A serialiser failure is unreachable for a valid payload, but
                // it is handled the SAME way as a failed `put`: record the first
                // error and DROP this StartWorkflow — starting an instance whose
                // intent did not persist would leave it non-re-emittable, the
                // exact invariant the intent-persist-before-engine-start
                // ordering protects.
                let archived = match start.archive_for_store() {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        if first_error.is_none() {
                            first_error =
                                Some(ShimError::WorkflowIntent { message: err.to_string() });
                        }
                        continue;
                    }
                };
                match store.put(key.as_bytes(), archived.as_ref()).await {
                    // Intent durable — engine-start may proceed for this
                    // StartWorkflow.
                    Ok(()) => dispatchable.push(action),
                    // Intent write failed — DROP this StartWorkflow (it is
                    // not re-emittable if started without durable intent),
                    // record the first such error, and let the rest of the
                    // batch survive.
                    Err(err) => {
                        if first_error.is_none() {
                            first_error =
                                Some(ShimError::WorkflowIntent { message: err.to_string() });
                        }
                    }
                }
            }
            // Non-workflow actions carry no pre-flight intent; always
            // dispatch them.
            _ => dispatchable.push(action),
        }
    }

    (dispatchable, first_error)
}

/// AppState-aware dispatch that persists workflow-instance desired-intent
/// for every `Action::StartWorkflow` BEFORE handing the surviving actions
/// to [`dispatch`], threaded the real engine from `state.workflow_engine`
/// (ADR-0064 §5).
///
/// This is the production commit point for a reconciler-emitted
/// `StartWorkflow`: a committed action both (1) persists the instance's
/// desired-intent (`workflows/<correlation>` → the workflow spec inputs,
/// per `development.md` § "Persist inputs, not derived state") so the
/// `WorkflowLifecycle` reconciler's `hydrate_desired` can read it back on
/// every tick (and re-emit on restart), AND (2) drives the engine off the
/// shim. Intent persistence is FIRST so a crash between the two leaves the
/// instance re-emittable (the level-triggered reconciler re-drives it).
///
/// Per-action isolation holds **at the pre-flight stage too**: the
/// intent-persist loop ([`persist_workflow_intents`]) does NOT early-return
/// on the first failed `put`. A failed intent write drops only its own
/// `StartWorkflow` (which would not be re-emittable if started without
/// durable intent) and records the first such error; every other action in
/// the batch — already-persisted earlier `StartWorkflow`s and all
/// non-workflow actions — still flows into [`dispatch`], which applies its
/// own first-error aggregation over the survivors. The pre-flight error
/// (chronologically first) wins over the dispatch result on failure.
///
/// Mirrors the `StartAllocation → workload-intent → WorkloadLifecycle`
/// precedent for `StartWorkflow → workflow-intent → WorkflowLifecycle`.
///
/// # Errors
///
/// - [`ShimError::WorkflowIntent`] — persisting a workflow-instance intent
///   failed (the first such failure; the offending `StartWorkflow` is
///   dropped, the rest of the batch still dispatches).
/// - Any error [`dispatch`] surfaces (driver / observation / dataplane /
///   workflow-engine failure), with per-action isolation.
pub async fn dispatch_with_workflow_intent(
    actions: Vec<Action>,
    state: &crate::AppState,
    tick: &TickContext,
) -> Result<(), ShimError> {
    let (dispatchable, preflight_err) =
        persist_workflow_intents(state.store.as_ref(), actions).await;

    let dispatch_result = dispatch(
        dispatchable,
        state.drivers.as_ref(),
        &state.alloc_drivers,
        state.obs.as_ref(),
        state.dataplane.as_ref(),
        state.ca.as_ref(),
        state.clock.as_ref(),
        state.identity.as_ref(),
        state.lifecycle_events.as_ref(),
        tick,
        &state.node_id,
        std::sync::Arc::clone(&state.allocator),
        state.runtime.broker_mutex(),
        Some(state.workflow_engine.as_ref()),
        state.mtls_worker.as_ref(),
        &state.net_slot_allocator,
        state.vm_host_state.as_ref(),
    )
    .await;

    // Pre-flight error wins (it is chronologically first); otherwise the
    // dispatch result (which itself carries dispatch()'s own first_error
    // aggregation over the surviving actions).
    preflight_err.map_or(dispatch_result, Err)
}

/// Integration-test form of [`dispatch_with_workflow_intent`] that preserves
/// the production workflow-intent preflight and dispatch composition while
/// replacing only the privileged host-network adapter.
///
/// # Errors
///
/// Returns the same errors as [`dispatch_with_workflow_intent`].
#[doc(hidden)]
#[cfg(any(test, feature = "integration-tests"))]
pub async fn dispatch_with_workflow_intent_and_network_provisioner_for_test(
    actions: Vec<Action>,
    state: &crate::AppState,
    tick: &TickContext,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError> {
    let (dispatchable, preflight_err) =
        persist_workflow_intents(state.store.as_ref(), actions).await;

    let dispatch_result = dispatch_with_network_provisioner(
        dispatchable,
        state.drivers.as_ref(),
        &state.alloc_drivers,
        state.obs.as_ref(),
        state.dataplane.as_ref(),
        state.ca.as_ref(),
        state.clock.as_ref(),
        state.identity.as_ref(),
        state.lifecycle_events.as_ref(),
        tick,
        &state.node_id,
        Arc::clone(&state.allocator),
        state.runtime.broker_mutex(),
        Some(state.workflow_engine.as_ref()),
        state.mtls_worker.as_ref(),
        &state.net_slot_allocator,
        network_provisioner,
        state.vm_host_state.as_ref(),
    )
    .await;

    preflight_err.map_or(dispatch_result, Err)
}

/// C3 PROVISION SEAM (transparent-mtls-enrollment D-TME-12 G1/G2/G3 + JOIN-2,
/// step 04-01). At the TOP of each `Start`/`RestartAllocation` arm, BEFORE
/// `Driver::start`: assign the per-host network slot, derive the netns+veth
/// plan (responder = the per-netns gateway, G1), provision the netns+veth
/// (fail-closed — G2), then branch on [`DriverPayload`]. Exec receives the
/// transit address; VM derives and converges the same-slot [`VmTapPlan`],
/// receives the guest address as `workload_addr`, and receives the guest-net
/// attachment channel. Both arms receive the slot-derived netns and host-veth
/// names.
///
/// Every VM is assigned its guest-network channel, independently of whether
/// the optional mTLS interception worker is composed. Exec keeps its legacy
/// host-network behaviour when interception is absent; when interception is
/// present it receives the same per-allocation transit network as before.
///
/// # Errors
///
/// - [`ShimError::NetSlotExhausted`] — no free slot (the alloc is refused
///   rather than dropped onto a shared veth/subnet).
/// - [`ShimError::WorkloadNetnsProvision`] — the netns/veth/tap provision failed
///   (fail-closed: the workload must not spawn without its netns).
fn network_assignment_required(driver_type: DriverType, mtls_composed: bool) -> bool {
    mtls_composed || driver_type == DriverType::Vm
}

fn provision_and_inject_netns(
    spec: &mut AllocationSpec,
    net_slot_allocator: &NetSlotAllocator,
    mtls_worker: Option<&Arc<MtlsInterceptWorker>>,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError> {
    // A VM cannot boot without the C3 guest-network channel. This remains true
    // when a caller substitutes or omits the optional dataplane worker. Exec
    // retains its pre-join host-netns behaviour when interception is absent.
    let network_required =
        network_assignment_required(spec.driver.driver_type(), mtls_worker.is_some());
    if !network_required {
        return Ok(());
    }
    // G3: assign the smallest-free slot (idempotent re-entry for an already-
    // held alloc — a Restart reuses the same slot). Exhaustion REFUSES.
    let slot = net_slot_allocator.assign(spec.alloc.clone())?;
    // G1: responder == the per-netns gateway (plan.host_addr).
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    debug_assert_eq!(
        plan.responder_addr, plan.host_addr,
        "G1: the responder address MUST be the per-netns gateway (plan.host_addr)"
    );
    // G2: provision the netns + veth BEFORE Driver::start. Fail-closed — a
    // provision failure aborts the start (the `?` surfaces
    // ShimError::WorkloadNetnsProvision). Idempotent converge-on-boot: a
    // re-provision under the same slot (Restart) is a no-op.
    // ADR-0089 C3 VM branch: reuse the SAME slot as the transit plan, derive
    // and converge the guest-half tap wire, then inject the guest address and
    // guest-net inputs. Exec keeps the pre-existing transit address and no
    // guest-net fields.
    let vm_tap = matches!(&spec.driver, DriverPayload::Vm(_))
        .then(|| derive_vm_tap_plan(slot, plan.responder_addr));
    network_provisioner.provision(&plan, vm_tap.as_ref())?;
    inject_workload_network(spec, &plan, vm_tap.as_ref());
    Ok(())
}

/// Pure C3 handoff from converged network plans into the transient driver
/// spec. The VM arm replaces the transit forwarding hop with the guest address
/// as the canonical workload address and fills the guest-net channel; Exec
/// retains the existing transit address and leaves that channel absent.
fn inject_workload_network(
    spec: &mut AllocationSpec,
    workload: &WorkloadNetnsPlan,
    vm_tap: Option<&VmTapPlan>,
) {
    spec.netns = Some(workload.netns.clone());
    spec.host_veth = Some(workload.host_veth.clone());
    if let Some(tap) = vm_tap {
        spec.workload_addr = Some(tap.guest_addr);
        spec.guest_tap = Some(tap.tap.clone());
        spec.guest_mac = Some(tap.mac);
        spec.guest_gateway = Some(tap.tap_gateway);
        spec.guest_prefix_len = Some(tap.guest_network.prefix_len());
        spec.guest_dns = Some(tap.responder_addr);
    } else {
        spec.workload_addr = Some(workload.workload_addr);
        spec.guest_tap = None;
        spec.guest_mac = None;
        spec.guest_gateway = None;
        spec.guest_prefix_len = None;
        spec.guest_dns = None;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test fixtures fail immediately on invalid static ids")]
mod vm_tap_spec_injection_tests {
    use std::path::PathBuf;

    use overdrive_core::SpiffeId;
    use overdrive_core::traits::driver::{
        AllocationSpec, DriverPayload, ExecPayload, Resources, VmPayload,
    };

    use super::{
        derive_vm_tap_plan, derive_workload_netns_plan, inject_workload_network,
        responder_addr_for_slot,
    };
    use crate::veth_provisioner::NetSlot;

    fn spec(driver: DriverPayload) -> AllocationSpec {
        AllocationSpec {
            alloc: overdrive_core::AllocationId::new("vm-tap-inject").expect("valid alloc id"),
            identity: SpiffeId::new("spiffe://overdrive.local/workload/test/alloc/01")
                .expect("valid identity"),
            driver,
            resources: Resources { cpu_milli: 100, memory_bytes: 64 * 1024 * 1024 },
            probe_descriptors: Vec::new(),
            netns: None,
            host_veth: None,
            workload_addr: None,
            guest_tap: None,
            guest_mac: None,
            guest_gateway: None,
            guest_prefix_len: None,
            guest_dns: None,
            service_ports: Vec::new(),
        }
    }

    /// VM C3 injection uses the guest address and carries every guest-net
    /// input from the same slot-derived tap plan.
    /// CONTRACT_SHAPE: bounded-change (netns, host_veth, workload_addr,
    /// guest_tap, guest_mac, guest_gateway, guest_prefix_len, guest_dns only).
    #[test]
    fn vm_injection_uses_guest_address_and_complete_guest_net_channel() {
        let slot = NetSlot::new(7).expect("valid slot");
        let responder = responder_addr_for_slot(slot);
        let workload = derive_workload_netns_plan(slot, responder);
        let tap = derive_vm_tap_plan(slot, responder);
        let mut spec = spec(DriverPayload::Vm(VmPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
        }));

        let before = spec.clone();

        inject_workload_network(&mut spec, &workload, Some(&tap));

        assert_eq!(spec.netns.as_ref(), Some(&workload.netns));
        assert_eq!(spec.host_veth.as_deref(), Some(workload.host_veth.as_str()));
        assert_eq!(spec.workload_addr, Some(tap.guest_addr));
        assert_eq!(spec.guest_tap.as_deref(), Some(tap.tap.as_str()));
        assert_eq!(spec.guest_mac, Some(tap.mac));
        assert_eq!(spec.guest_gateway, Some(tap.tap_gateway));
        assert_eq!(spec.guest_prefix_len, Some(tap.guest_network.prefix_len()));
        assert_eq!(spec.guest_dns, Some(tap.responder_addr));

        let mut expected = before;
        expected.netns = Some(workload.netns.clone());
        expected.host_veth = Some(workload.host_veth.clone());
        expected.workload_addr = Some(tap.guest_addr);
        expected.guest_tap = Some(tap.tap.clone());
        expected.guest_mac = Some(tap.mac);
        expected.guest_gateway = Some(tap.tap_gateway);
        expected.guest_prefix_len = Some(tap.guest_network.prefix_len());
        expected.guest_dns = Some(tap.responder_addr);
        assert_eq!(
            spec, expected,
            "VM injection may change only its declared network handoff fields",
        );
    }

    /// Exec retains the transit address and the VM-only channel remains fully
    /// absent.
    /// CONTRACT_SHAPE: bounded-change (netns, host_veth, workload_addr only).
    #[test]
    fn exec_injection_keeps_transit_address_and_no_guest_net_channel() {
        let slot = NetSlot::new(8).expect("valid slot");
        let responder = responder_addr_for_slot(slot);
        let workload = derive_workload_netns_plan(slot, responder);
        let mut spec = spec(DriverPayload::Exec(ExecPayload {
            command: "/bin/true".to_owned(),
            args: Vec::new(),
        }));

        let before = spec.clone();

        inject_workload_network(&mut spec, &workload, None);

        assert_eq!(spec.workload_addr, Some(workload.workload_addr));
        assert_eq!(
            (
                spec.guest_tap.as_deref(),
                spec.guest_mac,
                spec.guest_gateway,
                spec.guest_prefix_len,
                spec.guest_dns,
            ),
            (None, None, None, None, None),
        );

        let mut expected = before;
        expected.netns = Some(workload.netns.clone());
        expected.host_veth = Some(workload.host_veth.clone());
        expected.workload_addr = Some(workload.workload_addr);
        assert_eq!(
            spec, expected,
            "Exec injection may change only its declared network handoff fields",
        );
    }
}

/// C3 TEARDOWN SEAM (transparent-mtls-enrollment D-TME-12 G2, step 04-01). At
/// the terminal arms, AFTER the driver stop: tear down the per-alloc netns +
/// veth, THEN release the slot (release-AFTER-teardown, so a crash between the
/// two leaves the slot HELD = the resource still exists and is reclaimable,
/// never a released-but-undestroyed leak).
///
/// Both halves are idempotent: teardown swallows "absent" (converge-on-boot
/// shape), `release` is a `BTreeMap::remove` of a possibly-absent key. A
/// terminal for an alloc that never provisioned (no held slot) is a benign
/// no-op. Teardown is keyed by the allocator itself rather than the optional
/// mTLS worker because every VM now owns a slot.
///
/// # Errors
///
/// [`ShimError::WorkloadNetnsProvision`] only on a NON-benign teardown failure
/// (e.g. permission denied removing the netns); "absent" is swallowed. The
/// slot is released only AFTER a successful teardown.
fn teardown_and_release_netns(
    alloc_id: &AllocationId,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), ShimError> {
    // Find the slot this alloc holds (if any). An alloc that never reached the
    // provision seam holds no slot — teardown + release are then no-ops.
    let Some(slot) = net_slot_allocator.snapshot().get(alloc_id).copied() else {
        return Ok(());
    };
    let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
    // Teardown FIRST (idempotent — swallows "absent"); release the slot only
    // after teardown succeeds, so a crash between the two leaves the slot HELD
    // (the netns still exists and is reclaimable on the next terminal).
    network_provisioner.teardown(&plan)?;
    net_slot_allocator.release(alloc_id);
    Ok(())
}

/// Dispatch a single action. Each variant is independent; the caller
/// loops over a `Vec<Action>` and aggregates errors.
#[allow(clippy::too_many_lines)]
#[allow(
    clippy::too_many_arguments,
    reason = "See dispatch() docstring — port-trait dependencies are required at call site, not optional via a builder."
)]
async fn dispatch_single(
    action: Action,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    obs: &dyn ObservationStore,
    dataplane: &dyn Dataplane,
    ca: &dyn Ca,
    clock: &dyn Clock,
    identity: &IdentityMgr,
    bus: &broadcast::Sender<LifecycleEvent>,
    tick: &TickContext,
    writer_node: &NodeId,
    allocator: &Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
    broker: &parking_lot::Mutex<EvaluationBroker>,
    workflow_engine: Option<&WorkflowEngine>,
    mtls_worker: Option<&Arc<MtlsInterceptWorker>>,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
    host: &dyn VmHostState,
) -> Result<(), ShimError> {
    match action {
        // No-op (Action::Noop) and the Phase 3 HttpCall placeholder are
        // "no dispatch needed" — observation-only or deferred. (Per
        // ADR-0064 §5 `StartWorkflow` is NO LONGER in this no-op group —
        // it has its own arm below that hands the instance to the
        // WorkflowEngine off the shim.)
        Action::Noop | Action::HttpCall { .. } => Ok(()),
        // StartWorkflow: hand the instance to the WorkflowEngine off the
        // shim — exactly as Action::StartAllocation -> Driver::start
        // (ADR-0064 §5, DDD-5, the RATIFY-flagged engine↔reconciler
        // boundary). The engine spawns the author's `async fn run` as a
        // tracked async task and journals its terminal; it is NOT run as
        // a per-tick reconcile loop. The emitting workflow-lifecycle
        // reconciler stays pure-sync.
        //
        // When no engine is wired (the production reconciler-runtime path
        // until 01-06's full boot composition lands), the start is a
        // no-op for this tick — the level-triggered reconciler re-emits
        // it once the engine is composed. This mirrors the
        // StartAllocation arm's tolerance of a level-triggered re-enqueue.
        Action::StartWorkflow { start, correlation } => {
            let Some(engine) = workflow_engine else {
                return Ok(());
            };
            // Derive the per-instance journal id deterministically from the
            // action's correlation (ADR-0064 §5): the SAME instance always
            // resolves to the SAME `WorkflowId`, so a crash-resume
            // re-emit re-targets the same journal (the engine's
            // `load_journal` then RESUMES rather than cold-starts).
            let workflow_id = WorkflowId::for_correlation(&correlation);
            engine
                .start(&start, &correlation, &workflow_id)
                .await
                .map_err(|err| ShimError::WorkflowEngine { message: err.to_string() })
        }
        // FinalizeFailed: the reconciler has decided this allocation
        // has reached a terminal lifecycle moment. Per ADR-0037 §4 the
        // shim threads the `Action.terminal` value onto BOTH
        // `AllocStatusRow.terminal` (durable surface, written via
        // `obs.write`) AND `LifecycleEvent.terminal` (broadcast surface,
        // emitted via `bus.send`) in the same call frame — both
        // surfaces come from the same source value, so drift is
        // structurally impossible.
        //
        // The reconciler emits `FinalizeFailed` for several distinct
        // typed terminal claims, all flowing through the same arm
        // here unchanged:
        // - `BackoffExhausted { attempts }` — restart budget exceeded
        //   for a Service-shape workload (per existing ADR-0037 §4).
        // - `Completed { exit_code: 0 }` — Job-kind workload exited
        //   cleanly (per ADR-0037 Amendment 2026-05-10 / ADR-0047 §1,
        //   landed in slice 02-04).
        // - `Failed { exit_code: N }` — Job-kind workload exited with
        //   non-zero status (per ADR-0037 Amendment 2026-05-10).
        //
        // The row is written with `state: Failed` (per ADR-0032 §5
        // distinguishes "operator stopped" → Terminated from
        // "driver could not start / budget exhausted / Job exit" →
        // Failed). The `reason` field propagates the prior row's typed
        // leaf cause unchanged (e.g. `ExecBinaryNotFound { path }`);
        // the typed terminal claim lives on the orthogonal `terminal`
        // field per ADR-0037 §4. Synthesising a derived `reason` here
        // (e.g. `RestartBudgetExhausted { attempts, last_cause_summary }`)
        // would duplicate `attempts` (already on `terminal`) and
        // stringify the typed leaf cause into `last_cause_summary` —
        // both violations of `.claude/rules/development.md`
        // § "Persist inputs, not derived state". Wire consumers wanting
        // the "we gave up after N" / "exited cleanly" / "exited with N"
        // framing render it from `terminal` directly. The streaming
        // dispatcher's `workload_event_from_terminal` projection maps
        // each `TerminalCondition` to its `JobSubmitEvent`
        // (`Completed → Succeeded`, `Failed → Failed`,
        // `BackoffExhausted → Failed`, `Stopped → Stopped`).
        Action::FinalizeFailed { alloc_id, terminal } => {
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                // No prior row — nothing to finalize against. This is
                // structurally rare (the WorkloadLifecycle only emits
                // FinalizeFailed against a known-failed alloc) but
                // we tolerate it as a no-op so a level-triggered
                // re-enqueue against a torn-down alloc does not
                // surface as a ShimError.
                return Ok(());
            };
            let prior_state: AllocStateWire = prior_row.state.into();
            // Per slice 02-06: propagate the prior row's `stderr_tail`
            // forward onto the typed terminal row so the streaming
            // layer's `JobSubmitEvent::Failed` projection can render
            // the workload's stderr verbatim. The exit observer (per
            // slice 02-05 / ADR-0033 Amendment 2026-05-10) populates
            // `stderr_tail` on the per-attempt failure row; without
            // this propagation, the FinalizeFailed write would
            // overwrite that row with `stderr_tail: None`, breaking
            // S-02-02's stderr-tail rendering assertion.
            let prior_stderr_tail = prior_row.stderr_tail.clone();
            // FinalizeFailed is a terminal claim — preserve the prior
            // row's `started_at` verbatim. If the prior row never
            // reached Running (Pending only), `started_at` is `None`
            // and stays `None` here. Same forward-carry pattern as
            // `stderr_tail` / `detail` / `kind`.
            let prior_started_at = prior_row.started_at;
            // Forward-carry the prior row's canonical workload address. The
            // `Stable` (still-Running) arm below keeps the alloc Running, so it
            // MUST keep its per-instance backend address too — dropping it to
            // `None` here is the walking-skeleton backend-drop the GAP-9 guard
            // only HALF-closed (it preserved the Running *state* but not the
            // address, so the BackendDiscoveryBridge silently reverted to its
            // host_ipv4 fallback and the dial-by-name egress translation
            // targeted an unreachable addr). A genuine terminal (`Failed`)
            // passes `None` below — a dead alloc is not a live backend. The
            // residual host_ipv4-fallback masking is tracked in #248.
            let prior_workload_addr = prior_row.workload_addr;
            // GAP-9 — a `Stable` terminal is a SUCCESS claim, not a
            // failure: the Service alloc has passed its startup probes
            // and is healthily serving. It MUST remain `Running` so the
            // BackendDiscoveryBridge (which renders backends from the
            // `state == Running` set) keeps the backend registered.
            // Every other `TerminalCondition` (ServiceFailed /
            // BackoffExhausted / Completed / Stopped …) is a genuine
            // terminal and lands `Failed`.
            //
            // Pre-GAP-9 the `service-lifecycle` reconciler never ran in
            // production, so `FinalizeFailed { Stable }` was never
            // emitted and this arm only ever saw real failures — the
            // unconditional `AllocState::Failed` was latently wrong but
            // unreachable. GAP-9 makes the Stable path live, surfacing
            // the bug as a walking-skeleton backend-drop; this guard
            // closes it. The terminal CLAIM (`terminal`) is still
            // written verbatim onto the row + lifecycle event, so the
            // streaming layer's `ServiceSubmitEvent::Stable` projection
            // (which reads `event.terminal`, not the state) is unchanged.
            //
            // The SAME `Stable` discriminator gates the destructive
            // infrastructure teardowns below (canonical-address inbound
            // RCA §9, GH #241): a `Stable` FinalizeFailed is a SUCCESS
            // claim that keeps the alloc Running and still serving on its
            // netns/leg-C, so it MUST NOT tear down the per-workload
            // netns/veth/nft or detach the mTLS intercept. Only a
            // genuine terminal (the `finalized_state == Failed` set) reaps
            // the alloc. Hoisted once here and reused at both teardown
            // sites so the row-state and teardown decisions cannot drift.
            let is_stable = matches!(terminal, Some(TerminalCondition::Stable { .. }));
            // The `terminal` claim's own variant identity IS the success/failure
            // classification (per `TerminalCondition::Completed`'s and `::Failed`'s
            // own docs: "downstream consumers must not redo the comparison ...
            // branch on the variant, never on exit_code != 0"). `Stable` keeps the
            // row `Running` (handled above, and unaffected by this addition — every
            // OTHER `is_stable`-gated decision in this function, e.g. `workload_addr`
            // forward-carry and the driver/teardown hooks below, still keys off
            // `is_stable` alone; only the STATE computation gains a second case).
            // Of the remaining terminal claims, `Completed` is ALSO a success (a
            // Job-kind clean exit) and MUST land `Terminated`, matching
            // `exit_observer::classify()`'s own `ExitKind::CleanExit ->
            // AllocState::Terminated` mapping (the observer's own row, written on
            // the prior tick, already carries `Terminated`). Forcing it to `Failed`
            // here silently overwrote that success with a failure while still
            // forward-carrying the observer's `reason: Stopped { by: Process }`
            // verbatim (a few lines below) -- landing a `Failed` + `Stopped { by:
            // Process }` row that `classify()` itself never constructs (the
            // microvm-driver-cloud-hypervisor S-VM-01 walking-skeleton finding: a
            // VM guest that exited 0 was reported Failed to the operator). Every
            // OTHER terminal claim (`Failed` / `BackoffExhausted` / `ServiceFailed`
            // / `Stopped` / `Custom`) is a genuine failure and lands `Failed`.
            let is_completed = matches!(terminal, Some(TerminalCondition::Completed { .. }));
            let finalized_state = if is_stable {
                prior_row.state
            } else if is_completed {
                AllocState::Terminated
            } else {
                AllocState::Failed
            };
            let updated_at = LogicalTimestamp::dominating(
                tick.tick,
                prior_row.node_id.clone(),
                Some(&prior_row.updated_at),
            );
            // ADR-0078 § D2 borrow-ordering constraint: `prior` is supplied as
            // a borrow of the SAME row whose `workload_id` / `node_id` feed
            // the builder's earlier parameters. Rust resolves argument
            // expressions left-to-right, so those two identity fields are
            // cloned out FIRST and the row itself stays alive to be borrowed
            // in the final position.
            let workload_id = prior_row.workload_id.clone();
            let node_id = prior_row.node_id.clone();
            let row = build_alloc_status_row(
                alloc_id,
                workload_id,
                node_id,
                finalized_state,
                updated_at,
                prior_row.reason.clone(),
                // Propagate the prior row's verbatim driver text. The
                // last failed Start/RestartAllocation populates `detail`
                // with the `DriverError::StartRejected.reason_text`
                // (per the StartAllocation arm above); the streaming
                // surface's failed-terminal rendering reads this
                // through `event.detail`. Hardcoding `None` here would
                // drop the operator-visible cause text on the
                // budget-exhausted terminal, even though the prior
                // attempt rows carry it.
                prior_row.detail.clone(),
                terminal,
                prior_stderr_tail,
                prior_row.kind,
                prior_started_at,
                // Keep the per-instance address only while the row stays
                // Running (the Stable success claim); a genuine terminal
                // drops it to `None` — a dead alloc is not a live backend.
                if is_stable { prior_workload_addr } else { None },
                // ADR-0078 § D2 site 3: FORWARDS, it does NOT snapshot. The
                // dominant case is `terminal → terminal` — an already-`Failed`
                // row re-stamped with a terminal claim while forward-carrying
                // that same row's `reason` / `detail` / `stderr_tail` /
                // `started_at`. Snapshotting here would put those five facts
                // on one row TWICE (once as row fields, once inside
                // `last_terminated`), the two-sources-of-truth duplication
                // § D1 rejects. The `Stable` arm forwards too — the prior is
                // `Running`.
                Some(&prior_row),
            );
            obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
            // Service-health-check-probes step 01-03d / ADR-0054 § 2 +
            // ADR-0080 § D4: fire the probe lifecycle hook matching what
            // this FinalizeFailed actually claims.
            //
            // `Stable` is NON-TERMINAL (ADR-0055): the alloc stays Running
            // and keeps serving, and readiness / liveness are continuous
            // post-Stable per `ProbeRole`'s contract. Routing it through
            // `on_alloc_terminal` tore the whole supervisor down and
            // cancelled every role — which made readiness and liveness
            // structurally unreachable for their entire intended lifetime
            // and left `Backend.healthy` a constant. `on_alloc_stable`
            // retires the startup role only; the supervisor survives.
            //
            // A genuine terminal (BackoffExhausted / Completed / Failed)
            // still tears the whole supervisor down. Both hooks default to
            // no-op when no ProbeRunner is wired.
            //
            // ADR-0083 §D2a(b) (GH #42): `FinalizeFailed` carries no spec,
            // so the driver is read from the alloc→driver-kind index
            // written at Start/Restart (falling back to every composed
            // driver on a miss — see `resolve_drivers_for_alloc`). The
            // index entry is removed ONLY on a genuine terminal — a
            // `Stable` claim keeps the alloc Running, so a later
            // stop/terminal arm still needs the entry.
            for driver in resolve_drivers_for_alloc(drivers, alloc_drivers, &row.alloc_id) {
                if is_stable {
                    driver.on_alloc_stable(&row.alloc_id);
                } else {
                    driver.on_alloc_terminal(&row.alloc_id);
                }
            }
            if !is_stable {
                alloc_drivers.lock().remove(&row.alloc_id);
            }
            // The mTLS-intercept detach and the C3 netns teardown are both
            // gated on `!is_stable`: a `Stable` FinalizeFailed is a success
            // claim (the alloc stays Running and keeps serving on leg-C / its
            // netns), so detaching the intercept or reaping the netns would
            // leave a healthy workload running but UNREACHABLE. Both fire only
            // for a genuine terminal (the `finalized_state == Failed` set).
            if !is_stable {
                // transparent-mtls-host-socket (step 06-03): tear down the
                // alloc's mTLS intercept (detach the cgroup program, remove
                // the TPROXY rule, drain the per-connection teardown set
                // fail-closed). Idempotent for an alloc with no intercept.
                if let Some(worker) = mtls_worker {
                    worker.stop_alloc(&row.alloc_id);
                }
                // C3 TEARDOWN SEAM (D-TME-12 G2, step 04-01): tear down the
                // per-alloc netns + veth, THEN release the slot — AFTER the
                // driver stop. Idempotent; no-op for an alloc that never
                // provisioned.
                teardown_and_release_netns(&row.alloc_id, net_slot_allocator, network_provisioner)?;
            }
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
            Ok(())
        }
        // Start: spawn the allocation via the driver and write a
        // Running AllocStatusRow on success. On StartRejected, write
        // a `Failed` row recording the typed cause-class
        // (ADR-0032 §5 + §4 Amendment).
        Action::StartAllocation { alloc_id, workload_id, node_id, mut spec, kind } => {
            // Read prior obs row before the driver call so we capture
            // the allocation's state before this transition. For first-
            // seen allocs (no prior row) default to Pending — consistent
            // with how existing tests model the initial transition.
            //
            // The row's `updated_at` is bound alongside `state` (ADR-0077
            // § D2 site 3): this lookup already happens, so prior-derived
            // stamping on the alloc-start path costs ZERO extra store
            // reads — the value was simply being discarded.
            //
            // ADR-0078 § D2 site 4: `prior_row` must STAY ALIVE through the
            // `driver.start` call below, because the final `state` (and hence
            // the crash facts derived from it) is not known until the driver
            // has answered. Neither binding below consumes it —
            // `prior_updated_at` clones the stamp rather than moving it.
            let prior_row = find_prior_alloc_row(obs, &alloc_id).await?;
            let prior_state: AllocStateWire =
                prior_row.as_ref().map_or(AllocStateWire::Pending, |r| r.state.into());
            let prior_updated_at: Option<LogicalTimestamp> =
                prior_row.as_ref().map(|r| r.updated_at.clone());

            // C3 PROVISION SEAM (D-TME-12 G1/G2/G3 + JOIN-2/6 + AC14, step
            // 04-01): assign the slot, provision the per-workload netns + veth,
            // and inject `spec.netns` + `spec.host_veth` — all BEFORE
            // `driver.start` so the workload is spawned INTO its netns. Fail-
            // closed: a provision failure (or slot exhaustion) aborts the start.
            // Off the mTLS gate this remains mandatory for VM and is a no-op
            // only for Exec.
            //
            // AC14 sub-claim 4: a PERSISTENT provision failure (slot exhaustion,
            // EPERM creating the netns/veth) must drive the alloc to a `Failed`
            // terminal — NOT bubble `Err` and loop `Pending` forever as the
            // reconciler re-emits StartAllocation each tick. Mirror the
            // `StartRejected → Failed` + mTLS-install fail-closed precedents: on
            // a provision-seam error, write a fresh `Failed` row carrying the
            // `WorkloadNetnsProvisionFailed` cause-class and return `Ok(())`. The
            // provision precedes `driver.start`, so nothing was spawned and the
            // "never Running-with-no-netns" safety invariant is preserved (the
            // alloc is Pending → Failed, never Running). A non-provision
            // `ShimError` (unreachable here, but kept exhaustive) propagates
            // unchanged.
            if let Err(err) = provision_and_inject_netns(
                &mut spec,
                net_slot_allocator,
                mtls_worker,
                network_provisioner,
            ) {
                let Some(cause) = netns_provision_cause(&err) else {
                    return Err(err);
                };
                return fail_closed_on_netns_provision(
                    obs,
                    bus,
                    tick,
                    alloc_id,
                    workload_id,
                    node_id,
                    kind,
                    prior_state,
                    cause,
                    prior_row.as_ref(),
                )
                .await;
            }

            // Registry lookup (ADR-0083 §D1/§D2a, GH #42): the routing
            // key is the payload's own driver kind — the driver names
            // itself, no second source of truth to drift. A missing
            // registry entry (no such driver composed on this node)
            // synthesizes the identical `StartRejected` shape a real
            // driver's own rejection would produce, so the SAME
            // Failed-row construction path below handles both —
            // SD-5's admission-time capability gate (step 01-09) is a
            // separate, earlier check; this is the dispatch-time
            // fallback for whatever reaches here regardless.
            let driver_kind = spec.driver.driver_type();
            let start_outcome: Result<AllocationHandle, DriverError> =
                match drivers.get(driver_kind) {
                    Some(driver) => driver.start(&spec).await,
                    None => Err(DriverError::StartRejected {
                        failure: DriverStartFailure {
                            class: DriverStartClass::Unclassified { driver: driver_kind },
                            // ADR-0083 §D3d / DWD-25: name the absent
                            // capability and point at the executed boot
                            // reason, rather than reading as an internal
                            // defect. Driver-kind-generic (the same shape
                            // serves a future `wasm` miss), so no per-driver
                            // branch or parser is introduced; free-form
                            // verbatim `detail` per DWD-24, never a
                            // classification input, so no contract or
                            // conversion moves. The `driver.{kind}.not_composed`
                            // pointer names the startup-log event the driver's
                            // own composition emits (§D3c) where the specific
                            // probe reason was recorded.
                            detail: format!(
                                "no {driver_kind} driver composed on this node: the node's \
                                 {driver_kind} capability probe did not pass; see the startup \
                                 log's `driver.{driver_kind}.not_composed` reason for the \
                                 specific cause"
                            ),
                        },
                    }),
                };
            // Per ADR-0032 §4 Amendment 2026-04-30: classify the
            // driver's `StartRejected.reason` text into a typed
            // cause-class `TransitionReason` variant. State on
            // failure is `Failed` (not `Terminated`) — distinguishes
            // operator-stop from driver-could-not-start.
            let (handle_opt, state, reason, detail, source, cleanup_recovered): (
                Option<AllocationHandle>,
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
                bool,
            ) = match start_outcome {
                Ok(handle) => (
                    Some(handle),
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                    false,
                ),
                // DWD-24: apply the total, core-owned conversion and
                // preserve the driver's verbatim diagnostic separately.
                // No parsing, no prefix table, no `DriverType` dispatch —
                // the family comes from the typed class itself.
                Err(DriverError::StartRejected { failure })
                    if is_duplicate_vm_owner(&failure, &alloc_id) =>
                {
                    return Ok(());
                }
                Err(DriverError::StartRejected { failure }) => (
                    None,
                    AllocState::Failed,
                    Some(TransitionReason::from(&failure)),
                    Some(failure.detail.clone()),
                    TransitionSource::Driver(failure.class.driver_type()),
                    false,
                ),
                Err(other) => {
                    let Some(cleanup) = other.start_cleanup_failure() else {
                        return Err(ShimError::Driver(other));
                    };
                    let failure = cleanup.as_start_failure();
                    let cleanup_recovered = cleanup.recovery_complete();
                    (
                        None,
                        if cleanup_recovered {
                            AllocState::Failed
                        } else {
                            // D12: residue still has one in-process cleanup
                            // owner. Pending is the existing retryable state;
                            // publishing terminal Failed here would authorize
                            // VM reclamation to become a second teardown owner.
                            AllocState::Pending
                        },
                        Some(TransitionReason::from(&failure)),
                        Some(failure.detail.clone()),
                        TransitionSource::Driver(failure.class.driver_type()),
                        cleanup_recovered,
                    )
                }
            };
            if state == AllocState::Running
                && mtls_worker.is_some()
                && matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm)
                && let Err(issue_err) = ensure_intercept_identity(
                    &alloc_id,
                    &workload_id,
                    &node_id,
                    ca,
                    obs,
                    clock,
                    identity,
                )
                .await
            {
                // No Running observation or intercept release may follow an
                // identity prerequisite failure. Tear the just-started driver
                // down while its Running/EXEC gate is still closed, then let
                // the normal recoverable shim-error path retry issuance.
                if let Some(handle) = &handle_opt
                    && let Some(driver) = drivers.get(driver_kind)
                {
                    let _ = driver.stop(handle).await;
                    driver.release_supervision(&handle.alloc);
                }
                return Err(issue_err);
            }
            // Per ADR-0037 §4: StartAllocation is never a terminal
            // claim — WorkloadLifecycle emits FinalizeFailed on a separate
            // tick when restart budget is exhausted, and the row that
            // gets the BackoffExhausted terminal is written by that
            // arm. A successful start or a single mid-budget failed
            // start carries `terminal: None`.
            //
            // Subsidiary GAP-1 fix: capture the wall-clock at the
            // Pending → Running transition. On a successful start
            // (`state == AllocState::Running`) the row carries
            // `Some(tick.now_unix)` — the same `Clock` port DST
            // already controls. On a failed start
            // (`state == AllocState::Failed`) the alloc never
            // reached Running and there is no "started at"
            // wall-clock; the row carries `None`. The reconciler's
            // EarlyExit / StartupProbeFailed / Stable gates branch
            // on `None` explicitly (no silent-zero collapse).
            let started_at = if state == AllocState::Running { Some(tick.now_unix) } else { None };
            // canonical-workload-address-inbound-tproxy (D-A1 / D-BLOCKER2, GH
            // #241): the StartAllocation Running write is the INITIAL
            // population of the canonical workload address — `spec.workload_addr`
            // (injected at the C3 provision seam off `plan.workload_addr`), an
            // OBSERVED INPUT (the materialised slot×base-at-provision snapshot
            // the node provisioned this alloc into; no recompute, no derivation).
            // A failed start (`state == Failed`) never reached the provisioned
            // netns, so it carries `None`. Successor / terminal writers (exit
            // observer, FinalizeFailed) forward-carry `prior.workload_addr`.
            let workload_addr =
                if state == AllocState::Running { spec.workload_addr } else { None };
            let updated_at =
                LogicalTimestamp::dominating(tick.tick, node_id.clone(), prior_updated_at.as_ref());
            let row = build_alloc_status_row(
                alloc_id,
                workload_id,
                node_id,
                state,
                updated_at,
                reason,
                detail,
                None,
                None,
                kind,
                started_at,
                workload_addr,
                // ADR-0078 § D2 site 4: `(None, 0)` on a genuinely fresh key.
                // On a re-dispatch against a surviving row it forwards, and on
                // a `terminal → Running` re-dispatch it snapshots + increments
                // exactly as the RestartAllocation arm does.
                prior_row.as_ref(),
            );
            // Fires the Running-confirmed gate exposed by Driver::start.
            // Required for liveness — the watcher parks on this gate
            // before emitting ExitEvent. The two firing sites
            // (post-Running-Ok and post-degraded-escalation) are jointly
            // load-bearing; missing either leaks the watcher. Per RCA
            // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
            // (Solution 1'). Standard convention: fire the gate
            // IMMEDIATELY after `obs.write` resolves Ok, BEFORE the
            // lifecycle-event emit.
            //
            // Failed-row branch (state == Failed, handle_opt == None) does
            // NOT fire the gate per AC2 — the alloc never reached Running
            // and no watcher exists for a never-spawned alloc. The
            // driver's `release_for_exit_emission` is idempotent for
            // unknown allocs anyway; the explicit None-check here makes
            // the AC contract structurally readable at the call site.
            // The Running write is this alloc's INITIAL durable record. If
            // it fails AFTER a workload was actually started (Ok(handle) →
            // state == Running), the just-spawned driver process is left
            // running with its exit watcher parked on the Running-confirmed
            // gate: the gate fires only AFTER this write commits (below), so
            // a failed write both STRANDS the watcher (no ExitEvent ever
            // emitted) and LEAKS the workload — no terminal row, and
            // `VmReclamation` cannot reclaim it because `live_allocations()`
            // still reports the alloc as held, so `reclamation_authorised`
            // is false by design (reclamation must never race a live
            // driver). Tear the workload down before propagating:
            // `driver.stop` drops the stashed gate sender — releasing the
            // watcher via the `Driver::start` § "Sender drop (orphan path)"
            // — and reclaims the host footprint, turning an orphaned-live
            // workload into a clean failed start the reconciler
            // re-dispatches. This is the pre-`Running`-committed analogue of
            // `fail_closed_on_mtls_install`'s stop-on-failure, minus the
            // `Failed`-row supersede: the obs store that just rejected this
            // write cannot durably record anything, so recovery is
            // teardown-now + re-dispatch-later. Symmetric across every
            // `ExitEvent`-emitting driver — Exec and VM both honour the gate
            // contract and both strand identically without this.
            // `@mandatory:mutation_target` — a mutant that drops the
            // `driver.stop` leaves the started workload orphaned;
            // `running_write_failure_stops_the_started_alloc` catches it.
            if let Err(write_err) =
                obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await
            {
                if cleanup_recovered && let Some(driver) = drivers.get(driver_kind) {
                    // The cleanup is complete, but its authoritative
                    // disposition is not. Return the single in-flight slot so
                    // a later convergence tick can retry this same row while
                    // the allocation claim remains held.
                    driver.retry_start_cleanup_disposition(&row.alloc_id);
                }
                if state == AllocState::Running
                    && let Some(handle) = &handle_opt
                    && let Some(driver) = drivers.get(driver_kind)
                {
                    let _ = driver.stop(handle).await;
                    // `stop` alone does NOT clear a phased driver's claim:
                    // `VmDriver::stop` leaves the entry `EndingInFlight`
                    // (never a full remove — its own double-authorship
                    // guard), and the watcher it unparks finds that
                    // already-ending entry, so `try_begin_ending` returns
                    // false and NO `ExitEvent` is emitted. With no exit
                    // event the exit observer never fires the
                    // `release_supervision` that would clear the entry, and
                    // this arm returns `Err` below before its own release —
                    // so the torn-down alloc is reported by
                    // `live_allocations()` forever and `VmReclamation` is
                    // pinned `!reclamation_authorised` for a dead id
                    // (greptile PR #268 P1 "Ending claim survives
                    // teardown"). Release it here, mirroring the terminal
                    // StopAllocation arm's `stop()` → `release_supervision()`
                    // pairing. Safe: `stop` has fully awaited teardown, so
                    // there is no live process for reclamation to race;
                    // idempotent and a no-op for drivers on the trait
                    // default (Exec/Sim keep the phase-less `stop`).
                    // `@mandatory:mutation_target` — dropping this leaks the
                    // `EndingInFlight` supervision entry the greptile P1
                    // flagged.
                    driver.release_supervision(&handle.alloc);
                }
                return Err(write_err.into());
            }
            if cleanup_recovered {
                // A cleanup retry retains driver authorship until this exact
                // Failed disposition commits. Release only after the write so
                // no concurrent start can publish Running and then be
                // superseded by this older cleanup outcome.
                drivers
                    .get(driver_kind)
                    .unwrap_or_else(|| {
                        unreachable!(
                            "a recovered cleanup outcome can only come from a composed driver"
                        )
                    })
                    .release_supervision(&row.alloc_id);
            }
            if state == AllocState::Running {
                let intercept_required = mtls_worker.is_some()
                    && matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm);
                let mut stable_exact_rule_baseline = !intercept_required;
                // ADR-0083 §D2a(b) (GH #42): record the alloc→driver-kind
                // routing entry now, while the payload is in hand — the
                // stop/terminal Actions (StopAllocation, FinalizeFailed)
                // carry no spec and read this back.
                alloc_drivers.lock().insert(row.alloc_id.clone(), driver_kind);
                // `state == Running` was reached only via `Ok(handle)` from
                // `drivers.get(driver_kind)` above, so the registry entry
                // is guaranteed present here.
                let driver = drivers.get(driver_kind).unwrap_or_else(|| {
                    unreachable!(
                        "state == Running implies driver.start() succeeded via a registry \
                         entry for driver_kind"
                    )
                });
                // transparent-mtls-host-socket (D-MTLS-15/16/17, step
                // 06-03): fire the (β) mTLS intercept-and-enforce
                // lifecycle alongside the driver hook. `Some` only on the
                // production boot (real `EbpfDataplane` + `MtlsDataplane`
                // composed post-`IdentityMgr`); `None` for the non-mTLS
                // fixture surface. Gated on `DriverType::Exec | Vm`: exec
                // workloads traverse the host veth directly, while VM
                // workloads now traverse the same host veth through their
                // tap-fed per-allocation netns (ADR-0089 §1 D6).
                // `start_alloc` installs the intercept but does NOT program
                // `MTLS_REDIRECT_DEST` (#241-deferred). `ExecDriver` is
                // UNTOUCHED.
                //
                // Fail-closed (D-MTLS-18): the install is a security
                // control, not a best-effort hook. On `Err` the alloc MUST
                // NOT run with cleartext, so we stop the just-spawned driver
                // process and supersede the already-committed `Running` row
                // with a `Failed` row carrying the typed cause-class —
                // mirroring the `StartRejected → Failed` precedent above
                // (`:823-832`, `:852-865`). The install gate is fired BEFORE
                // `release_for_exit_emission` precisely so the Running-gate /
                // exit-observer watcher is NEVER released for a now-`Failed`
                // alloc (the existing Failed-branch rule, `:876-881`): an
                // install failure leaves the watcher un-released, exactly as a
                // never-Running alloc does.
                if let Some(worker) = mtls_worker
                    && matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm)
                {
                    if let Err(cause) = worker.start_alloc(&spec) {
                        return fail_closed_on_mtls_install(
                            driver.as_ref(),
                            obs,
                            bus,
                            tick,
                            &row,
                            prior_state,
                            handle_opt.as_ref(),
                            &cause,
                        )
                        .await;
                    }
                    stable_exact_rule_baseline = true;
                    tracing::info!(
                        name: "mtls.intercept.install.success",
                        alloc = %spec.alloc,
                        driver = ?driver_kind,
                        "installed allocation mTLS intercept"
                    );
                }
                if exec_release_permitted(true, intercept_required, stable_exact_rule_baseline)
                    && let Some(handle) = &handle_opt
                {
                    // For VmDriver this existing hook first releases the
                    // deferred BeaconMessage::Exec reply, then the exit-event
                    // gate. Its placement strictly after start_alloc Ok is the
                    // born-captured ordering invariant (ADR-0089 §1/Q9).
                    driver.release_for_exit_emission(handle).await;
                }
                // Service-health-check-probes step 01-03d / ADR-0054
                // § 2: fire the lifecycle hook so the driver can
                // dispatch to its configured `ProbeRunner`. Default
                // no-op for SimDriver and any driver wired without
                // a probe runner.
                driver.on_alloc_running(&spec);
            }
            emit_event(bus, build_lifecycle_event(&row, prior_state, source));
            Ok(())
        }
        // Restart: stop-then-start, reusing the same alloc id. Per
        // ADR-0023 §2 Restart is semantically `stop + start` against
        // the prior alloc. Per ADR-0031 §5 the action carries a
        // fully-populated `AllocationSpec` constructed in the
        // reconciler from the live `Job`; the shim reads it straight
        // off the action. `find_prior_alloc_row` is still needed to
        // recover `(workload_id, node_id)` for the `AllocStatusRow` write.
        // Per ADR-0023 §2 / ADR-0037 §4 a restart is semantically
        // `stop + start` regardless of cause, and RestartAllocation
        // never carries a terminal claim. Under ADR-0087
        // `RestartAllocation` carries no cause field at all — the cause
        // is the prior observed alloc row's terminal (a crash terminal,
        // or `Stopped { by: LivenessProbe }` for a liveness kill), which
        // `WorkloadLifecycle` reads directly as the sole restart
        // authority.
        Action::RestartAllocation { alloc_id, mut spec, kind } => {
            // Stop half — Phase 1 uses an empty AllocationHandle (no pid
            // tracking yet). `NotFound` and ordinary stop errors are
            // best-effort, but a retained failed-start cleanup disposition is
            // authoritative: it is fed into this arm's observation write and
            // the start half is skipped.
            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            // Read-then-write (ADR-0083 §D2a(b), GH #42): the stop-half
            // reads the alloc→driver-kind index for the PRIOR instance
            // before the start-half re-inserts under the (possibly
            // unchanged) new spec's driver kind. Falls back to every
            // composed driver on a miss (see `resolve_drivers_for_alloc`)
            // — best-effort either way, mirroring the existing NotFound-
            // tolerant `driver.stop` semantics.
            let mut cleanup_retry: Option<(Arc<dyn Driver>, DriverError)> = None;
            let mut probed_prior_owner = false;
            let mut every_stop_was_not_found = true;
            for driver in resolve_drivers_for_alloc(drivers, alloc_drivers, &alloc_id) {
                probed_prior_owner = true;
                match driver.stop(&handle).await {
                    Ok(()) => every_stop_was_not_found = false,
                    Err(error) => {
                        if !matches!(&error, DriverError::NotFound { .. }) {
                            every_stop_was_not_found = false;
                        }
                        if let DriverError::StartRejected { failure } = &error
                            && is_duplicate_vm_owner(failure, &alloc_id)
                        {
                            // A recovered cleanup disposition is already in flight
                            // from an earlier write attempt. Do not create a second
                            // author or start a new VM behind it.
                            return Ok(());
                        }
                        if error.start_cleanup_failure().is_some() {
                            cleanup_retry = Some((Arc::clone(driver), error));
                            break;
                        }
                    }
                }
            }

            // Recover `(workload_id, node_id)` for the AllocStatusRow write
            // BEFORE the provision seam — the AC14 provision-failure → Failed-row
            // path needs the alloc's identity to write its `Failed` row, and a
            // restart with no prior row is a HandleMissing error regardless.
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Err(ShimError::HandleMissing { alloc_id });
            };
            // Extract prior_state before prior_row moves into build_alloc_status_row.
            let prior_state: AllocStateWire = prior_row.state.into();
            let driver_kind = spec.driver.driver_type();

            // Crash closure for retained start cleanup: the prior process can
            // remove the final residue and die after `VmDriver::stop` returns
            // its recovered disposition but before this arm writes `Failed`.
            // A fresh driver then has no process-local owner, so every
            // composed stop probe above returns ordinary `NotFound`. An `Ok`
            // or any other error is deliberately not treated as orphan proof.
            // The durable Pending discriminator is the only
            // surviving proof that the allocation was cleanup work rather
            // than spare capacity. Author its ending before provisioning or
            // calling start; a later lifecycle tick may apply ordinary Job /
            // Service policy to that Failed row with normal restart
            // accounting. The original diagnostic is forwarded byte-for-byte.
            if cleanup_retry.is_none()
                && probed_prior_owner
                && every_stop_was_not_found
                && is_start_cleanup_pending_row(&prior_row)
            {
                let updated_at = LogicalTimestamp::dominating(
                    tick.tick,
                    prior_row.node_id.clone(),
                    Some(&prior_row.updated_at),
                );
                let row = build_alloc_status_row(
                    alloc_id,
                    prior_row.workload_id.clone(),
                    prior_row.node_id.clone(),
                    AllocState::Failed,
                    updated_at,
                    prior_row.reason.clone(),
                    prior_row.detail.clone(),
                    None,
                    None,
                    kind,
                    prior_row.started_at,
                    None,
                    Some(&prior_row),
                );
                obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await?;
                emit_event(
                    bus,
                    build_lifecycle_event(&row, prior_state, TransitionSource::Driver(driver_kind)),
                );
                return Ok(());
            }

            // C3 PROVISION SEAM (D-TME-12 G2/G3 + JOIN-2/6 + AC14, step 04-01):
            // after the stop-half, before the start-half. `assign` is idempotent
            // for an already-held alloc (a Restart reuses the same slot) and
            // `provision_workload_netns` is idempotent converge-on-boot, so a
            // restart re-converges its existing netns rather than recreating it.
            // Fail-closed before `driver.start`. Off the mTLS gate this is a
            // no-op only for Exec; VM re-convergence remains mandatory.
            //
            // AC14 sub-claim 4: a persistent provision failure drives the alloc
            // to `Failed` (carrying `WorkloadNetnsProvisionFailed`) instead of
            // bubbling `Err` → indefinite Pending retry. Symmetric with the
            // StartAllocation arm above; the prior row supplies the identity for
            // the Failed-row write.
            let (cleanup_retry_driver, start_outcome): (
                Option<Arc<dyn Driver>>,
                Result<AllocationHandle, DriverError>,
            ) = if let Some((driver, error)) = cleanup_retry {
                // The stop-half already performed the one serialized cleanup
                // attempt. Feed that disposition through the existing
                // Pending/Failed observation path; provisioning or calling
                // start again would duplicate the cleanup attempt and could
                // strand a recovered disposition in flight.
                (Some(driver), Err(error))
            } else {
                if let Err(err) = provision_and_inject_netns(
                    &mut spec,
                    net_slot_allocator,
                    mtls_worker,
                    network_provisioner,
                ) {
                    let Some(cause) = netns_provision_cause(&err) else {
                        return Err(err);
                    };
                    return fail_closed_on_netns_provision(
                        obs,
                        bus,
                        tick,
                        alloc_id,
                        prior_row.workload_id.clone(),
                        prior_row.node_id.clone(),
                        kind,
                        prior_state,
                        cause,
                        Some(&prior_row),
                    )
                    .await;
                }

                // Registry lookup (ADR-0083 §D1/§D2a, GH #42) — same shape as
                // the StartAllocation arm above.
                let outcome = match drivers.get(driver_kind) {
                    Some(driver) => driver.start(&spec).await,
                    None => Err(DriverError::StartRejected {
                        failure: DriverStartFailure {
                            class: DriverStartClass::Unclassified { driver: driver_kind },
                            // ADR-0083 §D3d / DWD-25: name the absent
                            // capability and point at the executed boot
                            // reason, rather than reading as an internal
                            // defect. Driver-kind-generic (the same shape
                            // serves a future `wasm` miss), so no per-driver
                            // branch or parser is introduced; free-form
                            // verbatim `detail` per DWD-24, never a
                            // classification input, so no contract or
                            // conversion moves. The `driver.{kind}.not_composed`
                            // pointer names the startup-log event the driver's
                            // own composition emits (§D3c) where the specific
                            // probe reason was recorded.
                            detail: format!(
                                "no {driver_kind} driver composed on this node: the node's \
                                 {driver_kind} capability probe did not pass; see the startup \
                                 log's `driver.{driver_kind}.not_composed` reason for the \
                                 specific cause"
                            ),
                        },
                    }),
                };
                (None, outcome)
            };
            // Failed restart — same cause-class classification path
            // as StartAllocation. Per ADR-0032 §5: state is `Failed`
            // on driver `StartRejected`.
            let (handle_opt, state, reason, detail, source, cleanup_recovered): (
                Option<AllocationHandle>,
                AllocState,
                Option<TransitionReason>,
                Option<String>,
                TransitionSource,
                bool,
            ) = match start_outcome {
                Ok(handle) => (
                    Some(handle),
                    AllocState::Running,
                    Some(TransitionReason::Started),
                    None,
                    TransitionSource::Driver(driver_kind),
                    false,
                ),
                // DWD-24: apply the total, core-owned conversion and
                // preserve the driver's verbatim diagnostic separately.
                // No parsing, no prefix table, no `DriverType` dispatch —
                // the family comes from the typed class itself.
                Err(DriverError::StartRejected { failure })
                    if is_duplicate_vm_owner(&failure, &alloc_id) =>
                {
                    return Ok(());
                }
                Err(DriverError::StartRejected { failure }) => (
                    None,
                    AllocState::Failed,
                    Some(TransitionReason::from(&failure)),
                    Some(failure.detail.clone()),
                    TransitionSource::Driver(failure.class.driver_type()),
                    false,
                ),
                Err(other) => {
                    let Some(cleanup) = other.start_cleanup_failure() else {
                        return Err(ShimError::Driver(other));
                    };
                    let failure = cleanup.as_start_failure();
                    let cleanup_recovered = cleanup.recovery_complete();
                    (
                        None,
                        if cleanup_recovered { AllocState::Failed } else { AllocState::Pending },
                        Some(TransitionReason::from(&failure)),
                        Some(failure.detail.clone()),
                        TransitionSource::Driver(failure.class.driver_type()),
                        cleanup_recovered,
                    )
                }
            };
            if state == AllocState::Running
                && mtls_worker.is_some()
                && matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm)
                && let Err(issue_err) = ensure_intercept_identity(
                    &alloc_id,
                    &prior_row.workload_id,
                    &prior_row.node_id,
                    ca,
                    obs,
                    clock,
                    identity,
                )
                .await
            {
                if let Some(handle) = &handle_opt
                    && let Some(driver) = drivers.get(driver_kind)
                {
                    let _ = driver.stop(handle).await;
                    driver.release_supervision(&handle.alloc);
                }
                return Err(issue_err);
            }
            // Per ADR-0037 §4: RestartAllocation is never a terminal
            // claim. Same rationale as StartAllocation — restart is a
            // mid-budget recovery attempt; only `FinalizeFailed`
            // carries the BackoffExhausted terminal.
            //
            // Per ADR-0047 §1 / step 02-02 [D4]: kind comes from the
            // emitting action (sourced by the reconciler from the
            // hydrated `WorkloadLifecycleState.workload_kind`), NOT from
            // the prior row. The action's kind is the authoritative
            // value at every restart write.
            //
            // Subsidiary GAP-1 fix: a restart is a fresh process spawn
            // (`stop + start` per ADR-0023 §2) — capture a fresh
            // wall-clock for the new Pending → Running transition.
            // The reconciler's startup-probe / EarlyExit gates measure
            // elapsed since THIS process reached Running, not since
            // the prior (now-stopped) process did. On a failed restart
            // (`state == AllocState::Failed`) no new Running state was
            // reached; carry `None` forward — and a Phase-1
            // restart-rejected row that does not observe Running is
            // semantically equivalent to "never started."
            let started_at = if state == AllocState::Running {
                Some(tick.now_unix)
            } else {
                // Restart was rejected — never observed Running on
                // this attempt. Preserve the prior row's value (if
                // any) so a downstream FinalizeFailed terminal still
                // carries the prior generation's "started at" if it
                // ever reached Running.
                prior_row.started_at
            };
            // canonical-workload-address-inbound-tproxy (D-A1 / D-BLOCKER2, GH
            // #241): a restart re-provisions the netns/veth under the same
            // slot (idempotent converge-on-boot) — `spec.workload_addr` is
            // re-injected at the C3 seam. On reaching Running, populate the
            // row's canonical address from the freshly-provisioned spec, same
            // observed-input semantics as the StartAllocation arm above. A
            // rejected restart (`state == Failed`) carries `None`; the prior
            // generation's address (if any) is irrelevant to a never-Running
            // attempt.
            let workload_addr =
                if state == AllocState::Running { spec.workload_addr } else { None };
            let updated_at = LogicalTimestamp::dominating(
                tick.tick,
                prior_row.node_id.clone(),
                Some(&prior_row.updated_at),
            );
            // ADR-0078 § D2 borrow-ordering constraint — see the FinalizeFailed
            // arm above. Clone the two identity fields first so `prior_row`
            // survives to be borrowed in the final position.
            let workload_id = prior_row.workload_id.clone();
            let node_id = prior_row.node_id.clone();
            let row = build_alloc_status_row(
                alloc_id,
                workload_id,
                node_id,
                state,
                updated_at,
                reason,
                detail,
                None,
                None,
                kind,
                started_at,
                workload_addr,
                // ADR-0078 § D2 site 5 — THE crash-observability site. On a
                // successful restart the prior is terminal and `state` is
                // `Running`, so `advance` SNAPSHOTS the superseded terminal
                // into `last_terminated` and INCREMENTS `restart_count`. On a
                // driver-rejected restart (`state == Failed`) it forwards both
                // unchanged: nothing restarted.
                Some(&prior_row),
            );
            // Fires the Running-confirmed gate exposed by Driver::start.
            // Required for liveness — the watcher parks on this gate
            // before emitting ExitEvent. The two firing sites
            // (post-Running-Ok and post-degraded-escalation) are jointly
            // load-bearing; missing either leaks the watcher. Per RCA
            // `docs/feature/fix-exit-observer-running-gate/deliver/rca.md`
            // (Solution 1'). Symmetric with the StartAllocation arm
            // above. Failed-row branch (state == Failed, handle_opt ==
            // None) does NOT fire — restart-rejected reuses the prior
            // alloc id, but the new watcher was never spawned, so no
            // gate is awaited.
            // Symmetric with the StartAllocation arm above: on a Running-
            // write failure after a successful (re)start, tear the just-
            // spawned instance down before propagating so its exit watcher
            // is not stranded on the never-fired Running-confirmed gate and
            // its host footprint is not leaked past `live_allocations()`
            // (which would keep `VmReclamation` from reclaiming it). See
            // that arm for the full rationale.
            // `@mandatory:mutation_target`.
            if let Err(write_err) =
                obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await
            {
                if cleanup_recovered {
                    cleanup_retry_driver
                        .as_ref()
                        .or_else(|| drivers.get(driver_kind))
                        .unwrap_or_else(|| {
                            unreachable!(
                                "a recovered cleanup outcome can only come from a composed driver"
                            )
                        })
                        .retry_start_cleanup_disposition(&row.alloc_id);
                }
                if state == AllocState::Running
                    && let Some(handle) = &handle_opt
                    && let Some(driver) = drivers.get(driver_kind)
                {
                    let _ = driver.stop(handle).await;
                    // Clear the claim `stop` left `EndingInFlight` on a
                    // phased driver — without this the torn-down alloc
                    // leaks past `live_allocations()` and pins
                    // `VmReclamation` forever (greptile PR #268 P1). See the
                    // StartAllocation arm above for the full rationale.
                    // `@mandatory:mutation_target`.
                    driver.release_supervision(&handle.alloc);
                }
                return Err(write_err.into());
            }
            if cleanup_recovered {
                // Symmetric with StartAllocation: the restart stop-half cannot
                // release a Starting cleanup claim, and the start-half releases
                // it only after the authoritative Failed row is durable.
                cleanup_retry_driver
                    .as_ref()
                    .or_else(|| drivers.get(driver_kind))
                    .unwrap_or_else(|| {
                        unreachable!(
                            "a recovered cleanup outcome can only come from a composed driver"
                        )
                    })
                    .release_supervision(&row.alloc_id);
            }
            // mutants::skip — Running gate exercised by exit_observer_running_gate integration test; dispatch_single requires full Driver+ObservationStore wiring
            if state == AllocState::Running {
                // ADR-0083 §D2a(b) (GH #42): re-insert (the read-then-write
                // this arm's stop-half read before) — a restart re-inserts
                // the SAME key, so the index does not grow per restart.
                alloc_drivers.lock().insert(row.alloc_id.clone(), driver_kind);
                let driver = drivers.get(driver_kind).unwrap_or_else(|| {
                    unreachable!(
                        "state == Running implies driver.start() succeeded via a registry \
                         entry for driver_kind"
                    )
                });
                // transparent-mtls-host-socket (step 06-03): re-install
                // the mTLS intercept for the restarted alloc (reuses the
                // alloc id). `start_alloc` is idempotent — it tears the
                // prior intercept down first. Symmetric with the
                // StartAllocation arm above, including the D-MTLS-18
                // fail-closed handling: on install `Err`, stop the
                // just-spawned driver process and supersede the `Running`
                // row with a `Failed` row, BEFORE releasing the exit-emission
                // gate (so a now-`Failed` restart never releases the watcher).
                // Gated on `DriverType::Exec | Vm` — symmetric with the
                // StartAllocation arm above.
                if let Some(worker) = mtls_worker
                    && matches!(spec.driver.driver_type(), DriverType::Exec | DriverType::Vm)
                {
                    if let Err(cause) = worker.start_alloc(&spec) {
                        return fail_closed_on_mtls_install(
                            driver.as_ref(),
                            obs,
                            bus,
                            tick,
                            &row,
                            prior_state,
                            handle_opt.as_ref(),
                            &cause,
                        )
                        .await;
                    }
                    tracing::info!(
                        name: "mtls.intercept.install.success",
                        alloc = %spec.alloc,
                        driver = ?driver_kind,
                        "installed allocation mTLS intercept"
                    );
                }
                if let Some(handle) = &handle_opt {
                    // Symmetric with fresh start: post-install release is the
                    // only path that can send a VM guest its deferred EXEC.
                    driver.release_for_exit_emission(handle).await;
                }
                // Service-health-check-probes step 01-03d / ADR-0054
                // § 2: symmetric with the StartAllocation arm above.
                driver.on_alloc_running(&spec);
            }
            emit_event(bus, build_lifecycle_event(&row, prior_state, source));
            Ok(())
        }
        // Stop: best-effort driver stop, then write a Terminated row
        // for the alloc. Per ADR-0023 §2 the stop path is best-effort
        // — if the driver no longer tracks the alloc (NotFound), the
        // shim still records Terminated so the next tick's hydrate
        // sees the alloc gone. Per-variant error isolation: a Stop
        // failure does NOT abort dispatch of subsequent actions.
        // Per ADR-0037 §4: the `terminal` field on the action carries
        // the reconciler's typed terminal claim. The shim threads it
        // onto BOTH `AllocStatusRow.terminal` (durable surface) AND
        // `LifecycleEvent.terminal` (broadcast surface) from the SAME
        // dispatch call frame — drift between the two is structurally
        // impossible because both are populated from the same
        // `terminal` value at the same source site.
        Action::StopAllocation { alloc_id, terminal } => {
            // Look up prior obs row to recover (workload_id, node_id) for
            // the Terminated row we will write. If the alloc has no
            // obs row at all (e.g. the reconciler emitted Stop
            // without ever having seen the alloc Running) there is
            // nothing to write — return Ok.
            let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
                return Ok(());
            };
            // Extract prior_state before prior_row moves into build_alloc_status_row.
            let prior_state: AllocStateWire = prior_row.state.into();

            let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
            // ADR-0083 §D2a(b) (GH #42): `StopAllocation` carries no spec,
            // so the driver that owns this alloc is read from the index
            // written at Start/Restart, falling back to every composed
            // driver on a miss (see `resolve_drivers_for_alloc`). Ordinary
            // stop errors remain best-effort. A retained failed-start cleanup
            // error is different: incomplete cleanup keeps the row Pending,
            // while recovered cleanup is committed as the authoritative
            // terminal disposition before supervision is released. This
            // mirrors the Restart arm.
            let mut cleanup_retry_driver: Option<Arc<dyn Driver>> = None;
            let mut cleanup_reason: Option<TransitionReason> = None;
            let mut cleanup_detail: Option<String> = None;
            for driver in resolve_drivers_for_alloc(drivers, alloc_drivers, &alloc_id) {
                if let Err(error) = driver.stop(&handle).await {
                    if let DriverError::StartRejected { failure } = &error
                        && is_duplicate_vm_owner(failure, &alloc_id)
                    {
                        return Ok(());
                    }
                    if let Some(cleanup) = error.start_cleanup_failure() {
                        if !cleanup.recovery_complete() {
                            // Retain the Pending row, driver claim, and exact
                            // diagnostic. The lifecycle View timestamp drives
                            // the next bounded StopAllocation retry.
                            return Ok(());
                        }
                        let failure = cleanup.as_start_failure();
                        cleanup_reason = Some(TransitionReason::from(&failure));
                        cleanup_detail = Some(failure.detail);
                        cleanup_retry_driver = Some(Arc::clone(driver));
                        break;
                    }
                }
            }
            // The `reason` field carries the cause-class summary on
            // the row; the `terminal` field is the reconciler's
            // typed terminal claim and is the source of truth for
            // *who* initiated the stop (Operator vs Reconciler).
            // Phase 1 surfaces the legacy `Stopped { by: Reconciler }`
            // reason here for backwards compatibility on the wire-side
            // `last_transition.reason`; the operator-attribution lands
            // exclusively on `terminal`.
            // Subsidiary GAP-1 fix: StopAllocation is a terminal
            // operator-initiated stop — preserve the prior row's
            // `started_at` verbatim so downstream consumers
            // (e.g. settled-in / uptime renderers) still see when
            // the alloc reached Running. If it never reached Running
            // (Pending → Stopped), the prior value is `None` and
            // stays `None`.
            let prior_started_at = prior_row.started_at;
            let updated_at = LogicalTimestamp::dominating(
                tick.tick,
                prior_row.node_id.clone(),
                Some(&prior_row.updated_at),
            );
            // ADR-0078 § D2 borrow-ordering constraint — see the FinalizeFailed
            // arm above. Clone the two identity fields first so `prior_row`
            // survives to be borrowed in the final position.
            let workload_id = prior_row.workload_id.clone();
            let node_id = prior_row.node_id.clone();
            let preserves_cleanup_diagnostic = is_start_cleanup_pending_row(&prior_row);
            let reason = cleanup_reason
                .or_else(|| {
                    preserves_cleanup_diagnostic.then(|| {
                        prior_row.reason.clone().unwrap_or_else(|| {
                            unreachable!("cleanup Pending discriminator requires a reason")
                        })
                    })
                })
                .unwrap_or(TransitionReason::Stopped {
                    by: overdrive_core::transition_reason::StoppedBy::Reconciler,
                });
            let detail = cleanup_detail.or_else(|| {
                if preserves_cleanup_diagnostic { prior_row.detail.clone() } else { None }
            });
            let row = build_alloc_status_row(
                alloc_id,
                workload_id,
                node_id,
                AllocState::Terminated,
                updated_at,
                Some(reason),
                detail,
                terminal,
                None,
                prior_row.kind,
                prior_started_at,
                // Terminated row — a stopped alloc is not a live backend (the
                // bridge renders only `state == Running`), so it carries no
                // per-instance address.
                None,
                // ADR-0078 § D2 site 6: FORWARDS — `Running → Terminated` is a
                // non-terminal prior, so no snapshot and no increment. A
                // Terminated row carrying a prior generation's
                // `last_terminated` keeps that history visible on the durable
                // surface.
                Some(&prior_row),
            );
            if let Err(write_error) =
                obs.write(ObservationRow::AllocStatus(Box::new(row.clone()))).await
            {
                if let Some(driver) = &cleanup_retry_driver {
                    driver.retry_start_cleanup_disposition(&row.alloc_id);
                }
                return Err(write_error.into());
            }
            // Service-health-check-probes step 01-03d / ADR-0054 § 2:
            // fire the terminal lifecycle hook so the driver can
            // cancel every per-probe task spawned under this
            // alloc's supervisor. Default no-op for drivers wired
            // without a `ProbeRunner`. We use `row.alloc_id` rather
            // than the moved `alloc_id` binding because the latter
            // was consumed by `build_alloc_status_row` above. Falls back
            // to every composed driver on an index miss (see
            // `resolve_drivers_for_alloc`).
            for driver in resolve_drivers_for_alloc(drivers, alloc_drivers, &row.alloc_id) {
                driver.on_alloc_terminal(&row.alloc_id);
                // brief.md §105a.3 transition 6 / DD-1(b.i) (ADR-0083 §D7,
                // GH #42): every shim arm that writes a terminal row calls
                // `release_supervision` AFTER the write resolves `Ok` — the
                // claim is on AUTHORING AN ENDING, not a grip on a running
                // process, and this write IS that authored ending. Without
                // this call a driver whose claim carries a phase (VmDriver:
                // `Live` -> `EndingInFlight` on `stop()`) never releases —
                // `live_allocations()` reports `EndingInFlight` entries as
                // still held (by construction: it returns every key in the
                // map), so `SupervisionSet::reclamation_authorised` would
                // read `false` for this alloc forever, and `VmReclamation`
                // could never reclaim an artifact this same stop() left
                // stranded. Idempotent / a no-op for drivers that do not
                // report supervision (`ExecDriver` keeps the trait default),
                // so this fires unconditionally alongside `on_alloc_terminal`
                // for every driver kind.
                driver.release_supervision(&row.alloc_id);
            }
            // ADR-0083 §D2a(b) (GH #42) — this IS the operator-stop
            // terminal-row authoring the shim's stop arm owns (brief
            // §105a.3 transition 3b / ADR-0082 §D4 reconciliation) — the
            // exit watcher no longer emits an ExitEvent for an operator
            // stop, so this write is the sole author of the Terminated
            // row above. Remove the alloc_drivers ROUTING INDEX entry now
            // that the terminal write has landed — lifetime bounded by
            // "started this boot", per the ADR's own accounting. Distinct
            // from `release_supervision` immediately above: this index is
            // the shim's own alloc-to-driver-kind lookup table, not the
            // driver's supervision claim.
            alloc_drivers.lock().remove(&row.alloc_id);
            // transparent-mtls-host-socket (step 06-03): tear down the
            // alloc's mTLS intercept on Stop — symmetric with the
            // FinalizeFailed arm above. Idempotent.
            if let Some(worker) = mtls_worker {
                worker.stop_alloc(&row.alloc_id);
            }
            // C3 TEARDOWN SEAM (D-TME-12 G2, step 04-01): tear down the
            // per-alloc netns + veth, THEN release the slot — AFTER the driver
            // stop. Symmetric with the StopAllocation arm. Idempotent; no-op
            // for an alloc that never provisioned.
            teardown_and_release_netns(&row.alloc_id, net_slot_allocator, network_provisioner)?;
            emit_event(bus, build_lifecycle_event(&row, prior_state, TransitionSource::Reconciler));
            Ok(())
        }
        // phase-2-xdp-service-map Slice 08 (US-08; ASR-2.2-04) —
        // The shim invokes `Dataplane::update_service(...)` via the
        // canonical per-arm dispatch fn at
        // `dataplane_update_service::dispatch`, which writes the
        // outcome row to `service_hydration_results` per
        // architecture.md § 7 *Failure surface*. A
        // `Dataplane::update_service` failure does NOT surface as
        // `ShimError` — it lands as a `Failed` observation row and
        // dispatch returns `Ok(DispatchOutcome::Failed)`. Only an
        // ObservationStore write failure surfaces as
        // `ShimError::Observation`.
        action @ Action::DataplaneUpdateService { .. } => {
            dataplane_update_service::dispatch(&action, dataplane, obs, tick, writer_node)
                .await
                .map_err(|e| match e {
                    dataplane_update_service::ServiceHydrationDispatchError::ObservationWrite {
                        source,
                    } => ShimError::Observation(source),
                    dataplane_update_service::ServiceHydrationDispatchError::Ipv6Unsupported {
                        ..
                    } => {
                        unreachable!(
                            "Ipv6Unsupported is handled inside dispatch — it writes \
                             a Failed row and returns Ok(DispatchOutcome::Failed)"
                        )
                    }
                })?;
            Ok(())
        }
        // service-vip-allocator step 03-02 — real dispatch arm per
        // ADR-0049 (amended 2026-05-15). Threads the digest +
        // correlation into the per-arm `release_service_vip::dispatch`
        // which owns the `tokio::sync::Mutex` guard + the
        // `PersistentServiceVipAllocator::release` call (memo +
        // IntentStore allocator_entries row removal in
        // fsync-then-memory order). On Ok, the released VIP returns to
        // the pool for reallocation on the next `allocate(&fresh)`. On
        // Err, the typed `PersistentAllocatorError` surfaces via
        // `ShimError::AllocatorRelease { #[from] source }` so callers
        // can `matches!` on the structured cause without re-parsing
        // `Display` (per `.claude/rules/development.md` § "Never
        // flatten a typed error to `Internal(String)`").
        Action::ReleaseServiceVip { spec_digest, correlation } => {
            release_service_vip::dispatch(&spec_digest, &correlation, allocator).await
        }
        // backend-discovery-bridge-service-reachability step 01-04 —
        // GREEN. The per-arm dispatch wrapper in
        // `crates/overdrive-control-plane/src/action_shim/
        // write_service_backend_row.rs` writes the row via
        // `ObservationStore::write(ObservationRow::ServiceBackend(row))`.
        // No correlation-driven follow-up at the shim level — the
        // bridge's next tick reads the row stream (transitively
        // through the runtime's hydrate path) and observes its own
        // write via the dedup fingerprint in
        // `BackendDiscoveryBridgeView::last_written_fingerprint`. An
        // `ObservationStore::write` failure surfaces as
        // `ShimError::Observation` via the typed `#[from]` variant
        // per `.claude/rules/development.md` § Errors / pass-through.
        action @ Action::WriteServiceBackendRow { .. } => {
            write_service_backend_row::dispatch(&action, obs).await.map_err(ShimError::from)
        }
        // backend-discovery-bridge-service-reachability UI-05 —
        // cross-reconciler handoff at the action boundary. The
        // wrapper takes a brief lock-grab-submit-release on the
        // broker mutex; per `.claude/rules/development.md`
        // § Concurrency & async the guard is dropped before any
        // subsequent `.await` (the wrapper is a sync function and
        // the per-action loop awaits between iterations).
        action @ Action::EnqueueEvaluation { .. } => {
            let mut guard = broker.lock();
            enqueue_evaluation::dispatch(&action, &mut guard);
            drop(guard);
            Ok(())
        }
        // ADR-0053 § 3 — same-host backend delivery via
        // cgroup_sock_addr. The hydrator's classifier emits this
        // variant for every backend whose IP matches `host_ipv4`
        // (Phase 1 single-node: every Running alloc). The shim
        // invokes `Dataplane::register_local_backend` which writes
        // the LOCAL_BACKEND_MAP entry the cgroup_connect4_service
        // program reads on every connect(2). No observation row
        // dispatch — the cgroup hook is not an HTTP-call surface.
        action @ Action::RegisterLocalBackend { .. } => {
            register_local_backend::dispatch(&action, dataplane).await.map_err(ShimError::from)?;
            Ok(())
        }
        action @ Action::DeregisterLocalBackend { .. } => {
            deregister_local_backend::dispatch(&action, dataplane)
                .await
                .map_err(ShimError::from)?;
            Ok(())
        }
        // workload-identity-manager step 01-06 (ADR-0067 D3) — the
        // `issue_svid` executor is the ONE place workload-CA I/O happens.
        // `IssueSvid` mints the leaf + writes the `issued_certificates` audit
        // row + binds the two via `ca_issuance::issue_and_audit` (reused
        // wholesale), then holds the returned `SvidMaterial` in `IdentityMgr`
        // and refreshes the trust bundle (D6). On an audit-write failure the
        // executor returns `Err` and holds NOTHING — no unaudited SVID escapes
        // (K4 fail-closed). `DropSvid` removes the held entry so the node-held
        // leaf key is no longer reachable (O2/K2). The validity window is
        // `issue_and_audit`'s sole concern (single clock read); the executor
        // never reads a clock to build one.
        action @ Action::IssueSvid { .. } => {
            issue_svid::dispatch_issue(&action, ca, obs, clock, identity)
                .await
                .map_err(ShimError::from)?;
            Ok(())
        }
        action @ Action::DropSvid { .. } => {
            issue_svid::dispatch_drop(&action, identity);
            Ok(())
        }
        // microvm-driver-cloud-hypervisor step 02-03 (ADR-0083 §D7,
        // brief.md §105a.5/§105a.6, GH #42). `ReclaimAllocation` authors a
        // Platform Reclamation ending (execution-time supervision lease ->
        // write-time terminality guard -> kill -> discard -> write -> four
        // evaluations); a refused race
        // returns `Ok(())` by design, never a `ShimError`.
        Action::ReclaimAllocation { alloc_id } => reclamation::execute_reclaim_allocation(
            &alloc_id,
            drivers,
            host,
            obs,
            clock,
            writer_node,
            broker,
        )
        .await
        .map_err(ShimError::from),
        // `DiscardStrandedArtifacts` authors NO ending: kill -> discard,
        // nothing else (no row write, no evaluation — DD-5's "declared
        // delta empty over the observation universe" is structural, per
        // the executor's own signature carrying no `ObservationStore`
        // and no broker parameter).
        Action::DiscardStrandedArtifacts { alloc_id } => {
            reclamation::execute_discard_stranded_artifacts(&alloc_id, drivers, host)
                .await
                .map_err(ShimError::from)
        }
    }
}

/// Look up the LWW-winner observation row for `alloc_id`, used by the
/// Restart and Stop variants to recover `(workload_id, node_id)` for the
/// Terminated row they write. Returns `Ok(None)` when no row exists —
/// callers decide whether that is an error (Restart) or a no-op (Stop).
async fn find_prior_alloc_row(
    obs: &dyn ObservationStore,
    alloc_id: &AllocationId,
) -> Result<Option<AllocStatusRow>, ShimError> {
    Ok(obs.alloc_status_row(alloc_id).await?)
}

/// Errors from [`dispatch`] that cannot be resolved into an
/// observation row. Per ADR-0023 §3.
#[derive(Debug, thiserror::Error)]
pub enum ShimError {
    /// A driver failure that did not fit the `SpawnFailed` shape (i.e.
    /// the shim cannot record it as `state: Failed`).
    #[error("driver failure")]
    Driver(#[from] DriverError),
    /// The observation store itself rejected the write.
    #[error("observation write failure")]
    Observation(#[from] ObservationStoreError),
    /// The shim could not look up an `AllocationHandle` for the
    /// requested `alloc_id` — typically when a Stop / Restart action
    /// arrives for an alloc the driver no longer tracks.
    #[error("alloc handle missing for {alloc_id}")]
    HandleMissing {
        /// The allocation whose handle is missing.
        alloc_id: overdrive_core::id::AllocationId,
    },
    /// The [`PersistentServiceVipAllocator::release`] call failed —
    /// typically a byte-level `IntentStore::delete` rejection (disk
    /// full, file corruption, redb internal error). Pass-through
    /// `#[from]` per `.claude/rules/development.md` § Errors so the
    /// typed cause is preserved end-to-end. Service-vip-allocator
    /// step 03-02 / ADR-0049.
    #[error("release_service_vip failed: {source}")]
    AllocatorRelease {
        /// The underlying typed error from the allocator.
        #[from]
        source: PersistentAllocatorError,
    },
    /// `register_local_backend` shim dispatch failed (ADR-0053 § 3).
    /// Pass-through `#[from]` preserves the typed
    /// `DataplaneError::LocalBackendInsert` cause.
    #[error("register_local_backend dispatch failed")]
    RegisterLocalBackend(#[from] register_local_backend::RegisterLocalBackendDispatchError),
    /// `deregister_local_backend` shim dispatch failed (ADR-0053 § 3).
    #[error("deregister_local_backend dispatch failed")]
    DeregisterLocalBackend(#[from] deregister_local_backend::DeregisterLocalBackendDispatchError),
    /// `issue_svid` executor dispatch failed (ADR-0067 D3) — CA signing OR an
    /// audit-write failure. Pass-through `#[from]` preserves the typed
    /// `IssueSvidDispatchError` (which itself embeds the typed
    /// `CaIssuanceError`), so callers can `matches!` on `Audit` vs `Ca`
    /// without `Display`-grepping. On either, the held map is left untouched
    /// (K4 fail-closed — no unaudited SVID held).
    #[error("issue_svid dispatch failed")]
    IssueSvid(#[from] issue_svid::IssueSvidDispatchError),
    /// The `WorkflowEngine::start` dispatch failed (ADR-0064 §5) — the
    /// engine could not resolve the workflow kind or load its journal.
    /// The shim surfaces this as a typed `ShimError` rather than swallow
    /// it, mirroring the StartAllocation driver-failure surface.
    #[error("workflow engine start failed: {message}")]
    WorkflowEngine {
        /// Cause string from the engine's typed `WorkflowEngineError`.
        message: String,
    },

    /// Persisting a workflow-instance desired-intent on
    /// `Action::StartWorkflow` commit failed (ADR-0064 §5). The intent
    /// write is the `hydrate_desired` SSOT the workflow-lifecycle
    /// reconciler reads back; a failure here means the instance would not
    /// be re-emittable on restart, so it is surfaced rather than dropped.
    #[error("workflow intent persistence failed: {message}")]
    WorkflowIntent {
        /// Cause string from the `IntentStore` error.
        message: String,
    },

    /// The per-allocation netns + veth (and, for VM, guest TAP wire) could not
    /// be provisioned at the C3 seam BEFORE `Driver::start`
    /// (transparent-mtls-enrollment D-TME-12 G2,
    /// step 04-01). Fail-closed: the workload MUST NOT spawn into the host
    /// netns when its per-workload netns could not be created — the
    /// confidentiality boundary the netns establishes would be absent.
    /// Pass-through `#[from]` per `.claude/rules/development.md` § Errors so
    /// the typed per-step `VethProvisionError` cause is preserved.
    #[error("workload netns provisioning failed")]
    WorkloadNetnsProvision(#[from] VethProvisionError),

    /// Every per-host network slot (`0..=NET_SLOT_MAX`) is held, so a NEW
    /// allocation cannot be assigned a collision-free slot at the C3 seam
    /// (transparent-mtls-enrollment D-TME-12 G3, step 04-01). The alloc is
    /// REFUSED rather than dropped onto a shared veth/subnet (the B1
    /// collision the slot model exists to prevent). Pass-through `#[from]`.
    #[error("network slot exhausted")]
    NetSlotExhausted(#[from] NetSlotExhausted),
    /// A `VmReclamation` executor failed (`Action::ReclaimAllocation` /
    /// `Action::DiscardStrandedArtifacts`, ADR-0083 §D7, brief.md
    /// §105a.5). Pass-through `#[from]` preserves the typed
    /// `reclamation::ReclamationError` (itself a `VmHostState` or
    /// `ObservationStore` substrate failure) — never emitted for a
    /// refused race, which returns `Ok(())` by design.
    #[error("VmReclamation executor failed")]
    Reclamation(#[from] reclamation::ReclamationError),
}

#[cfg(test)]
mod tests {
    //! Pre-flight per-action isolation regression (ADR-0064 §5).
    //!
    //! Drives the pure-async helper [`persist_workflow_intents`] directly
    //! — it IS the driving port for the intent-persist pre-flight stage.
    //! The observable universe is `(dispatchable, first_error)` plus the
    //! intent store's persisted bytes (read back via `get`). No real
    //! redb / FS — a fault-injecting in-memory `IntentStore` keeps this in
    //! the default lane.

    // Test-double constructors + `.expect()` on infallible test reads are
    // idiomatic in test code; the const-fn / expect-used lints add ceremony
    // with no test value (mirrors the file-level allow on the sibling
    // `tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs`).
    #![allow(clippy::expect_used, clippy::missing_const_for_fn)]

    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use bytes::Bytes;
    use futures::Stream;
    use overdrive_core::aggregate::IntentKey;
    use overdrive_core::id::{ContentHash, CorrelationKey};
    use overdrive_core::traits::driver::DriverType;
    use overdrive_core::traits::intent_store::{
        IntentStore, IntentStoreError, PutOutcome, StateSnapshot, TxnOp, TxnOutcome,
    };
    use overdrive_core::workflow::{WorkflowName, WorkflowStart};

    use super::{Action, ShimError, network_assignment_required, persist_workflow_intents};

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_vm_requires_network_assignment_even_without_mtls_composition() {
        assert!(network_assignment_required(DriverType::Vm, false));
        assert!(network_assignment_required(DriverType::Vm, true));
        assert!(network_assignment_required(DriverType::Exec, true));
        assert!(!network_assignment_required(DriverType::Exec, false));
    }

    /// In-memory `IntentStore` that fails `put` for one configured
    /// "poison" key and otherwise stores the bytes. `get` reflects what
    /// actually persisted so the test can assert which intents landed.
    /// Ordered map per `.claude/rules/development.md` § "Ordered-collection
    /// choice".
    struct FaultInjectingIntentStore {
        stored: parking_lot::Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
        poison_key: Vec<u8>,
    }

    impl FaultInjectingIntentStore {
        fn with_poison(poison_key: Vec<u8>) -> Self {
            Self { stored: parking_lot::Mutex::new(BTreeMap::new()), poison_key }
        }
    }

    #[async_trait]
    impl IntentStore for FaultInjectingIntentStore {
        async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError> {
            Ok(self.stored.lock().get(key).map(|v| Bytes::copy_from_slice(v)))
        }

        async fn put(&self, key: &[u8], value: &[u8]) -> Result<(), IntentStoreError> {
            if key == self.poison_key.as_slice() {
                return Err(IntentStoreError::Busy);
            }
            self.stored.lock().insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        async fn put_if_absent(
            &self,
            _key: &[u8],
            _value: &[u8],
        ) -> Result<PutOutcome, IntentStoreError> {
            Ok(PutOutcome::Inserted)
        }

        async fn delete(&self, key: &[u8]) -> Result<(), IntentStoreError> {
            self.stored.lock().remove(key);
            Ok(())
        }

        async fn txn(&self, ops: Vec<TxnOp>) -> Result<TxnOutcome, IntentStoreError> {
            // Faithful in-memory apply, mirroring the production
            // `LocalIntentStore::txn` arm (`redb_backend.rs`). The match is
            // exhaustive (no `_` catch-all) so a future `TxnOp` variant forces
            // a compile decision here rather than being silently ignored. The
            // poison-key fault is a `put`-level fault for the
            // `persist_workflow_intents` tests and deliberately does NOT apply
            // to the txn path — `txn` always commits, matching the production
            // contract.
            {
                let mut stored = self.stored.lock();
                for op in ops {
                    match op {
                        TxnOp::Put { key, value } => {
                            stored.insert(key.to_vec(), value.to_vec());
                        }
                        TxnOp::Delete { key } => {
                            stored.remove(key.as_ref());
                        }
                        TxnOp::IncrementU64 { key } => {
                            // Length-guarded BE-u64 decode per development.md §
                            // "Safe byte-slice access": absent or non-8-byte
                            // row decodes as 0 (never `bytes[0..8]`, never a
                            // panic); write `current + 1` saturating at
                            // `u64::MAX`.
                            let current = stored
                                .get(key.as_ref())
                                .and_then(|v| <[u8; 8]>::try_from(v.as_slice()).ok())
                                .map_or(0u64, u64::from_be_bytes);
                            let next = current.saturating_add(1);
                            stored.insert(key.to_vec(), next.to_be_bytes().to_vec());
                        }
                    }
                }
            }
            Ok(TxnOutcome::Committed)
        }

        async fn watch(
            &self,
            _prefix: &[u8],
        ) -> Result<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + Unpin>, IntentStoreError>
        {
            Ok(Box::new(futures::stream::empty()))
        }

        async fn scan_prefix(
            &self,
            prefix: &[u8],
        ) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError> {
            Ok(self
                .stored
                .lock()
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| (Bytes::copy_from_slice(k), Bytes::copy_from_slice(v)))
                .collect())
        }

        async fn export_snapshot(&self) -> Result<StateSnapshot, IntentStoreError> {
            Ok(StateSnapshot::from_parts(0, Vec::new(), Vec::new()))
        }

        async fn bootstrap_from(&self, _snapshot: StateSnapshot) -> Result<(), IntentStoreError> {
            Ok(())
        }
    }

    fn start_workflow(slug: &str) -> (Action, CorrelationKey, WorkflowStart) {
        let spec = WorkflowStart {
            name: WorkflowName::new("provision-record").expect("valid kebab name"),
            input: Vec::new(),
        };
        // Correlation is derived from the workflow-KIND identity (the spec
        // name) — unrelated to the persisted intent VALUE, which is now the
        // full `archive_for_store` envelope (the #217 fix above), never the
        // name bytes.
        let kind_name = spec.name.as_str();
        let kind_digest = ContentHash::of(kind_name.as_bytes());
        let correlation = CorrelationKey::derive(slug, &kind_digest, "start-workflow");
        let action =
            Action::StartWorkflow { start: spec.clone(), correlation: correlation.clone() };
        (action, correlation, spec)
    }

    /// One failed intent write (for B) must NOT discard the rest of the
    /// batch: A, C, and the interleaved non-workflow action survive into
    /// `dispatchable`; B is dropped; the first error surfaces; and exactly
    /// A's and C's intents persist — never B's. This is the per-action
    /// isolation the pre-flight `?` early-return previously bypassed.
    #[tokio::test]
    async fn preflight_isolation_one_failed_intent_does_not_drop_the_batch() {
        let (action_a, corr_a, _) = start_workflow("wf-a-0001");
        let (action_b, corr_b, _) = start_workflow("wf-b-0002");
        let (action_c, corr_c, _) = start_workflow("wf-c-0003");

        let key_a = IntentKey::for_workflow_instance(&corr_a).as_bytes().to_vec();
        let key_b = IntentKey::for_workflow_instance(&corr_b).as_bytes().to_vec();
        let key_c = IntentKey::for_workflow_instance(&corr_c).as_bytes().to_vec();

        // Fail ONLY B's intent write.
        let store = FaultInjectingIntentStore::with_poison(key_b.clone());

        // Interleave a non-workflow action so the test also pins that
        // non-StartWorkflow actions always survive.
        let actions = vec![action_a.clone(), action_b, Action::Noop, action_c.clone()];

        let (dispatchable, first_error) = persist_workflow_intents(&store, actions).await;

        // 1. The first error is a WorkflowIntent failure.
        assert!(
            matches!(first_error, Some(ShimError::WorkflowIntent { .. })),
            "B's failed intent write must surface as ShimError::WorkflowIntent; \
             got {first_error:?}"
        );

        // 2. dispatchable contains A, C, and the Noop — and NOT B.
        assert!(
            dispatchable.contains(&action_a),
            "A (intent persisted) must survive into dispatchable; got {dispatchable:?}"
        );
        assert!(
            dispatchable.contains(&action_c),
            "C (intent persisted) must survive into dispatchable; got {dispatchable:?}"
        );
        assert!(
            dispatchable.contains(&Action::Noop),
            "the non-workflow action must always survive into dispatchable; \
             got {dispatchable:?}"
        );
        assert!(
            !dispatchable.iter().any(|a| matches!(
                a,
                Action::StartWorkflow { correlation, .. } if *correlation == corr_b
            )),
            "B (intent write failed) must be DROPPED from dispatchable — starting it \
             would leave a non-re-emittable instance; got {dispatchable:?}"
        );

        // 3. The store persisted A's and C's intents, and NOT B's.
        assert!(store.get(&key_a).await.expect("get a").is_some(), "A's intent must be persisted");
        assert!(store.get(&key_c).await.expect("get c").is_some(), "C's intent must be persisted");
        assert!(
            store.get(&key_b).await.expect("get b").is_none(),
            "B's intent must NOT be persisted (its put failed)"
        );
    }
}

#[cfg(test)]
mod guest_exec_release_tests {
    #![allow(clippy::doc_markdown)]

    use super::exec_release_permitted;

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn exec_release_requires_a_stable_exact_rule_baseline() {
        for running in [false, true] {
            for required in [false, true] {
                for stable_exact in [false, true] {
                    let permitted = exec_release_permitted(running, required, stable_exact);
                    assert_eq!(
                        permitted,
                        running && (!required || stable_exact),
                        "release matrix drifted for running={running}, required={required}, stable_exact={stable_exact}",
                    );
                }
            }
        }
        assert!(!exec_release_permitted(true, true, false));
        assert!(exec_release_permitted(true, true, true));
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test fixtures may panic on programmer error per project precedent in tests/"
)]
mod fail_closed_mtls_tests {
    //! Contract of the transparent-mTLS intercept-install fail-closed handler
    //! [`fail_closed_on_mtls_install`] (GH #250 / ADR-0076).
    //!
    //! The helper is module-private and every one of its eight arguments is
    //! default-lane constructible with no I/O, so this module drives it
    //! DIRECTLY — no socket, no netns, no subprocess, no tempdir. That is a
    //! deliberate, bounded departure from the driving-port rule: the
    //! call-site ordering property (the arms `return` BEFORE
    //! `driver.release_for_exit_emission`) is NOT observable here and is
    //! defended separately by the integration-lane test that drives
    //! [`dispatch`].
    //!
    //! # SUT state machine
    //!
    //! ```text
    //!   Pending --driver.start Ok--> Running(row written, watcher parked)
    //!       |                            |
    //!       |                            +-- start_alloc Ok --> release gate, on_alloc_running
    //!       |                            |
    //!       |                            +-- start_alloc Err --> [fail_closed_on_mtls_install]
    //!       |                                                       stop driver (best effort)
    //!       |                                                       write superseding Failed row
    //!       |                                                       emit LifecycleEvent
    //!       |                                                       return WITHOUT releasing gate
    //!       +-- driver.start StartRejected --> Failed (handle None, gate never armed;
    //!                                          the mTLS guard is unreachable — it sits
    //!                                          inside `if state == AllocState::Running`)
    //! ```
    //!
    //! `Failed` is terminal within this helper: the guard fires at most once
    //! per dispatch, so a second install failure for an already-`Failed`
    //! alloc is structurally unreachable.
    //!
    //! # Universe
    //!
    //! Port-exposed observables only: the returned `Result<(), ShimError>`;
    //! every `AllocStatusRow` readable from the `ObservationStore` for the
    //! alloc; [`RecordingDriver`]'s `stops` / `releases` /
    //! `on_alloc_running_calls` records; every [`LifecycleEvent`] received on
    //! the broadcast bus. Nothing private is read.

    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::time::{Duration, Instant};

    use overdrive_core::UnixInstant;
    use overdrive_core::aggregate::WorkloadKind;
    use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
    use overdrive_core::reconcilers::TickContext;
    use overdrive_core::traits::driver::{
        AllocationHandle, AllocationSpec, AllocationState, Driver, DriverError, DriverStartClass,
        DriverStartFailure, DriverType, Resources,
    };
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
        ObservationStoreError,
    };
    use overdrive_sim::adapters::observation_store::SimObservationStore;
    use overdrive_worker::mtls_intercept::{InterceptError, NetlinkError};
    use overdrive_worker::mtls_intercept_worker::MtlsInterceptInstallError;
    use tokio::sync::broadcast;

    use super::{
        AllocStateWire, LifecycleEvent, ShimError, TransitionReason, TransitionSource,
        fail_closed_on_mtls_install,
    };

    /// What [`RecordingDriver::stop`] reports back. The helper's stop is
    /// deliberately best-effort (`let _ = driver.stop(handle).await;`), so
    /// both arms must leave the Failed row and the un-released gate intact.
    #[derive(Debug, Clone, Copy)]
    enum StopOutcome {
        /// The workload was still there and stopped cleanly.
        Ok,
        /// The workload already exited between `driver.start` returning and
        /// the intercept install completing — the production-reachable
        /// `DriverError::NotFound` shape the helper's own comment names.
        NotFound,
    }

    /// Recording [`Driver`] double (the `InertDriver` precedent at
    /// `tests/acceptance/finalize_failed_forward_carries_workload_addr.rs`).
    ///
    /// Records `stop`, and overrides the two DEFAULTED no-op lifecycle hooks
    /// — `release_for_exit_emission` and `on_alloc_running` — so their
    /// non-invocation is observable rather than assumed.
    struct RecordingDriver {
        stops: parking_lot::Mutex<Vec<AllocationId>>,
        releases: parking_lot::Mutex<Vec<AllocationId>>,
        on_alloc_running_calls: parking_lot::Mutex<Vec<AllocationId>>,
        stop_outcome: StopOutcome,
    }

    impl RecordingDriver {
        fn new(stop_outcome: StopOutcome) -> Self {
            Self {
                stops: parking_lot::Mutex::new(Vec::new()),
                releases: parking_lot::Mutex::new(Vec::new()),
                on_alloc_running_calls: parking_lot::Mutex::new(Vec::new()),
                stop_outcome,
            }
        }
    }

    #[async_trait::async_trait]
    impl Driver for RecordingDriver {
        fn r#type(&self) -> DriverType {
            DriverType::Exec
        }

        async fn start(&self, _spec: &AllocationSpec) -> Result<AllocationHandle, DriverError> {
            Err(DriverError::StartRejected {
                failure: DriverStartFailure {
                    class: DriverStartClass::Unclassified { driver: DriverType::Exec },
                    detail: "RecordingDriver: start() is not on the fail-closed path".to_owned(),
                },
            })
        }

        async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError> {
            self.stops.lock().push(handle.alloc.clone());
            match self.stop_outcome {
                StopOutcome::Ok => Ok(()),
                StopOutcome::NotFound => Err(DriverError::NotFound { alloc: handle.alloc.clone() }),
            }
        }

        async fn status(&self, handle: &AllocationHandle) -> Result<AllocationState, DriverError> {
            Err(DriverError::NotFound { alloc: handle.alloc.clone() })
        }

        async fn resize(
            &self,
            _handle: &AllocationHandle,
            _resources: Resources,
        ) -> Result<(), DriverError> {
            Ok(())
        }

        async fn release_for_exit_emission(&self, handle: &AllocationHandle) {
            self.releases.lock().push(handle.alloc.clone());
        }

        fn on_alloc_running(&self, spec: &AllocationSpec) {
            self.on_alloc_running_calls.lock().push(spec.alloc.clone());
        }
    }

    /// The `updated_at.counter` the seeded `Running` row carries — deliberately
    /// `TICK + 1`, i.e. EXACTLY the tick floor a row written during [`TICK`]
    /// resolves to. This models the REAL production shape: the `Running` row
    /// and the superseding `Failed` row are written in the SAME tick by the SAME
    /// node, so a tick-derived counter on the successor would TIE and the
    /// `Failed` row would be silently dropped by LWW (GH #250 / ADR-0076
    /// § Decision 7).
    ///
    /// The prior value (`0`, against `TICK = 7`) let a tick-derived counter
    /// clear the seed ARTIFICIALLY — a different-counter shape production never
    /// produces — which is precisely what masked the defect through steps
    /// 01-01, 02-01 and 03-01. The successor now dominates only because
    /// [`LogicalTimestamp::dominating`] derives its counter from the row being
    /// superseded, with the tick as a mere floor (ADR-0077 § D1).
    const SEEDED_RUNNING_COUNTER: u64 = TICK + 1;
    /// The tick the helper reads its `LogicalTimestamp` from.
    const TICK: u64 = 7;
    /// Wall-clock the alloc reached Running at — forward-carried verbatim
    /// onto the `Failed` row (assertion A-9, the #248 drop shape).
    const STARTED_AT_SECS: u64 = 1_700_000_000;
    /// The per-instance address the seeded `Running` row owns. The `Failed`
    /// row must NOT inherit it (assertion A-4) — a failed alloc is not a live
    /// backend.
    const RUNNING_WORKLOAD_ADDR: Ipv4Addr = Ipv4Addr::new(10, 42, 0, 2);

    /// The alloc identity triple every case shares.
    fn ids() -> (AllocationId, WorkloadId, NodeId) {
        (
            AllocationId::new("mif-alloc-0").expect("valid alloc id"),
            WorkloadId::new("mif-workload").expect("valid workload id"),
            NodeId::new("mif-node").expect("valid node id"),
        )
    }

    /// The `Running` `AllocStatusRow` the helper supersedes. Carries a
    /// `Some(..)` `started_at` and a `Some(..)` `workload_addr` so the
    /// forward-carry (A-9) and the deliberate non-carry (A-4) are both
    /// falsifiable.
    fn seeded_running_row(
        alloc: &AllocationId,
        workload: &WorkloadId,
        node: &NodeId,
    ) -> AllocStatusRow {
        AllocStatusRow {
            alloc_id: alloc.clone(),
            workload_id: workload.clone(),
            node_id: node.clone(),
            state: AllocState::Running,
            updated_at: LogicalTimestamp { counter: SEEDED_RUNNING_COUNTER, writer: node.clone() },
            reason: Some(TransitionReason::Started),
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: WorkloadKind::Service,
            listeners: Vec::new(),
            started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(STARTED_AT_SECS))),
            workload_addr: Some(RUNNING_WORKLOAD_ADDR),
            last_terminated: None,
            restart_count: 0,
        }
    }

    fn tick_context() -> TickContext {
        let now = Instant::now();
        TickContext {
            now,
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(STARTED_AT_SECS + 5)),
            tick: TICK,
            deadline: now + Duration::from_millis(100),
        }
    }

    /// `InterceptError::TransparentListener` carrying the `errno` the real
    /// syscall would have returned (`EPERM` = missing `CAP_NET_ADMIN`).
    fn transparent_listener(errno: i32) -> InterceptError {
        InterceptError::TransparentListener {
            addr: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
            source: std::io::Error::from_raw_os_error(errno),
        }
    }

    /// `InterceptError::NftRuleInstallFailed` carrying the failing nft `op` and
    /// the real errno-carrying [`NetlinkError::Nft`] source the hand-rolled
    /// `nft::append_rule` path produces (ADR-0085 D3) — the typed, op-keyed
    /// replacement for the removed nft-stderr string catch-all.
    fn nft_install_failed(op: &'static str, errno: i32) -> InterceptError {
        InterceptError::NftRuleInstallFailed {
            op,
            source: NetlinkError::nft(op, std::io::Error::from_raw_os_error(errno)),
        }
    }

    /// The SIX constructible `MtlsInterceptInstallError` shapes paired with
    /// the `stage()` string each must map onto, against the CLOSED four-value
    /// vocabulary. Cases 5 and 6 pin the two ALIAS arms — `LegFLocalAddr` and
    /// `LegCLocalAddr` fold onto their bind siblings' stage strings, and
    /// without them a future edit splitting the alias arms goes unnoticed.
    ///
    /// Every shape is built from an EXISTING public variant with public field
    /// types: enum-level `#[non_exhaustive]` blocks exhaustive MATCHING, not
    /// CONSTRUCTION. No new constructor, no `#[doc(hidden)]`, no new variant.
    fn install_error_cases() -> Vec<(&'static str, MtlsInterceptInstallError, &'static str)> {
        vec![
            (
                "outbound nft-TPROXY install",
                MtlsInterceptInstallError::OutboundTproxyInstall(nft_install_failed(
                    "append-egress",
                    libc::EPERM,
                )),
                "outbound_tproxy_install",
            ),
            (
                "leg-F transparent bind",
                MtlsInterceptInstallError::LegFBind(transparent_listener(libc::EPERM)),
                "leg_f_bind",
            ),
            (
                "leg-C transparent bind",
                MtlsInterceptInstallError::Inbound(transparent_listener(libc::ENOPROTOOPT)),
                "leg_c_transparent_listener",
            ),
            (
                "inbound nft-TPROXY install",
                MtlsInterceptInstallError::Inbound(nft_install_failed(
                    "append-inbound",
                    libc::ENOENT,
                )),
                "inbound_tproxy",
            ),
            (
                "leg-F getsockname capture (alias arm)",
                MtlsInterceptInstallError::LegFLocalAddr {
                    source: std::io::Error::from_raw_os_error(libc::ENOTSOCK),
                },
                "leg_f_bind",
            ),
            (
                "leg-C getsockname capture (alias arm)",
                MtlsInterceptInstallError::LegCLocalAddr {
                    source: std::io::Error::from_raw_os_error(libc::ENOTSOCK),
                },
                "leg_c_transparent_listener",
            ),
        ]
    }

    /// The complete port-exposed universe one drive of the helper produces.
    /// Captured as owned data so each assertion helper reads a snapshot
    /// rather than re-locking the double mid-assertion.
    struct Observed {
        outcome: Result<(), ShimError>,
        row: Option<AllocStatusRow>,
        stops: Vec<AllocationId>,
        releases: Vec<AllocationId>,
        on_alloc_running_calls: Vec<AllocationId>,
        events: Vec<LifecycleEvent>,
    }

    /// Seed a `Running` row, drive [`fail_closed_on_mtls_install`] once, and
    /// capture the whole universe. Returns the seeded row alongside the
    /// observation so supersession is asserted against a real predecessor.
    ///
    /// `reject_write` arms `SimObservationStore::inject_write_failure` AFTER
    /// the seed — the queue is FIFO-consumed, so the helper's own write is
    /// the one refused.
    async fn drive(
        cause: &MtlsInterceptInstallError,
        stop_outcome: StopOutcome,
        reject_write: bool,
    ) -> (AllocStatusRow, Observed) {
        let (alloc, workload, node) = ids();
        let store = SimObservationStore::single_peer(node.clone(), 0);
        let running_row = seeded_running_row(&alloc, &workload, &node);
        store
            .write(ObservationRow::AllocStatus(Box::new(running_row.clone())))
            .await
            .expect("seeding the Running row must succeed");
        if reject_write {
            store.inject_write_failure(ObservationStoreError::Unreachable {
                peer: "mif-peer".to_owned(),
            });
        }

        let driver = RecordingDriver::new(stop_outcome);
        let (bus, mut events) = broadcast::channel::<LifecycleEvent>(16);
        let tick = tick_context();
        let handle = AllocationHandle { alloc: alloc.clone(), pid: Some(4242) };

        let outcome = fail_closed_on_mtls_install(
            &driver,
            &store,
            &bus,
            &tick,
            &running_row,
            AllocStateWire::Running,
            Some(&handle),
            cause,
        )
        .await;

        let mut drained = Vec::new();
        while let Ok(event) = events.try_recv() {
            drained.push(event);
        }
        let observed = Observed {
            outcome,
            row: store.alloc_status_row(&alloc).await.expect("reading the alloc row must succeed"),
            stops: driver.stops.lock().clone(),
            releases: driver.releases.lock().clone(),
            on_alloc_running_calls: driver.on_alloc_running_calls.lock().clone(),
            events: drained,
        };
        (running_row, observed)
    }

    /// Assertions A-2, A-3, A-4, A-8, A-9 — the superseding `Failed` row's
    /// full field contract, including the forward-carried identity/kind and
    /// the deliberately NON-carried per-instance address.
    fn assert_superseding_failed_row(
        label: &str,
        seeded: &AllocStatusRow,
        observed: &Observed,
        expected_stage: &str,
        expected_detail: &str,
    ) {
        let row = observed.row.as_ref().unwrap_or_else(|| {
            panic!("[{label}] the store must hold a row for the alloc after fail-closed")
        });

        // A-2 — Failed, strictly dominating the seeded Running row under LWW.
        assert_eq!(
            row.state,
            AllocState::Failed,
            "[{label}] the fail-closed handler must supersede Running with Failed"
        );
        assert_eq!(
            row.updated_at.counter,
            SEEDED_RUNNING_COUNTER + 1,
            "[{label}] the Failed row must carry the seeded Running row's counter PLUS ONE so it \
             strictly dominates under LWW — the two rows are written in the SAME tick by the SAME \
             node, so a tick-derived counter would tie and the Failed row would be silently \
             dropped; got {} vs seeded {}",
            row.updated_at.counter,
            seeded.updated_at.counter
        );

        // A-3 — the typed cause-class, its per-case `stage`, the verbatim detail.
        match row.reason.as_ref() {
            Some(TransitionReason::MtlsInterceptInstallFailed { stage, detail }) => {
                assert_eq!(
                    stage, expected_stage,
                    "[{label}] the Failed row must name the refusing install stage against \
                     the closed vocabulary"
                );
                assert_eq!(
                    detail, expected_detail,
                    "[{label}] the reason detail must be the verbatim cause Display"
                );
            }
            other => panic!(
                "[{label}] the Failed row must carry \
                 TransitionReason::MtlsInterceptInstallFailed; got {other:?}"
            ),
        }
        assert_eq!(
            row.detail.as_deref(),
            Some(expected_detail),
            "[{label}] the detail column must carry the verbatim cause Display"
        );

        // A-4 — a failed alloc is not a live backend and makes no terminal claim.
        assert_eq!(
            row.workload_addr, None,
            "[{label}] the Failed row must NOT inherit the Running row's per-instance \
             address — a failed alloc is not a live backend"
        );
        assert_eq!(
            row.terminal, None,
            "[{label}] a single mid-budget install failure is not a terminal claim — \
             WorkloadLifecycle owns BackoffExhausted"
        );

        // A-8 — identity + kind forward-carried byte-equal.
        assert_eq!(
            row.alloc_id, seeded.alloc_id,
            "[{label}] alloc_id must be forward-carried verbatim"
        );
        assert_eq!(
            row.workload_id, seeded.workload_id,
            "[{label}] workload_id must be forward-carried verbatim"
        );
        assert_eq!(
            row.node_id, seeded.node_id,
            "[{label}] node_id must be forward-carried verbatim"
        );
        assert_eq!(row.kind, seeded.kind, "[{label}] kind must be forward-carried verbatim");

        // A-9 — `started_at` forward-carried as `Some(..)`, never dropped to
        // `None` (the GH #248 forward-carry drop shape).
        assert_eq!(
            row.started_at, seeded.started_at,
            "[{label}] started_at must be forward-carried verbatim, never dropped to None"
        );
        assert!(
            row.started_at.is_some(),
            "[{label}] the fixture's Running row carries Some(started_at), so a None here \
             is the forward-carry drop, not an honest absence"
        );
    }

    /// Assertions A-5 and A-6 — the workload is stopped exactly once, and the
    /// exit-emission gate is NEVER released for a now-`Failed` alloc.
    fn assert_stopped_once_and_gate_never_released(
        label: &str,
        alloc: &AllocationId,
        observed: &Observed,
    ) {
        assert_eq!(
            observed.stops.as_slice(),
            std::slice::from_ref(alloc),
            "[{label}] the fail-closed handler must stop the spawned workload exactly once"
        );
        assert!(
            observed.releases.is_empty(),
            "[{label}] the exit-emission gate must NEVER be released by the fail-closed \
             handler; got {:?}",
            observed.releases
        );
        assert!(
            observed.on_alloc_running_calls.is_empty(),
            "[{label}] a now-Failed alloc must never be announced Running to its driver; \
             got {:?}",
            observed.on_alloc_running_calls
        );
    }

    /// Assertions A-7 and A-10 — exactly ONE lifecycle transition, from the
    /// prior state to `Failed`, attributed to the reconciler.
    fn assert_single_reconciler_failed_event(label: &str, observed: &Observed) {
        assert_eq!(
            observed.events.len(),
            1,
            "[{label}] exactly ONE LifecycleEvent must be emitted; got {:?}",
            observed.events
        );
        let event = &observed.events[0];
        assert_eq!(
            event.to,
            AllocStateWire::Failed,
            "[{label}] the announced transition must land on Failed"
        );
        assert_eq!(
            event.from,
            AllocStateWire::Running,
            "[{label}] the announced transition must originate at the prior state"
        );
        assert_eq!(
            event.source,
            TransitionSource::Reconciler,
            "[{label}] the fail-closed transition is reconciler-attributed"
        );
    }

    /// S-MIF-01 — an intercept-install failure drives the allocation `Failed`,
    /// stops the driver, and never releases the exit gate.
    ///
    /// Table-driven over the six constructible cause shapes (the module
    /// idiom; the argument space is a closed six-element set, so a generator
    /// would be parametrisation theatre). Assertions A-1 … A-10 are
    /// case-invariant except A-3's `stage`, pinned per case.
    #[tokio::test]
    async fn install_failure_supersedes_running_with_failed_and_never_releases_the_gate() {
        for (label, cause, expected_stage) in install_error_cases() {
            let expected_detail = cause.to_string();
            let (seeded, observed) = drive(&cause, StopOutcome::Ok, false).await;

            // A-1 — the dispatch itself succeeded; the alloc is durably Failed.
            assert!(
                observed.outcome.is_ok(),
                "[{label}] the handler must return Ok(()) — the failure is RECORDED, not \
                 surfaced as ShimError; got {:?}",
                observed.outcome
            );

            assert_superseding_failed_row(
                label,
                &seeded,
                &observed,
                expected_stage,
                &expected_detail,
            );
            assert_stopped_once_and_gate_never_released(label, &seeded.alloc_id, &observed);
            assert_single_reconciler_failed_event(label, &observed);
        }
    }

    /// S-MIF-02 — a driver stop that errors does not prevent the Failed row.
    ///
    /// A workload that exits between `driver.start` returning and the
    /// intercept install completing yields `DriverError::NotFound` from
    /// `driver.stop`. The helper's stop is deliberately best-effort; turning
    /// it into a `?` would abort BEFORE the Failed row is written, leaving the
    /// alloc recorded `Running` with no interception installed — the
    /// un-alarmed exclusion-mechanism failure.
    #[tokio::test]
    async fn a_vanished_workload_still_yields_the_superseding_failed_row() {
        let cause = MtlsInterceptInstallError::LegFBind(transparent_listener(libc::EPERM));
        let (seeded, observed) = drive(&cause, StopOutcome::NotFound, false).await;

        assert!(
            observed.outcome.is_ok(),
            "a NotFound stop is tolerated — the handler must still report Ok(()); got {:?}",
            observed.outcome
        );
        let row = observed.row.as_ref().expect("the store must hold a row for the alloc");
        assert_eq!(
            row.state,
            AllocState::Failed,
            "a vanished workload must NOT prevent the superseding Failed row — otherwise the \
             alloc stays recorded Running with no mTLS interception installed"
        );
        assert!(
            matches!(row.reason, Some(TransitionReason::MtlsInterceptInstallFailed { .. })),
            "the Failed row must still carry the install-failure cause-class; got {:?}",
            row.reason
        );
        assert_stopped_once_and_gate_never_released(
            "vanished workload",
            &seeded.alloc_id,
            &observed,
        );
    }

    /// S-MIF-03 — an observation-store write rejection surfaces as an error
    /// and emits no lifecycle event.
    ///
    /// Pins the write-then-emit ORDERING: an edit that emitted the transition
    /// before (or regardless of) the write would announce a `Failed`
    /// transition no durable row backs.
    #[tokio::test]
    async fn a_rejected_observation_write_surfaces_and_announces_nothing() {
        let cause = MtlsInterceptInstallError::LegFBind(transparent_listener(libc::EPERM));
        let (_seeded, observed) = drive(&cause, StopOutcome::Ok, true).await;

        assert!(
            matches!(observed.outcome, Err(ShimError::Observation(_))),
            "a rejected observation write must be REPORTED as ShimError::Observation, never \
             swallowed; got {:?}",
            observed.outcome
        );
        assert!(
            observed.events.is_empty(),
            "no lifecycle transition may be announced without a durable row behind it; \
             got {:?}",
            observed.events
        );
        assert!(
            observed.releases.is_empty(),
            "the exit-emission gate must still never be released; got {:?}",
            observed.releases
        );
    }
}
