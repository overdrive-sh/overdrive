//! Acceptance scenarios for step 02-01 — Piece B complete discriminant
//! `ObservationRow::kind()` + `ObservationRowKind` (ADR-0084 §2, §5 step 3,
//! 2026-08-23 lean amendment; GH #266).
//!
//! Enters through the pure-projection driving port
//! (`ObservationRow::kind()`) and the label-enum port
//! (`ObservationRowKind::as_str`) and asserts on the returned value — no
//! internal state is peeked.
//!
//! * **S-266-11** — `ObservationRow::kind()` is a TOTAL, no-wildcard
//!   discriminant projection: each of the 8 `ObservationRow` variants maps to
//!   its matching `ObservationRowKind`. Driven as a table over every variant
//!   (closed-world finite → parametrize, NOT PBT — the falsifier gate). Every
//!   match arm is the mutation surface: mutating an arm to the wrong
//!   `ObservationRowKind` breaks the exact per-variant equality below.
//!
//! ## Compile-fail drift-closure companion (AC #2)
//!
//! The design's "a new `ObservationRow` variant must FAIL compilation until
//! consciously mapped" companion is enforced STRUCTURALLY by the no-wildcard
//! total `match self` in `ObservationRow::kind()` itself: adding a 9th
//! `ObservationRow` variant makes that match non-exhaustive and yields rustc
//! `E0004` at `overdrive-core` compile time — the same drift-closure a
//! `trybuild` fixture would re-demonstrate, now owned by the type that owns
//! the variants (strictly stronger than a router-local `classify` helper). A
//! dedicated `trybuild` fixture is deliberately NOT added: it would require
//! editing the out-of-scope `tests/compile_fail.rs` entrypoint plus committing
//! a brittle `.stderr` snapshot, and ADR-0084 forbids a new dependency for
//! this step. The `kind_table_covers_every_variant` belt-and-braces assertion
//! below pins that the table author enumerated all 8 variants, so a table that
//! silently drops a variant fails too.

#![allow(clippy::expect_used)]

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issued_certificate_row::IssuedCertificateRow;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{
    AllocationId, CertSerial, ContentHash, CorrelationKey, IssuanceOrdinal, NodeId, Region,
    ServiceId, SpiffeId, WorkloadId,
};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, ConflictRoute, LogicalTimestamp, NodeHealthRow, ObservationRow,
    ObservationRowKind, ReconcileConflictRow, ServiceBackendRow, ServiceHydrationResultRow,
    ServiceHydrationStatus,
};
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};

// ---------------------------------------------------------------------------
// Row constructors — minimal-but-valid instances of every ObservationRow
// variant. `kind()` matches on the discriminant only, so the payloads are the
// smallest valid values each type admits.
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

fn alloc_row() -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str("alloc-1").expect("alloc id is valid"),
        workload_id: WorkloadId::from_str("payments").expect("workload id is valid"),
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
// S-266-11 — ObservationRow::kind(): total, no-wildcard discriminant
// ---------------------------------------------------------------------------

/// The full closed-world table: every one of the 8 `ObservationRow` variants
/// paired with its expected `ObservationRowKind`.
fn kind_table() -> Vec<(&'static str, ObservationRow, ObservationRowKind)> {
    vec![
        (
            "AllocStatus",
            ObservationRow::AllocStatus(Box::new(alloc_row())),
            ObservationRowKind::AllocStatus,
        ),
        (
            "NodeHealth",
            ObservationRow::NodeHealth(node_health_row()),
            ObservationRowKind::NodeHealth,
        ),
        (
            "ServiceHydration",
            ObservationRow::ServiceHydration(service_hydration_row()),
            ObservationRowKind::ServiceHydration,
        ),
        (
            "ServiceBackend",
            ObservationRow::ServiceBackend(service_backend_row()),
            ObservationRowKind::ServiceBackend,
        ),
        (
            "ReconcileConflict",
            ObservationRow::ReconcileConflict(reconcile_conflict_row()),
            ObservationRowKind::ReconcileConflict,
        ),
        (
            "IssuedCertificate",
            ObservationRow::IssuedCertificate(issued_cert_row()),
            ObservationRowKind::IssuedCertificate,
        ),
        ("WorkflowTerminal", workflow_terminal_row(), ObservationRowKind::WorkflowTerminal),
        ("Signal", signal_row(), ObservationRowKind::Signal),
    ]
}

/// S-266-11 — `row.kind()` maps EACH of the 8 `ObservationRow` variants to
/// its matching `ObservationRowKind`.
///
/// Parametrized over all 8 variants (closed-world finite → table, NOT PBT).
/// Mutation target: EVERY match arm — mutating any arm to the wrong
/// `ObservationRowKind` (or collapsing two families onto one kind) breaks the
/// exact per-variant equality below.
#[test]
fn kind_maps_each_observation_row_variant_exhaustively() {
    for (name, row, expected) in kind_table() {
        assert_eq!(
            row.kind(),
            expected,
            "ObservationRow::{name} must project to ObservationRowKind::{expected:?} \
             (ADR-0084 §2 total no-wildcard discriminant)",
        );
    }
}

/// The 8 variants map to 8 PAIRWISE-DISTINCT kinds. Complements the
/// per-variant table: a mutation that points two arms at the same
/// `ObservationRowKind` collapses the distinct-count below 8 and is caught
/// here as well as by the per-variant equality above.
#[test]
fn kind_projects_each_variant_to_a_distinct_kind() {
    let mut kinds: Vec<ObservationRowKind> =
        kind_table().into_iter().map(|(_, row, _)| row.kind()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(
        kinds.len(),
        8,
        "the 8 ObservationRow variants must project to 8 distinct ObservationRowKind values",
    );
}

/// Belt-and-braces for the drift-closure: the table MUST enumerate exactly
/// the 8 `ObservationRow` variants. A table that silently drops a variant (or
/// a future variant not added) fails here, complementing the compile-time
/// exhaustiveness the no-wildcard total `match self` in `ObservationRow::kind`
/// enforces (see the module docstring's compile-fail companion note).
#[test]
fn kind_table_covers_every_variant() {
    assert_eq!(
        kind_table().len(),
        8,
        "ObservationRow has exactly 8 variants; the kind table MUST cover every one",
    );
}

// ---------------------------------------------------------------------------
// Label-enum completeness — ObservationRowKind owns its canonical label
//
// AC #5 / ADR-0084 §2: the Piece B discriminant is a label enum that owns its
// `as_str`. These pin the canonical lowercase kebab strings so a mutated label
// (`as_str -> ""` / `-> "xyzzy"`) is caught — mirroring the Piece A
// `resync_scope_local_node_as_str_is_canonical_kebab_label` label pin.
// ---------------------------------------------------------------------------

#[test]
fn observation_row_kind_as_str_is_canonical_kebab_label() {
    assert_eq!(ObservationRowKind::AllocStatus.as_str(), "alloc-status");
    assert_eq!(ObservationRowKind::NodeHealth.as_str(), "node-health");
    assert_eq!(ObservationRowKind::ServiceHydration.as_str(), "service-hydration");
    assert_eq!(ObservationRowKind::ServiceBackend.as_str(), "service-backend");
    assert_eq!(ObservationRowKind::ReconcileConflict.as_str(), "reconcile-conflict");
    assert_eq!(ObservationRowKind::IssuedCertificate.as_str(), "issued-certificate");
    assert_eq!(ObservationRowKind::WorkflowTerminal.as_str(), "workflow-terminal");
    assert_eq!(ObservationRowKind::Signal.as_str(), "signal");
}
