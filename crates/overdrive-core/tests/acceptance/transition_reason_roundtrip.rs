//! Slice 02a (workload-kind-discriminator step 02-03) — typed
//! `TerminalCondition::Completed { exit_code }` / `Failed { exit_code }`
//! variants. ADR-0037 Amendment 2026-05-10.
//!
//! This step is type-level only — runtime emission of these variants
//! lands in 02-04 (`WorkloadLifecycle` reconciler natural-exit emission), and
//! the row-shape change lands in 02-05. Here we pin two structural
//! correctness properties:
//!
//! 1. **rkyv archive/deserialise roundtrip** — the new variants survive
//!    the durable codec for every `i32` exit code, including boundary
//!    cases (`0`, `1`, `-1`, `i32::MIN`, `i32::MAX`) and common Unix
//!    exit codes (`127` = command-not-found, `137` = SIGKILL,
//!    `255` = generic shell failure).
//! 2. **serde JSON roundtrip** — the streaming wire surface preserves
//!    the same payload across `serde_json::to_string` →
//!    `serde_json::from_str`.
//!
//! The proptest is the canonical RED → GREEN gate for this step. It
//! references `TerminalCondition::Completed { exit_code: i32 }` and
//! `TerminalCondition::Failed { exit_code: i32 }` — variants that do
//! not yet exist, so the file fails to compile until GREEN adds them.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::transition_reason::TerminalCondition;
use proptest::prelude::*;
use rkyv::rancor;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Strategy that biases toward exit-code boundaries operators actually
/// see in production while still covering the full `i32` range. The
/// `prop_oneof` weights are deliberate: the boundary-case set is small
/// (8 values), so a flat weight against `any::<i32>()` would only
/// surface them ~once per 1024 cases. Boosting their weight to 1:1
/// gives every Unix-exit-code edge ~512 cases over the budget.
fn arb_exit_code() -> impl Strategy<Value = i32> {
    prop_oneof![
        // Boundary + common-Unix-exit-code corpus.
        Just(0_i32),
        Just(1_i32),
        Just(-1_i32),
        Just(i32::MIN),
        Just(i32::MAX),
        Just(127_i32),
        Just(137_i32),
        Just(255_i32),
        // Full-range exploration for everything else.
        any::<i32>(),
    ]
}

fn arb_completed() -> impl Strategy<Value = TerminalCondition> {
    arb_exit_code().prop_map(|exit_code| TerminalCondition::Completed { exit_code })
}

fn arb_failed() -> impl Strategy<Value = TerminalCondition> {
    arb_exit_code().prop_map(|exit_code| TerminalCondition::Failed { exit_code })
}

fn arb_completed_or_failed() -> impl Strategy<Value = TerminalCondition> {
    prop_oneof![arb_completed(), arb_failed()]
}

// ---------------------------------------------------------------------------
// Property — rkyv + serde JSON roundtrip on the two new variants over
// arbitrary `i32` exit codes.
// ---------------------------------------------------------------------------

proptest! {
    /// AC#3 (slice 02a) — for every `Arbitrary` `Completed { exit_code }`
    /// or `Failed { exit_code }` value `v`, both the rkyv codec and the
    /// serde JSON codec round-trip `v == decode(encode(v))`.
    ///
    /// This pins the two new variants as wire-stable across both
    /// surfaces in the same property. The `AllocStatusRow.terminal`
    /// field-level rkyv property already exists in
    /// `terminal_condition_roundtrip.rs`; when that test is regenerated
    /// to include the new variants in slice 02-05 (row-shape change),
    /// this enum-level test continues to pin the variant identity in
    /// isolation from the row.
    #[test]
    fn transition_reason_roundtrip_completed_failed_proptest(
        original in arb_completed_or_failed(),
    ) {
        // -----------------------------------------------------------
        // rkyv roundtrip — the durable codec.
        // -----------------------------------------------------------
        let bytes = rkyv::to_bytes::<rancor::Error>(&original)
            .expect("rkyv archival of TerminalCondition::Completed/Failed must succeed");

        let archived =
            rkyv::access::<rkyv::Archived<TerminalCondition>, rancor::Error>(&bytes)
                .expect("archived bytes must validate as ArchivedTerminalCondition");

        let rkyv_round: TerminalCondition =
            rkyv::deserialize::<TerminalCondition, rancor::Error>(archived).expect(
                "ArchivedTerminalCondition must deserialize back to TerminalCondition",
            );

        prop_assert_eq!(
            &rkyv_round,
            &original,
            "rkyv roundtrip on TerminalCondition must preserve Completed/Failed exit_code equality"
        );

        // -----------------------------------------------------------
        // serde JSON roundtrip — the streaming wire surface.
        // -----------------------------------------------------------
        let json = serde_json::to_string(&original)
            .expect("serde JSON encode of TerminalCondition::Completed/Failed must succeed");

        let serde_round: TerminalCondition = serde_json::from_str(&json)
            .expect("serde JSON decode must yield the same TerminalCondition variant");

        prop_assert_eq!(
            serde_round,
            original,
            "serde JSON roundtrip on TerminalCondition must preserve \
             Completed/Failed exit_code equality"
        );
    }
}
