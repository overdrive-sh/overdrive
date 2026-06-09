//! Acceptance tests for the `SvidLifecycle::reconcile` NEAR-EXPIRY ROTATION
//! branch per `built-in-ca-operator-composition` Slice ① (folds GH #40).
//!
//! Layer 1: pure reconciler scenarios (Tier-1 DST shape — pure fn over typed
//! `State`, no adapter, deterministic, default lane). The pure `reconcile`
//! method IS the driving port — calling it directly IS port-to-port at the
//! domain layer. Every assertion is on the PORT-OBSERVABLE universe: the
//! returned `Vec<Action>` (partitioned by variant / correlation purpose) and
//! the next `View`. No private threshold constant is ever inspected.
//!
//! Settled design (feature-delta.md D-OC-1/2/3/8; `.claude/rules/workflows.md`):
//! internal SVID near-expiry reissue is a reconciler ACTION, NOT a workflow.
//! `running ∧ held(near-expiry) → Action::IssueSvid("rotate-svid")`
//! UNCONDITIONALLY. The threshold is ½ × `WORKLOAD_SVID_TTL` (1800s today),
//! derived from the TTL const so the fixtures track a TTL-policy change.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use overdrive_core::ca::WORKLOAD_SVID_TTL;
use overdrive_core::id::{AllocationId, ContentHash, CorrelationKey, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::{
    HeldSvidFacts, RunningAlloc, SvidLifecycle, SvidLifecycleState, SvidLifecycleView,
};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;

/// ½ × `WORKLOAD_SVID_TTL` — the near-expiry threshold the rotate branch must
/// track, DERIVED from the TTL const (never a bare `1800`), so a TTL-policy
/// change moves the boundary and the threshold-tracking scenario reds.
const HALF_TTL_SECS: u64 = WORKLOAD_SVID_TTL.as_secs() / 2;

fn workload() -> WorkloadId {
    WorkloadId::new("payments").expect("valid WorkloadId")
}

fn node() -> NodeId {
    NodeId::new("local").expect("valid NodeId")
}

fn running_alloc() -> RunningAlloc {
    RunningAlloc { workload_id: workload(), node_id: node() }
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn held_facts(alloc: &AllocationId, not_after_secs: u64) -> HeldSvidFacts {
    HeldSvidFacts {
        spiffe_id: SpiffeId::for_allocation(&workload(), alloc),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    }
}

/// A `SvidLifecycleState` with an EMPTY `ever_issued` set — the default for the
/// `running ∧ held` rotation scenarios (the held alloc has no bearing on the
/// restart-recovery audit set; that branch is exercised explicitly in S-OC-05).
const fn state_no_audit(
    desired: BTreeMap<AllocationId, RunningAlloc>,
    actual: BTreeMap<AllocationId, HeldSvidFacts>,
) -> SvidLifecycleState {
    SvidLifecycleState { desired, actual, ever_issued: BTreeSet::new() }
}

/// The deterministic `"rotate-svid"`-purpose correlation the rotate branch must
/// derive (ADR-0067 D2 shape, mirrored from the producer): `target =
/// "svid-lifecycle/<alloc>"`, `spec_hash = ContentHash::of(<spiffe-uri bytes>)`,
/// `purpose = "rotate-svid"`.
fn expected_rotate_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "rotate-svid")
}

/// The deterministic `"issue-svid"`-purpose correlation (restart-recovery /
/// first-issue path), for the distinctness assertion in S-OC-05.
fn expected_issue_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "issue-svid")
}

fn start_workflow_count(actions: &[Action]) -> usize {
    actions.iter().filter(|a| matches!(a, Action::StartWorkflow { .. })).count()
}

fn issue_actions(actions: &[Action]) -> Vec<&Action> {
    actions.iter().filter(|a| matches!(a, Action::IssueSvid { .. })).collect()
}

// S-OC-01 `@dst @property @driving_port @slice-1` — for an arbitrary `now` and
// arbitrary slack STRICTLY INSIDE the near-expiry window (`held.not_after <=
// now + WORKLOAD_SVID_TTL/2`), a `running ∧ held` allocation yields EXACTLY ONE
// `Action::IssueSvid` carrying the held `spiffe_id`, the running `node_id`, and
// a `"rotate-svid"` correlation — and ZERO `StartWorkflow` (the cert_rotation
// workflow no longer exists). Universe: the emitted action list + next View.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn near_expiry_held_alloc_emits_one_rotate_issue_svid(
        now_secs in 1_000_000u64..2_000_000_000u64,
        // Slack STRICTLY INSIDE the half-TTL window: the held cert expires
        // between `now` (already near-expiry) and `now + HALF_TTL - 1`.
        slack_secs in 0u64..HALF_TTL_SECS,
    ) {
        let reconciler = SvidLifecycle::canonical();
        let alloc = AllocationId::new("payments-near-expiry").expect("valid AllocationId");

        // desired = the alloc is Running.
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        desired.insert(alloc.clone(), running_alloc());

        // actual = the alloc is HELD, its REAL not_after strictly inside the
        // half-TTL window of `now` (running ∧ held(near-expiry)).
        let not_after = now_secs + slack_secs;
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        let held = held_facts(&alloc, not_after);
        actual.insert(alloc.clone(), held.clone());

        let state = state_no_audit(desired, actual);
        let view = SvidLifecycleView::default();
        let tick = make_tick(now_secs);

        let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // ZERO StartWorkflow — the rotate is a reconciler ACTION, not a workflow.
        prop_assert_eq!(
            start_workflow_count(&actions),
            0,
            "S-OC-01: a near-expiry rotate emits no StartWorkflow; got {:?}",
            actions
        );

        // EXACTLY ONE IssueSvid, carrying the held spiffe_id, the running
        // node_id, and the "rotate-svid" correlation.
        let issues = issue_actions(&actions);
        prop_assert_eq!(
            issues.len(),
            1,
            "S-OC-01: a near-expiry held alloc emits exactly one rotate IssueSvid; got {:?}",
            actions
        );
        match issues[0] {
            Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation } => {
                prop_assert_eq!(alloc_id, &alloc, "rotate IssueSvid names the held alloc");
                prop_assert_eq!(
                    spiffe_id,
                    &held.spiffe_id,
                    "rotate IssueSvid carries the HELD spiffe_id (off actual)"
                );
                prop_assert_eq!(node_id, &node(), "rotate IssueSvid carries the running node_id");
                prop_assert_eq!(
                    correlation,
                    &expected_rotate_correlation(&alloc, &held.spiffe_id),
                    "rotate IssueSvid carries the (svid-lifecycle/<alloc>, spec_hash, rotate-svid) \
                     correlation"
                );
            }
            other => prop_assert!(false, "expected IssueSvid, got {:?}", other),
        }

        // The held-running alloc's retry memory stays clear (clear-on-success);
        // the rotate path records no failed-issue entry.
        prop_assert!(
            !next_view.retry.contains_key(&alloc),
            "S-OC-01: a held-running alloc carries no retry entry; got {:?}",
            next_view.retry
        );
    }
}

// S-OC-02 `@dst @property @error @driving_port @slice-1` — a `running ∧ held`
// allocation whose `not_after` is far-future (STRICTLY beyond `now +
// WORKLOAD_SVID_TTL/2`) emits NO `Action::IssueSvid`; the converged action
// vector for a single held-running alloc is exactly `[Noop]`. Universe: the
// emitted action list. Property over arbitrary `now` + arbitrary far-future
// `not_after`.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn held_alloc_outside_near_expiry_window_emits_no_issue_svid(
        now_secs in 1_000_000u64..2_000_000_000u64,
        // STRICTLY beyond the half-TTL window: `not_after > now + HALF_TTL`.
        beyond_secs in (HALF_TTL_SECS + 1)..1_000_000_000u64,
    ) {
        let reconciler = SvidLifecycle::canonical();
        let alloc = AllocationId::new("payments-fresh").expect("valid AllocationId");

        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        desired.insert(alloc.clone(), running_alloc());

        // actual = held, not_after strictly beyond the half-TTL window.
        let not_after = now_secs + beyond_secs;
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        actual.insert(alloc.clone(), held_facts(&alloc, not_after));

        let state = state_no_audit(desired, actual);
        let view = SvidLifecycleView::default();
        let tick = make_tick(now_secs);

        let (actions, _next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // The converged tick for a single not-near-expiry held-running alloc is
        // exactly `[Noop]` — no IssueSvid, no StartWorkflow, no DropSvid.
        prop_assert_eq!(
            actions.as_slice(),
            [Action::Noop].as_slice(),
            "S-OC-02: a held alloc outside the half-TTL window converges to [Noop]; got {:?}",
            actions
        );
    }
}

// S-OC-03 `@dst @error @driving_port @slice-1` — the near-expiry `<=` boundary
// is INCLUSIVE at half-TTL: `not_after == now + 1800s` rotates (emits one
// IssueSvid); `not_after == now + 1801s` does not. This is the LIVE mutation
// target (D-OC-8 — the `#[mutants::skip]` and the `.cargo/mutants.toml`
// exclude_re entry are removed in Slice ①): this scenario must KILL `<=`→`<`
// and `<=`→`==`. Two pinned boundary examples, NOT PBT. Universe: the emitted
// IssueSvid count across the two fixtures.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_boundary_is_inclusive_at_half_ttl() {
    panic!(
        "Not yet implemented -- RED scaffold (S-OC-03 / near-expiry <= boundary inclusive at \
         now + 1800s, exclusive at now + 1801s -- the live mutation kill-test)"
    );
}

// S-OC-04 `@dst @driving_port @slice-1` — the rotation threshold TRACKS ½ ×
// `WORKLOAD_SVID_TTL` (with WORKLOAD_SVID_TTL = 3600s sourced from validity.rs),
// proven through the PORT-OBSERVABLE emitted action — NOT by inspecting a
// private threshold constant. Two TTL-derived boundary fixtures: a held alloc
// expiring at `now + WORKLOAD_SVID_TTL/2` emits exactly one rotate IssueSvid; a
// held alloc expiring at `now + WORKLOAD_SVID_TTL/2 + 1s` emits none. The
// fixtures are computed FROM the TTL const (not a bare `1800`), so a regression
// that hardcodes the threshold (ignoring a TTL policy change) flips the emit
// decision and reds this test. Distinct from S-OC-03 (literal `<=` boundary
// mutation kill-test): S-OC-04 proves the boundary tracks the TTL. Universe: the
// emitted IssueSvid count across the two TTL-derived fixtures (action list only).
#[test]
fn rotation_threshold_tracks_half_of_workload_svid_ttl_via_emitted_action() {
    let reconciler = SvidLifecycle::canonical();
    let now_secs = 1_700_000_000u64;
    let half_ttl = WORKLOAD_SVID_TTL.as_secs() / 2;

    // Fixture AT the boundary: not_after == now + TTL/2 ⇒ INCLUSIVE ⇒ rotates.
    let at_boundary = {
        let alloc = AllocationId::new("payments-at-boundary").expect("valid AllocationId");
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        desired.insert(alloc.clone(), running_alloc());
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        actual.insert(alloc.clone(), held_facts(&alloc, now_secs + half_ttl));
        let state = state_no_audit(desired, actual);
        let (actions, _) = reconciler.reconcile(
            &state,
            &state,
            &SvidLifecycleView::default(),
            &make_tick(now_secs),
        );
        actions
    };

    // Fixture ONE SECOND BEYOND the boundary: not_after == now + TTL/2 + 1 ⇒
    // EXCLUSIVE ⇒ does not rotate.
    let beyond_boundary = {
        let alloc = AllocationId::new("payments-beyond-boundary").expect("valid AllocationId");
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        desired.insert(alloc.clone(), running_alloc());
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        actual.insert(alloc.clone(), held_facts(&alloc, now_secs + half_ttl + 1));
        let state = state_no_audit(desired, actual);
        let (actions, _) = reconciler.reconcile(
            &state,
            &state,
            &SvidLifecycleView::default(),
            &make_tick(now_secs),
        );
        actions
    };

    assert_eq!(
        issue_actions(&at_boundary).len(),
        1,
        "S-OC-04: not_after == now + WORKLOAD_SVID_TTL/2 rotates (one IssueSvid); got {at_boundary:?}"
    );
    assert_eq!(
        issue_actions(&beyond_boundary).len(),
        0,
        "S-OC-04: not_after == now + WORKLOAD_SVID_TTL/2 + 1s does not rotate; got {beyond_boundary:?}"
    );
}

// S-OC-05 `@dst @property @error @driving_port @slice-1` — rotate is DISTINCT
// from restart-recovery re-issue. A `running ∧ held(near-expiry)` alloc emits a
// `"rotate-svid"`-correlation IssueSvid; a separate `running ∧ ¬held ∧
// ever_issued` alloc emits an `"issue-svid"`-correlation IssueSvid; neither
// emits any `StartWorkflow`, and the two correlations are distinct. Proves the
// rotate branch is NOT routed through the (deleted) gated seam and is
// independent of the restart-recovery branch (ADR-0067 rev 6 D10). Universe:
// the emitted action list partitioned by correlation purpose. Property over
// arbitrary near-expiry slack + arbitrary far-future restart-recovery
// `not_after` (the latter is unheld, so its `not_after` is moot).
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn rotate_is_distinct_from_restart_recovery_reissue(
        now_secs in 1_000_000u64..2_000_000_000u64,
        rotate_slack in 0u64..HALF_TTL_SECS,
    ) {
        let reconciler = SvidLifecycle::canonical();

        // alloc R: running ∧ held ∧ near-expiry → rotate ("rotate-svid").
        let rotate_alloc = AllocationId::new("payments-rotate").expect("valid AllocationId");
        // alloc I: running ∧ ¬held ∧ ever_issued → restart recovery ("issue-svid").
        let recover_alloc = AllocationId::new("payments-recover").expect("valid AllocationId");

        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        desired.insert(rotate_alloc.clone(), running_alloc());
        desired.insert(recover_alloc.clone(), running_alloc());

        // R is held & near-expiry; I is NOT held.
        let rotate_held = held_facts(&rotate_alloc, now_secs + rotate_slack);
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        actual.insert(rotate_alloc.clone(), rotate_held.clone());

        // I's derived identity is in the audit set (ever_issued) — the restart
        // marker that drives the recovery re-issue.
        let recover_spiffe = SpiffeId::for_allocation(&workload(), &recover_alloc);
        let mut ever_issued: BTreeSet<SpiffeId> = BTreeSet::new();
        ever_issued.insert(recover_spiffe.clone());

        let state = SvidLifecycleState { desired, actual, ever_issued };
        let view = SvidLifecycleView::default();
        let tick = make_tick(now_secs);

        let (actions, _next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // Neither branch emits a StartWorkflow.
        prop_assert_eq!(
            start_workflow_count(&actions),
            0,
            "S-OC-05: neither rotate nor restart-recovery emits StartWorkflow; got {:?}",
            actions
        );

        // Partition the emitted IssueSvid actions by alloc.
        let mut rotate_corr: Option<CorrelationKey> = None;
        let mut recover_corr: Option<CorrelationKey> = None;
        for action in &actions {
            if let Action::IssueSvid { alloc_id, correlation, .. } = action {
                if alloc_id == &rotate_alloc {
                    rotate_corr = Some(correlation.clone());
                } else if alloc_id == &recover_alloc {
                    recover_corr = Some(correlation.clone());
                }
            }
        }

        let rotate_corr = rotate_corr.expect("S-OC-05: the near-expiry held alloc emits a rotate IssueSvid");
        let recover_corr =
            recover_corr.expect("S-OC-05: the unheld ever_issued alloc emits a restart-recovery IssueSvid");

        // The rotate alloc carries the "rotate-svid" correlation.
        prop_assert_eq!(
            &rotate_corr,
            &expected_rotate_correlation(&rotate_alloc, &rotate_held.spiffe_id),
            "S-OC-05: the rotate alloc carries the rotate-svid correlation"
        );
        // The restart-recovery alloc carries the "issue-svid" correlation.
        prop_assert_eq!(
            &recover_corr,
            &expected_issue_correlation(&recover_alloc, &recover_spiffe),
            "S-OC-05: the restart-recovery alloc carries the issue-svid correlation"
        );
        // The two correlations are distinct (different purpose AND different alloc).
        prop_assert_ne!(
            &rotate_corr,
            &recover_corr,
            "S-OC-05: rotate and restart-recovery correlations are distinct"
        );
    }
}
