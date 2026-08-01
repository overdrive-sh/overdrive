//! T1 (ADR-0077 § D7 Layer 3) — the postcondition of
//! [`LogicalTimestamp::dominating`], the single sanctioned way to mint a
//! durable observation row's LWW stamp.
//!
//! WHY-NEW-FILE: crates/overdrive-core/tests/acceptance/logical_timestamp_dominating.rs
//!   CLOSEST-EXISTING: crates/overdrive-core/tests/acceptance/logical_timestamp_dominates.rs
//!   EXTENSION-COST: that file's module doc and filename declare it the
//!     mutation-killing surface for the `dominates` COMPARATOR, and it
//!     explains at length why it uses hand-picked tuples rather than
//!     random draws ("no proptest seed is required"); folding a
//!     `proptest!`-driven constructor suite in would make the filename
//!     false and contradict the stated determinism rationale in the same
//!     file.
//!   PARALLEL-RATIONALE: different function under test (a constructor,
//!     not a comparator), different mutation surface (`max`/`min`,
//!     `saturating_add` arity, the `Some`/`None` arms — none of which
//!     exist in `dominates`), and a different paradigm (property-based
//!     with a generated `(tick_floor, writer, prior)` space vs. a fixed
//!     branch-covering table).
//!
//! # The contract under test (ADR-0077 § D1)
//!
//! The LWW counter is a **per-key version number**, not a scheduling
//! coordinate. `dominating` derives it from the row it replaces and uses
//! the convergence tick only as a floor. The defect this closes: a
//! tick-derived counter regresses to `0` on every process start while the
//! durable rows do not, so every write for a surviving row silently lost
//! the LWW merge for a window equal to the previous process's uptime
//! (`docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`
//! measured 29 s and 52 s).
//!
//! # Mutation surface
//!
//! `dominating` is comparison-and-arithmetic, so it carries the canonical
//! `max`/`min`, `+1`/`+0` mutation surface and a 100 % kill obligation
//! (`.claude/rules/testing.md` § "Mandatory targets"). Each mutant and
//! the case that flips it:
//!
//! | Mutation | Killed by |
//! |---|---|
//! | `floor.max(..)` → `floor.min(..)` | [`post_restart_prior_ahead_of_tick_yields_prior_plus_one`] — `min` returns the regressed floor, which does not dominate |
//! | `tick_floor.saturating_add(1)` → `+ 0` | [`no_prior_yields_tick_floor_plus_one`] and the proptest's `counter >= tick_floor + 1` leg |
//! | `p.counter.saturating_add(1)` → `+ 0` | [`post_restart_prior_ahead_of_tick_yields_prior_plus_one`] — `max(floor, p.counter)` ties the prior, and an equal `(counter, writer)` does NOT dominate |
//! | either arm of the `match prior` | the `None` case and the `Some` cases assert different results |

use std::str::FromStr;

use overdrive_core::id::NodeId;
use overdrive_core::traits::observation_store::LogicalTimestamp;
use proptest::prelude::*;

/// A small writer pool. Deliberately small so the generated `writer` and
/// `prior.writer` COLLIDE often: with equal writers, `dominates` reduces
/// to a strict counter comparison, which is the strong form of the
/// postcondition. A large pool would let the `NodeId` tiebreak mask a
/// counter that merely ties.
const WRITERS: &[&str] = &["writer-aaa", "writer-mmm", "writer-zzz"];

fn node(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid node id")
}

fn ts(counter: u64, writer: &str) -> LogicalTimestamp {
    LogicalTimestamp { counter, writer: node(writer) }
}

prop_compose! {
    fn arb_writer()(idx in 0..WRITERS.len()) -> NodeId {
        node(WRITERS[idx])
    }
}

prop_compose! {
    /// `counter` is bounded BELOW `u64::MAX`: the contract states
    /// explicitly that `p.counter == u64::MAX` saturates and the
    /// postcondition does not hold there (unreachable at the 100 ms
    /// cadence — ~5.8 × 10¹⁰ years). Excluding it here is the stated
    /// precondition, not a convenience.
    fn arb_prior()(counter in 0..u64::MAX, writer in arb_writer()) -> LogicalTimestamp {
        LogicalTimestamp { counter, writer }
    }
}

proptest! {
    /// The postcondition, verbatim from ADR-0077 § D1:
    /// `dominating(t, w, Some(p)).dominates(p) == true` for every `t`,
    /// every `w`, and every `p` with `p.counter < u64::MAX`; and the
    /// returned counter is `>= tick_floor + 1`.
    ///
    /// `tick_floor` is drawn from the FULL `u64` range including
    /// `u64::MAX` — at the top the floor saturates and still dominates
    /// any in-contract prior, so no bound is warranted.
    #[test]
    fn dominating_a_prior_always_dominates_it_and_clears_the_tick_floor(
        tick_floor in any::<u64>(),
        writer in arb_writer(),
        prior in arb_prior(),
    ) {
        let stamp = LogicalTimestamp::dominating(tick_floor, writer, Some(&prior));

        prop_assert!(
            stamp.dominates(&prior),
            "postcondition: dominating({tick_floor}, _, {prior:?}) = {stamp:?} must dominate its prior"
        );
        prop_assert!(
            stamp.counter >= tick_floor.saturating_add(1),
            "postcondition: counter {} must be >= tick_floor + 1 ({})",
            stamp.counter,
            tick_floor.saturating_add(1)
        );
    }

    /// The `None` arm: a genuinely-first write at the key stamps exactly
    /// `tick_floor + 1` — it has no prior to derive from.
    #[test]
    fn dominating_without_a_prior_is_exactly_the_tick_floor(
        tick_floor in any::<u64>(),
        writer in arb_writer(),
    ) {
        let stamp = LogicalTimestamp::dominating(tick_floor, writer, None);
        prop_assert_eq!(
            stamp.counter,
            tick_floor.saturating_add(1),
            "no prior at the key: the counter IS the tick floor"
        );
    }

    /// `writer` is NOT derived from `prior` — each site passes its own,
    /// preserving today's tiebreak behaviour unchanged (§ D1 edge cases).
    #[test]
    fn writer_is_carried_through_verbatim_never_taken_from_the_prior(
        tick_floor in any::<u64>(),
        writer in arb_writer(),
        prior in arb_prior(),
    ) {
        let stamp = LogicalTimestamp::dominating(tick_floor, writer.clone(), Some(&prior));
        prop_assert_eq!(&stamp.writer, &writer, "the passed writer must survive verbatim");
    }
}

// ---------------------------------------------------------------------------
// The defect this ADR closes — pinned deterministically so the mutants that
// the proptest kills only probabilistically are killed on every run.
// ---------------------------------------------------------------------------

/// THE post-restart shape: the durable prior row is far AHEAD of the tick
/// (which reset to `0` on process start). The counter must come from the
/// prior, not the floor.
///
/// This is the case the pre-ADR-0077 `timestamp_for` got wrong: it stamped
/// `tick + 1 = 1` against a prior of `522`, so `dominates` returned `false`
/// and the write was silently dropped for the next 52 s (RCA § 3).
#[test]
fn post_restart_prior_ahead_of_tick_yields_prior_plus_one() {
    let prior = ts(522, "writer-aaa");
    // Same writer as the prior — the production shape (`writer =
    // prior_row.node_id`), and the shape under which `dominates` is a
    // strict counter comparison with no tiebreak escape hatch.
    let stamp = LogicalTimestamp::dominating(0, node("writer-aaa"), Some(&prior));

    assert_eq!(stamp.counter, 523, "the prior, not the regressed tick, sets the counter");
    assert!(stamp.dominates(&prior), "the post-restart write must WIN the LWW merge");
}

/// The steady-state shape: the tick is ahead of the prior, so the floor
/// wins and the emitted counter is byte-identical to what the deleted
/// `timestamp_for` produced. This is the "no observable change outside the
/// defect window" half of ADR-0077 § Consequences.
#[test]
fn steady_state_tick_ahead_of_prior_yields_the_tick_floor() {
    let prior = ts(3, "writer-aaa");
    let stamp = LogicalTimestamp::dominating(99, node("writer-aaa"), Some(&prior));

    assert_eq!(stamp.counter, 100, "tick + 1 when the tick is ahead — unchanged from before");
    assert!(stamp.dominates(&prior), "and it still dominates");
}

/// `tick_floor == 0` with a prior → `prior + 1`, i.e. EXACTLY the exit
/// observer's historical shape (§ D1 edge cases). The exit observer runs
/// outside any convergence loop and passes `0`; this pins that its emitted
/// counters are byte-identical to the hand-rolled `prior.counter + 1` it
/// migrated from.
#[test]
fn tick_floor_zero_reproduces_the_exit_observer_shape() {
    for prior_counter in [0_u64, 1, 2, 41, 1_000_000] {
        let prior = ts(prior_counter, "writer-mmm");
        let stamp = LogicalTimestamp::dominating(0, node("writer-mmm"), Some(&prior));
        assert_eq!(
            stamp.counter,
            prior_counter + 1,
            "tick_floor 0 must reduce to prior + 1 for prior counter {prior_counter}"
        );
    }
}

/// The Observable invariant (§ D1): successive `dominating` calls that each
/// read the current row produce a STRICTLY INCREASING counter chain,
/// **independently of the tick sequence that fed them** — including across
/// a restart, and including across two different tick sequences.
///
/// The tick sequence below deliberately climbs, RESETS TO ZERO (a restart),
/// climbs again, and interleaves a second sequence stuck at `0` (the
/// `spawn_workflow_emit_drain` loop, § D4). The chain must stay strictly
/// increasing throughout.
#[test]
fn counter_chain_is_strictly_increasing_across_a_tick_reset() {
    let writer = node("writer-aaa");
    let tick_sequence =
        [0_u64, 1, 2, 3, 268, 269, /* restart */ 0, 1, 2, /* other loop */ 0, 0, 0];

    let mut current: Option<LogicalTimestamp> = None;
    for tick in tick_sequence {
        let next = LogicalTimestamp::dominating(tick, writer.clone(), current.as_ref());
        if let Some(prev) = &current {
            assert!(
                next.dominates(prev),
                "tick {tick}: {next:?} must dominate the row it replaces ({prev:?})"
            );
            assert!(
                next.counter > prev.counter,
                "tick {tick}: counter must strictly increase ({} -> {})",
                prev.counter,
                next.counter
            );
        }
        current = Some(next);
    }

    // Walk the chain: 1, 2, 3, 4, then the tick jump to 268 pulls the floor
    // up (269), then 270 — and from the restart onward EVERY write adds
    // exactly 1 off the prior, regardless of the tick regressing to 0. Seven
    // writes follow the 269 peak, so the chain lands at 276. A tick-derived
    // chain would instead have collapsed back to `0 + 1 = 1` at the restart
    // and stayed there — the whole defect.
    let final_counter = current.expect("the chain has at least one element").counter;
    assert_eq!(
        final_counter, 276,
        "the chain is driven by the PRIOR: it peaks at 269 on the tick jump and then \
         advances by exactly 1 per write across the restart and the second loop"
    );
}

/// A prior at `u64::MAX` is the stated out-of-contract edge: `saturating_add`
/// clamps and the postcondition does NOT hold. Pinned so the documented
/// limit is falsifiable rather than folklore — if a future change makes this
/// dominate, the contract's "Edge cases" section is stale.
#[test]
fn saturated_prior_counter_does_not_dominate_as_documented() {
    let prior = ts(u64::MAX, "writer-aaa");
    let stamp = LogicalTimestamp::dominating(7, node("writer-aaa"), Some(&prior));

    assert_eq!(stamp.counter, u64::MAX, "saturating_add clamps at the ceiling");
    assert!(
        !stamp.dominates(&prior),
        "documented limit: at u64::MAX the postcondition does not hold (equal counter, \
         equal writer is the LWW idempotency case)"
    );
}
