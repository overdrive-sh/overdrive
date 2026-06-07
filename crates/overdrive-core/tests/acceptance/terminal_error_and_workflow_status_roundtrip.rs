//! Acceptance — `TerminalError` + `WorkflowStatus` serde roundtrip across
//! every variant (`workflow-result-error-model` step 01-01, ADR-0065 §2/§3).
//!
//! # What this pins
//!
//! ADR-0065 introduces the typed workflow terminal model (the trait reshape
//! to `Result<Output, TerminalError>` landed in step 01-03):
//!
//! * `TerminalError { kind: TerminalErrorKind, detail: String }` — the
//!   workflow body's terminal-failure channel. Rides in the journal
//!   `Terminal` command and the terminal observation row as an INPUT, so it
//!   MUST round-trip byte-equal through serde (ADR-0065 §2).
//! * `WorkflowStatus` — the engine-owned control-plane projection
//!   (`Completed { output } | Failed { terminal } | Cancelled | TimedOut`).
//!   Written to the `workflow_terminal` observation row, so it too MUST
//!   round-trip (ADR-0065 §3).
//!
//! # Port-to-port
//!
//! Both types ARE their own driving ports — they are core value types whose
//! public surface (validating constructors + `kind()` accessor + serde
//! impls) IS the interface. The observable outcome is the decoded value
//! `== ` the encoded value. No engine internals are touched; the slice-01
//! types land additively and nothing consumes them yet.
//!
//! # Paradigm
//!
//! Layer 1 (pure, no I/O, default lane). The acceptance scenario is framed
//! as a PROPERTY (per the standing PBT + state-delta mandate): for EVERY
//! `TerminalError` / `WorkflowStatus` value across all variants, the CBOR
//! serde roundtrip is the identity. The single-example assertions below the
//! property pin the canonical readable shapes (a `Scenario:`-style anchor);
//! the unit-level proptests in `src/workflow/mod.rs` carry the exhaustive
//! variant-space coverage (`TerminalErrorRoundtrip`, `WorkflowStatusRoundtrip`).
//!
//! Scenario traces to: ADR-0065 §2 (`TerminalError` serde), §3 (`WorkflowStatus`
//! serde), roadmap step 01-01 AC#1/#4/#7. Tags: `@in-memory` `@property`.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::workflow::{TerminalError, TerminalErrorKind, WorkflowStatus};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators — every PUBLICLY-constructible TerminalError + every
// WorkflowStatus variant. `BudgetExhausted` is engine-only (pub(crate)) so
// it is NOT reachable from this integration test; it is covered at the unit
// level inside the crate where the pub(crate) ctor is in scope.
// ---------------------------------------------------------------------------

/// Author-supplied detail strategy — ASCII printable, bounded under the
/// construction-time length cap so the generated detail survives verbatim
/// (the over-the-cap truncation determinism is a separate unit property).
fn arb_detail() -> impl Strategy<Value = String> {
    "[A-Za-z0-9 ./:_-]{0,64}"
}

/// Every `TerminalError` reachable through a PUBLIC validating constructor.
fn arb_public_terminal_error() -> impl Strategy<Value = TerminalError> {
    prop_oneof![
        arb_detail().prop_map(|d| TerminalError::explicit(&d)),
        arb_detail().prop_map(|d| TerminalError::malformed_input(&d)),
        arb_detail().prop_map(|d| TerminalError::output_encode(&d)),
    ]
}

/// Every `WorkflowStatus` variant. `Completed` carries opaque CBOR output
/// bytes; `Failed` carries a publicly-constructible `TerminalError`;
/// `Cancelled` / `TimedOut` are the engine-authored forward variants.
fn arb_workflow_status() -> impl Strategy<Value = WorkflowStatus> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..=64)
            .prop_map(|output| WorkflowStatus::Completed { output }),
        arb_public_terminal_error().prop_map(|terminal| WorkflowStatus::Failed { terminal }),
        Just(WorkflowStatus::Cancelled),
        Just(WorkflowStatus::TimedOut),
    ]
}

fn cbor_roundtrip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("encode value to CBOR");
    ciborium::from_reader(bytes.as_slice()).expect("decode value from CBOR")
}

proptest! {
    /// `@property` — for every publicly-constructible `TerminalError`, the
    /// CBOR serde roundtrip is the identity (`encode → decode == original`).
    /// The `kind()` accessor observes the same kind before and after, so the
    /// structured cause survives the durable terminal encoding (ADR-0065 §2).
    #[test]
    fn terminal_error_serde_round_trips_for_every_public_variant(
        error in arb_public_terminal_error()
    ) {
        let decoded = cbor_roundtrip(&error);
        prop_assert_eq!(&decoded, &error, "TerminalError round-trips byte-equal through CBOR");
        prop_assert_eq!(
            decoded.kind(),
            error.kind(),
            "kind() observes the same structured cause after the roundtrip"
        );
    }

    /// `@property` — for every `WorkflowStatus` variant (`Completed`,
    /// `Failed`, `Cancelled`, `TimedOut`), the CBOR serde roundtrip is the
    /// identity. `Completed`'s opaque output bytes and `Failed`'s embedded
    /// `TerminalError` both survive byte-equal (ADR-0065 §3).
    #[test]
    fn workflow_status_serde_round_trips_for_every_variant(
        status in arb_workflow_status()
    ) {
        let decoded = cbor_roundtrip(&status);
        prop_assert_eq!(&decoded, &status, "WorkflowStatus round-trips byte-equal through CBOR");
    }
}

/// `@in-memory` — canonical readable anchor: every variant of both types
/// serialize-then-deserialize back to an equal value. This is the
/// single-example proof the scenario name (`terminal_error_and_workflow_
/// status_serialize_and_deserialize_for_every_variant`) describes; the
/// properties above generalise it across the variant space.
#[test]
fn terminal_error_and_workflow_status_serialize_and_deserialize_for_every_variant() {
    // TerminalError — each public kind round-trips and reports its kind.
    let explicit = TerminalError::explicit("operator aborted the rollout");
    assert_eq!(cbor_roundtrip(&explicit), explicit);
    assert_eq!(explicit.kind(), TerminalErrorKind::Explicit);

    let malformed = TerminalError::malformed_input("input bytes were not valid CBOR");
    assert_eq!(cbor_roundtrip(&malformed), malformed);
    assert_eq!(malformed.kind(), TerminalErrorKind::MalformedInput);

    let output_encode = TerminalError::output_encode("output type serde impl failed");
    assert_eq!(cbor_roundtrip(&output_encode), output_encode);
    assert_eq!(output_encode.kind(), TerminalErrorKind::OutputEncode);

    // WorkflowStatus — every variant round-trips byte-equal.
    let completed = WorkflowStatus::Completed { output: vec![0xCA, 0xFE, 0xBA, 0xBE] };
    assert_eq!(cbor_roundtrip(&completed), completed);

    let failed = WorkflowStatus::Failed { terminal: explicit };
    assert_eq!(cbor_roundtrip(&failed), failed);

    let cancelled = WorkflowStatus::Cancelled;
    assert_eq!(cbor_roundtrip(&cancelled), cancelled);

    let timed_out = WorkflowStatus::TimedOut;
    assert_eq!(cbor_roundtrip(&timed_out), timed_out);
}
