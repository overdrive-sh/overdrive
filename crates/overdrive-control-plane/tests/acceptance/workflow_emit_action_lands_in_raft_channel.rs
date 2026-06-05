//! Slice 03 / US-WP-5 AC2 — `ctx.emit_action` lands the typed Action in
//! the Raft channel with no direct `IntentStore` write; slice-03 AC2.
//!
//! Scenario S-WP-03-03. O3. A sequence that calls `ctx.emit_action
//! (action)`: the typed Action lands in the Action channel the reconciler
//! runtime consumes (→ Raft / Phase-1 `IntentStore` commit path), and the
//! workflow performs NO direct `IntentStore` write (`development.md`
//! Workflow contract rule 6 — the workflow never bypasses Raft). The
//! observable universe is "Action-channel arrivals" + "`IntentStore` writes
//! BY the workflow"; the latter must be empty. ADR-0064 §4/§5.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `ctx.emit_action`, the engine action-channel handoff, and the no-
//! bypass assertion surface do not exist yet (DELIVER slice 03).
//! `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-BROKEN
//! and compiling without those unbuilt types.

#[test]
#[should_panic(expected = "RED scaffold")]
fn emit_action_lands_in_the_action_channel_and_the_workflow_makes_no_direct_intent_store_write() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-03-03 / ctx.emit_action lands the typed Action in the Raft/Action channel; the workflow performs NO direct IntentStore write -- no Raft bypass)"
    );
}
