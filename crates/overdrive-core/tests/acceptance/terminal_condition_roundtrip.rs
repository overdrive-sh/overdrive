//! ADR-0037 prerequisite — `TerminalCondition` enum + `AllocStatusRow.terminal`
//! field roundtrip property.
//!
//! Per `docs/feature/reconciler-memory-redb/deliver/roadmap.json` step 01-02
//! AC#3: for every `Arbitrary` `TerminalCondition` value `v`, `encode(v) →
//! decode → equal-to-v` (covers all three variants + `None` at the
//! `AllocStatusRow` level).
//!
//! The codec exercised here is rkyv (matches the existing `AllocStatusRow`
//! derive surface — `rkyv::Archive, rkyv::Serialize, rkyv::Deserialize`).
//! Pinning the property at the row level ensures the new field is wired
//! through the archived shape, not just on the standalone enum.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use std::str::FromStr;

use overdrive_core::id::{AllocationId, JobId, NodeId};
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow, LogicalTimestamp};
use overdrive_core::transition_reason::{StoppedBy, TerminalCondition};
use proptest::prelude::*;
use rkyv::rancor;

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

fn arb_stopped_by() -> impl Strategy<Value = StoppedBy> {
    prop_oneof![Just(StoppedBy::Operator), Just(StoppedBy::Reconciler), Just(StoppedBy::Process),]
}

/// A `type_name` matching the ADR-0037 §1 shape for `Custom.type_name`:
/// a CamelCase identifier scoped by the reconciler (e.g.
/// `"vendor.io/quota.QuotaExhausted"`). Generator stays inside ASCII
/// printable to keep counter-examples readable.
fn arb_custom_type_name() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9_./-]{0,63}"
}

/// Optional opaque payload — `None` and `Some(bytes)` both arise; bytes
/// are bounded to keep the proptest budget tight.
fn arb_custom_detail() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![Just(None), prop::collection::vec(any::<u8>(), 0..=64).prop_map(Some),]
}

fn arb_terminal_condition() -> impl Strategy<Value = TerminalCondition> {
    prop_oneof![
        any::<u32>().prop_map(|attempts| TerminalCondition::BackoffExhausted { attempts }),
        arb_stopped_by().prop_map(|by| TerminalCondition::Stopped { by }),
        (arb_custom_type_name(), arb_custom_detail())
            .prop_map(|(type_name, detail)| TerminalCondition::Custom { type_name, detail }),
    ]
}

/// Generator for the `Option<TerminalCondition>` field on `AllocStatusRow`
/// — both `None` and every variant of `Some(...)` arise.
fn arb_terminal() -> impl Strategy<Value = Option<TerminalCondition>> {
    prop_oneof![Just(None), arb_terminal_condition().prop_map(Some),]
}

fn fixed_alloc_id() -> AllocationId {
    AllocationId::from_str("alloc-tc-roundtrip-0").expect("valid alloc id")
}

fn fixed_job_id() -> JobId {
    JobId::from_str("payments").expect("valid job id")
}

fn fixed_node_id() -> NodeId {
    NodeId::from_str("control-plane-0").expect("valid node id")
}

fn fixed_timestamp() -> LogicalTimestamp {
    LogicalTimestamp { counter: 1, writer: fixed_node_id() }
}

fn build_row(terminal: Option<TerminalCondition>) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: fixed_alloc_id(),
        job_id: fixed_job_id(),
        node_id: fixed_node_id(),
        state: AllocState::Failed,
        updated_at: fixed_timestamp(),
        reason: None,
        detail: None,
        terminal,
    }
}

// ---------------------------------------------------------------------------
// Property — every variant + None survives the rkyv roundtrip on the
// `AllocStatusRow.terminal` field.
// ---------------------------------------------------------------------------

proptest! {
    /// AC#3 — for every `Arbitrary` `TerminalCondition` value `v` (and
    /// for `None`), placing it on `AllocStatusRow.terminal`, encoding the
    /// row to rkyv bytes, then decoding back, returns a row with
    /// `terminal == original`.
    ///
    /// This is the row-level property — ADR-0037 §3 is explicit that
    /// `AllocStatusRow.terminal` is the *durable* home; pinning the
    /// roundtrip on the row guarantees the new field actually flows
    /// through the archived shape, not just at the enum level.
    #[test]
    fn terminal_condition_serializes_and_deserializes_for_every_variant(
        terminal in arb_terminal(),
    ) {
        let original = build_row(terminal);

        let bytes = rkyv::to_bytes::<rancor::Error>(&original)
            .expect("rkyv archival of AllocStatusRow with terminal field must succeed");

        let archived = rkyv::access::<rkyv::Archived<AllocStatusRow>, rancor::Error>(&bytes)
            .expect("archived bytes must validate as ArchivedAllocStatusRow");

        let deserialized: AllocStatusRow =
            rkyv::deserialize::<AllocStatusRow, rancor::Error>(archived)
                .expect("ArchivedAllocStatusRow must deserialize back to AllocStatusRow");

        prop_assert_eq!(
            deserialized,
            original,
            "rkyv roundtrip on AllocStatusRow must preserve the terminal field equality \
             across every TerminalCondition variant + None"
        );
    }
}
