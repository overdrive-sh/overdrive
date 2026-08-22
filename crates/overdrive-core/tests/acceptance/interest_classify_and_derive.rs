//! Acceptance scenarios for step 02-01 — Piece B pure routing functions
//! `classify` and `derive_target` (ADR-0081 §2, §5 step 3).
//!
//! Both enter through the pure-function driving port
//! (`overdrive_core::reconcilers::{classify, derive_target}`) and assert on
//! the returned value — no internal state is peeked.
//!
//! * **S-266-11** — `classify` is an EXHAUSTIVE per-variant match over all
//!   8 `ObservationRow` variants with NO wildcard: `AllocStatus` maps to
//!   `Some(RowKind::AllocStatus)`, the other seven each to `None`. Driven
//!   as a table over every variant (closed-world finite → parametrize, NOT
//!   PBT — the falsifier gate). Every match arm is the mutation surface.
//! * **S-266-21** — `derive_target(TargetFrom::Workload, row)` is a TOTAL
//!   mapping over the single-variant `TargetFrom` returning exactly
//!   `TargetResource("workload/<W>")` for a row carrying workload id `W`.
//!   Driven as a proptest over `W`.
//!
//! ## Compile-fail drift-closure companion (AC #2)
//!
//! The design's "a new `ObservationRow` variant must FAIL compilation
//! until consciously mapped `Some`/`None`" companion is enforced
//! STRUCTURALLY by the no-wildcard exhaustive `match` in `classify`
//! itself: adding a 9th `ObservationRow` variant makes that match
//! non-exhaustive and yields rustc `E0004` at `overdrive-core` compile
//! time — the same drift-closure a `trybuild` fixture would re-demonstrate.
//! A dedicated `trybuild` fixture is deliberately NOT added here: it would
//! require editing the out-of-scope `tests/compile_fail.rs` entrypoint plus
//! committing a brittle `.stderr` snapshot, and ADR-0081 forbids a new
//! dependency for this step. The `classify_table_covers_every_variant`
//! belt-and-braces assertion below pins that the table author enumerated
//! all 8 variants, so a table that silently drops a variant fails too.

#![allow(clippy::expect_used)]

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::Duration;

use proptest::prelude::*;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{
    AllocationId, CertSerial, ContentHash, CorrelationKey, IssuanceOrdinal, NodeId, Region,
    ServiceId, SpiffeId, WorkloadId,
};
use overdrive_core::reconcilers::{RowKind, TargetFrom, TargetResource, classify, derive_target};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, ConflictRoute, LogicalTimestamp, NodeHealthRow, ObservationRow,
    ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow, ServiceHydrationStatus,
};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};

// ---------------------------------------------------------------------------
// Row constructors — minimal-but-valid instances of every ObservationRow
// variant. `classify` matches on the discriminant only, so the payloads are
// the smallest valid values each type admits.
// ---------------------------------------------------------------------------

fn nid(raw: &str) -> NodeId {
    NodeId::from_str(raw).expect("node id is valid")
}

fn ts() -> LogicalTimestamp {
    LogicalTimestamp { counter: 1, writer: nid("cp-0") }
}

fn service_id() -> ServiceId {
    ServiceId::new(1).expect("any u64 is a valid ServiceId")
}

fn alloc_row_with(workload_id: WorkloadId) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-1").expect("alloc id is valid"),
        workload_id,
        node_id: nid("cp-0"),
        state: AllocState::Running,
        updated_at: ts(),
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

fn node_health_row() -> NodeHealthRow {
    NodeHealthRow {
        node_id: nid("cp-0"),
        region: Region::from_str("local").expect("region is valid"),
        last_heartbeat: ts(),
    }
}

fn service_hydration_row() -> ServiceHydrationResultRow {
    ServiceHydrationResultRow {
        service_id: service_id(),
        // `BackendSetFingerprint` is a `u64` type alias.
        fingerprint: 0,
        status: ServiceHydrationStatus::Pending,
        updated_at: ts(),
    }
}

fn service_backend_row() -> ServiceBackendRow {
    ServiceBackendRow {
        service_id: service_id(),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: Vec::new(),
        updated_at: ts(),
    }
}

fn reconcile_conflict_row() -> ReconcileConflictRow {
    ReconcileConflictRow {
        service_id: service_id(),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        port: 0,
        proto: Proto::Tcp,
        first_route: ConflictRoute::Xdp,
        second_route: ConflictRoute::Xdp,
        updated_at: ts(),
    }
}

fn issued_cert_row() -> IssuedCertificateRow {
    let at = UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000));
    IssuedCertificateRow {
        serial: CertSerial::new("01").expect("serial parses"),
        spiffe_id: SpiffeId::from_str("spiffe://overdrive.local/workload/payments")
            .expect("spiffe id is valid"),
        issuer_serial: CertSerial::new("00").expect("issuer serial parses"),
        not_before: at,
        not_after: at,
        node_id: nid("cp-0"),
        issued_at: at,
        issuance_ordinal: IssuanceOrdinal::new(0),
    }
}

fn workflow_terminal_row() -> ObservationRow {
    ObservationRow::WorkflowTerminal {
        correlation: CorrelationKey::derive(
            "workflow/all",
            &ContentHash::from_bytes([0u8; 32]),
            "terminal",
        ),
        status: WorkflowStatus::Cancelled,
    }
}

fn signal_row() -> ObservationRow {
    ObservationRow::Signal {
        key: SignalKey::new("sig-a").expect("signal key is valid"),
        value: SignalValue::new("v"),
    }
}

// ---------------------------------------------------------------------------
// S-266-11 — classify: exhaustive per-variant mapping, no wildcard
// ---------------------------------------------------------------------------

/// The full closed-world table: every one of the 8 `ObservationRow`
/// variants paired with its expected `classify` result.
fn classify_table() -> Vec<(&'static str, ObservationRow, Option<RowKind>)> {
    vec![
        (
            "AllocStatus",
            ObservationRow::AllocStatus(Box::new(alloc_row_with(
                WorkloadId::from_str("payments").expect("workload id is valid"),
            ))),
            Some(RowKind::AllocStatus),
        ),
        ("NodeHealth", ObservationRow::NodeHealth(node_health_row()), None),
        ("ServiceHydration", ObservationRow::ServiceHydration(service_hydration_row()), None),
        ("ServiceBackend", ObservationRow::ServiceBackend(service_backend_row()), None),
        ("ReconcileConflict", ObservationRow::ReconcileConflict(reconcile_conflict_row()), None),
        ("IssuedCertificate", ObservationRow::IssuedCertificate(issued_cert_row()), None),
        ("WorkflowTerminal", workflow_terminal_row(), None),
        ("Signal", signal_row(), None),
    ]
}

/// S-266-11 — `classify(&row)` maps `AllocStatus => Some(RowKind::AllocStatus)`
/// and each of the other seven `ObservationRow` variants `=> None`.
///
/// Parametrized over all 8 variants (closed-world finite → table, NOT PBT).
/// Mutation target: EVERY match arm — flipping any `None → Some`, swapping
/// the `Some(RowKind::AllocStatus)` arm, or collapsing the mapping breaks
/// the exact per-variant equality below.
#[test]
fn classify_maps_each_observation_row_variant_exhaustively() {
    for (name, row, expected) in classify_table() {
        assert_eq!(
            classify(&row),
            expected,
            "classify({name}) must map to {expected:?} (ADR-0081 §5 step 3 \
             exhaustive no-wildcard classify)",
        );
    }
}

/// Belt-and-braces for the drift-closure: the table MUST enumerate exactly
/// the 8 `ObservationRow` variants. A table that silently drops a variant
/// (or a future variant not added) fails here, complementing the
/// compile-time exhaustiveness the no-wildcard `match` in `classify`
/// enforces (see the module docstring's compile-fail companion note).
#[test]
fn classify_table_covers_every_variant() {
    assert_eq!(
        classify_table().len(),
        8,
        "ObservationRow has exactly 8 variants; the classify table MUST cover every one",
    );
}

/// Exactly one variant classifies to `Some` at Phase 1 (`AllocStatus`);
/// the other seven are `None`. Pins the "`AllocStatus` is the sole routed
/// family" shape so a mutation that makes a second arm `Some` is caught by
/// the count as well as by the per-variant assertion above.
#[test]
fn classify_routes_exactly_the_alloc_status_family() {
    let routed: Vec<&str> = classify_table()
        .into_iter()
        .filter_map(|(name, row, _)| classify(&row).map(|_| name))
        .collect();

    assert_eq!(
        routed,
        vec!["AllocStatus"],
        "exactly the AllocStatus family routes (Some) at Phase 1; all others are None",
    );
}

// ---------------------------------------------------------------------------
// Label-enum completeness — RowKind / TargetFrom own their canonical label
//
// AC #6 / ADR-0081 §2: the Piece B label enums own their `as_str`. These
// pin the canonical lowercase strings so a mutated label
// (`as_str -> ""` / `-> "xyzzy"`) is caught — mirroring the Piece A
// `resync_scope_local_node_as_str_is_canonical_kebab_label` label pin.
// ---------------------------------------------------------------------------

#[test]
fn row_kind_as_str_is_canonical_kebab_label() {
    assert_eq!(RowKind::AllocStatus.as_str(), "alloc-status");
}

#[test]
fn target_from_as_str_is_canonical_kebab_label() {
    assert_eq!(TargetFrom::Workload.as_str(), "workload");
}

// ---------------------------------------------------------------------------
// S-266-21 — derive_target: total over TargetFrom::Workload
// ---------------------------------------------------------------------------

/// Strategy yielding an arbitrary VALID `WorkloadId`.
///
/// Mirrors the `validate_label` contract (`crates/overdrive-core/src/id.rs`):
/// non-empty, chars in `[a-z0-9-_.]`, first/last char alphanumeric. First
/// and last glyphs are drawn from the alphanumeric class; interior glyphs
/// from the full label class. Same shape as `valid_node_id` in the Piece A
/// `resolve_scope` proptest.
fn valid_workload_id() -> impl Strategy<Value = WorkloadId> {
    let alnum: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
    let interior: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789-_.".chars().collect();
    (
        proptest::sample::select(alnum.clone()),
        proptest::collection::vec(proptest::sample::select(interior), 0..=16),
        proptest::sample::select(alnum),
    )
        .prop_map(|(first, mid, last)| {
            let mut raw = String::with_capacity(2 + mid.len());
            raw.push(first);
            raw.extend(mid);
            raw.push(last);
            WorkloadId::new(&raw).expect("generator yields only valid WorkloadIds")
        })
}

proptest! {
    /// S-266-21 — `derive_target(TargetFrom::Workload, row)` is a TOTAL
    /// mapping over the single-variant `TargetFrom`, returning exactly
    /// `TargetResource("workload/<W>")` for a row carrying workload id `W`.
    ///
    /// Mutation target: the `TargetFrom::Workload → workload/<id>`
    /// derivation (mutation-surface #3; pure-fn sibling of the S-266-12
    /// DST-through-router path). A mutated prefix / dropped id / wrong field
    /// must break the exact `TargetResource` equality below.
    #[test]
    fn workload_target_derivation_is_pure_and_total(workload_id in valid_workload_id()) {
        let row = alloc_row_with(workload_id.clone());

        let derived = derive_target(TargetFrom::Workload, &row);

        let expected = TargetResource::new(&format!("workload/{}", workload_id.as_str()))
            .expect("workload/<valid WorkloadId> is a canonical TargetResource");

        prop_assert_eq!(derived, expected);
    }
}
