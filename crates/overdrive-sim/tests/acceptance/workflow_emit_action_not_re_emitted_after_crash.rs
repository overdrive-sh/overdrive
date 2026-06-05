//! Slice 03 / US-WP-5 AC3 — an emitted Action is not re-emitted after a
//! crash (idempotent emit); slice-03 AC3.
//!
//! Scenario S-WP-03-04. K1 (O1). A sequence that recorded `ActionEmitted`
//! for a `ctx.emit_action` before terminal is killed after the emit
//! records but before terminal, and restarted on the same node; the
//! Action is NOT re-emitted on resume (the `ActionEmitted` journal entry
//! makes the emit idempotent) — exactly one cluster mutation across the
//! crash. ADR-0063 §2 (`ActionEmitted { action_digest }`), ADR-0064 §4.
//!
//! SINGLE-NODE SCOPE (D3 / #205): process-local kill/restart on one node.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `ctx.emit_action`, the `ActionEmitted` variant, and idempotent-emit
//! replay do not exist yet (DELIVER slice 03). `#[should_panic(expected =
//! "RED scaffold")]` keeps this RED-not-BROKEN and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn an_action_emitted_before_the_crash_is_not_re_emitted_on_resume() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-03-04 / crash after ctx.emit_action records but before terminal: the Action is NOT re-emitted on resume -- exactly one cluster mutation across the crash)"
    );
}
