//! Slice 02 step 02-02 — `SubmitEvent` wire enum serialisation.
//!
//! Per the criteria for step 02-02 (cli-submit-vs-deploy-and-alloc-status),
//! the `SubmitEvent` enum is the wire shape for the streaming
//! `Accept: application/x-ndjson` lane on `POST /v1/jobs`. It carries:
//!
//!   * `Accepted { spec_digest, intent_key, outcome }`
//!   * `LifecycleTransition { alloc_id, from, to, reason, detail, source, at }`
//!   * `ConvergedRunning { alloc_id, started_at }`
//!   * `ConvergedFailed { alloc_id, terminal_reason, reason, error }`
//!
//! Wire shape: `#[serde(tag = "kind", content = "data", rename_all =
//! "snake_case")]` — same as `TransitionReason` and `TerminalReason`.
//!
//! This file pins three properties:
//!
//!   1. **Round-trip** — every variant of `SubmitEvent` (with proptest-
//!      generated payloads sourced from the cause-class
//!      `TransitionReason` taxonomy) round-trips through
//!      `serde_json::to_string` / `from_str`. 1024 cases per
//!      `.claude/rules/testing.md` § Property-based testing.
//!   2. **Wire-shape regression** — hand-picked cause-class payloads
//!      serialise to the literal nested structures called out in step
//!      02-02 task description (the load-bearing wire shape per
//!      ADR-0032 §3 Amendment 2026-04-30).
//!   3. **NDJSON line emit invariant** — `serde_json::to_writer` + `\n`
//!      produces exactly one line with no trailing bytes.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::expect_fun_call)]

use overdrive_control_plane::api::{
    AllocStateWire, IdempotencyOutcome, SubmitEvent, TerminalReason, TransitionSource,
};
use overdrive_core::TransitionReason;
use overdrive_core::traits::driver::DriverType;
use overdrive_core::transition_reason::{CancelledBy, ResourceEnvelope, StoppedBy};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Bounded ASCII label used for free-form `String` payload generation.
/// Matches the convention from
/// `tests/acceptance/alloc_status_row_archive_roundtrip.rs`.
fn arb_label() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,15}".prop_map(String::from)
}

/// Generator for every `TransitionReason` variant. Mirrors
/// `tests/acceptance/alloc_status_row_archive_roundtrip.rs::arb_transition_reason`.
#[allow(clippy::too_many_lines)]
fn arb_transition_reason() -> impl Strategy<Value = TransitionReason> {
    prop_oneof![
        // ---- progress markers (5) ----
        Just(TransitionReason::Scheduling),
        Just(TransitionReason::Starting),
        Just(TransitionReason::Started),
        any::<u32>().prop_map(|attempt| TransitionReason::BackoffPending { attempt }),
        prop_oneof![
            Just(StoppedBy::Operator),
            Just(StoppedBy::Reconciler),
            Just(StoppedBy::Process)
        ]
        .prop_map(|by| TransitionReason::Stopped { by }),
        // ---- cause-class failures, Phase-1 emit (9) ----
        arb_label().prop_map(|path| TransitionReason::ExecBinaryNotFound { path }),
        arb_label().prop_map(|path| TransitionReason::ExecPermissionDenied { path }),
        (arb_label(), arb_label())
            .prop_map(|(path, kind)| TransitionReason::ExecBinaryInvalid { path, kind }),
        (arb_label(), arb_label())
            .prop_map(|(kind, source)| TransitionReason::CgroupSetupFailed { kind, source }),
        arb_label().prop_map(|detail| TransitionReason::DriverInternalError { detail }),
        (any::<u32>(), arb_label()).prop_map(|(attempts, last_cause_summary)| {
            TransitionReason::RestartBudgetExhausted { attempts, last_cause_summary }
        }),
        prop_oneof![Just(CancelledBy::Operator), Just(CancelledBy::Cluster)]
            .prop_map(|by| TransitionReason::Cancelled { by }),
        (any::<u32>(), any::<u64>(), any::<u32>(), any::<u64>()).prop_map(
            |(req_cpu, req_mem, free_cpu, free_mem)| TransitionReason::NoCapacity {
                requested: ResourceEnvelope { cpu_milli: req_cpu, memory_bytes: req_mem },
                free: ResourceEnvelope { cpu_milli: free_cpu, memory_bytes: free_mem },
            }
        ),
        // ---- cause-class failures, Phase-2 forward-compat (2) ----
        (any::<u64>(), any::<u64>()).prop_map(|(peak_bytes, limit_bytes)| {
            TransitionReason::OutOfMemory { peak_bytes, limit_bytes }
        }),
        (
            proptest::option::of(any::<i32>()),
            proptest::option::of(any::<u8>()),
            proptest::option::of(arb_label()),
        )
            .prop_map(|(exit_code, signal, stderr_tail)| {
                TransitionReason::WorkloadCrashedImmediately { exit_code, signal, stderr_tail }
            }),
    ]
}

fn arb_alloc_state_wire() -> impl Strategy<Value = AllocStateWire> {
    prop_oneof![
        Just(AllocStateWire::Pending),
        Just(AllocStateWire::Running),
        Just(AllocStateWire::Draining),
        Just(AllocStateWire::Suspended),
        Just(AllocStateWire::Terminated),
        Just(AllocStateWire::Failed),
    ]
}

fn arb_idempotency_outcome() -> impl Strategy<Value = IdempotencyOutcome> {
    prop_oneof![Just(IdempotencyOutcome::Inserted), Just(IdempotencyOutcome::Unchanged)]
}

fn arb_driver_type() -> impl Strategy<Value = DriverType> {
    // Phase 1: only `Exec` is in scope. Adding more variants is purely
    // additive — the proptest will exercise them automatically once the
    // generator is extended.
    Just(DriverType::Exec)
}

fn arb_transition_source() -> impl Strategy<Value = TransitionSource> {
    prop_oneof![
        Just(TransitionSource::Reconciler),
        arb_driver_type().prop_map(TransitionSource::Driver),
    ]
}

fn arb_terminal_reason() -> impl Strategy<Value = TerminalReason> {
    prop_oneof![
        arb_transition_reason()
            .prop_map(|cause| TerminalReason::DriverError { cause })
            .boxed(),
        (any::<u32>(), arb_transition_reason())
            .prop_map(|(attempts, cause)| { TerminalReason::BackoffExhausted { attempts, cause } })
            .boxed(),
        any::<u32>()
            .prop_map(|after_seconds| TerminalReason::Timeout { after_seconds })
            .boxed(),
        // Step 01-01 (RED-scaffold companion): exercise the new
        // `TerminalReason::StreamInterrupted` variant through the
        // existing serde round-trip property. Payload-free unit
        // variant per `crates/overdrive-control-plane/src/api.rs`.
        Just(TerminalReason::StreamInterrupted).boxed(),
    ]
}

/// Generator for every `SubmitEvent` variant. Each variant gets its
/// payload subspace generated independently.
fn arb_submit_event() -> impl Strategy<Value = SubmitEvent> {
    prop_oneof![
        // ---- Accepted ----
        (arb_label(), arb_label(), arb_idempotency_outcome()).prop_map(
            |(spec_digest, intent_key, outcome)| SubmitEvent::Accepted {
                spec_digest,
                intent_key,
                outcome,
            }
        ),
        // ---- LifecycleTransition ----
        (
            arb_label(),
            arb_alloc_state_wire(),
            arb_alloc_state_wire(),
            arb_transition_reason(),
            proptest::option::of(arb_label()),
            arb_transition_source(),
            arb_label(),
        )
            .prop_map(|(alloc_id, from, to, reason, detail, source, at)| {
                SubmitEvent::LifecycleTransition { alloc_id, from, to, reason, detail, source, at }
            }),
        // ---- ConvergedRunning ----
        (arb_label(), arb_label()).prop_map(|(alloc_id, started_at)| {
            SubmitEvent::ConvergedRunning { alloc_id, started_at }
        }),
        // ---- ConvergedFailed ----
        (
            proptest::option::of(arb_label()),
            arb_terminal_reason(),
            proptest::option::of(arb_transition_reason()),
            proptest::option::of(arb_label()),
        )
            .prop_map(|(alloc_id, terminal_reason, reason, error)| {
                SubmitEvent::ConvergedFailed { alloc_id, terminal_reason, reason, error }
            }),
    ]
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    /// Round-trip property: every `SubmitEvent` value round-trips
    /// through `serde_json::to_string` / `from_str` to an equal value.
    /// 1024 cases (proptest default).
    #[test]
    fn submit_event_serde_round_trips(event in arb_submit_event()) {
        let serialised = serde_json::to_string(&event)
            .expect("SubmitEvent must serialise");
        let back: SubmitEvent = serde_json::from_str(&serialised)
            .expect("SubmitEvent must deserialise from its own output");
        prop_assert_eq!(back, event);
    }

    /// Round-trip property: every `TerminalReason` value (with
    /// proptest-generated cause-class payloads) round-trips through
    /// serde JSON to an equal value.
    #[test]
    fn terminal_reason_serde_round_trips(reason in arb_terminal_reason()) {
        let serialised = serde_json::to_string(&reason)
            .expect("TerminalReason must serialise");
        let back: TerminalReason = serde_json::from_str(&serialised)
            .expect("TerminalReason must deserialise from its own output");
        prop_assert_eq!(back, reason);
    }
}

// ---------------------------------------------------------------------------
// Wire-shape regression assertions — load-bearing literal JSON snippets
// per ADR-0032 §3 Amendment 2026-04-30 (cause-class refactor).
// ---------------------------------------------------------------------------

#[test]
fn lifecycle_transition_serialises_with_typed_cause_class_reason() {
    let event = SubmitEvent::LifecycleTransition {
        alloc_id: "alloc-a1b2".to_string(),
        from: AllocStateWire::Pending,
        to: AllocStateWire::Failed,
        reason: TransitionReason::ExecBinaryNotFound {
            path: "/usr/local/bin/payments".to_string(),
        },
        detail: Some("stat /usr/local/bin/payments: ...".to_string()),
        source: TransitionSource::Driver(DriverType::Exec),
        at: "1@node-a".to_string(),
    };

    let serialised = serde_json::to_string(&event).expect("serialise");

    // Outer envelope carries `kind: lifecycle_transition` per
    // `#[serde(tag = "kind", rename_all = "snake_case")]`.
    assert!(
        serialised.contains(r#""kind":"lifecycle_transition""#),
        "outer envelope kind discriminator missing: {serialised}"
    );

    // Nested cause-class structure on `reason`. The path payload is
    // structured data, not a stringified `detail`.
    assert!(
        serialised.contains(
            r#""reason":{"kind":"exec_binary_not_found","data":{"path":"/usr/local/bin/payments"}}"#
        ),
        "nested cause-class structure missing or malformed: {serialised}"
    );

    // `source` is `{"kind":"driver","data":"exec"}` — the inner
    // `DriverType` is a unit-variant enum so `data` is just `"exec"`.
    assert!(
        serialised.contains(r#""source":{"kind":"driver","data":"exec""#),
        "transition source shape missing: {serialised}"
    );
}

#[test]
fn converged_failed_serialises_terminal_reason_with_inner_seconds_payload() {
    let event = SubmitEvent::ConvergedFailed {
        alloc_id: Some("alloc-a1b2".to_string()),
        terminal_reason: TerminalReason::Timeout { after_seconds: 60 },
        reason: None,
        error: None,
    };

    let serialised = serde_json::to_string(&event).expect("serialise");

    assert!(
        serialised.contains(r#""kind":"converged_failed""#),
        "outer envelope discriminator missing: {serialised}"
    );

    // Timeout carries `after_seconds: u32` — the typed payload
    // serialises directly without stringification.
    assert!(
        serialised.contains(r#""terminal_reason":{"kind":"timeout","data":{"after_seconds":60}}"#),
        "terminal_reason Timeout payload shape missing: {serialised}"
    );
}

#[test]
fn converged_failed_carries_nested_cause_class_under_backoff_exhausted() {
    let cause = TransitionReason::ExecBinaryNotFound { path: "/x".to_string() };
    let event = SubmitEvent::ConvergedFailed {
        alloc_id: Some("alloc-a1b2".to_string()),
        terminal_reason: TerminalReason::BackoffExhausted { attempts: 5, cause: cause.clone() },
        reason: Some(cause),
        error: Some("stat /x: no such file or directory".to_string()),
    };

    let serialised = serde_json::to_string(&event).expect("serialise");

    // `BackoffExhausted` carries `attempts: u32` AND a nested
    // cause-class `cause: TransitionReason`. The outer terminal_reason
    // entry must contain BOTH the attempts integer and the nested
    // cause structure.
    assert!(
        serialised.contains(r#""terminal_reason":{"kind":"backoff_exhausted""#),
        "terminal_reason BackoffExhausted discriminator missing: {serialised}"
    );
    assert!(
        serialised.contains(r#""attempts":5"#),
        "BackoffExhausted attempts field missing: {serialised}"
    );
    assert!(
        serialised.contains(r#""cause":{"kind":"exec_binary_not_found","data":{"path":"/x"}}"#),
        "nested cause-class TransitionReason missing under BackoffExhausted: {serialised}"
    );
}

#[test]
fn accepted_serialises_with_inserted_outcome_lowercase() {
    let event = SubmitEvent::Accepted {
        spec_digest: "sha256:abc".to_string(),
        intent_key: "jobs/payments".to_string(),
        outcome: IdempotencyOutcome::Inserted,
    };

    let serialised = serde_json::to_string(&event).expect("serialise");

    assert!(
        serialised.contains(r#""kind":"accepted""#),
        "outer envelope discriminator missing: {serialised}"
    );
    // `IdempotencyOutcome` carries `#[serde(rename_all = "lowercase")]`,
    // not snake_case — so `Inserted` becomes `"inserted"`.
    assert!(
        serialised.contains(r#""outcome":"inserted""#),
        "outcome lowercase rendering missing: {serialised}"
    );
    assert!(
        serialised.contains(r#""intent_key":"jobs/payments""#),
        "intent_key field missing: {serialised}"
    );
}

#[test]
fn converged_running_carries_alloc_id_and_started_at() {
    let event = SubmitEvent::ConvergedRunning {
        alloc_id: "alloc-r0".to_string(),
        started_at: "5@node-a".to_string(),
    };

    let serialised = serde_json::to_string(&event).expect("serialise");

    assert!(
        serialised.contains(r#""kind":"converged_running""#),
        "outer envelope discriminator missing: {serialised}"
    );
    assert!(
        serialised.contains(r#""alloc_id":"alloc-r0""#),
        "alloc_id field missing: {serialised}"
    );
    assert!(
        serialised.contains(r#""started_at":"5@node-a""#),
        "started_at field missing: {serialised}"
    );
}

// ---------------------------------------------------------------------------
// NDJSON line emit invariant — one event renders as one line with no
// trailing bytes when written via `serde_json::to_writer` + `b'\n'`.
// ---------------------------------------------------------------------------

#[test]
fn ndjson_line_emit_produces_single_line_with_trailing_newline() {
    let event = SubmitEvent::ConvergedRunning {
        alloc_id: "alloc-x".to_string(),
        started_at: "1@node-a".to_string(),
    };

    let mut buf: Vec<u8> = Vec::new();
    serde_json::to_writer(&mut buf, &event).expect("serialise into writer");
    buf.push(b'\n');

    // Exactly one newline, at the end.
    // The buffer is bounded (one event); the simple iter-filter-count
    // pattern is clearer than pulling `bytecount`.
    #[allow(clippy::naive_bytecount)]
    let newlines = buf.iter().filter(|b| **b == b'\n').count();
    assert_eq!(newlines, 1, "expected exactly one newline, got {newlines}");
    assert_eq!(buf.last(), Some(&b'\n'), "newline must be the trailing byte");

    // Body before the newline must be valid JSON that parses back.
    let body = std::str::from_utf8(&buf[..buf.len() - 1]).expect("utf-8");
    let parsed: SubmitEvent = serde_json::from_str(body).expect("round-trip from line body");
    assert_eq!(parsed, event);
}
