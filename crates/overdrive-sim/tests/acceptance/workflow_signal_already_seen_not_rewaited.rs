//! Slice 03 / AC1 (resume re-checks satisfaction) — a satisfied signal is
//! not re-waited on resume.
//!
//! Scenario S-WP-03-02. O1. A sequence that recorded `SignalSeen` for
//! `key` before the crash is killed and restarted on the same node; on
//! resume it does NOT re-block on `key` — it reads the recorded signal
//! value and proceeds (check-then-record on replay). ADR-0063 §2
//! (`SignalSeen { value_digest }`), ADR-0064 §3.
//!
//! # RED scaffold (`.claude/rules/testing.md` § "RED scaffolds")
//!
//! `SignalSeen` replay does not exist yet (DELIVER slice 03).
//! `#[should_panic(expected = "RED scaffold")]` keeps this RED-not-BROKEN
//! and compiling.

#[test]
#[should_panic(expected = "RED scaffold")]
fn a_signal_seen_before_the_crash_is_not_rewaited_on_resume() {
    panic!(
        "Not yet implemented -- RED scaffold (S-WP-03-02 / a SignalSeen recorded before the crash is read back on resume -- the workflow does not re-block on an already-satisfied signal)"
    );
}
