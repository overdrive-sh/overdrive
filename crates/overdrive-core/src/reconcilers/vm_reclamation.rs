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
//! This module also carries the ADR-0081 D1 Ending-Class classification
//! (`is_intentional_stop` / `is_workload_failure` / `is_platform_reclamation`)
//! as pure predicates over a terminal [`AllocStatusRow`] — kept as
//! predicates rather than a stored `EndingClass` enum per `brief.md`'s
//! Domain Model section ("VM workloads — the ending taxonomy"): the
//! totality/disjointness property (P1, `brief.md` §105a.10) is proven by
//! proptest, not by a compile-time exhaustive match. **This step does NOT
//! wire these predicates into `is_natural_exit` /
//! `startup_probe_failed_action`** (ADR-0081's binding-sites table,
//! `brief.md` lines ~900-910) — that integration is a later step's
//! obligation; this module only defines the classification itself, proven
//! total and disjoint in isolation.
//!
//! `is_platform_reclamation` is structurally `false` for every row
//! representable today: `StoppedBy::PlatformReclaimed` (ADR-0081 D5) has
//! not landed. ADR-0081's "Narrows ADR-0078 §D1" section binds the commit
//! that appends that variant to ALSO carry a same-commit amendment note on
//! ADR-0078 §D1 and a docstring correction in `observation_store.rs` —
//! that bundle lands with `execute_reclaim_allocation` (the first real
//! producer of the disposition), not with this reconciler skeleton. See
//! this feature's DELIVER step notes for the explicit flag.

use std::collections::{BTreeMap, BTreeSet};

use crate::AllocationId;
use crate::id::WorkloadId;
use crate::traits::observation_store::{AllocState, AllocStatusRow};
use crate::traits::vm_host_state::VmHostObservation;
use crate::transition_reason::{StoppedBy, TerminalCondition, TransitionReason};

use super::{Action, Reconciler, ReconcilerName, TickContext};

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
/// [`Default`]; `hydrate_actual`'s arm calls [`VmHostState::observe`](crate::traits::vm_host_state::VmHostState::observe)
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
pub fn plan_reclamation(
    desired: &VmReclamationState,
    actual: &VmReclamationState,
) -> Vec<Action> {
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
}

// ---------------------------------------------------------------------------
// Ending Class — ADR-0081 D1, three classes, total and disjoint
// ---------------------------------------------------------------------------

/// "Intentional Stop" (ADR-0081 D1) — `state == Terminated` AND
/// `StoppedBy::{Operator, SystemGc}` on EITHER `terminal` or `reason`.
/// Mirrors `workload_lifecycle::is_intentionally_stopped`'s semantics
/// exactly (`brief.md` §Domain Model's binding-sites table: "unchanged in
/// meaning — Platform Reclamation must not match it").
#[must_use]
pub fn is_intentional_stop(row: &AllocStatusRow) -> bool {
    row.state == AllocState::Terminated
        && (matches!(
            row.terminal,
            Some(TerminalCondition::Stopped { by: StoppedBy::Operator | StoppedBy::SystemGc })
        ) || matches!(
            row.reason,
            Some(TransitionReason::Stopped { by: StoppedBy::Operator | StoppedBy::SystemGc })
        ))
}

/// "Platform Reclamation" (ADR-0081 D1) — the platform destroyed one
/// runtime instance while the workload's intent still stands.
///
/// Structurally `false` for every row representable today:
/// `StoppedBy::PlatformReclaimed` (ADR-0081 D5) has not landed — its
/// addition is bundled by ADR-0081's own "Narrows ADR-0078 §D1" section
/// with a same-commit ADR-0078 amendment and an `observation_store.rs`
/// docstring correction, which lands with `execute_reclaim_allocation`
/// (the first real producer of the disposition), not with this
/// reconciler skeleton.
#[must_use]
pub const fn is_platform_reclamation(_row: &AllocStatusRow) -> bool {
    false
}

/// "Workload Failure" (ADR-0081 D1) — the workload's own run concluded
/// (successfully or not) with no authority withdrawal and no platform
/// destruction: `state.is_terminal() && !is_intentional_stop(row) &&
/// !is_platform_reclamation(row)`. Today (with
/// [`is_platform_reclamation`] structurally `false`) this is exactly
/// `workload_lifecycle::is_natural_exit`'s bucket.
#[must_use]
pub fn is_workload_failure(row: &AllocStatusRow) -> bool {
    row.state.is_terminal() && !is_intentional_stop(row) && !is_platform_reclamation(row)
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
