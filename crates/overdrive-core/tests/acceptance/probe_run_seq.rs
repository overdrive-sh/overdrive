//! `ProbeRunSeq` / `SameStatusRuns` newtype completeness and the
//! `ProbeResultRowV2` codec (R5-PROBE-V2, netns-density-295 PROBE-RUN-COUNT,
//! ADR-0191).
//!
//! Per `.claude/rules/testing.md` § "Property-based testing" → "Newtype
//! roundtrip": every newtype's `Display` / `FromStr` / serde round-trips for
//! every valid input, and every invalid input is rejected with the structured
//! parse error. Driving ports: the newtypes' public constructors and trait
//! impls; the row codec through `ProbeResultRowEnvelope::latest` and
//! `decode_envelope_bytes`.

#![allow(
    clippy::doc_markdown,
    clippy::redundant_clone,
    reason = "DISTILL test support and conformance drivers: handler signatures follow the action map and fixtures assert their preconditions"
)]

use std::str::FromStr;

use overdrive_core::codec::{VersionedEnvelope, decode_envelope_bytes};
use overdrive_core::id::AllocationId;
use overdrive_core::observation::{
    ProbeIdx, ProbeResultRowEnvelope, ProbeResultRowV1, ProbeResultRowV2, ProbeRole, ProbeRunSeq,
    ProbeRunSeqParseError, ProbeStatus, SameStatusRuns, SameStatusRunsParseError,
};
use proptest::prelude::*;

fn arb_role() -> impl Strategy<Value = ProbeRole> {
    prop_oneof![Just(ProbeRole::Startup), Just(ProbeRole::Readiness), Just(ProbeRole::Liveness)]
}

fn arb_status() -> impl Strategy<Value = ProbeStatus> {
    prop_oneof![
        Just(ProbeStatus::Pass),
        "[a-z ]{0,24}".prop_map(|last_fail_reason| ProbeStatus::Fail { last_fail_reason }),
    ]
}

fn arb_v1() -> impl Strategy<Value = ProbeResultRowV1> {
    (
        "[a-z][a-z0-9-]{1,19}[a-z0-9]",
        any::<u32>(),
        arb_role(),
        arb_status(),
        any::<u64>(),
        any::<bool>(),
    )
        .prop_map(|(alloc, idx, role, status, at, inferred)| ProbeResultRowV1 {
            alloc_id: AllocationId::new(&alloc).expect("generated alloc id is valid"),
            probe_idx: ProbeIdx::new(idx),
            role,
            status,
            last_observed_at_unix_ms: at,
            inferred,
        })
}

fn arb_v2() -> impl Strategy<Value = ProbeResultRowV2> {
    (arb_v1(), 1..=u64::MAX, 1..=u32::MAX).prop_map(|(v1, seq, runs)| ProbeResultRowV2 {
        alloc_id: v1.alloc_id,
        probe_idx: v1.probe_idx,
        role: v1.role,
        status: v1.status,
        last_observed_at_unix_ms: v1.last_observed_at_unix_ms,
        inferred: v1.inferred,
        run_seq: ProbeRunSeq::new(seq).expect("generated run sequence is non-zero"),
        same_status_runs: SameStatusRuns::new(runs).expect("generated streak is non-zero"),
    })
}

proptest! {
    /// Every run sequence ≥ 1 round-trips through `Display` / `FromStr` as
    /// its decimal form, and through serde as the bare `u64`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn probe_run_seq_round_trips_as_its_decimal_value(seq in 1..=u64::MAX) {
        let value = ProbeRunSeq::new(seq).expect("non-zero");
        prop_assert_eq!(value.get(), seq);
        prop_assert_eq!(value.to_string(), seq.to_string());
        prop_assert_eq!(ProbeRunSeq::from_str(&seq.to_string()), Ok(value));
        let json = serde_json::to_string(&value).expect("serialize");
        prop_assert_eq!(&json, &seq.to_string());
        prop_assert_eq!(serde_json::from_str::<ProbeRunSeq>(&json).expect("deserialize"), value);
    }

    /// Every streak ≥ 1 round-trips through `Display` / `FromStr` as its
    /// decimal form, and through serde as the bare `u32`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn same_status_runs_round_trips_as_its_decimal_value(runs in 1..=u32::MAX) {
        let value = SameStatusRuns::new(runs).expect("non-zero");
        prop_assert_eq!(value.get(), runs);
        prop_assert_eq!(value.to_string(), runs.to_string());
        prop_assert_eq!(SameStatusRuns::from_str(&runs.to_string()), Ok(value));
        let json = serde_json::to_string(&value).expect("serialize");
        prop_assert_eq!(&json, &runs.to_string());
        prop_assert_eq!(serde_json::from_str::<SameStatusRuns>(&json).expect("deserialize"), value);
    }

    /// Input that is not a decimal number is rejected as malformed, never
    /// panics and never yields a value.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn non_decimal_input_is_rejected_as_malformed(text in "[^0-9]{1,12}|[0-9]{0,4}[a-z][0-9]{0,4}|-[1-9][0-9]{0,5}") {
        prop_assert!(
            matches!(ProbeRunSeq::from_str(&text), Err(ProbeRunSeqParseError::Malformed(_))),
            "ProbeRunSeq must reject {:?} as Malformed", text
        );
        prop_assert!(
            matches!(SameStatusRuns::from_str(&text), Err(SameStatusRunsParseError::Malformed(_))),
            "SameStatusRuns must reject {:?} as Malformed", text
        );
    }

    /// The successor is one greater, up to the ceiling where it saturates.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn successor_is_one_greater(seq in 1..u64::MAX, runs in 1..u32::MAX) {
        prop_assert_eq!(ProbeRunSeq::new(seq).expect("non-zero").succ().get(), seq + 1);
        prop_assert_eq!(SameStatusRuns::new(runs).expect("non-zero").succ().get(), runs + 1);
    }

    /// A V1 row up-converts to the V2 row carrying every V1 field unchanged,
    /// run sequence 1 and streak 1 (a V1 row is never counted toward a live
    /// allocation).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn v1_row_up_converts_with_first_run_and_streak_one(v1 in arb_v1()) {
        let expected = ProbeResultRowV2 {
            alloc_id: v1.alloc_id.clone(),
            probe_idx: v1.probe_idx,
            role: v1.role,
            status: v1.status.clone(),
            last_observed_at_unix_ms: v1.last_observed_at_unix_ms,
            inferred: v1.inferred,
            run_seq: ProbeRunSeq::FIRST,
            same_status_runs: SameStatusRuns::ONE,
        };
        prop_assert_eq!(ProbeResultRowV2::from(v1.clone()), expected.clone());
        prop_assert_eq!(ProbeResultRowEnvelope::V1(v1).into_latest().expect("V1 projects"), expected);
    }

    /// Every V2 row round-trips bit-equivalent through the envelope codec the
    /// stores use (`latest` → rkyv archive → `decode_envelope_bytes`).
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn v2_row_round_trips_through_the_envelope_codec(row in arb_v2()) {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&ProbeResultRowEnvelope::latest(row.clone()))
            .expect("archive");
        let decoded = decode_envelope_bytes::<ProbeResultRowEnvelope>(bytes.as_ref()).expect("decode");
        prop_assert_eq!(decoded, row);
    }

    /// Every V2 row round-trips through serde with `run_seq` and
    /// `same_status_runs` carried as their bare integers.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn v2_row_round_trips_through_serde(row in arb_v2()) {
        let json = serde_json::to_value(&row).expect("serialize");
        prop_assert_eq!(&json["run_seq"], &serde_json::json!(row.run_seq.get()));
        prop_assert_eq!(&json["same_status_runs"], &serde_json::json!(row.same_status_runs.get()));
        prop_assert_eq!(serde_json::from_value::<ProbeResultRowV2>(json).expect("deserialize"), row);
    }
}

/// 0 is not a run sequence or a streak, whichever surface constructs it.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn zero_is_rejected_by_every_constructor() {
    assert_eq!(ProbeRunSeq::new(0), Err(ProbeRunSeqParseError::Zero));
    assert_eq!(ProbeRunSeq::try_from(0_u64), Err(ProbeRunSeqParseError::Zero));
    assert_eq!(ProbeRunSeq::from_str("0"), Err(ProbeRunSeqParseError::Zero));
    assert!(serde_json::from_str::<ProbeRunSeq>("0").is_err());
    assert_eq!(SameStatusRuns::new(0), Err(SameStatusRunsParseError::Zero));
    assert_eq!(SameStatusRuns::try_from(0_u32), Err(SameStatusRunsParseError::Zero));
    assert_eq!(SameStatusRuns::from_str("0"), Err(SameStatusRunsParseError::Zero));
    assert!(serde_json::from_str::<SameStatusRuns>("0").is_err());
}

/// The first written row carries run sequence 1 and streak 1, and the
/// successors saturate at the ceiling rather than wrapping.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn first_values_are_one_and_successors_saturate() {
    assert_eq!(ProbeRunSeq::FIRST.get(), 1);
    assert_eq!(SameStatusRuns::ONE.get(), 1);
    assert_eq!(ProbeRunSeq::new(u64::MAX).expect("max").succ().get(), u64::MAX);
    assert_eq!(SameStatusRuns::new(u32::MAX).expect("max").succ().get(), u32::MAX);
    assert_eq!(ProbeRunSeq::FIRST.succ(), ProbeRunSeq::new(2).expect("two"));
}
