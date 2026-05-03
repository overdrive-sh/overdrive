//! Acceptance scenarios for issue #141 step 02-02 — `JobLifecycleView`
//! persists *inputs*, deadline recomputed each tick.
//!
//! Two port-to-port scenarios at domain scope (the `JobLifecycle::reconcile`
//! public signature IS the driving port per
//! `~/.claude/skills/nw-tdd-methodology/SKILL.md` § "Port-to-Port at
//! Domain Layer"):
//!
//! 1. `recomputes_deadline_at_window_boundary` — seed the view with
//!    `restart_counts={alloc→2}` and `last_failure_seen_at={alloc→t0}`.
//!    Call reconcile at `tick.now_unix = t0 + (RESTART_BACKOFF_DURATION
//!    - 1ns)` — backoff window NOT elapsed → empty actions. Call
//!    reconcile at `tick.now_unix = t0 + RESTART_BACKOFF_DURATION` —
//!    backoff window JUST elapsed → restart Action emitted. The
//!    inequality `tick.now_unix < seen_at + backoff` is the gate; the
//!    boundary cases pin the `<` semantics (vs `<=` / `==` / `>`).
//!
//! 2. `restart_survival_idempotence` — the load-bearing property of
//!    "persist inputs, not derived state". Run reconcile against a
//!    seeded view, capture (Actions_A, NextView_A). Clone the resulting
//!    `(restart_counts, last_failure_seen_at)` into a brand-new
//!    `JobLifecycleView` (simulating libSQL persistence + rehydration)
//!    and run reconcile again with an *identical* `TickContext`.
//!    Assert the second call produces the same Actions and NextView.
//!    Under Option B (persist a precomputed deadline) this would still
//!    pass for one tick — but the property fails if a future
//!    `backoff_for_attempt` policy change lands while the deadline is
//!    in flight: the persisted deadline locks in the policy-at-write-time
//!    and a freshly-rehydrated view recomputes against the new policy.
//!    Under the persist-inputs shape both reads use the same policy
//!    against the same inputs and produce the same output deterministically.
//!
//! See `.claude/rules/development.md` § "Persist inputs, not derived
//! state" — the architectural spine of issue #141.

#![allow(clippy::expect_used)]
// Module-doc and test-doc references mention symbol-shaped tokens
// (`Actions_A`, `NextView_A`, `Option B`, `Action`) in plain prose
// where backticking every occurrence would cost more readability than
// it buys. The acceptance suite is internal to this crate's test
// binary; the doc-markdown lint is high-signal for public API and
// noise here.
#![allow(clippy::doc_markdown)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::{Exec, Job, Node, WorkloadDriver};
use overdrive_core::id::{AllocationId, JobId, NodeId, Region};
use overdrive_core::reconciler::{
    Action, JobLifecycle, JobLifecycleState, JobLifecycleView, RESTART_BACKOFF_DURATION,
    Reconciler, TickContext,
};
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};

// -------------------------------------------------------------------
// fixtures (mirror job_lifecycle_reconcile_branches.rs shape)
// -------------------------------------------------------------------

fn nid(s: &str) -> NodeId {
    NodeId::new(s).expect("valid NodeId")
}

fn jid(s: &str) -> JobId {
    JobId::new(s).expect("valid JobId")
}

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

fn local_region() -> Region {
    Region::new("local").expect("valid Region")
}

fn make_node(id: &str, capacity: Resources) -> Node {
    Node { id: nid(id), region: local_region(), capacity }
}

fn make_job(id: &str) -> Job {
    Job {
        id: jid(id),
        replicas: NonZeroU32::new(1).expect("1 is non-zero"),
        resources: Resources { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: WorkloadDriver::Exec(Exec { command: "/bin/true".to_string(), args: vec![] }),
    }
}

fn alloc_with_state(
    alloc_id: &str,
    job_id: &str,
    node_id: &str,
    state: AllocState,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: aid(alloc_id),
        job_id: jid(job_id),
        node_id: nid(node_id),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: nid(node_id) },
        reason: None,
        detail: None,
    }
}

fn one_node_map(node_id: &str) -> BTreeMap<NodeId, Node> {
    let n =
        make_node(node_id, Resources { cpu_milli: 4_000, memory_bytes: 8 * 1024 * 1024 * 1024 });
    let mut m = BTreeMap::new();
    m.insert(n.id.clone(), n);
    m
}

fn one_alloc_map(alloc_id: &str, row: AllocStatusRow) -> BTreeMap<AllocationId, AllocStatusRow> {
    let mut m = BTreeMap::new();
    m.insert(aid(alloc_id), row);
    m
}

/// Build a TickContext snapshot at a given wall-clock instant. The
/// monotonic `now` and `deadline` are unrelated to the wall-clock
/// gate under test — they are populated with sensible defaults.
fn tick_at(now_unix: UnixInstant, tick: u64) -> TickContext {
    let now = Instant::now();
    TickContext { now, now_unix, tick, deadline: now + Duration::from_secs(1) }
}

/// Build a `JobLifecycleState` (desired,actual) pair where the alloc
/// for `payments` is in the requested terminal state on `local`.
fn failed_alloc_state(state: AllocState) -> (JobLifecycleState, JobLifecycleState) {
    let nodes = one_node_map("local");
    let allocations = one_alloc_map(
        "alloc-payments-0",
        alloc_with_state("alloc-payments-0", "payments", "local", state),
    );
    let desired = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes: nodes.clone(),
        allocations: BTreeMap::new(),
    };
    let actual = JobLifecycleState {
        job: Some(make_job("payments")),
        desired_to_stop: false,
        nodes,
        allocations,
    };
    (desired, actual)
}

// -------------------------------------------------------------------
// Scenario 1 — Deadline is recomputed from persisted inputs each tick
// -------------------------------------------------------------------

/// GIVEN a view seeded with `restart_counts={alloc→2}` and
/// `last_failure_seen_at={alloc→t0}` — the persisted *inputs* —
/// WHEN reconcile runs at `tick.now_unix = t0 + (RESTART_BACKOFF_DURATION - 1ns)`
/// THEN no actions are emitted (backoff window NOT elapsed),
/// AND when reconcile runs at `tick.now_unix = t0 + RESTART_BACKOFF_DURATION`
/// THEN one `RestartAllocation` action is emitted (window JUST elapsed).
///
/// The boundary cases pin the `<` semantics in the read site
/// (`tick.now_unix < seen_at + backoff_for_attempt(restart_count)`) —
/// exactly equal means the window has elapsed and the restart fires.
#[test]
fn recomputes_deadline_at_window_boundary() {
    // Pick a t0 well above zero so the `t0 + (window - 1ns)` arithmetic
    // does not underflow.
    let t0 = UnixInstant::from_unix_duration(Duration::from_secs(1_000_000));

    // Seed both inputs: restart_counts AND last_failure_seen_at. These
    // are the only inputs the read site consults (plus the policy
    // function `backoff_for_attempt`).
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), 2_u32);
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(aid("alloc-payments-0"), t0);
    let view = JobLifecycleView { restart_counts, last_failure_seen_at };

    let (desired, actual) = failed_alloc_state(AllocState::Terminated);
    let r = JobLifecycle::canonical();

    // --- Sub-case A: window NOT yet elapsed ---
    // tick.now_unix = t0 + (window - 1ns) → strictly less than the
    // recomputed deadline `t0 + window`. Read site:
    // `tick.now_unix < t0 + backoff_for_attempt(2)` → true → suppress.
    let just_before = t0
        + RESTART_BACKOFF_DURATION
            .checked_sub(Duration::from_nanos(1))
            .expect("RESTART_BACKOFF_DURATION > 1 ns");
    let tick_before = tick_at(just_before, 0);
    let (actions_before, _next_before) = r.reconcile(&desired, &actual, &view, &tick_before);
    assert!(
        actions_before.is_empty(),
        "tick.now_unix < seen_at + backoff_for_attempt(restart_count) must \
         suppress RestartAllocation (backoff window not elapsed); got {actions_before:?}",
    );

    // --- Sub-case B: window JUST elapsed ---
    // tick.now_unix = t0 + window → equals the recomputed deadline.
    // Read site: `tick.now_unix < seen_at + backoff` → false → emit.
    let at_boundary = t0 + RESTART_BACKOFF_DURATION;
    let tick_at_boundary = tick_at(at_boundary, 1);
    let (actions_at, _next_at) = r.reconcile(&desired, &actual, &view, &tick_at_boundary);
    assert_eq!(
        actions_at.len(),
        1,
        "tick.now_unix == seen_at + backoff_for_attempt(restart_count) must \
         emit RestartAllocation (backoff window elapsed under `<`); got {actions_at:?}",
    );
    assert!(
        matches!(actions_at[0], Action::RestartAllocation { .. }),
        "first action must be RestartAllocation; got {:?}",
        actions_at[0],
    );
}

// -------------------------------------------------------------------
// Scenario 2 — Restart-survival idempotence
// -------------------------------------------------------------------

/// GIVEN reconcile has been run once against a seeded view and
/// produced (Actions_A, NextView_A) — representing the in-memory state
/// at the end of tick A —
/// WHEN the persisted *inputs* `(restart_counts, last_failure_seen_at)`
/// are cloned into a fresh `JobLifecycleView` (simulating libSQL
/// persistence + rehydration on a subsequent process incarnation)
/// AND reconcile runs against an identical `TickContext` (same
/// `tick.now_unix`, same `tick.tick`, same `tick.now`, same
/// `tick.deadline`)
/// THEN the resulting (Actions_B, NextView_B) is equal to (Actions_A,
/// NextView_A).
///
/// This is the structural property "persist inputs, not derived state"
/// guarantees: the deadline is recomputed from the same inputs against
/// the same policy on every read, so a freshly-rehydrated view at the
/// same tick is indistinguishable from the live view.
///
/// Under the rejected alternative (persisting a precomputed deadline
/// instead of the observation timestamp), this property would still
/// pass *today* — but the view shape would lock in the
/// policy-at-write-time, and a future `backoff_for_attempt` policy
/// change would silently no-op against in-flight rows. The
/// persist-inputs shape makes the property *structural* rather than
/// coincidental.
#[test]
fn restart_survival_idempotence() {
    // Seed inputs as in scenario 1 — restart_counts > 0,
    // last_failure_seen_at populated. Run reconcile at a tick PAST the
    // backoff window (so a Restart is emitted and the write site fires
    // — exercising NextView's last_failure_seen_at update path).
    let t0 = UnixInstant::from_unix_duration(Duration::from_secs(2_000_000));
    let mut restart_counts = BTreeMap::new();
    restart_counts.insert(aid("alloc-payments-0"), 1_u32);
    let mut last_failure_seen_at = BTreeMap::new();
    last_failure_seen_at.insert(aid("alloc-payments-0"), t0);
    let view_a = JobLifecycleView { restart_counts, last_failure_seen_at };

    let (desired, actual) = failed_alloc_state(AllocState::Terminated);
    let r = JobLifecycle::canonical();

    // Pin a single TickContext to use for both calls — same monotonic
    // `now`, same `now_unix`, same `tick`, same `deadline`.
    let now = Instant::now();
    let now_unix = t0 + RESTART_BACKOFF_DURATION + Duration::from_secs(1);
    let tick = TickContext { now, now_unix, tick: 7, deadline: now + Duration::from_secs(1) };

    let (actions_a, next_view_a) = r.reconcile(&desired, &actual, &view_a, &tick);

    // Sanity: the test setup actually exercises the non-trivial path —
    // we want the restart Action emitted, otherwise we are only
    // exercising the "no failed alloc" branch and the property is
    // vacuous.
    assert_eq!(
        actions_a.len(),
        1,
        "test fixture must exercise the RestartAllocation path; got {actions_a:?}",
    );

    // Simulate libSQL persistence + rehydration: move view_a into
    // view_b unchanged. semantically this is "rehydrate from persisted
    // state to a fresh struct equal to the source by content"; in
    // Rust the cheapest expression is a move, since view_a is unused
    // after this point. (view_a is the value that *would* have been
    // serialised to libSQL; view_b is what *would* be reconstructed
    // on the next process incarnation. The test does not need both
    // alive at the same time.)
    let view_b: JobLifecycleView = view_a;

    let (actions_b, next_view_b) = r.reconcile(&desired, &actual, &view_b, &tick);

    // The load-bearing assertion: rehydrating from inputs and running
    // reconcile against the same TickContext produces an identical
    // trajectory. Under the persist-inputs shape this is structural
    // (same inputs + same policy + same tick → same output by purity).
    assert_eq!(
        actions_a, actions_b,
        "reconcile against rehydrated view must produce identical Actions",
    );
    assert_eq!(
        next_view_a, next_view_b,
        "reconcile against rehydrated view must produce identical NextView",
    );
}
