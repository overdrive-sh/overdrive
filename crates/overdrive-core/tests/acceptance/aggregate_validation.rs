//! Acceptance scenarios for phase-1-control-plane-core step 01-02 —
//! validating constructors reject malformed specs with structured
//! [`AggregateError`] before any archival attempt.
//!
//! Covers the §2.2 error-boundary scenarios from
//! `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`:
//!
//! * Scenario — Node construction rejects zero-byte memory capacity.
//! * Scenario — Job construction rejects a zero-replica count (field +
//!   value echoed in the message).
//! * Scenario — Job construction rejects a malformed JobId before any
//!   archive attempt (pass-through via `AggregateError::Id(..)`).
//!
//! Also pins:
//!
//! * `Node::new` rejects forbidden chars in id / empty region via the
//!   same `#[from]` pass-through shape.
//! * `Allocation::new` rejects forbidden chars in each of its three
//!   id fields independently.
//! * Structural invariant — on the `Err` branch no `Job` / `Node` /
//!   `Allocation` value is constructed (the return type is `Result`,
//!   which makes archival-on-invalid unrepresentable).
//!
//! Per ADR-0015 §1 `ControlPlaneError::Aggregate(_)` will wrap these
//! variants via `#[from]` to surface as HTTP 400 in Slice 3 — the
//! variant shape pinned here is the one the HTTP layer consumes, so
//! these tests also defend the downstream contract.

// `expect` / `expect_err` are the standard idiom in test code — a panic
// with a message is exactly what you want when a precondition fails.
#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{
    AggregateError, Allocation, AllocationSpecInput, Job, JobSpecInput, Node, NodeSpecInput,
};
use overdrive_core::id::IdParseError;

// ---------------------------------------------------------------------------
// Job: zero replicas → Validation { field: "replicas", message contains "0" }
// ---------------------------------------------------------------------------

#[test]
fn job_from_spec_rejects_zero_replicas_with_validation_variant_naming_replicas_field_and_including_value_0_in_message()
 {
    // Given a Job spec whose replicas field is zero.
    let spec = JobSpecInput {
        id: "payments".to_string(),
        replicas: 0,
        cpu_milli: 2000,
        memory_bytes: 4 * 1024 * 1024 * 1024,
    };

    // When Ana calls the validating constructor.
    let err = Job::from_spec(spec).expect_err("zero replicas must be rejected");

    // Then the error is the Validation variant naming the replicas field.
    match err {
        AggregateError::Validation { field, ref message } => {
            assert_eq!(field, "replicas", "field must name `replicas`; got {field:?}");
            assert!(
                message.contains('0'),
                "message must echo the invalid value 0; got {message:?}"
            );
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
    // And the Display form ties field + message together so the HTTP
    // layer can render it verbatim per ADR-0015.
    assert!(err.to_string().contains("replicas"), "Display must include the field; got {}", err);
}

// ---------------------------------------------------------------------------
// Job: forbidden space in id → Id(IdParseError::InvalidChar { .. })
// Structural guarantee: on Err, no Job value is constructed (return type).
// ---------------------------------------------------------------------------

#[test]
fn job_from_spec_rejects_forbidden_space_in_id_via_id_parse_error_passthrough_without_constructing_job()
 {
    // Given a Job spec whose id contains a forbidden space character.
    let spec = JobSpecInput {
        id: "PAY MENTS".to_string(),
        replicas: 1,
        cpu_milli: 2000,
        memory_bytes: 4 * 1024 * 1024 * 1024,
    };

    // When Ana calls the validating constructor.
    let result = Job::from_spec(spec);

    // Then the result is Err — and the type system guarantees no Job
    // is constructed on this branch. `Result<Job, _>::Err` does not
    // carry a `Job`; any downstream archival call would require a
    // `Job` by type, so no rkyv::to_bytes call on an invalid input is
    // reachable from this code path. The sentinel here is structural.
    let err = result.expect_err("forbidden char in id must be rejected");

    // And the variant is the pass-through from IdParseError via `#[from]`.
    match err {
        AggregateError::Id(IdParseError::InvalidChar { kind, ch, index }) => {
            assert_eq!(kind, "JobId", "InvalidChar must name the JobId kind; got {kind:?}");
            assert_eq!(ch, ' ', "InvalidChar must carry the offending char; got {ch:?}");
            // The space sits at index 3 in "pay mentS" after lowercasing
            // ("PAY MENTS" -> "pay ments"), per validate_label's
            // case-insensitive pipeline. Assert the index is within the
            // string — not the exact position, since the lowering is an
            // implementation detail that shifts indices only if the
            // casing rule changes.
            assert!(
                index < "PAY MENTS".len(),
                "InvalidChar index must be within the input; got {index}"
            );
        }
        other => panic!("expected AggregateError::Id(InvalidChar), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Job: zero memory → Validation { field: "memory_bytes", .. }
// ---------------------------------------------------------------------------

#[test]
fn job_from_spec_rejects_zero_memory_with_validation_variant_naming_memory_bytes_field() {
    // Given a Job spec with zero memory_bytes and every other field valid.
    let spec =
        JobSpecInput { id: "payments".to_string(), replicas: 1, cpu_milli: 2000, memory_bytes: 0 };

    // When Ana calls the validating constructor.
    let err = Job::from_spec(spec).expect_err("zero memory must be rejected");

    // Then the error names the memory_bytes field.
    match err {
        AggregateError::Validation { field, message: _ } => {
            assert_eq!(field, "memory_bytes", "field must name `memory_bytes`; got {field:?}");
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Node: zero memory → Validation { field: "memory_bytes", .. }
// ---------------------------------------------------------------------------

#[test]
fn node_new_rejects_zero_memory_with_validation_variant_naming_memory_bytes_field() {
    // Given a Node spec with zero memory_bytes and every other field valid.
    let spec = NodeSpecInput {
        id: "worker-01".to_string(),
        region: "eu-west-1".to_string(),
        cpu_milli: 8000,
        memory_bytes: 0,
    };

    // When Ana calls the validating constructor.
    let err = Node::new(spec).expect_err("zero memory must be rejected");

    // Then the error names the memory_bytes field.
    match err {
        AggregateError::Validation { field, message: _ } => {
            assert_eq!(field, "memory_bytes", "field must name `memory_bytes`; got {field:?}");
        }
        other => panic!("expected AggregateError::Validation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Node: forbidden char in id → Id(InvalidChar { kind: "NodeId", .. })
// ---------------------------------------------------------------------------

#[test]
fn node_new_rejects_forbidden_char_in_id_via_id_parse_error_passthrough() {
    // Given a Node spec whose id contains a forbidden space character.
    let spec = NodeSpecInput {
        id: "worker 01".to_string(),
        region: "eu-west-1".to_string(),
        cpu_milli: 8000,
        memory_bytes: 16 * 1024 * 1024 * 1024,
    };

    // When Ana calls the validating constructor.
    let err = Node::new(spec).expect_err("forbidden char in NodeId must be rejected");

    // Then the variant is the pass-through from IdParseError.
    match err {
        AggregateError::Id(IdParseError::InvalidChar { kind, ch, .. }) => {
            assert_eq!(kind, "NodeId", "InvalidChar must name the NodeId kind; got {kind:?}");
            assert_eq!(ch, ' ', "InvalidChar must carry the offending char; got {ch:?}");
        }
        other => panic!("expected AggregateError::Id(InvalidChar for NodeId), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Node: empty region → Id(Empty { kind: "Region" })
// ---------------------------------------------------------------------------

#[test]
fn node_new_rejects_empty_region_via_id_parse_error_passthrough() {
    // Given a Node spec whose region is empty.
    let spec = NodeSpecInput {
        id: "worker-01".to_string(),
        region: String::new(),
        cpu_milli: 8000,
        memory_bytes: 16 * 1024 * 1024 * 1024,
    };

    // When Ana calls the validating constructor.
    let err = Node::new(spec).expect_err("empty region must be rejected");

    // Then the variant is the pass-through from IdParseError::Empty.
    match err {
        AggregateError::Id(IdParseError::Empty { kind }) => {
            assert_eq!(kind, "Region", "Empty must name the Region kind; got {kind:?}");
        }
        other => panic!("expected AggregateError::Id(Empty for Region), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Allocation: forbidden char in each of the three id fields independently.
// ---------------------------------------------------------------------------

#[test]
fn allocation_new_rejects_forbidden_char_in_allocation_id() {
    let spec = AllocationSpecInput {
        id: "BAD ID".to_string(),
        job_id: "payments".to_string(),
        node_id: "worker-01".to_string(),
    };
    let err = Allocation::new(spec).expect_err("forbidden AllocationId char must be rejected");
    match err {
        AggregateError::Id(IdParseError::InvalidChar { kind, .. }) => {
            assert_eq!(kind, "AllocationId", "must name AllocationId kind; got {kind:?}");
        }
        other => panic!("expected AggregateError::Id(InvalidChar for AllocationId), got {other:?}"),
    }
}

#[test]
fn allocation_new_rejects_forbidden_char_in_job_id() {
    // Given a valid AllocationId and NodeId but a forbidden char in job_id.
    let spec = AllocationSpecInput {
        id: "a1b2c3d4".to_string(),
        job_id: "BAD JOB".to_string(),
        node_id: "worker-01".to_string(),
    };
    let err = Allocation::new(spec).expect_err("forbidden JobId char must be rejected");
    match err {
        AggregateError::Id(IdParseError::InvalidChar { kind, .. }) => {
            assert_eq!(kind, "JobId", "must name JobId kind; got {kind:?}");
        }
        other => panic!("expected AggregateError::Id(InvalidChar for JobId), got {other:?}"),
    }
}

#[test]
fn allocation_new_rejects_forbidden_char_in_node_id() {
    // Given a valid AllocationId and JobId but a forbidden char in node_id.
    let spec = AllocationSpecInput {
        id: "a1b2c3d4".to_string(),
        job_id: "payments".to_string(),
        node_id: "BAD NODE".to_string(),
    };
    let err = Allocation::new(spec).expect_err("forbidden NodeId char must be rejected");
    match err {
        AggregateError::Id(IdParseError::InvalidChar { kind, .. }) => {
            assert_eq!(kind, "NodeId", "must name NodeId kind; got {kind:?}");
        }
        other => panic!("expected AggregateError::Id(InvalidChar for NodeId), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Structural rkyv-sentinel check — `Job::from_spec` signature returns
// `Result<Job, AggregateError>`. On the Err branch no `Job` exists by
// construction, so no downstream archival call is reachable. This test
// documents the structural guarantee; no runtime wrapper is needed
// because the type system enforces it.
// ---------------------------------------------------------------------------

#[test]
fn err_branch_of_from_spec_carries_no_job_value_by_construction() {
    // Given invalid input.
    let spec = JobSpecInput {
        id: "PAY MENTS".to_string(),
        replicas: 0, // triply-invalid, picks up whichever fails first
        cpu_milli: 2000,
        memory_bytes: 0,
    };

    // When Ana calls from_spec.
    let result: Result<Job, AggregateError> = Job::from_spec(spec);

    // Then the return type is Result<Job, _>. On Err, the value
    // discriminant carries no Job. This is the structural sentinel —
    // a future implementation cannot "leak" a partially-constructed
    // Job to the archival path unless it changes the return type.
    // Any call site that attempts to pass an `AggregateError` to
    // something expecting a `Job` will fail to compile.
    assert!(result.is_err(), "triply-invalid input must produce Err");

    // And assert the Err arm pattern — this is what downstream
    // archival-gating code will branch on.
    match result {
        Err(_) => {
            // No `Job` in scope on this branch. The type system
            // enforces it — no runtime sentinel needed.
        }
        Ok(job) => panic!("Ok branch should be unreachable, got {job:?}"),
    }
}

// ---------------------------------------------------------------------------
// Property test — every zero-replica input produces the same variant shape.
// ---------------------------------------------------------------------------

mod property {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// For any otherwise-valid Job spec where `replicas == 0`,
        /// `from_spec` must always return `Validation { field: "replicas", .. }`.
        /// This closes the mutation gap on the `replicas == 0` guard per
        /// testing.md mutation target "Newtype FromStr and validators".
        #[test]
        fn zero_replicas_always_yields_replicas_validation(
            cpu in 0u32..10_000,
            mem in 1u64..=u64::MAX,
        ) {
            let spec = JobSpecInput {
                id: "payments".to_string(),
                replicas: 0,
                cpu_milli: cpu,
                memory_bytes: mem,
            };
            match Job::from_spec(spec) {
                Err(AggregateError::Validation { field, .. }) => {
                    prop_assert_eq!(field, "replicas");
                }
                other => prop_assert!(
                    false,
                    "expected Validation{{field: \"replicas\"}}, got {:?}",
                    other
                ),
            }
        }

        /// For any otherwise-valid Job spec where `memory_bytes == 0`
        /// and `replicas >= 1`, `from_spec` must return
        /// `Validation { field: "memory_bytes", .. }`.
        #[test]
        fn zero_memory_always_yields_memory_bytes_validation(
            cpu in 0u32..10_000,
            replicas in 1u32..=1_000_000,
        ) {
            let spec = JobSpecInput {
                id: "payments".to_string(),
                replicas,
                cpu_milli: cpu,
                memory_bytes: 0,
            };
            match Job::from_spec(spec) {
                Err(AggregateError::Validation { field, .. }) => {
                    prop_assert_eq!(field, "memory_bytes");
                }
                other => prop_assert!(
                    false,
                    "expected Validation{{field: \"memory_bytes\"}}, got {:?}",
                    other
                ),
            }
        }

        /// For any otherwise-valid Node spec where `memory_bytes == 0`,
        /// `new` must return `Validation { field: "memory_bytes", .. }`.
        #[test]
        fn node_zero_memory_always_yields_memory_bytes_validation(
            cpu in 0u32..10_000,
        ) {
            let spec = NodeSpecInput {
                id: "worker-01".to_string(),
                region: "eu-west-1".to_string(),
                cpu_milli: cpu,
                memory_bytes: 0,
            };
            match Node::new(spec) {
                Err(AggregateError::Validation { field, .. }) => {
                    prop_assert_eq!(field, "memory_bytes");
                }
                other => prop_assert!(
                    false,
                    "expected Validation{{field: \"memory_bytes\"}}, got {:?}",
                    other
                ),
            }
        }
    }
}
