//! S-AS-02 — `TransitionRecord.reason` and `TransitionReason` are the same Rust type.
//!
//! This is the compile-time type-identity contract from
//! `docs/feature/cli-submit-vs-deploy-and-alloc-status/distill/test-scenarios.md`
//! S-AS-02. The streaming `SubmitEvent::LifecycleTransition.reason` and
//! the snapshot `AllocStatusRowBody.last_transition.reason` MUST carry
//! the same `TransitionReason` enum so byte-equality across surfaces is
//! a structural guarantee, not a discipline rule.
//!
//! For step 01-02 the assertion is scoped to `TransitionRecord.reason ==
//! TransitionReason`. The full cross-surface assertion (`TransitionRecord
//! .reason == SubmitEvent::LifecycleTransition.reason`) lands in step
//! 02-02 when `SubmitEvent` is declared per DWD-03's progressive
//! strengthening.
//!
//! ## Why a witness function rather than `static_assertions`
//!
//! `static_assertions::assert_type_eq_all!` asserts two named types are
//! equal — it does not directly assert "field X of struct Y has type
//! Z." A witness function pattern (a function whose body type-checks
//! only when the field is the exact named type) is the standard idiom
//! for the latter. The `static_assertions` crate is not in the
//! workspace dep graph; the witness function is portable and equally
//! load-bearing.

use overdrive_control_plane::api::TransitionRecord;
use overdrive_core::TransitionReason;

/// Compile-time witness: the `record.reason` field destructures into a
/// `TransitionReason`-typed local. If `TransitionRecord.reason` ever
/// drifts from `TransitionReason` — to a `String`, to a wrapper, to a
/// renamed sibling type — this function fails to compile with a
/// "expected `T`, found `U`" diagnostic, which is the load-bearing
/// signal. The runtime body of the test below exists so nextest counts
/// the assertion in the test inventory.
const fn _transition_record_reason_is_transition_reason_witness(
    record: &TransitionRecord,
    reason: &TransitionReason,
) {
    let _: &TransitionReason = &record.reason;
    let _: &TransitionReason = reason;
}

#[test]
fn transition_record_reason_is_transition_reason() {
    // The type-equality assertion above is enforced by the compiler.
    // This runtime body exists so nextest reports the test in its
    // inventory and the test catalogue audit picks it up. A failing
    // compile of the witness function above is the actual RED signal.
}
