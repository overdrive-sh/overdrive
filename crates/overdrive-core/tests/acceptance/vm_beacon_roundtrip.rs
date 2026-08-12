//! Acceptance test for the `vm::beacon` Published Language round-trip —
//! `microvm-driver-cloud-hypervisor` (GH #42), Slice 01, step 01-03.
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/deliver/roadmap.json`
//! step 01-03 — S-VM-01's guest-side prerequisite; no DISTILL scenario
//! names `overdrive_core::vm::beacon` directly, so this file is authored
//! fresh per the roadmap's own `implementation_notes` (DWD-06a: crafter
//! authors RED scaffolds outside the 15 DISTILL-committed scaffolds — see
//! `docs/feature/microvm-driver-cloud-hypervisor/distill/wave-decisions.md`).
//!
//! ADR-0082 §D7 pins the wire shape as a Published Language shared by
//! `overdrive-init` (guest) and the host-side beacon session (`VmDriver`,
//! landing step 01-07) so the two sides cannot drift (Hera's DD-6):
//!
//! ```text
//! guest -> host   "READY pid=<u32> port=<u32>"   exactly once, before exec
//! host  -> guest  "EXEC <json-argv>"             exactly once, after READY, before exec
//! guest -> host   "EXIT <i32>"                   exactly once, after waitpid
//! host  -> guest  "SHUTDOWN"                     at most once (§D4)
//!                 EOF                             terminates the session
//! ```
//!
//! The `pid=`/`port=` READY payload matches the ONLY concrete
//! instantiation of §D7's generic `"READY <k=v>...\n"` notation anywhere
//! in this feature's design record — the spike's measured capture
//! (`spike/findings.md:230`, `"READY pid=1 port=1234\n"`), cited by §D7's
//! own supporting evidence (P2). Roadmap AC 4: "A round-trip proptest on
//! the beacon parser/formatter passes for arbitrary READY/EXIT payloads."
//!
//! `EXEC` (ADR-0082 §D7 amendment 2026-08-12, GH #42) is added by the
//! 01-03 review-remediation: the operator-command->guest channel rides
//! the beacon as a fourth `BeaconMessage` variant carrying a
//! JSON-encoded argv. Unlike READY/EXIT/SHUTDOWN, `EXEC`'s wire shape is
//! a design pin, not a spike measurement — the host->guest write is
//! explicitly unprobed (§D7: "every vsock probe in the spike exercised
//! guest->host only").

#![allow(clippy::unwrap_used)]

use overdrive_core::vm::beacon::{BeaconMessage, BeaconParseError};
use overdrive_core::vm::config::VsockPort;
use proptest::prelude::*;

fn beacon_message_strategy() -> impl Strategy<Value = BeaconMessage> {
    prop_oneof![
        (any::<u32>(), any::<u32>())
            .prop_map(|(pid, port)| BeaconMessage::Ready { pid, port: VsockPort::new(port) }),
        any::<i32>().prop_map(|status| BeaconMessage::Exit { status }),
        Just(BeaconMessage::Shutdown),
        // Non-empty argv only — an all-empty argv is structurally
        // rejected by `FromStr` (`BeaconParseError::EmptyArgv`), so it is
        // not a value this round-trip property holds for. Individual
        // elements are unconstrained `String`s, which already covers
        // spaces, embedded newlines, and empty-string elements.
        proptest::collection::vec(any::<String>(), 1..8)
            .prop_map(|argv| BeaconMessage::Exec { argv }),
    ]
}

proptest! {
    /// Every `BeaconMessage` — arbitrary READY `pid`/`port`, arbitrary
    /// EXIT `status`, and SHUTDOWN — round-trips bit-exact through
    /// `Display -> FromStr`. This is the roadmap's mandatory call site.
    #[test]
    fn beacon_message_roundtrips_through_display_and_from_str(msg in beacon_message_strategy()) {
        let rendered = msg.to_string();
        let parsed: BeaconMessage = rendered.parse().expect("rendered beacon line must re-parse");
        prop_assert_eq!(parsed, msg);
    }

    /// The rendered wire line never carries a line terminator — framing
    /// (`\n` on write, `read_line`/`lines()` stripping on read) is the
    /// transport's job, not the Published Language's.
    #[test]
    fn beacon_message_display_never_embeds_a_line_terminator(msg in beacon_message_strategy()) {
        let rendered = msg.to_string();
        prop_assert!(!rendered.contains('\n'));
        prop_assert!(!rendered.contains('\r'));
    }
}

/// READY's rendered wire form matches the exact measured shape —
/// `"READY pid=<pid> port=<port>"` — pinned so a mutation reordering the
/// two fields or renaming a key is caught (`spike/findings.md:230`).
#[test]
fn ready_message_renders_the_measured_wire_shape() {
    let msg = BeaconMessage::Ready { pid: 1, port: VsockPort::new(1234) };
    assert_eq!(msg.to_string(), "READY pid=1 port=1234");
}

/// EXIT's rendered wire form matches the exact measured shape
/// (`spike/findings.md:555`, `"EXIT 7"`).
#[test]
fn exit_message_renders_the_measured_wire_shape() {
    let msg = BeaconMessage::Exit { status: 7 };
    assert_eq!(msg.to_string(), "EXIT 7");
}

/// SHUTDOWN's rendered wire form matches ADR-0082 §D4's exact token.
#[test]
fn shutdown_message_renders_the_pinned_token() {
    assert_eq!(BeaconMessage::Shutdown.to_string(), "SHUTDOWN");
}

/// `FromStr` tolerates a trailing `\n` a caller read via
/// `read_until(b'\n', ..)` rather than `BufRead::lines()` — the parser
/// never requires the caller to have already stripped it, even though
/// `Display` itself never emits one.
#[test]
fn from_str_tolerates_an_unstripped_trailing_newline() {
    let parsed: BeaconMessage = "READY pid=1 port=1234\n".parse().unwrap();
    assert_eq!(parsed, BeaconMessage::Ready { pid: 1, port: VsockPort::new(1234) });

    let parsed: BeaconMessage = "EXIT 7\n".parse().unwrap();
    assert_eq!(parsed, BeaconMessage::Exit { status: 7 });
}

// ---------------------------------------------------------------------------
// Malformed-line rejection — every parse failure is a structured
// BeaconParseError, never a panic. One test per accept/reject branch
// (`.claude/rules/testing.md` § "Property-based testing" — mandatory
// newtype/FromStr coverage).
// ---------------------------------------------------------------------------

#[test]
fn from_str_rejects_an_empty_line() {
    let err = "".parse::<BeaconMessage>().unwrap_err();
    assert_eq!(err, BeaconParseError::Empty);
}

#[test]
fn from_str_rejects_an_unrecognised_kind() {
    let err = "PING".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::UnknownKind { .. }), "{err:?}");
}

#[test]
fn from_str_rejects_ready_with_wrong_field_count() {
    let err = "READY pid=1".parse::<BeaconMessage>().unwrap_err();
    assert!(
        matches!(err, BeaconParseError::FieldCount { kind: "READY", expected: 2, actual: 1, .. }),
        "{err:?}"
    );

    let err = "READY pid=1 port=2 extra=3".parse::<BeaconMessage>().unwrap_err();
    assert!(
        matches!(err, BeaconParseError::FieldCount { kind: "READY", expected: 2, actual: 3, .. }),
        "{err:?}"
    );
}

#[test]
fn from_str_rejects_ready_with_missing_key_prefix() {
    let err = "READY 1 port=2".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::MissingKey { kind: "READY", field: "pid", .. }), "{err:?}");
}

#[test]
fn from_str_rejects_ready_with_non_integer_pid() {
    let err = "READY pid=abc port=2".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::InvalidInt { kind: "READY", field: "pid", .. }), "{err:?}");
}

#[test]
fn from_str_rejects_exit_with_wrong_field_count() {
    let err = "EXIT".parse::<BeaconMessage>().unwrap_err();
    assert!(
        matches!(err, BeaconParseError::FieldCount { kind: "EXIT", expected: 1, actual: 0, .. }),
        "{err:?}"
    );
}

#[test]
fn from_str_rejects_exit_with_non_integer_status() {
    let err = "EXIT not-a-number".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::InvalidInt { kind: "EXIT", field: "status", .. }), "{err:?}");
}

#[test]
fn from_str_rejects_shutdown_with_a_payload() {
    let err = "SHUTDOWN now".parse::<BeaconMessage>().unwrap_err();
    assert!(
        matches!(err, BeaconParseError::FieldCount { kind: "SHUTDOWN", expected: 0, actual: 1, .. }),
        "{err:?}"
    );
}

#[test]
fn from_str_rejects_exec_with_empty_argv() {
    let err = "EXEC []".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::EmptyArgv { .. }), "{err:?}");
}

#[test]
fn from_str_rejects_exec_with_malformed_json() {
    let err = "EXEC not-json".parse::<BeaconMessage>().unwrap_err();
    assert!(matches!(err, BeaconParseError::MalformedArgv { .. }), "{err:?}");
}
