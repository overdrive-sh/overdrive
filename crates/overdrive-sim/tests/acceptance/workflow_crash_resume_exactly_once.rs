//! Slice 01 — WALKING SKELETON. US-WP-3 AC1 (O1) / AC2 (O4) / AC3 (O2
//! single-node) / AC4 (re-hydrate); slice-01 AC1/AC2/AC5.
//!
//! Scenario S-WP-01-06 — the headline durable-execution journey: kill the
//! process AFTER `ctx.call` records but BEFORE terminal, restart on the
//! SAME node, and the external effect is NOT repeated (`SimTransport` call
//! count == 1), the resumed `WorkflowResult` is byte-identical to the
//! uninterrupted run, and the `ObservationStore` carries a terminal-result
//! row keyed by `CorrelationKey`. This is the `WorkflowExactlyOnceEffect
//! OnResume` DST invariant (ADR-0064 §6). K1(O1), K3(O4), K2(O2
//! single-node).
//!
//! SINGLE-NODE SCOPE (D3 / #205): the kill-and-restart is process-local
//! on ONE node. No cross-node resume is claimed; the redb-journal design
//! does not preclude it but it is not demonstrated across nodes here.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! The `WorkflowEngine`, `SimJournalStore`, the `WorkflowExactlyOnce
//! EffectOnResume` invariant, and the `ProvisionRecord` consumer do not
//! exist yet (DELIVER slice 01). `#[should_panic(expected = "RED
//! scaffold")]` keeps this RED-not-BROKEN and compiling without those
//! unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn killing_after_step_records_does_not_repeat_the_effect_on_resume() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-01-06 WALKING SKELETON / kill after ctx.call records, restart same node: SimTransport call count == 1, resumed WorkflowResult byte-identical, terminal row keyed by CorrelationKey -- single-node only)"
    );
}
