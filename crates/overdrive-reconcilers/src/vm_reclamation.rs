//! `VmReclamation` — SD-1's Bar-2 [`Reconciler`] (`.claude/rules/reconcilers.md`).
//!
//! Per ADR-0083 §D7 / `brief.md` §105a: `reconcile` is `(plan_reclamation(d,
//! a), VmReclamationView::default())` — the pure diff [`plan_reclamation`]
//! takes NO port and IS the whole safety property (the bug class "the
//! observe pass wrote something" is structurally unrepresentable, since the
//! function has nothing to write with). The two [`Action`] variants it can
//! emit — [`Action::ReclaimAllocation`] (authors a Platform-Reclamation
//! ending) and [`Action::DiscardStrandedArtifacts`] (authors none) — ARE the
//! plan; the executors (a later step) are the impure half.
//!
//! ADR-0081 D1's Ending-Class classification (Intentional Stop / Workload
//! Failure / Platform Reclamation) is NOT defined in this module (02-01
//! review D1 correction — a prior revision duplicated it here as
//! `is_intentional_stop` / `is_workload_failure` / `is_platform_reclamation`,
//! which this fix removes). Per ADR-0083 §D6, exactly ONE new PUBLIC
//! predicate is sanctioned: `is_platform_reclaimed`, co-located with the
//! vocabulary it reads in `overdrive_core::transition_reason`. The Intentional Stop
//! leg REUSES the existing `workload_lifecycle::is_intentionally_stopped`
//! (module-private, unchanged in meaning — Platform Reclamation must not
//! match it); Workload Failure has no named predicate at all — it is the
//! unnamed residual `state.is_terminal() && !is_intentionally_stopped(row)
//! && !is_platform_reclaimed(row)`, expressed inline wherever it is
//! needed. This module does NOT wire these predicates into
//! `is_natural_exit` / `startup_probe_failed_action` (ADR-0081's
//! binding-sites table, `brief.md` lines ~900-910) — that integration is a
//! later step's obligation.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use overdrive_core::AllocationId;
use overdrive_core::aggregate::{WorkloadDriver, WorkloadIntent, scan_workload_intents};
use overdrive_core::id::WorkloadId;
use overdrive_core::reconcilers::{HydrateError, HydrationContext};
use overdrive_core::traits::driver::DriverType;
use overdrive_core::traits::vm_host_state::VmHostObservation;

use super::{
    Action, Reconciler, ReconcilerName, ResyncSchedule, ResyncScope, TargetResource, TickContext,
};

// ---------------------------------------------------------------------------
// SupervisionSet — the observed supervision discriminator (brief §105a.3)
// ---------------------------------------------------------------------------

/// The observed supervision discriminator — an input on `actual`, never a
/// `View` marker (`brief.md` §105a.3 / §105a.6 — "Where the discriminating
/// fact must live"). Fails safe by construction: [`Self::Unavailable`] is
/// the [`Default`] and authorises nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SupervisionSet {
    /// DEFAULT. The platform's live-supervision handles have not been
    /// enumerated on this half of the state, or the enumeration failed.
    /// Authorises NOTHING — absence of evidence is not evidence of
    /// absence.
    #[default]
    Unavailable,
    /// A SUCCESSFUL enumeration. Membership is authoritative, and an
    /// EMPTY set means "the platform supervises nothing" — it means that
    /// only because the enumeration succeeded.
    Observed(BTreeSet<AllocationId>),
}

impl SupervisionSet {
    /// The ONE kill-authorising predicate in the design: every
    /// [`plan_reclamation`] row that can reach a LIVE VMM consults it,
    /// with exactly one stated exemption whose value is a theorem rather
    /// than an observation (the terminal-row case in [`plan_reclamation`]
    /// below). `Unavailable` is `false` — never "unsupervised". Mandatory
    /// mutation target (`brief.md` §105a.3, §113).
    #[must_use]
    pub fn reclamation_authorised(&self, alloc: &AllocationId) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Observed(held) => !held.contains(alloc),
        }
    }
}

// ---------------------------------------------------------------------------
// State — the hydration seam is a named, separable step (brief §105a.2)
// ---------------------------------------------------------------------------

/// Per-allocation facts the DESIRED half carries — the two-surface join
/// (intent-side `WorkloadDriver == Vm`, applied once at hydration) plus
/// whether the allocation's row is terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmAllocFacts {
    /// Owning workload — needed by the executor to derive the four
    /// evaluation `TargetResource`s (`brief.md` §105a.5); NOT carried on
    /// the emitted [`Action`] itself (DD-5's `alloc_id`-only payload).
    pub workload_id: WorkloadId,
    /// Whether this allocation's `AllocStatusRow.state` is terminal
    /// (`AllocState::is_terminal()`).
    pub terminal: bool,
}

/// `VmReclamation`'s [`Reconciler::State`] projection.
///
/// `hydrate_desired`'s arm fills `allocations` and leaves the other two at
/// [`Default`]; `hydrate_actual`'s arm calls [`VmHostState::observe`](overdrive_core::traits::vm_host_state::VmHostState::observe)
/// and reads the supervision set, leaving `allocations` empty — mirroring
/// `BackendDiscoveryBridge`'s two arms exactly (`brief.md` §105a.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VmReclamationState {
    /// DESIRED half — hydrated from the intent + observation stores.
    /// Contains ONLY allocations whose intent-side `WorkloadDriver` is
    /// `Vm`.
    pub allocations: BTreeMap<AllocationId, VmAllocFacts>,
    /// ACTUAL half — the resource this reconciler manages: the three
    /// host-observation surfaces.
    pub host: VmHostObservation,
    /// ACTUAL half — the supervision discriminator.
    pub supervision: SupervisionSet,
}

/// `VmReclamation`'s [`Reconciler::View`] projection. **FIELD-LESS**, per
/// the ADR-0079 precedent (`BackendDiscoveryBridgeView`) — retry falls out
/// of the runtime's `has_work` self-re-enqueue; no `View` field, no backoff
/// memo (ADR-0079's ruling, adopted verbatim per `brief.md` §105a.1).
///
/// Why field-less is safe here, stated rather than assumed: the diff is
/// `desired` versus **observed** `actual`; nothing this reconciler ever
/// emitted is consulted. A marker here would gate whether a live VM is
/// killed — the exact `reconcilers.md` fingerprint-as-diff shape this
/// project structurally refuses.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VmReclamationView {}

// ---------------------------------------------------------------------------
// The diff — one pure function, and it is the whole safety property
// (brief §105a.4)
// ---------------------------------------------------------------------------

/// PURE. No port parameter, no clock, no I/O — the bug class "the observe
/// pass wrote something" is not representable because this function has
/// nothing to write with. Mandatory mutation target (`brief.md` §105a.4,
/// §108, §113).
///
/// For every allocation id appearing on any of `actual.host`'s three
/// surfaces, the six-row decision table (`brief.md` §105a.4):
///
/// | `desired.allocations` says | authorised? | Emit |
/// |---|---|---|
/// | non-terminal VM allocation | `true` | [`Action::ReclaimAllocation`] |
/// | non-terminal VM allocation | `false` | nothing |
/// | terminal VM allocation | exempt (theorem, not observation) | [`Action::DiscardStrandedArtifacts`] |
/// | no entry, VM-exclusive surface | `true` | [`Action::DiscardStrandedArtifacts`] |
/// | no entry, VM-exclusive surface | `false` | nothing |
/// | no entry, cgroup-scope-only | not reached | nothing |
#[must_use]
pub fn plan_reclamation(desired: &VmReclamationState, actual: &VmReclamationState) -> Vec<Action> {
    let mut host_ids: BTreeSet<AllocationId> = BTreeSet::new();
    host_ids.extend(actual.host.scopes.keys().cloned());
    host_ids.extend(actual.host.run_dirs.iter().cloned());
    host_ids.extend(actual.host.clones.keys().cloned());

    let mut actions = Vec::new();
    for alloc_id in host_ids {
        match desired.allocations.get(&alloc_id) {
            Some(facts) if facts.terminal => {
                // Row 3 — an EXEMPTION, not skipped: under DD-1(b.i)'s
                // corollary a terminal-row instance is never still
                // claimed, so `reclamation_authorised` is *provably*
                // true here. Calling it would be a tautology.
                actions.push(Action::DiscardStrandedArtifacts { alloc_id });
            }
            Some(_non_terminal_facts) => {
                // Rows 1-2 — a non-terminal VM allocation. The ONE
                // kill-authorising predicate decides.
                if actual.supervision.reclamation_authorised(&alloc_id) {
                    actions.push(Action::ReclaimAllocation { alloc_id });
                }
            }
            None => {
                // Rows 4-6 — no entry. Gated on being a VM-exclusive
                // surface (run dir or clone) — a cgroup-scope-only
                // presence is shared with exec allocations and is left
                // alone (row 6).
                let vm_exclusive = actual.host.run_dirs.contains(&alloc_id)
                    || actual.host.clones.contains_key(&alloc_id);
                if vm_exclusive && actual.supervision.reclamation_authorised(&alloc_id) {
                    actions.push(Action::DiscardStrandedArtifacts { alloc_id });
                }
            }
        }
    }
    actions
}

// ---------------------------------------------------------------------------
// VmReclamation — the Reconciler impl (brief §105a.1, ADR-0083 §D7)
// ---------------------------------------------------------------------------

/// SD-1's Bar-2 registered [`Reconciler`]. `TargetResource` is
/// **node-scoped** (`node/<node_id>`) — every OTHER reconciler is
/// `workload/<id>`-scoped; this one observes a whole-node tree, so a
/// per-workload target would re-walk it once per workload.
#[derive(Debug, Clone)]
pub struct VmReclamation {
    name: ReconcilerName,
}

impl VmReclamation {
    /// Construct the reconciler. Carries no per-node state of its own —
    /// the node it observes is supplied externally via the
    /// `TargetResource` (`node/<node_id>`) the runtime evaluates it
    /// against, mirroring every other reconciler's `hydrate_desired` /
    /// `hydrate_actual` target-parsing convention.
    #[must_use]
    pub fn new() -> Self {
        #[allow(clippy::expect_used)]
        let name = ReconcilerName::new(<Self as Reconciler>::NAME)
            .expect("'vm-reclamation' satisfies ReconcilerName::new's validator");
        Self { name }
    }
}

impl Default for VmReclamation {
    fn default() -> Self {
        Self::new()
    }
}

/// Level-triggered resync cadence for `vm-reclamation` — the period at
/// which the Piece A cadence loop re-submits a `node/<local_node_id>`
/// sweep (ADR-0084 §4; GH #266). `VmReclamation` is host-backed
/// (`reconcile` hydrates `actual` live from the host via the
/// `VmHostState` port and declares no event interests), so this periodic
/// resync is its SOLE trigger — the level-triggered safety net with no
/// edge to fall back on. Formerly the `VM_RECLAMATION_SWEEP_INTERVAL`
/// hardcode inside `spawn_convergence_loop`; unified onto the generic
/// cadence hook so the loop names no reconciler and carries no cadence
/// constant.
pub const VM_RECLAMATION_SWEEP_INTERVAL: Duration = Duration::from_secs(30);

#[async_trait::async_trait]
impl Reconciler for VmReclamation {
    const NAME: &'static str = "vm-reclamation";

    type State = VmReclamationState;
    type View = VmReclamationView;

    fn name(&self) -> &ReconcilerName {
        &self.name
    }

    fn reconcile(
        &self,
        desired: &Self::State,
        actual: &Self::State,
        _view: &Self::View,
        _tick: &TickContext,
    ) -> (Vec<Action>, Self::View) {
        (plan_reclamation(desired, actual), VmReclamationView::default())
    }

    fn resync_schedule(&self) -> Option<ResyncSchedule> {
        // Host-backed ⇒ resync-only (ADR-0084 §1/§4, Piece A): `actual` is
        // hydrated live from the host and no observation-row change wakes
        // this reconciler, so the level-triggered cadence is its ONLY
        // trigger. Fires `node/<local_node_id>` every
        // `VM_RECLAMATION_SWEEP_INTERVAL`; the loop resolves the target from
        // the `NodeId` it owns via `resolve_scope(LocalNode, ..)`. This is
        // the generic-hook replacement for the former `spawn_convergence_loop`
        // hardcode.
        Some(ResyncSchedule {
            period: VM_RECLAMATION_SWEEP_INTERVAL,
            scope: ResyncScope::LocalNode,
        })
    }

    /// Hydrate the `desired` two-surface join (VM-driven intent × alloc rows;
    /// ADR-0086 D1). Target-agnostic — the desired join scans the whole-node
    /// `workloads/` prefix. Moved off `reconciler_runtime::hydrate_desired`
    /// `VmReclamation` arm (S3).
    async fn hydrate_desired(
        &self,
        ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let allocations = hydrate_vm_reclamation_desired(ctx).await?;
        Ok(VmReclamationState { allocations, ..Default::default() })
    }

    /// Hydrate the `actual` side: `VmHostState::observe()` FIRST, then the VM
    /// supervision set LAST (brief.md §105a.2 ordering). `allocations` stays
    /// empty (the desired arm owns it). Moved off
    /// `reconciler_runtime::hydrate_vm_reclamation_actual` (S3).
    async fn hydrate_actual(
        &self,
        ctx: &HydrationContext<'_>,
        _target: &TargetResource,
    ) -> Result<Self::State, HydrateError> {
        let host = ctx
            .vm_host_state
            .observe()
            .await
            .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;
        // No `Vm` registry entry ⇒ `Observed(∅)` — a KNOWN fact (the platform
        // holds no VM supervision handle), not a missing observation (S-VM-30).
        let supervision = ctx.drivers.get(DriverType::Vm).map_or_else(
            || SupervisionSet::Observed(BTreeSet::new()),
            |driver| {
                driver.live_allocations().map_or(SupervisionSet::Unavailable, |ids| {
                    SupervisionSet::Observed(ids.into_iter().collect())
                })
            },
        );
        Ok(VmReclamationState { allocations: BTreeMap::new(), host, supervision })
    }
}

// ---------------------------------------------------------------------------
// Hydration body — moved off the central `reconciler_runtime` free fn
// (ADR-0086 S3). `pub` because the boot-epoch drive
// (`vm_reclamation_boot::converge`) calls the SAME desired-side join per
// brief.md §105a.6 ("one observation function").
// ---------------------------------------------------------------------------

/// Desired-side two-surface join for `VmReclamation` (ADR-0083 §D7): scan the
/// whole-node `workloads/` intent prefix for `WorkloadIntent::Job` intents whose
/// driver is `WorkloadDriver::Vm`, then join that set against
/// `ObservationStore::alloc_status_rows()` to populate `VmAllocFacts` per
/// `AllocationId`.
pub async fn hydrate_vm_reclamation_desired(
    ctx: &HydrationContext<'_>,
) -> Result<BTreeMap<AllocationId, VmAllocFacts>, HydrateError> {
    let records = scan_workload_intents(ctx.intent_store, ctx.intent_redb_path)
        .await
        .map_err(|e| HydrateError::IntentRead(e.to_string()))?;

    let mut vm_workloads: BTreeSet<WorkloadId> = BTreeSet::new();
    for (_key, intent) in records {
        let WorkloadIntent::Job(job) = &intent else { continue };
        if matches!(job.driver, WorkloadDriver::Vm(_)) {
            vm_workloads.insert(job.id.clone());
        }
    }

    if vm_workloads.is_empty() {
        return Ok(BTreeMap::new());
    }

    let alloc_rows = ctx
        .observation_store
        .alloc_status_rows()
        .await
        .map_err(|e| HydrateError::ObservationRead(e.to_string()))?;

    let mut allocations = BTreeMap::new();
    for row in alloc_rows {
        if vm_workloads.contains(&row.workload_id) {
            allocations.insert(
                row.alloc_id.clone(),
                VmAllocFacts {
                    workload_id: row.workload_id.clone(),
                    terminal: row.state.is_terminal(),
                },
            );
        }
    }
    Ok(allocations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervision_set_default_is_unavailable() {
        assert_eq!(SupervisionSet::default(), SupervisionSet::Unavailable);
    }

    #[test]
    fn vm_reclamation_name_is_kebab_case_and_valid() {
        let r = VmReclamation::new();
        assert_eq!(r.name().as_str(), "vm-reclamation");
        assert_eq!(<VmReclamation as Reconciler>::NAME, "vm-reclamation");
    }

    #[test]
    fn plan_reclamation_empty_state_emits_nothing() {
        let empty = VmReclamationState::default();
        assert_eq!(plan_reclamation(&empty, &empty), Vec::new());
    }
}
