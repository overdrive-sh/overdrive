//! Unit tests for [`LogicalTimestamp::dominates`] — step 01-02.
//!
//! `LogicalTimestamp::dominates` is the single comparator
//! every `ObservationStore` adapter consults to apply LWW on `write`.
//! Per `docs/feature/fix-observation-lww-merge/deliver/rca.md` Cause C
//! the comparator was promoted from `overdrive-sim` to `overdrive-core`
//! exactly so both adapters call the same primitive — and so the
//! function-level invariant has a home in this crate's own test suite.
//!
//! These tests are the mutation-killing surface for `dominates`'s three
//! branches: the counter `Greater` arm, the counter `Less` arm, and
//! the tiebreak `Equal` arm. Tuples are hand-picked to flip every
//! mutation `cargo-mutants` will generate at this site:
//!
//! | Mutation | Killed by case |
//! |---|---|
//! | body replaced with `true` | tuples expecting `false` |
//! | body replaced with `false` | tuples expecting `true` |
//! | tiebreak `>` flipped to `<` | identical-counter, `aaa < zzz` |
//! | tiebreak `>` flipped to `==` | identical-counter, identical-writer |
//! | tiebreak `>` flipped to `>=` | identical-counter, identical-writer (idempotency) |
//!
//! The conformance harness at
//! `overdrive_core::testing::observation_store::run_lww_conformance`
//! is the trait-level invariant exercised from each adapter; this
//! file is the function-level invariant exercised from the comparator's
//! own crate. Both layers are needed because mutation testing on the
//! crate that owns the function only runs that crate's tests.

use std::str::FromStr;

use overdrive_core::id::NodeId;
use overdrive_core::traits::observation_store::LogicalTimestamp;

fn ts(counter: u64, writer: &str) -> LogicalTimestamp {
    LogicalTimestamp { counter, writer: NodeId::from_str(writer).expect("valid node id") }
}

// ---------------------------------------------------------------------------
// Counter `Greater` arm — strictly newer counter dominates regardless
// of writer ordering.
// ---------------------------------------------------------------------------

#[test]
fn greater_counter_dominates_lower_counter_same_writer() {
    let a = ts(5, "writer-aaa");
    let b = ts(2, "writer-aaa");
    assert!(a.dominates(&b), "5 must dominate 2 with same writer");
}

#[test]
fn greater_counter_dominates_lower_counter_lex_greater_writer() {
    let a = ts(5, "writer-zzz");
    let b = ts(2, "writer-aaa");
    assert!(a.dominates(&b), "counter dominates first — writer is irrelevant when counters differ");
}

#[test]
fn greater_counter_dominates_lower_counter_lex_lesser_writer() {
    // Counter dominates even when writer would lose tiebreak. This is
    // the tuple that distinguishes `match counter.cmp` from a single
    // unified comparison.
    let a = ts(5, "writer-aaa");
    let b = ts(2, "writer-zzz");
    assert!(
        a.dominates(&b),
        "counter dominates first — writer cannot reverse a counter difference"
    );
}

// ---------------------------------------------------------------------------
// Counter `Less` arm — strictly older counter never dominates.
// ---------------------------------------------------------------------------

#[test]
fn lesser_counter_does_not_dominate_higher_counter_same_writer() {
    let a = ts(2, "writer-aaa");
    let b = ts(5, "writer-aaa");
    assert!(!a.dominates(&b), "2 must NOT dominate 5 with same writer");
}

#[test]
fn lesser_counter_does_not_dominate_higher_counter_lex_greater_writer() {
    // Even when the writer would WIN a tiebreak, a strictly lower
    // counter never dominates.
    let a = ts(2, "writer-zzz");
    let b = ts(5, "writer-aaa");
    assert!(!a.dominates(&b), "lex-greater writer cannot rescue a strictly-lower counter");
}

// ---------------------------------------------------------------------------
// Counter `Equal` / tiebreak arm — `>` strict on writer Display order.
// Equal counter AND equal writer → idempotency case (does NOT dominate).
// ---------------------------------------------------------------------------

#[test]
fn equal_counter_lex_greater_writer_dominates() {
    let a = ts(7, "writer-zzz");
    let b = ts(7, "writer-aaa");
    assert!(a.dominates(&b), "tiebreak: lex-greater writer must dominate at equal counters");
}

#[test]
fn equal_counter_lex_lesser_writer_does_not_dominate() {
    // Symmetric counter-example to the case above. Catches a tiebreak
    // `>` flipped to `<`.
    let a = ts(7, "writer-aaa");
    let b = ts(7, "writer-zzz");
    assert!(!a.dominates(&b), "tiebreak: lex-lesser writer must NOT dominate at equal counters");
}

#[test]
fn identical_timestamp_does_not_dominate_idempotency_case() {
    // The LWW idempotency case: re-delivering the same row is a no-op.
    // Catches a tiebreak `>` flipped to `>=` (which would misclassify
    // re-delivery as dominant), and the body-as-`true` mutation.
    let a = ts(3, "writer-aaa");
    let b = ts(3, "writer-aaa");
    assert!(
        !a.dominates(&b),
        "(idempotency) identical timestamp must NOT dominate — re-delivery is a no-op"
    );
}

// ---------------------------------------------------------------------------
// Cross-axis covering — table-driven sweep that catches a range of
// mutation variants simultaneously. Mirrors the property-loop in the
// conformance harness so the function-level invariant matches the
// trait-level one tuple-for-tuple.
// ---------------------------------------------------------------------------

#[test]
fn comparator_covers_every_branch() {
    // (counter_a, writer_a, counter_b, writer_b, expected_a_dominates_b)
    let cases: &[(u64, &str, u64, &str, bool)] = &[
        // counter_a > counter_b → dominates regardless of writer
        (5, "writer-aaa", 2, "writer-aaa", true),
        (5, "writer-zzz", 2, "writer-aaa", true),
        (5, "writer-aaa", 2, "writer-zzz", true),
        // counter_a < counter_b → never dominates
        (2, "writer-aaa", 5, "writer-aaa", false),
        (2, "writer-zzz", 5, "writer-aaa", false),
        (2, "writer-aaa", 5, "writer-zzz", false),
        // Equal counter, lex-greater writer → dominates
        (3, "writer-zzz", 3, "writer-aaa", true),
        // Equal counter, lex-lesser writer → does NOT dominate
        (3, "writer-aaa", 3, "writer-zzz", false),
        // Equal counter, equal writer → idempotency, does NOT dominate
        (3, "writer-aaa", 3, "writer-aaa", false),
        (0, "writer-aaa", 0, "writer-aaa", false),
        (u64::MAX, "writer-aaa", u64::MAX, "writer-aaa", false),
    ];

    for (idx, (ca, wa, cb, wb, expected)) in cases.iter().enumerate() {
        let a = ts(*ca, wa);
        let b = ts(*cb, wb);
        let observed = a.dominates(&b);
        assert_eq!(
            observed, *expected,
            "case {idx}: ({ca}, {wa}).dominates(({cb}, {wb})) — expected {expected}, got {observed}"
        );
    }
}
