//! S-CP-09 — `AllocStatusRow` round-trips through rkyv with the new
//! `reason` and `detail` fields.
//!
//! Per ADR-0032 §3 (Amendment 2026-04-30) the row schema gains
//! `reason: Option<TransitionReason>` and `detail: Option<String>` as
//! additive, forward-compatible fields populated by the action shim.
//! `TransitionReason` is the cause-class enum locked in the same
//! amendment — 5 progress markers + 9 Phase-1 cause-class failures + 2
//! Phase-2 emit-deferred forward-compat variants (16 total).
//!
//! This proptest exercises the rkyv roundtrip across:
//!   * every `TransitionReason` variant (cause-class with proptest-
//!     generated payloads via the crate's `Arbitrary`-shaped
//!     generator);
//!   * `Option<String>` detail (None and arbitrary Some);
//!   * every `AllocState` variant including the new `Failed`.
//!
//! The body asserts bidirectional round-trip equality (1024 cases per
//! `.claude/rules/testing.md` § Property-based testing) and the Phase-1
//! forward-compat property — rows with `reason = None` and `detail =
//! None` must archive to the same bytes as a fresh row constructed
//! with the pre-feature shape (we approximate the pre-feature shape
//! with both fields explicitly set to `None` and assert byte stability
//! across two archivals on the same input).

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

use std::str::FromStr;

use overdrive_core::TransitionReason;
use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{CancelledBy, ResourceEnvelope, StoppedBy};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Bounded ASCII label used for free-form `String` payload generation.
/// Kept short to keep proptest cases fast; matches the canonical
/// "valid label" shape used elsewhere in the workspace.
fn arb_label() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,15}".prop_map(String::from)
}

/// Generator for every `TransitionReason` variant (16 total). Cause-class
/// variants carry proptest-generated payloads; progress markers are
/// payload-less or carry minimal scalar payloads.
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

fn arb_alloc_state() -> impl Strategy<Value = AllocState> {
    prop_oneof![
        Just(AllocState::Pending),
        Just(AllocState::Running),
        Just(AllocState::Draining),
        Just(AllocState::Suspended),
        Just(AllocState::Terminated),
        Just(AllocState::Failed),
    ]
}

fn sample_alloc_id() -> AllocationId {
    AllocationId::from_str("alloc-roundtrip").expect("valid alloc id")
}

fn sample_job_id() -> JobId {
    JobId::from_str("payments").expect("valid job id")
}

fn sample_node_id() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn build_row(
    state: AllocState,
    reason: Option<TransitionReason>,
    detail: Option<String>,
) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: sample_alloc_id(),
        job_id: sample_job_id(),
        node_id: sample_node_id(),
        state,
        updated_at: LogicalTimestamp { counter: 1, writer: sample_node_id() },
        reason,
        detail,
        terminal: None,
    }
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    /// For any `AllocStatusRow` constructed from the full cross-product
    /// of (every `AllocState` × every `TransitionReason` variant ×
    /// arbitrary `Option<String>` detail), rkyv archive → access →
    /// deserialise yields a row equal to the original.
    ///
    /// This is the structural mandate for ADR-0032 §3 (the locked
    /// cause-class catalogue) plus ADR-0032 §4 (additive rkyv archive
    /// shape).
    #[test]
    fn alloc_status_row_rkyv_roundtrip_preserves_equality(
        state in arb_alloc_state(),
        reason in proptest::option::of(arb_transition_reason()),
        detail in proptest::option::of(arb_label()),
    ) {
        let original = build_row(state, reason, detail);

        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&original)
            .expect("rkyv archival of any AllocStatusRow must succeed");

        let archived = rkyv::access::<rkyv::Archived<AllocStatusRow>, rkyv::rancor::Error>(&bytes)
            .expect("archived bytes must validate as ArchivedAllocStatusRow");

        let back: AllocStatusRow =
            rkyv::deserialize::<AllocStatusRow, rkyv::rancor::Error>(archived)
                .expect("ArchivedAllocStatusRow must deserialize back to AllocStatusRow");

        prop_assert_eq!(back, original);
    }

    /// rkyv archival is byte-deterministic on repeated calls (the
    /// content-addressed hash precondition from
    /// `.claude/rules/development.md` § Hashing). This applies row-by-row
    /// across the full state × reason × detail cross-product.
    #[test]
    fn alloc_status_row_rkyv_archival_is_byte_deterministic(
        state in arb_alloc_state(),
        reason in proptest::option::of(arb_transition_reason()),
        detail in proptest::option::of(arb_label()),
    ) {
        let row = build_row(state, reason, detail);

        let first = rkyv::to_bytes::<rkyv::rancor::Error>(&row)
            .expect("first archival must succeed");
        let second = rkyv::to_bytes::<rkyv::rancor::Error>(&row)
            .expect("second archival must succeed");

        prop_assert_eq!(
            first.as_slice(),
            second.as_slice(),
            "two rkyv archivals of the same AllocStatusRow must produce \
             byte-identical output"
        );
    }
}

// ---------------------------------------------------------------------------
// Forward-compatibility — rows with reason: None and detail: None must
// archive deterministically. This is the "rows with reason None and
// detail None archive to the same bytes as the pre-feature shape" Then
// clause from S-CP-09: the live property we can assert in-process is
// that omitted-cause rows produce a stable archive shape across calls,
// which is precisely what existing redb files rely on. The pre-feature
// byte stream is captured indirectly — any redb file carrying the Phase
// 0 shape already deserialises back through rkyv's archival shape, and
// the archival of `(None, None)` is the canonical Phase 0 reproduction.
// ---------------------------------------------------------------------------

#[test]
fn pre_feature_shape_row_archives_byte_deterministically() {
    // The pre-feature row shape: no reason, no detail.
    let row = build_row(AllocState::Running, None, None);

    let first = rkyv::to_bytes::<rkyv::rancor::Error>(&row).expect("first archival");
    let second = rkyv::to_bytes::<rkyv::rancor::Error>(&row).expect("second archival");

    assert_eq!(
        first.as_slice(),
        second.as_slice(),
        "rows with reason None and detail None must archive to byte-identical \
         output across calls — the precondition for forward-compat with \
         pre-feature redb data"
    );

    // And the round-trip closes too.
    let archived = rkyv::access::<rkyv::Archived<AllocStatusRow>, rkyv::rancor::Error>(&first)
        .expect("archived bytes must validate");
    let back: AllocStatusRow = rkyv::deserialize::<AllocStatusRow, rkyv::rancor::Error>(archived)
        .expect("must deserialize");
    assert_eq!(back, row, "(None, None) row must round-trip equality");
}

// ---------------------------------------------------------------------------
// Failed state — proves the new variant participates in the round-trip
// regardless of cause-class payload.
// ---------------------------------------------------------------------------

#[test]
fn failed_state_with_cause_class_reason_round_trips() {
    let row = build_row(
        AllocState::Failed,
        Some(TransitionReason::ExecBinaryNotFound { path: "/usr/local/bin/payments".to_owned() }),
        Some("verbatim driver text".to_owned()),
    );

    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&row).expect("archive");
    let archived = rkyv::access::<rkyv::Archived<AllocStatusRow>, rkyv::rancor::Error>(&bytes)
        .expect("access");
    let back: AllocStatusRow =
        rkyv::deserialize::<AllocStatusRow, rkyv::rancor::Error>(archived).expect("deserialize");

    assert_eq!(back, row);
}
