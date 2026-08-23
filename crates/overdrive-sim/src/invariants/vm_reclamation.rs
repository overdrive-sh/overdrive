//! `VmReclamation` ESR invariants (`brief.md` §105a.11).
//!
//! The four progress/stability/safety specifications
//! `.claude/rules/testing.md` § "Tier 1 — Deterministic Simulation
//! Testing" requires for every registered reconciler.
//!
//! Per §105a.11, DST reachability for all four is `plan_reclamation`
//! (in-memory, driven directly — matches S-VM-24/87/88/89's "Driving
//! port" in `docs/feature/microvm-driver-cloud-hypervisor/distill/
//! test-scenarios.md`) composed with the REAL [`SimVmHostState`] port
//! adapter for the `host` half of `VmReclamationState` — the reason
//! `VmHostState::observe()` is a port at all (#197's generalisation
//! target). The `supervision` half has no Sim-side `Driver` counterpart
//! today (`live_allocations` / `release_supervision` are implemented only
//! by the real `VmDriver` in `overdrive-worker`, exercised at Tier 3 —
//! `overdrive-sim` cannot drive a real VMM's claim lifecycle without one),
//! so `SupervisionSet` values are constructed directly by each evaluator —
//! exactly as `overdrive-core`'s own `plan_reclamation` proptest suite
//! (`tests/acceptance/vm_reclamation_plan_purity.rs`) already does.
//!
//! | Invariant | Class | Evaluator | Scenario |
//! |---|---|---|---|
//! | `SupervisedVmSurvivesEveryTick` | safety (`assert_always!`) | [`evaluate_supervised_vm_survives_every_tick`] | S-VM-24 / AC4 |
//! | `VmReclamationIdempotentSteadyState` | stability (`assert_always!`) | [`evaluate_vm_reclamation_idempotent_steady_state`] | S-VM-87 |
//! | `VmReclamationConverges` | liveness (`assert_eventually!`) | [`evaluate_vm_reclamation_converges`] | S-VM-88 |
//! | `EndingInFlightIsNeverReclaimed` | safety (`assert_always!`) | [`evaluate_ending_in_flight_is_never_reclaimed`] | S-VM-89, `@mandatory:mutation_target` |
//!
//! `ReconcilerIsPure` (`invariants/mod.rs`) covers the diff's purity for
//! free — not repeated here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use overdrive_core::cgroup::CgroupPath;
use overdrive_core::id::{AllocationId, WorkloadId};
use overdrive_core::reconcilers::{
    Action, SupervisionSet, VmAllocFacts, VmReclamationState, plan_reclamation,
};
use overdrive_core::traits::vm_host_state::{ScopeFacts, VmHostState};

use crate::adapters::clock::SimClock;
use crate::adapters::vm_host_state::SimVmHostState;
use crate::harness::{InvariantResult, InvariantStatus};

/// Node string every evaluator reports its `InvariantResult` against —
/// these evaluators are single-node, in-memory (no turmoil hosts).
const NODE_NAME: &str = "host-0";

/// Tick budget for the [`evaluate_supervised_vm_survives_every_tick`]
/// "always" sweep and the [`evaluate_vm_reclamation_converges`]
/// "eventually" convergence loop — generous headroom over a handful of
/// generated allocations' worth of decision-table rows, so a genuine
/// regression (not merely a slow convergence) is what trips the budget.
const TICK_BUDGET: u32 = 8;

fn wl(s: &str) -> WorkloadId {
    WorkloadId::new(s).unwrap_or_else(|_| unreachable!("fixture id {s:?} is a valid WorkloadId"))
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s)
        .unwrap_or_else(|_| unreachable!("fixture id {s:?} is a valid AllocationId"))
}

fn pass(name: &str) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Pass,
        tick: 1,
        host: NODE_NAME.to_owned(),
        cause: None,
    }
}

fn fail(name: &str, cause: String) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status: InvariantStatus::Fail,
        tick: 1,
        host: NODE_NAME.to_owned(),
        cause: Some(cause),
    }
}

/// Apply `actions` to `sim` exactly as `execute_reclaim_allocation` /
/// `execute_discard_stranded_artifacts` touch the HOST half (`brief.md`
/// §105a.5): both executors call `kill_scope` then `discard_artifacts` and
/// nothing else mutates `VmHostObservation`'s three surfaces. The row
/// write + broker evaluations `execute_reclaim_allocation` also performs
/// are outside `VmHostObservation`'s universe and are Tier-3-covered
/// (`vm_reclamation_tier3.rs`) — these Tier-1 evaluators assert ONLY on
/// the host-state convergence `plan_reclamation` and these two port calls
/// are jointly responsible for.
async fn apply_actions(sim: &SimVmHostState, actions: &[Action]) {
    for action in actions {
        let (Action::ReclaimAllocation { alloc_id }
        | Action::DiscardStrandedArtifacts { alloc_id }) = action
        else {
            continue;
        };
        let _ = sim.kill_scope(&CgroupPath::for_alloc(alloc_id)).await;
        let _ = sim.discard_artifacts(alloc_id).await;
    }
}

// ---------------------------------------------------------------------------
// S-VM-24 / AC4 — SupervisedVmSurvivesEveryTick (safety, assert_always!)
// ---------------------------------------------------------------------------

/// `brief.md` §105a.10 AC4 / §105a.11: a supervised, non-terminal VM
/// survives every reclamation tick — WITHOUT this, the reconciler passes
/// its whole suite by killing everything.
pub async fn evaluate_supervised_vm_survives_every_tick() -> InvariantResult {
    const NAME: &str = "supervised-vm-survives-every-tick";

    let sim = SimVmHostState::new();
    let alloc = aid("vm-supervised-survivor");
    sim.set_scope(alloc.clone(), BTreeSet::from([4_242]));
    sim.set_run_dir(alloc.clone());
    sim.set_clone(alloc.clone(), PathBuf::from("/staging/vm-supervised-survivor.img"));

    let mut desired = VmReclamationState::default();
    desired.allocations.insert(
        alloc.clone(),
        VmAllocFacts { workload_id: wl("wl-supervised-survivor"), terminal: false },
    );
    let supervision = SupervisionSet::Observed(BTreeSet::from([alloc.clone()]));

    for tick_n in 0..TICK_BUDGET {
        let host = match sim.observe().await {
            Ok(h) => h,
            Err(e) => return fail(NAME, format!("tick {tick_n}: observe failed: {e}")),
        };
        let actual = VmReclamationState {
            allocations: BTreeMap::new(),
            host,
            supervision: supervision.clone(),
        };
        let actions = plan_reclamation(&desired, &actual);
        if !actions.is_empty() {
            return fail(
                NAME,
                format!(
                    "tick {tick_n}: a supervised, non-terminal VM must survive EVERY tick with \
                     zero actions emitted; got {actions:?}"
                ),
            );
        }
        if !sim.has_scope(&alloc) || sim.artifacts_absent(&alloc) {
            return fail(
                NAME,
                format!(
                    "tick {tick_n}: the supervised VM's host state was mutated though \
                     plan_reclamation emitted nothing this tick — a different code path touched it"
                ),
            );
        }
    }
    pass(NAME)
}

// ---------------------------------------------------------------------------
// S-VM-87 — VmReclamationIdempotentSteadyState (stability, assert_always!)
// ---------------------------------------------------------------------------

/// `brief.md` §105a.11: a second reconcile over an unchanged observation is
/// ALWAYS a no-op. Mirrors `HydratorIdempotentSteadyState`
/// (`invariants/mod.rs:360`).
pub async fn evaluate_vm_reclamation_idempotent_steady_state() -> InvariantResult {
    const NAME: &str = "vm-reclamation-idempotent-steady-state";

    let sim = SimVmHostState::new();
    let alloc = aid("vm-idempotent-steady");
    sim.set_scope(alloc.clone(), BTreeSet::new());

    let mut desired = VmReclamationState::default();
    desired.allocations.insert(
        alloc.clone(),
        VmAllocFacts { workload_id: wl("wl-idempotent-steady"), terminal: false },
    );
    let supervision = SupervisionSet::Observed(BTreeSet::from([alloc.clone()]));

    let host_first = match sim.observe().await {
        Ok(h) => h,
        Err(e) => return fail(NAME, format!("first observe: {e}")),
    };
    let actual = VmReclamationState {
        allocations: BTreeMap::new(),
        host: host_first,
        supervision: supervision.clone(),
    };
    let first = plan_reclamation(&desired, &actual);
    if !first.is_empty() {
        return fail(
            NAME,
            format!(
                "fixture bug: the steady-state fixture must already be empty on the first tick; \
                 got {first:?}"
            ),
        );
    }

    // "does not change between two consecutive ticks" — observe AGAIN
    // (nothing mutated the sim between calls) and reconcile a SECOND time
    // over the identical (desired, actual) pair.
    let host_second = match sim.observe().await {
        Ok(h) => h,
        Err(e) => return fail(NAME, format!("second observe: {e}")),
    };
    if host_second != actual.host {
        return fail(
            NAME,
            "fixture bug: host observation changed between two ticks though nothing mutated it"
                .to_owned(),
        );
    }
    let second = plan_reclamation(&desired, &actual);
    if !second.is_empty() {
        return fail(
            NAME,
            format!(
                "a second reconcile over an UNCHANGED observation must return an EMPTY \
                 Vec<Action>; got {second:?}"
            ),
        );
    }
    pass(NAME)
}

// ---------------------------------------------------------------------------
// S-VM-88 — VmReclamationConverges (liveness, assert_eventually!)
// ---------------------------------------------------------------------------

/// `brief.md` §105a.11: convergence under an arbitrary mix of allocations.
///
/// Seeding a mix of live, terminal and unknown VM allocations against
/// `SimVmHostState`, repeated ticks EVENTUALLY leave no host state on any
/// surface attributable to a terminal or unknown allocation — while a
/// genuinely live, supervised VM survives (the selectivity half, shared
/// with S-VM-24's fixture shape).
pub async fn evaluate_vm_reclamation_converges() -> InvariantResult {
    const NAME: &str = "vm-reclamation-converges";

    let sim = SimVmHostState::new();
    // DST-controllable stand-in for the 30s sweep cadence
    // (`overdrive_core::reconcilers::vm_reclamation::VM_RECLAMATION_SWEEP_INTERVAL`) —
    // `plan_reclamation` itself consults no clock (pure over `(desired,
    // actual)` alone), so advancing the SAME ratified constant here
    // documents the simulated pacing a real `spawn_convergence_loop` tick
    // runs at, per `brief.md` §105a.11's "driven with a SimClock" framing.
    let clock = SimClock::new();

    let alloc_terminal = aid("vm-converges-terminal");
    let alloc_unknown = aid("vm-converges-unknown");
    let alloc_live_unsupervised = aid("vm-converges-live-unsupervised");
    let alloc_supervised = aid("vm-converges-supervised");

    sim.set_scope(alloc_terminal.clone(), BTreeSet::new());
    sim.set_run_dir(alloc_terminal.clone());
    sim.set_run_dir(alloc_unknown.clone());
    sim.set_scope(alloc_live_unsupervised.clone(), BTreeSet::new());
    sim.set_scope(alloc_supervised.clone(), BTreeSet::from([777]));
    sim.set_run_dir(alloc_supervised.clone());

    let mut desired = VmReclamationState::default();
    desired.allocations.insert(
        alloc_terminal.clone(),
        VmAllocFacts { workload_id: wl("wl-converges-terminal"), terminal: true },
    );
    desired.allocations.insert(
        alloc_live_unsupervised.clone(),
        VmAllocFacts { workload_id: wl("wl-converges-live"), terminal: false },
    );
    desired.allocations.insert(
        alloc_supervised.clone(),
        VmAllocFacts { workload_id: wl("wl-converges-supervised"), terminal: false },
    );
    // alloc_unknown: deliberately no entry — the unknown-allocation sweep
    // (decision-table row 4).

    let supervision = SupervisionSet::Observed(BTreeSet::from([alloc_supervised.clone()]));

    let mut converged = false;
    for _tick_n in 0..TICK_BUDGET {
        clock.tick(overdrive_core::reconcilers::vm_reclamation::VM_RECLAMATION_SWEEP_INTERVAL);
        let host = match sim.observe().await {
            Ok(h) => h,
            Err(e) => return fail(NAME, format!("observe failed: {e}")),
        };
        let actual = VmReclamationState {
            allocations: BTreeMap::new(),
            host,
            supervision: supervision.clone(),
        };
        let actions = plan_reclamation(&desired, &actual);
        if actions.is_empty() {
            converged = true;
            break;
        }
        apply_actions(&sim, &actions).await;
    }

    if !converged {
        return fail(NAME, format!("did not converge to zero actions within {TICK_BUDGET} ticks"));
    }

    for (label, alloc) in [
        ("terminal", &alloc_terminal),
        ("unknown", &alloc_unknown),
        ("live-unsupervised", &alloc_live_unsupervised),
    ] {
        if sim.has_scope(alloc) || !sim.artifacts_absent(alloc) {
            return fail(
                NAME,
                format!(
                    "{label} allocation {alloc} is still attributable on a host surface after convergence"
                ),
            );
        }
    }
    if !sim.has_scope(&alloc_supervised) {
        return fail(
            NAME,
            "the supervised, non-terminal VM's scope was swept during convergence — selectivity \
             violated"
                .to_owned(),
        );
    }

    pass(NAME)
}

// ---------------------------------------------------------------------------
// S-VM-89 — EndingInFlightIsNeverReclaimed (safety, assert_always!,
// @mandatory:mutation_target)
// ---------------------------------------------------------------------------

/// Whether the modelled observed supervision set retains the
/// `EndingInFlight` claim — `brief.md` §105a.3: "EVERY variant is
/// supervised — `live_allocations()` reports all three [`Starting`,
/// `Live`, `EndingInFlight`]", so the claim's release point sits STRICTLY
/// AFTER the terminal-row write (transitions 5/6), never at the
/// process-death / watcher-return instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseTiming {
    /// Correct (`brief.md` §105a.3): the claim is STILL reported held.
    AfterTerminalRowWrite,
    /// The regression this invariant exists to kill: the claim was
    /// already released at the crash / process-death instant, before the
    /// terminal row was written.
    AtProcessDeath,
}

/// Build the "ending in flight" window (S-VM-89's Given): `alloc`'s VMM
/// has exited (or its stop has been issued) but the terminal row has NOT
/// yet been written (`VmAllocFacts.terminal == false`), and its cgroup
/// scope is still present (nobody has authorised a kill). `timing` selects
/// which release-point model the observed supervision set represents.
fn ending_in_flight_case(
    alloc: AllocationId,
    workload_id: WorkloadId,
    timing: ReleaseTiming,
) -> (VmReclamationState, VmReclamationState) {
    let mut desired = VmReclamationState::default();
    desired.allocations.insert(alloc.clone(), VmAllocFacts { workload_id, terminal: false });

    let mut actual = VmReclamationState::default();
    actual.host.scopes.insert(alloc.clone(), ScopeFacts::default());
    actual.supervision = match timing {
        ReleaseTiming::AfterTerminalRowWrite => SupervisionSet::Observed(BTreeSet::from([alloc])),
        ReleaseTiming::AtProcessDeath => SupervisionSet::Observed(BTreeSet::new()),
    };

    (desired, actual)
}

/// `true` iff `plan_reclamation(desired, actual)` emits `ReclaimAllocation`
/// for `alloc` — the ONE observable shape this invariant refuses.
fn reclaims(
    desired: &VmReclamationState,
    actual: &VmReclamationState,
    alloc: &AllocationId,
) -> bool {
    plan_reclamation(desired, actual)
        .iter()
        .any(|a| matches!(a, Action::ReclaimAllocation { alloc_id } if alloc_id == alloc))
}

/// `brief.md` §105a.11: an in-flight ending is never reclaimed.
///
/// An allocation whose ending is IN FLIGHT — its VMM has exited, or its
/// stop has been issued, and its terminal row is not yet written — is
/// ALWAYS absent from `plan_reclamation`'s `ReclaimAllocation` output.
/// Mandatory mutation target: a release-point regression (§105a.3's
/// abandonment-boundary pins) is EXACTLY what this invariant witnesses —
/// see the `ending_in_flight_teeth` test module below for the proof this
/// evaluator's own check DOES fail under that regression's observable
/// shape (AC-6, step 02-04 dispatch). Synchronous — the case fixtures are
/// constructed in-memory and `reclaims` calls the pure `plan_reclamation`
/// directly; no port I/O to await.
pub fn evaluate_ending_in_flight_is_never_reclaimed() -> InvariantResult {
    const NAME: &str = "ending-in-flight-is-never-reclaimed";

    let alloc = aid("vm-ending-in-flight");
    let (desired, actual) = ending_in_flight_case(
        alloc.clone(),
        wl("wl-ending-in-flight"),
        ReleaseTiming::AfterTerminalRowWrite,
    );

    if reclaims(&desired, &actual, &alloc) {
        return fail(
            NAME,
            format!(
                "ReclaimAllocation wrongly emitted for allocation {alloc}, whose ending is IN \
                 FLIGHT (VMM exited / stop issued, terminal row not yet written)"
            ),
        );
    }
    pass(NAME)
}

#[cfg(test)]
mod ending_in_flight_teeth {
    //! AC-6 (step 02-04 dispatch) — `brief.md` §105a.11's own wording: the
    //! invariant "fails the moment an implementation reverts to releasing
    //! the claim at process death, at `wait()`'s return, or at the
    //! watcher's return." These tests prove that release-point
    //! regression's OBSERVABLE shape (the supervision set no longer
    //! containing an in-flight allocation) is exactly what
    //! [`super::evaluate_ending_in_flight_is_never_reclaimed`]'s check
    //! catches — durable teeth, permanently defending the class, not a
    //! one-off manual proof.

    use super::{ReleaseTiming, aid, ending_in_flight_case, reclaims, wl};

    #[test]
    fn correct_release_point_never_reclaims_the_in_flight_allocation() {
        let alloc = aid("vm-teeth-correct-release");
        let (desired, actual) = ending_in_flight_case(
            alloc.clone(),
            wl("wl-teeth-correct-release"),
            ReleaseTiming::AfterTerminalRowWrite,
        );
        assert!(
            !reclaims(&desired, &actual, &alloc),
            "correct release timing (strictly after the terminal-row write) must NEVER reclaim \
             an in-flight ending"
        );
    }

    /// The teeth: a release-point regression that moves the claim's
    /// release to the process-death instant reproduces the EXACT
    /// violation `EndingInFlightIsNeverReclaimed` exists to catch —
    /// `ReclaimAllocation` fires for a genuinely in-flight ending.
    #[test]
    fn premature_release_at_process_death_wrongly_reclaims_the_in_flight_allocation() {
        let alloc = aid("vm-teeth-premature-release");
        let (desired, actual) = ending_in_flight_case(
            alloc.clone(),
            wl("wl-teeth-premature-release"),
            ReleaseTiming::AtProcessDeath,
        );
        assert!(
            reclaims(&desired, &actual, &alloc),
            "teeth check: a prematurely-released claim MUST reproduce the ReclaimAllocation \
             violation this invariant exists to catch — if this assertion fails, the fixture \
             itself is not exercising the regression shape the AC-6 dispatch demands"
        );
    }
}
