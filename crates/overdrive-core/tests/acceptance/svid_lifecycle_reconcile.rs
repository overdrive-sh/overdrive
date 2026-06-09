//! Acceptance tests for `SvidLifecycle::reconcile` per ADR-0067 D1/D2
//! (workload-identity-manager Slice 01, step 01-04).
//!
//! Layer 1: pure reconciler scenarios. The reconciler converges
//! `desired = running allocs` against `actual = the IdentityMgr held set`
//! and emits the diff (`running ∧ ¬held → IssueSvid`,
//! `¬running ∧ held → DropSvid`). `reconcile` is a pure function — no
//! `.await`, no CA / `ObservationStore` handle, wall-clock only via
//! `tick.now` — so these tests construct the typed `State` directly and
//! assert on the emitted action list (the observable universe) without any
//! infrastructure double.
//!
//! Lives in `tests/acceptance/` rather than `src/` because dst-lint scans
//! `src/**/*.rs` and bans `Instant::now()` there even under `#[cfg(test)]`;
//! the `TickContext.now` snapshot the reconcile signature requires forces
//! the `Instant` read into the test fixture here.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver, WorkloadKind};
use overdrive_core::id::{
    AllocationId, ContentHash, CorrelationKey, NodeId, Region, SpiffeId, WorkloadId,
};
use overdrive_core::reconcilers::svid_lifecycle::{
    IssueRetry, RunningAlloc, SvidLifecycle, SvidLifecycleState, SvidLifecycleView,
};
use overdrive_core::reconcilers::{
    Action, HeldSvidFacts, RESTART_BACKOFF_CEILING, Reconciler, TargetResource, TickContext,
    WorkloadLifecycle, WorkloadLifecycleState, WorkloadLifecycleView, backoff_for_attempt,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::TerminalCondition;
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;

const NODE_RAW: &str = "local";

/// Canonical kebab-case name of the `SvidLifecycle` reconciler — sourced
/// from the trait const so a rename is a compile error here, not a silent
/// assertion drift (mirrors the anti-drift const the producer uses).
const SVID_LIFECYCLE_NAME: &str = <SvidLifecycle as Reconciler>::NAME;

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn workload() -> WorkloadId {
    WorkloadId::new("payments").expect("valid WorkloadId")
}

fn node() -> NodeId {
    NodeId::new(NODE_RAW).expect("valid NodeId")
}

fn running_alloc() -> RunningAlloc {
    RunningAlloc { workload_id: workload(), node_id: node() }
}

/// Count the `Action::IssueSvid` entries in an emitted action vector.
fn issue_count(actions: &[Action]) -> usize {
    actions.iter().filter(|a| matches!(a, Action::IssueSvid { .. })).count()
}

fn held_facts(alloc: &AllocationId, not_after_secs: u64) -> HeldSvidFacts {
    HeldSvidFacts {
        spiffe_id: SpiffeId::for_allocation(&workload(), alloc),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    }
}

/// Build a `SvidLifecycleState` with an EMPTY `ever_issued` set — the default
/// for every scenario that does not exercise the rev-5 D10 restart-recovery
/// branch (a never-issued or already-held alloc). The `ever_issued`-driven
/// branch has its own dedicated fixtures below.
const fn state_no_audit(
    desired: BTreeMap<AllocationId, RunningAlloc>,
    actual: BTreeMap<AllocationId, HeldSvidFacts>,
) -> SvidLifecycleState {
    SvidLifecycleState { desired, actual, ever_issued: BTreeSet::new() }
}

/// The deterministic `IssueSvid` correlation the reconciler must derive for a
/// given allocation (ADR-0067 D2): `target = "svid-lifecycle/<alloc>"`,
/// `spec_hash = ContentHash::of(<spiffe-uri bytes>)`, `purpose = "issue-svid"`.
fn expected_issue_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "issue-svid")
}

/// The deterministic `DropSvid` correlation (purpose `"drop-svid"`); the
/// dropped allocation's `spec_hash` is derived from the held identity.
fn expected_drop_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "drop-svid")
}

// `@in-memory` `@property` `@S-WIM-01` — for an arbitrary running-set / held-set
// population, EVERY allocation that is Running but NOT held yields exactly one
// `Action::IssueSvid` carrying the four pinned fields (`alloc_id`, the
// `SpiffeId::for_allocation` identity, `node_id`, the derived correlation), and
// the reconcile body performs no CA / ObservationStore I/O (it is a pure
// function over its arguments — there is no handle to call). The observable
// universe is the emitted action list + the returned `next_view`.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn running_alloc_without_held_svid_emits_issue_svid(
        running_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 1..6),
        held_extra_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 0..4),
    ) {
        let reconciler = SvidLifecycle::canonical();

        // desired = the running set.
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        for suffix in &running_suffixes {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            desired.insert(alloc.clone(), running_alloc());
        }

        // actual = the held set: hold SVIDs for a DISJOINT extra set only, so
        // every running alloc is `running ∧ ¬held`. (Suffixes that collide with
        // the running set are filtered out to keep the property's precondition
        // — "running without held" — true for every running alloc.)
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        for suffix in held_extra_suffixes.iter().filter(|s| !running_suffixes.contains(*s)) {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            actual.insert(alloc.clone(), held_facts(&alloc, 1_700_000_000));
        }

        let state = state_no_audit(desired.clone(), actual.clone());
        let view = <SvidLifecycle as Reconciler>::View::default();
        let tick = make_tick(0);

        let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // Universe slot 1: the emitted IssueSvid set.
        let issues: BTreeMap<AllocationId, Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::IssueSvid { .. }))
            .map(|a| match a {
                Action::IssueSvid { alloc_id, .. } => (alloc_id.clone(), a.clone()),
                _ => unreachable!(),
            })
            .collect();

        // Exactly one IssueSvid per running-without-held alloc — no more, no less.
        prop_assert_eq!(
            issues.len(),
            running_suffixes.len(),
            "one IssueSvid per running ∧ ¬held alloc"
        );

        for alloc in desired.keys() {
            let expected_spiffe = SpiffeId::for_allocation(&workload(), alloc);
            let action = issues.get(alloc).expect("running ∧ ¬held alloc emits an IssueSvid");
            match action {
                Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation } => {
                    prop_assert_eq!(alloc_id, alloc, "IssueSvid names the running alloc");
                    prop_assert_eq!(
                        spiffe_id,
                        &expected_spiffe,
                        "IssueSvid carries SpiffeId::for_allocation"
                    );
                    prop_assert_eq!(node_id, &node(), "IssueSvid carries the issuing node");
                    prop_assert_eq!(
                        correlation,
                        &expected_issue_correlation(alloc, &expected_spiffe),
                        "IssueSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, issue-svid)"
                    );
                }
                _ => unreachable!(),
            }
        }

        // No held alloc that is also running was dropped, and no extra-held
        // alloc was issued (the extra held set is disjoint from running, so it
        // is `¬running ∧ held → DropSvid`, never IssueSvid).
        for alloc in actual.keys() {
            prop_assert!(
                !issues.contains_key(alloc),
                "a held (not-running) alloc is never issued"
            );
        }

        // Universe slot 2: the next_view records ONE failed-issue attempt per
        // re-issued (running ∧ ¬held) alloc — the retry memory (03-01). Each
        // issued alloc has `attempts == 1`; no other alloc has a retry entry.
        let recorded: BTreeMap<AllocationId, u32> =
            next_view.retry.iter().map(|(k, v)| (k.clone(), v.attempts)).collect();
        let expected: BTreeMap<AllocationId, u32> =
            desired.keys().map(|alloc| (alloc.clone(), 1)).collect();
        prop_assert_eq!(
            recorded,
            expected,
            "each running ∧ ¬held alloc records exactly one issue attempt in next_view.retry"
        );
    }
}

/// `@in-memory` `@S-WIM-03` — a stopped (no-longer-Running) allocation that still
/// has a held SVID yields exactly one `Action::DropSvid` for it (so the leaf key
/// becomes unreachable, ADR-0067 O2), and emits NO `IssueSvid` for the stopped
/// alloc. The still-running held alloc converges to a no-op.
#[test]
fn stopped_alloc_with_held_svid_emits_drop_svid() {
    let reconciler = SvidLifecycle::canonical();

    let stopped = AllocationId::new("payments-dead").expect("valid AllocationId");
    let still_running = AllocationId::new("payments-live").expect("valid AllocationId");

    // desired = only the still-running alloc.
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(still_running.clone(), running_alloc());

    // actual = both allocs are held (the stopped one is a leftover hold).
    let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
    actual.insert(stopped.clone(), held_facts(&stopped, 1_700_000_000));
    actual.insert(still_running.clone(), held_facts(&still_running, 1_700_000_000));

    let state = state_no_audit(desired, actual);
    let view = <SvidLifecycle as Reconciler>::View::default();
    let tick = make_tick(0);

    let (actions, _next_view) = reconciler.reconcile(&state, &state, &view, &tick);

    // Exactly one DropSvid, and it targets the stopped alloc.
    let drops: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::DropSvid { .. })).collect();
    assert_eq!(drops.len(), 1, "exactly one DropSvid for the ¬running ∧ held alloc");

    let stopped_spiffe = SpiffeId::for_allocation(&workload(), &stopped);
    match drops[0] {
        Action::DropSvid { alloc_id, correlation } => {
            assert_eq!(alloc_id, &stopped, "DropSvid names the stopped alloc");
            assert_eq!(
                correlation,
                &expected_drop_correlation(&stopped, &stopped_spiffe),
                "DropSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, drop-svid)"
            );
        }
        _ => unreachable!(),
    }

    // No IssueSvid for the stopped alloc, and the still-running held alloc is a
    // no-op (neither issued nor dropped).
    for action in &actions {
        match action {
            Action::IssueSvid { alloc_id, .. } => {
                panic!(
                    "no IssueSvid expected (stopped is dropped, live is already held): {alloc_id}"
                )
            }
            Action::DropSvid { alloc_id, .. } => {
                assert_ne!(
                    alloc_id, &still_running,
                    "the still-running held alloc is never dropped"
                );
            }
            _ => {}
        }
    }
}

/// `@in-memory` `@S-WIM-08` (first-issue / never-succeeded path) -- when the held
/// set is empty AND no audit row exists (`actual = ∅`, `ever_issued = ∅`), EVERY
/// still-Running allocation matches `running ∧ ¬held ∧ ¬ever_issued` and is issued
/// — one `IssueSvid` per running alloc, bounded (ADR-0067 D1). The returned
/// `next_view` records ONE failed-issue attempt per issued alloc (`attempts == 1`,
/// `last_failure_seen_at == tick.now_unix`) so an issue that then FAILS backs off
/// rather than hammering every tick (D8 — the `bump_if_dispatched` shape). The
/// IMMEDIATE restart-recovery path (`¬held ∧ ever_issued`, which bypasses the gate
/// and records NO attempt) is covered by
/// `ever_issued_unheld_alloc_reissues_immediately_bypassing_backoff_and_clears_retry`.
#[test]
fn first_issue_unheld_never_issued_alloc_issues_and_records_one_attempt() {
    let reconciler = SvidLifecycle::canonical();

    let a0 = AllocationId::new("payments-r0").expect("valid AllocationId");
    let a1 = AllocationId::new("payments-r1").expect("valid AllocationId");

    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(a0.clone(), running_alloc());
    desired.insert(a1.clone(), running_alloc());

    // actual = ∅, ever_issued = ∅ — never issued before (genuine first issue).
    let actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();

    let state = state_no_audit(desired.clone(), actual);
    let view = SvidLifecycleView::default();
    let now = 1_700_000_000;
    let tick = make_tick(now);

    let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

    // One IssueSvid per running alloc — bounded recovery.
    let issued: BTreeSet<AllocationId> = actions
        .iter()
        .filter_map(|a| match a {
            Action::IssueSvid { alloc_id, .. } => Some(alloc_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        issued,
        BTreeSet::from([a0.clone(), a1.clone()]),
        "restart recovery re-issues every running ∧ ¬held alloc exactly once"
    );

    // next_view records exactly one attempt per re-issued alloc, stamped now.
    let now_unix = UnixInstant::from_unix_duration(Duration::from_secs(now));
    for alloc in [&a0, &a1] {
        let retry = next_view.retry.get(alloc).expect("re-issued alloc has a retry entry");
        assert_eq!(retry.attempts, 1, "first re-issue records attempts == 1");
        assert_eq!(
            retry.last_failure_seen_at, now_unix,
            "re-issue stamps last_failure_seen_at with tick.now_unix (the input, not a deadline)"
        );
    }
}

/// `@in-memory` `@D10` (restart recovery — the rev-5 fix) -- a running alloc
/// that is `¬held` but `ever_issued` (a durable `issued_certificates` audit row
/// for its derived identity survived a restart that lost the held set) re-issues
/// `Action::IssueSvid` IMMEDIATELY, BYPASSING the backoff gate, EVEN WHEN a stale
/// `IssueRetry` entry (a record-on-emit artifact from the prior successful issue)
/// would otherwise suppress it inside the backoff window. The stale entry is
/// CLEARED in the same tick (D10 invariants 1 + 2). This is the regression anchor
/// for the characterized
/// `restart_after_successful_issue_before_clear_stalls_reissue` defect at the
/// pure-reconciler layer.
#[test]
fn ever_issued_unheld_alloc_reissues_immediately_bypassing_backoff_and_clears_retry() {
    let reconciler = SvidLifecycle::canonical();
    let alloc = AllocationId::new("payments-d10").expect("valid AllocationId");
    let identity = SpiffeId::for_allocation(&workload(), &alloc);

    // desired = the alloc is Running; actual = ¬held but ever_issued (audit row
    // survives the restart).
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(alloc.clone(), running_alloc());
    let state = SvidLifecycleState {
        desired,
        actual: BTreeMap::new(),
        ever_issued: BTreeSet::from([identity.clone()]),
    };

    // A STALE retry entry from the prior SUCCESSFUL issue (record-on-emit), with
    // a deadline strictly in the FUTURE relative to `now` — a backoff gate WOULD
    // suppress the re-issue here. The ever_issued branch must NOT consult it.
    let seen_at_secs = 1_000;
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(
        alloc.clone(),
        IssueRetry {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(
                seen_at_secs,
            )),
        },
    );
    let view = SvidLifecycleView { retry };

    // Tick AT logical `seen_at_secs` — STRICTLY INSIDE the backoff window
    // (`now < last_failure_seen_at + backoff_for_attempt(1)`). A `¬ever_issued`
    // alloc would be suppressed; this `ever_issued` one must re-issue NOW.
    let now = seen_at_secs; // < seen_at_secs + 1s backoff
    let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &make_tick(now));

    // IMMEDIATE re-issue despite the in-window stale entry.
    assert_eq!(
        issue_count(&actions),
        1,
        "¬held ∧ ever_issued re-issues IMMEDIATELY even inside the backoff window; got {actions:?}"
    );
    let issued_id = actions.iter().find_map(|a| match a {
        Action::IssueSvid { spiffe_id, .. } => Some(spiffe_id.clone()),
        _ => None,
    });
    assert_eq!(
        issued_id.as_ref(),
        Some(&identity),
        "the recovery re-issue carries the alloc's pure-derived identity"
    );

    // The stale retry entry is CLEARED — a durable success is proven, no failure
    // is pending (D10 invariant 2; this is what stops a stale entry persisting as
    // a live failure across a restart).
    assert!(
        !next_view.retry.contains_key(&alloc),
        "the stale retry entry is cleared by the ever_issued recovery branch; got {:?}",
        next_view.retry
    );
}

/// `@in-memory` `@D10` (the gate is NOT bypassed for the never-succeeded case) --
/// the SIBLING of the immediate-recovery test: a running alloc that is `¬held`
/// AND `¬ever_issued` (no audit row — a genuine never-succeeded / failing issue)
/// stays BACKOFF-GATED. Inside the backoff window with a recorded `IssueRetry`,
/// it emits NO `IssueSvid` and PRESERVES the retry entry. This proves the
/// `ever_issued` short-circuit does not weaken the genuine failed-issue backoff
/// (D10 invariant 3) — the membership test on `ever_issued` is what discriminates.
#[test]
fn unheld_never_issued_alloc_stays_backoff_gated_inside_window() {
    let reconciler = SvidLifecycle::canonical();
    let alloc = AllocationId::new("payments-d10-fail").expect("valid AllocationId");

    // desired = Running; actual = ¬held AND ever_issued is EMPTY (no audit row).
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(alloc.clone(), running_alloc());
    let state = state_no_audit(desired, BTreeMap::new());

    // An IssueRetry whose backoff window has NOT elapsed at `now`.
    let seen_at_secs = 1_000;
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(
        alloc.clone(),
        IssueRetry {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(
                seen_at_secs,
            )),
        },
    );
    let view = SvidLifecycleView { retry };

    // Tick strictly inside the backoff window.
    let now = seen_at_secs; // < seen_at_secs + backoff_for_attempt(1)
    let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &make_tick(now));

    // SUPPRESSED — the gate is NOT bypassed for a never-issued alloc.
    assert_eq!(
        issue_count(&actions),
        0,
        "¬held ∧ ¬ever_issued stays backoff-gated inside the window; got {actions:?}"
    );
    // The retry entry is PRESERVED (neither cleared nor bumped) so the deadline
    // recomputes identically next tick.
    let preserved = next_view.retry.get(&alloc).expect("the failing alloc keeps its retry entry");
    assert_eq!(
        preserved.attempts, 1,
        "a suppressed never-issued tick neither bumps nor clears the retry entry"
    );
}

/// Run one `reconcile` tick against `(running, held, ever_issued, view)` at
/// `now_secs`, returning the emitted actions + the next view. A free helper so
/// the Tier-1 DST restart scenario can drive a multi-tick trajectory
/// deterministically. `ever_issued` is the durable audit-row identity set
/// (rev 5 D10): the set of `SpiffeId`s for which an `issued_certificates` row
/// exists.
fn tick_once(
    reconciler: &SvidLifecycle,
    running: &BTreeMap<AllocationId, RunningAlloc>,
    held: &BTreeMap<AllocationId, HeldSvidFacts>,
    ever_issued: &BTreeSet<SpiffeId>,
    view: &SvidLifecycleView,
    now_secs: u64,
) -> (Vec<Action>, SvidLifecycleView) {
    let state = SvidLifecycleState {
        desired: running.clone(),
        actual: held.clone(),
        ever_issued: ever_issued.clone(),
    };
    reconciler.reconcile(&state, &state, view, &make_tick(now_secs))
}

/// `@in-memory` `@property` `@S-WIM-08` `@S-WIM-09` `@D10` (Tier-1 DST restart
/// scenario) -- the full restart-mid-run trajectory the reconciler must
/// converge under the rev-5 D10 model, asserted as a SEED-DETERMINISTIC twin
/// run (criterion 3):
///
/// 1. **Steady state** — N Running allocs, all held with a FAR-FUTURE `not_after`
///    (not near-expiry) AND `ever_issued` (an audit row exists per alloc). The
///    tick is a clean no-op (`[Noop]`): no IssueSvid, no DropSvid, and — load-
///    bearing for the gated seam (S-WIM-09) — NO `StartWorkflow`.
/// 2. **Restart (D10 — IMMEDIATE recovery)** — the in-memory held set is emptied
///    (`held = ∅`; the leaf key was never persisted, ADR-0063 D9) but the DURABLE
///    `ever_issued` audit-row set SURVIVES. Retick → every still-Running alloc
///    matches `¬held ∧ ever_issued → IssueSvid IMMEDIATELY` (bounded recovery,
///    one per alloc, BYPASSING the backoff gate). The retry View stays EMPTY —
///    the recovery branch clears any entry; recovery does not record a failure
///    (D10 invariant 1).
/// 3. **No backoff stall on restart** — even immediately after the restart tick
///    (logical time UNCHANGED from the restart tick), the still-`¬held ∧
///    ever_issued` allocs re-issue AGAIN immediately, with NO backoff window
///    elapsing and NO retry entry ever accumulating (D10 invariant 2 — a stale
///    retry entry cannot suppress recovery).
///
/// `reconcile` is a pure function → identical inputs yield identical
/// `(actions, next_view)`. The scenario runs TWICE (a "twin run" — the
/// seed-deterministic reproduction K3 demands) and asserts the two trajectories
/// are bit-identical at every step. The gated seam emits nothing throughout.
#[test]
fn dst_restart_scenario_reissues_immediately_via_ever_issued_and_is_twin_run_deterministic() {
    // The trajectory, as a pure function of nothing (deterministic inputs) →
    // returns the full per-step observable trace so a twin run can be diffed.
    fn trajectory() -> Vec<(Vec<Action>, SvidLifecycleView)> {
        let reconciler = SvidLifecycle::canonical();
        let mut trace: Vec<(Vec<Action>, SvidLifecycleView)> = Vec::new();

        // N Running allocs.
        let allocs: Vec<AllocationId> = (0..4)
            .map(|i| AllocationId::new(&format!("payments-dst-{i}")).expect("valid AllocationId"))
            .collect();
        let mut running: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        for a in &allocs {
            running.insert(a.clone(), running_alloc());
        }

        // ever_issued — an audit row exists for every alloc's derived identity
        // (each was successfully issued before; audit rows are durable and
        // survive the restart, ADR-0063 D6 audit-before-hold).
        let ever_issued: BTreeSet<SpiffeId> =
            allocs.iter().map(|a| SpiffeId::for_allocation(&workload(), a)).collect();

        // Step 1 — steady state: all held, far-future not_after (not near-expiry).
        let far_future = 4_000_000_000;
        let mut held: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        for a in &allocs {
            held.insert(a.clone(), held_facts(a, far_future));
        }
        let view0 = SvidLifecycleView::default();
        let step1 = tick_once(&reconciler, &running, &held, &ever_issued, &view0, 1_000);
        trace.push(step1.clone());

        // Step 2 — RESTART: held set emptied, ever_issued SURVIVES. Retick → every
        // alloc is `¬held ∧ ever_issued → IssueSvid IMMEDIATELY` (bypass backoff).
        let empty_held: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        let restart_now = 2_000;
        let step2 =
            tick_once(&reconciler, &running, &empty_held, &ever_issued, &step1.1, restart_now);
        trace.push(step2.clone());

        // Step 3 — STILL ¬held (the recovery re-issue has not yet been observed as
        // held) at the SAME logical time. A backoff gate WOULD suppress here; the
        // ever_issued branch does NOT — it re-issues again immediately, no retry
        // entry accrues.
        let step3 =
            tick_once(&reconciler, &running, &empty_held, &ever_issued, &step2.1, restart_now);
        trace.push(step3);

        trace
    }

    let run_a = trajectory();
    let run_b = trajectory();

    // Twin-run determinism (K3): the pure reconcile produces a bit-identical
    // trajectory for identical inputs.
    assert_eq!(
        run_a, run_b,
        "the restart trajectory is seed-deterministic (twin runs are identical)"
    );

    // No StartWorkflow anywhere in the trajectory — the gated seam stays silent
    // through steady-state-held and restart recovery (S-WIM-09).
    for (actions, _) in &run_a {
        let start_workflows =
            actions.iter().filter(|a| matches!(a, Action::StartWorkflow { .. })).count();
        assert_eq!(
            start_workflows, 0,
            "the gated #40 seam emits NO StartWorkflow at any step of the restart scenario; \
             got {actions:?}"
        );
    }

    // Step 1 (steady state, all held far-future): clean no-op.
    assert_eq!(
        run_a[0].0.as_slice(),
        [Action::Noop].as_slice(),
        "steady-state held tick is [Noop]"
    );

    // Step 2 (restart, D10 immediate recovery): one IssueSvid per alloc, and the
    // retry View stays EMPTY (recovery does not record a failure — D10 inv 1).
    assert_eq!(
        issue_count(&run_a[1].0),
        4,
        "restart re-issues every still-Running ever-issued alloc once, immediately"
    );
    assert!(
        run_a[1].1.retry.is_empty(),
        "restart recovery via ever_issued records NO retry entry; got {:?}",
        run_a[1].1.retry
    );

    // Step 3 (same logical time, still ¬held): the ever_issued branch re-issues
    // AGAIN immediately — the backoff gate is NEVER reached, so no stale entry
    // can ever stall recovery (D10 inv 2).
    assert_eq!(
        issue_count(&run_a[2].0),
        4,
        "at the SAME logical time, ever_issued recovery re-issues again with no backoff wait"
    );
    assert!(
        run_a[2].1.retry.is_empty(),
        "ever_issued recovery never accumulates a retry entry; got {:?}",
        run_a[2].1.retry
    );
}

/// `@in-memory` `@S-WIM-08` (backoff gate) -- once a `running ∧ ¬held` alloc has a
/// recorded `IssueRetry`, the next tick does NOT re-emit `IssueSvid` until the
/// backoff window has elapsed (`tick.now_unix >= last_failure_seen_at +
/// backoff_for_attempt(attempts)`), and DOES re-emit once it has. The deadline is
/// recomputed each tick from the persisted inputs + the live policy — never
/// persisted (ADR-0067 D8; the `development.md` Reconciler-I/O worked-example
/// shape). Universe: the emitted IssueSvid set across two ticks.
#[test]
fn backoff_gate_suppresses_reissue_until_window_elapses() {
    let reconciler = SvidLifecycle::canonical();
    let alloc = AllocationId::new("payments-b0").expect("valid AllocationId");

    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(alloc.clone(), running_alloc());
    let actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
    // ever_issued is EMPTY — this is the genuinely-failing `¬held ∧ ¬ever_issued`
    // path the backoff gate governs (NOT a restart-recovery alloc).
    let state = state_no_audit(desired, actual);

    // A View with a prior failed attempt at t=1000 (attempts==1).
    let seen_at_secs = 1000;
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(
        alloc.clone(),
        IssueRetry {
            attempts: 1,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(
                seen_at_secs,
            )),
        },
    );
    let view = SvidLifecycleView { retry };

    let deadline_secs = seen_at_secs + backoff_for_attempt(1).as_secs();

    // Tick INSIDE the backoff window — suppressed.
    let inside = reconciler.reconcile(&state, &state, &view, &make_tick(deadline_secs - 1));
    assert_eq!(issue_count(&inside.0), 0, "inside the backoff window: no IssueSvid re-emitted");

    // Tick AT the deadline — re-emitted, and the attempt count bumps.
    let elapsed = reconciler.reconcile(&state, &state, &view, &make_tick(deadline_secs));
    assert_eq!(issue_count(&elapsed.0), 1, "at the backoff deadline: IssueSvid re-emitted");
    let bumped = elapsed.1.retry.get(&alloc).expect("re-issued alloc keeps a retry entry");
    assert_eq!(bumped.attempts, 2, "a re-emitted attempt bumps attempts (1 → 2)");
}

/// `@in-memory` `@S-WIM-08` (clear-on-held + GC) -- when an alloc is
/// `running ∧ held` (the issue succeeded — it is now in `actual`), its
/// `IssueRetry` entry is cleared from `next_view`; and when an alloc is no longer
/// Running at all, its stale `IssueRetry` entry is garbage-collected. Universe:
/// the `next_view.retry` map (mirrors `WorkloadLifecycle`'s clear-on-success +
/// `ServiceMapHydrator`'s GC `retain`).
#[test]
fn clear_on_held_and_gc_on_non_running_prune_retry_memory() {
    let reconciler = SvidLifecycle::canonical();

    let held_running = AllocationId::new("payments-held").expect("valid AllocationId");
    let gone = AllocationId::new("payments-gone").expect("valid AllocationId");

    // desired = only the held-running alloc is still Running.
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(held_running.clone(), running_alloc());

    // actual = the held-running alloc is held (issue succeeded).
    let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
    actual.insert(held_running.clone(), held_facts(&held_running, 1_700_000_000));

    let state = state_no_audit(desired, actual);

    // The View still carries stale retry entries for BOTH allocs (a prior
    // failed attempt for each).
    let stale_entry = || IssueRetry {
        attempts: 2,
        last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(500)),
    };
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(held_running.clone(), stale_entry());
    retry.insert(gone.clone(), stale_entry());
    let view = SvidLifecycleView { retry };

    let (_actions, next_view) = reconciler.reconcile(&state, &state, &view, &make_tick(1_000_000));

    assert!(
        !next_view.retry.contains_key(&held_running),
        "clear-on-held: a now-held (issue-succeeded) alloc has its retry entry cleared"
    );
    assert!(
        !next_view.retry.contains_key(&gone),
        "GC: a no-longer-Running alloc has its stale retry entry garbage-collected"
    );
    assert!(
        next_view.retry.is_empty(),
        "retry memory is pruned to exactly the still-failing running set (here: empty)"
    );
}

/// `@in-memory` `@property` `@S-WIM-08` -- the View is retry memory only:
/// `IssueRetry { attempts, last_failure_seen_at }`, with no serial,
/// `issued_at`, `spiffe_id`, `expires_at`, or `next_renewal_at` success fact.
///
/// Universe: the public `SvidLifecycleView` type shape via a serde round-trip.
/// ADR-0067 D8 — the View's ONLY job is retry-policy memory for a FAILED
/// request; persisting a success fact (a `serial` the pure reconciler cannot
/// know, written BEFORE dispatch) or a derived future-event field
/// (`expires_at` / `next_renewal_at`) is a review-rejection smell. We assert
/// the serde JSON keys are EXACTLY `{retry → {<alloc> → {attempts,
/// last_failure_seen_at}}}` and NONE of the forbidden success/derived keys
/// appear anywhere in the serialized form, and that the round-trip is
/// value-preserving (so the asserted shape IS the persisted shape).
#[test]
fn svid_lifecycle_view_is_retry_memory_only() {
    let alloc = AllocationId::new("payments-a1b2").expect("valid AllocationId");
    let mut retry: BTreeMap<AllocationId, IssueRetry> = BTreeMap::new();
    retry.insert(
        alloc,
        IssueRetry {
            attempts: 3,
            last_failure_seen_at: UnixInstant::from_unix_duration(Duration::from_secs(
                1_700_000_500,
            )),
        },
    );
    let view = SvidLifecycleView { retry };

    let json = serde_json::to_value(&view).expect("View serializes");

    // The top-level object carries EXACTLY one key — `retry`.
    let obj = json.as_object().expect("View serializes to a JSON object");
    let top_keys: BTreeSet<&str> = obj.keys().map(String::as_str).collect();
    assert_eq!(
        top_keys,
        BTreeSet::from(["retry"]),
        "S-WIM-08: the View has EXACTLY one field `retry` (retry memory only); got {top_keys:?}"
    );

    // The per-allocation `IssueRetry` carries EXACTLY the two input fields.
    let entry = obj
        .get("retry")
        .and_then(|r| r.as_object())
        .and_then(|m| m.values().next())
        .and_then(|e| e.as_object())
        .expect("retry holds a per-allocation IssueRetry object");
    let entry_keys: BTreeSet<&str> = entry.keys().map(String::as_str).collect();
    assert_eq!(
        entry_keys,
        BTreeSet::from(["attempts", "last_failure_seen_at"]),
        "S-WIM-08: IssueRetry carries EXACTLY {{attempts, last_failure_seen_at}} \
         (persist inputs, not derived state); got {entry_keys:?}"
    );

    // No success fact (`serial` / `issued_at` / `spiffe_id`) and no derived
    // future-event field (`expires_at` / `next_renewal_at`) anywhere in the
    // serialized form — these are the ADR-0067 D8 review-rejection smells.
    let serialized = serde_json::to_string(&view).expect("View serializes to a string");
    for forbidden in ["serial", "issued_at", "spiffe_id", "expires_at", "next_renewal_at"] {
        assert!(
            !serialized.contains(forbidden),
            "S-WIM-08: the View MUST NOT carry the `{forbidden}` success/derived field; \
             found it in {serialized}"
        );
    }

    // The round-trip is value-preserving, so the shape asserted above IS the
    // shape the runtime persists and reloads.
    let restored: SvidLifecycleView =
        serde_json::from_value(json).expect("View round-trips through serde");
    assert_eq!(restored, view, "S-WIM-08: the View round-trips losslessly");
}

// ---------------------------------------------------------------------------
// S-WIM-10 — `WorkloadLifecycle::reconcile` enqueues `SvidLifecycle` (ADR-0067
// D5b, producer 1). The pure reconciler is its own driving port (calling
// `reconcile` directly IS port-to-port at the domain layer); the observable
// universe is the emitted action list. These fixtures mirror the UI-06 / GAP-9
// shape in `workload_lifecycle_enqueues_bridge_on_alloc_transitions.rs`.
// ---------------------------------------------------------------------------

fn wl_workload(s: &str) -> WorkloadId {
    WorkloadId::new(s).expect("valid WorkloadId")
}
fn wl_node_id(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}
fn wl_alloc(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}
fn wl_region() -> Region {
    Region::new("local").expect("valid Region")
}
fn wl_make_node(id: &str) -> Node {
    Node {
        id: wl_node_id(id),
        region: wl_region(),
        capacity: Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 },
    }
}
fn wl_make_job(id: &str) -> Job {
    Job {
        id: wl_workload(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}
fn wl_one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n = wl_make_node(node_id);
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}
fn wl_alloc_running(
    alloc_id: &str,
    workload_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: wl_alloc(alloc_id),
        workload_id: wl_workload(workload_id),
        node_id: wl_node_id(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: wl_node_id(node_id) },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: match state {
            AllocState::Pending => None,
            _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        },
    }
}
fn wl_tick() -> TickContext {
    let now = Instant::now();
    TickContext {
        now,
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(0)),
        tick: 0,
        deadline: now + Duration::from_secs(1),
    }
}

/// Assert `actions` contains exactly one `Action::EnqueueEvaluation` routed at
/// `svid-lifecycle` for `job/<workload_id>`. The SvidLifecycle enqueue is
/// UNGATED by workload kind — identity is needed by every running alloc.
fn assert_single_svid_enqueue(actions: &[Action], workload_id: &WorkloadId) {
    let mut count = 0;
    let mut found_target: Option<&TargetResource> = None;
    for action in actions {
        if let Action::EnqueueEvaluation { reconciler, target } = action
            && reconciler.as_str() == SVID_LIFECYCLE_NAME
        {
            count += 1;
            found_target = Some(target);
        }
    }
    assert_eq!(
        count, 1,
        "S-WIM-10 (ADR-0067 D5b): an alloc-mutating tick MUST emit exactly one \
         EnqueueEvaluation routed at 'svid-lifecycle'; got {count} in {actions:?}",
    );
    let target = found_target.expect("count==1 checked above");
    assert_eq!(
        target.as_str(),
        &format!("job/{workload_id}"),
        "S-WIM-10: svid-lifecycle enqueue target MUST be 'job/<workload_id>' (workload grain)"
    );
}

/// `@in-memory` `@S-WIM-10` -- `WorkloadLifecycle::reconcile` enqueues
/// `SvidLifecycle` evaluation for `job/<workload_id>` on a RUNNING-transition
/// class (`StartAllocation`). ADR-0067 D5b producer 1. Without this, the pure
/// reconciler can be correct but unreachable.
#[test]
fn start_allocation_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation (Running transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a STOPPED-transition class (`StopAllocation`)
/// also enqueues `SvidLifecycle` for `job/<workload_id>` so the
/// `¬running ∧ held → DropSvid` branch fires (ADR-0067 O2). Producer 1.
#[test]
fn stop_allocation_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: true,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StopAllocation { .. })),
        "expected StopAllocation (Stopped transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a `FinalizeFailed` (backoff-exhausted) terminal
/// transition also enqueues `SvidLifecycle`, completing the Stopped-transition
/// class coverage. Producer 1.
#[test]
fn finalize_failed_transition_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Failed),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let mut view = WorkloadLifecycleView::default();
    view.restart_counts.insert(wl_alloc("alloc-payments-0"), RESTART_BACKOFF_CEILING);
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::FinalizeFailed {
                terminal: Some(TerminalCondition::BackoffExhausted { .. }),
                ..
            }
        )),
        "expected FinalizeFailed (Stopped transition); got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- the SvidLifecycle enqueue is UNGATED by workload
/// kind: a Job-kind StartAllocation enqueues svid-lifecycle just like Service
/// (unlike the Service-gated service-lifecycle enqueue). Identity is needed by
/// every running alloc. ADR-0067 D5b "ungated by kind".
#[test]
fn job_kind_transition_still_enqueues_svid_lifecycle() {
    let workload_id = wl_workload("batch");
    let nodes = wl_one_node_map("local");
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("batch")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("batch")),
        desired_to_stop: false,
        nodes,
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Job,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    assert!(
        actions.iter().any(|a| matches!(a, Action::StartAllocation { .. })),
        "expected StartAllocation; got {actions:?}"
    );
    assert_single_svid_enqueue(&actions, &workload_id);
}

/// `@in-memory` `@S-WIM-10` -- a converged (no-op) tick emits ZERO
/// svid-lifecycle enqueues: the enqueue is paired ONLY with alloc-mutating
/// actions, so the broker is never churned on a quiet tick.
#[test]
fn converged_tick_emits_no_svid_enqueue() {
    let workload_id = wl_workload("payments");
    let nodes = wl_one_node_map("local");
    let mut allocations = BTreeMap::new();
    allocations.insert(
        wl_alloc("alloc-payments-0"),
        wl_alloc_running("alloc-payments-0", "payments", "local", AllocState::Running),
    );
    let desired = WorkloadLifecycleState {
        workload_id: workload_id.clone(),
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let actual = WorkloadLifecycleState {
        workload_id,
        job: Some(wl_make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
        workload_kind: WorkloadKind::Service,
        service_spec_digest: None,
        probe_descriptors: Vec::new(),
    };
    let view = WorkloadLifecycleView::default();
    let tick = wl_tick();

    let r = WorkloadLifecycle::canonical();
    let (actions, _next) = r.reconcile(&desired, &actual, &view, &tick);

    let svid_enqueues = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                Action::EnqueueEvaluation { reconciler, .. }
                    if reconciler.as_str() == SVID_LIFECYCLE_NAME
            )
        })
        .count();
    assert_eq!(
        svid_enqueues, 0,
        "converged tick must emit ZERO svid-lifecycle enqueues; got {svid_enqueues} in {actions:?}"
    );
}
