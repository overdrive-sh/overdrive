//! Step 02-03 — Shared REST request/response types carry utoipa
//! `ToSchema` derives and finalised field sets.
//!
//! Step 01-01 (`redesign-drop-commit-index`, ADR-0020): the
//! `commit_index` field is dropped from `SubmitJobResponse`,
//! `JobDescription`, and `ClusterStatus`; `SubmitJobResponse` now
//! carries `{job_id, spec_digest, outcome}` and `JobDescription`
//! carries `{spec, spec_digest}`. A new `IdempotencyOutcome` enum at
//! the api layer distinguishes `inserted` from `unchanged`. The 409
//! Conflict path remains an HTTP-status concern — never an enum value.
//!
//! These tests pin the wire contract exposed by
//! `overdrive_control_plane::api`. The CLI (Slice 5, ADR-0014) will
//! import these same types, so field names and shapes here ARE the
//! REST contract.
//!
//! Every request/response type:
//!   1. Round-trips through `serde_json::to_string` -> `from_str`.
//!   2. Carries `utoipa::ToSchema` so the `cargo xtask openapi-gen`
//!      checked-in artifact (ADR-0009) stays derivable.
//!   3. Matches the field set pinned by the step AC verbatim —
//!      renaming breaks the contract surface.

use overdrive_control_plane::api::{
    AllocStatusResponse, AllocStatusRowBody, BrokerCountersBody, ClusterStatus, ErrorBody,
    IdempotencyOutcome, JobDescription, NodeList, NodeRowBody, SubmitJobRequest, SubmitJobResponse,
};
use overdrive_core::aggregate::{DriverInput, ExecInput, JobSpecInput, ResourcesInput};
use utoipa::ToSchema;

fn sample_job_spec() -> JobSpecInput {
    JobSpecInput {
        id: "payments".to_string(),
        replicas: 3,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    }
}

#[test]
fn submit_job_request_round_trips_through_serde_json() {
    let original = SubmitJobRequest { spec: sample_job_spec() };
    let wire = serde_json::to_string(&original).expect("serialise SubmitJobRequest");
    let round_tripped: SubmitJobRequest =
        serde_json::from_str(&wire).expect("deserialise SubmitJobRequest");
    assert_eq!(round_tripped.spec, original.spec);
}

/// RED-phase pin (Step 01-01): `SubmitJobResponse` MUST carry
/// `{job_id, spec_digest, outcome}` per ADR-0020. The `outcome`
/// field MUST round-trip as `IdempotencyOutcome::Inserted` on the
/// fresh-insert path. This test compiles only against the new
/// post-ADR-0020 wire shape — by name, the
/// `submit_response_carries_spec_digest_and_outcome_inserted`
/// scenario.
#[test]
fn submit_response_carries_spec_digest_and_outcome_inserted() {
    let digest = "deadbeef".repeat(8);
    let original = SubmitJobResponse {
        job_id: "payments".to_string(),
        spec_digest: digest.clone(),
        outcome: IdempotencyOutcome::Inserted,
    };
    let wire = serde_json::to_string(&original).expect("serialise SubmitJobResponse");
    let round_tripped: SubmitJobResponse =
        serde_json::from_str(&wire).expect("deserialise SubmitJobResponse");
    assert_eq!(round_tripped.job_id, "payments");
    assert_eq!(round_tripped.spec_digest, digest);
    assert_eq!(round_tripped.outcome, IdempotencyOutcome::Inserted);
}

/// RED-phase pin (Step 01-01): the `IdempotencyOutcome::Unchanged`
/// arm renders to `"unchanged"` on the wire and round-trips back.
/// This is the byte-equal idempotent re-submission path —
/// distinct from the 409 Conflict path which is HTTP-status, not an
/// enum value (ADR-0015 §4 amendment via ADR-0020).
#[test]
fn submit_response_carries_outcome_unchanged_on_idempotent_resubmit() {
    let digest = "abcdef01".repeat(8);
    let original = SubmitJobResponse {
        job_id: "payments".to_string(),
        spec_digest: digest.clone(),
        outcome: IdempotencyOutcome::Unchanged,
    };
    let wire = serde_json::to_string(&original).expect("serialise SubmitJobResponse");
    // The lowercase JSON form is the wire contract — pinned via
    // `#[serde(rename_all = "lowercase")]` on `IdempotencyOutcome`.
    assert!(
        wire.contains(r#""outcome":"unchanged""#),
        "outcome must serialise lowercase; got: {wire}"
    );
    let round_tripped: SubmitJobResponse =
        serde_json::from_str(&wire).expect("deserialise SubmitJobResponse");
    assert_eq!(round_tripped.outcome, IdempotencyOutcome::Unchanged);
    assert_eq!(round_tripped.spec_digest, digest);
}

#[test]
fn idempotency_outcome_serialises_lowercase_inserted() {
    let wire = serde_json::to_string(&IdempotencyOutcome::Inserted).expect("serialise Inserted");
    assert_eq!(wire, r#""inserted""#);
    let parsed: IdempotencyOutcome = serde_json::from_str(&wire).expect("deserialise Inserted");
    assert_eq!(parsed, IdempotencyOutcome::Inserted);
}

#[test]
fn idempotency_outcome_serialises_lowercase_unchanged() {
    let wire = serde_json::to_string(&IdempotencyOutcome::Unchanged).expect("serialise Unchanged");
    assert_eq!(wire, r#""unchanged""#);
    let parsed: IdempotencyOutcome = serde_json::from_str(&wire).expect("deserialise Unchanged");
    assert_eq!(parsed, IdempotencyOutcome::Unchanged);
}

#[test]
fn job_description_round_trips_with_typed_spec() {
    let original = JobDescription { spec: sample_job_spec(), spec_digest: "deadbeef".repeat(8) };
    let wire = serde_json::to_string(&original).expect("serialise JobDescription");
    let round_tripped: JobDescription =
        serde_json::from_str(&wire).expect("deserialise JobDescription");
    assert_eq!(round_tripped.spec, original.spec);
    assert_eq!(round_tripped.spec_digest.len(), 64);
    assert_eq!(round_tripped.spec_digest, original.spec_digest);
}

#[test]
fn cluster_status_round_trips_through_serde_json() {
    let original = ClusterStatus {
        mode: "single".to_string(),
        region: "eu-west-1".to_string(),
        reconcilers: vec!["job_lifecycle".to_string(), "node_lifecycle".to_string()],
        broker: BrokerCountersBody { queued: 1, cancelled: 2, dispatched: 3 },
    };
    let wire = serde_json::to_string(&original).expect("serialise ClusterStatus");
    let round_tripped: ClusterStatus =
        serde_json::from_str(&wire).expect("deserialise ClusterStatus");
    assert_eq!(round_tripped.mode, "single");
    assert_eq!(round_tripped.region, "eu-west-1");
    assert_eq!(round_tripped.reconcilers, original.reconcilers);
    assert_eq!(round_tripped.broker.queued, 1);
    assert_eq!(round_tripped.broker.cancelled, 2);
    assert_eq!(round_tripped.broker.dispatched, 3);
}

/// RED-phase pin (Step 01-01): `ClusterStatus` MUST NOT carry a
/// `commit_index` field on the wire after ADR-0020. Pure-drop
/// (no rename to `writes_since_boot`); the four-field shape
/// `{mode, region, reconcilers, broker}` is the contract.
#[test]
fn cluster_status_does_not_serialise_commit_index_field() {
    let status = ClusterStatus {
        mode: "single".to_string(),
        region: "local".to_string(),
        reconcilers: Vec::new(),
        broker: BrokerCountersBody::default(),
    };
    let wire = serde_json::to_string(&status).expect("serialise ClusterStatus");
    assert!(
        !wire.contains("commit_index"),
        "ClusterStatus must not surface commit_index post-ADR-0020; got: {wire}"
    );
}

#[test]
fn broker_counters_body_round_trips_through_serde_json() {
    let original = BrokerCountersBody { queued: 100, cancelled: 5, dispatched: 95 };
    let wire = serde_json::to_string(&original).expect("serialise BrokerCountersBody");
    let round_tripped: BrokerCountersBody =
        serde_json::from_str(&wire).expect("deserialise BrokerCountersBody");
    assert_eq!(round_tripped.queued, 100);
    assert_eq!(round_tripped.cancelled, 5);
    assert_eq!(round_tripped.dispatched, 95);
}

#[test]
fn alloc_status_response_round_trips_with_empty_and_populated_rows() {
    // Phase 1 ships the empty-array case — US-03 AC pins this.
    let empty = AllocStatusResponse { rows: Vec::new() };
    let wire = serde_json::to_string(&empty).expect("serialise empty AllocStatusResponse");
    assert_eq!(wire, r#"{"rows":[]}"#);
    let round_tripped: AllocStatusResponse =
        serde_json::from_str(&wire).expect("deserialise empty AllocStatusResponse");
    assert!(round_tripped.rows.is_empty());

    // Step 03-03 populated `AllocStatusRowBody` with the minimal Phase 1
    // shape — alloc_id, job_id, node_id, state. The round-trip still
    // has to work — forward compatibility cuts in both directions.
    let populated = AllocStatusResponse {
        rows: vec![AllocStatusRowBody {
            alloc_id: "alloc-1".to_owned(),
            job_id: "payments".to_owned(),
            node_id: "node-a".to_owned(),
            state: "running".to_owned(),
            reason: None,
        }],
    };
    let wire = serde_json::to_string(&populated).expect("serialise populated AllocStatusResponse");
    let round_tripped: AllocStatusResponse =
        serde_json::from_str(&wire).expect("deserialise populated AllocStatusResponse");
    assert_eq!(round_tripped.rows.len(), 1);
}

#[test]
fn node_list_round_trips_with_empty_and_populated_rows() {
    let empty = NodeList { rows: Vec::new() };
    let wire = serde_json::to_string(&empty).expect("serialise empty NodeList");
    assert_eq!(wire, r#"{"rows":[]}"#);
    let round_tripped: NodeList = serde_json::from_str(&wire).expect("deserialise empty NodeList");
    assert!(round_tripped.rows.is_empty());

    let populated = NodeList {
        rows: vec![NodeRowBody { node_id: "node-a".to_owned(), region: "us-east-1".to_owned() }],
    };
    let wire = serde_json::to_string(&populated).expect("serialise populated NodeList");
    let round_tripped: NodeList =
        serde_json::from_str(&wire).expect("deserialise populated NodeList");
    assert_eq!(round_tripped.rows.len(), 1);
}

#[test]
fn error_body_round_trips_with_field_none_and_field_some() {
    // ADR-0015 pins `{ error, message, field }`. `field: None` must
    // serialise (axum error handlers emit it verbatim per ADR-0015).
    let without_field = ErrorBody {
        error: "validation".to_string(),
        message: "replicas must be non-zero".to_string(),
        field: None,
    };
    let wire = serde_json::to_string(&without_field).expect("serialise ErrorBody");
    let round_tripped: ErrorBody = serde_json::from_str(&wire).expect("deserialise ErrorBody");
    assert_eq!(round_tripped.error, "validation");
    assert_eq!(round_tripped.message, "replicas must be non-zero");
    assert_eq!(round_tripped.field, None);

    let with_field = ErrorBody {
        error: "validation".to_string(),
        message: "replica count must be non-zero".to_string(),
        field: Some("replicas".to_string()),
    };
    let wire = serde_json::to_string(&with_field).expect("serialise ErrorBody with field");
    let round_tripped: ErrorBody =
        serde_json::from_str(&wire).expect("deserialise ErrorBody with field");
    assert_eq!(round_tripped.field.as_deref(), Some("replicas"));
}

/// The `ToSchema` derive is what keeps `cargo xtask openapi-gen`
/// derivable (ADR-0009). Rather than inspecting the generated schema
/// shape (which is utoipa-version-sensitive), we pin that every type
/// in the API module implements `ToSchema` at all — a missing derive
/// fails compilation of this test via the trait-bounded generic helper.
#[test]
fn every_api_type_implements_utoipa_to_schema() {
    fn assert_to_schema<T: ToSchema>() {}

    assert_to_schema::<SubmitJobRequest>();
    assert_to_schema::<SubmitJobResponse>();
    assert_to_schema::<JobDescription>();
    assert_to_schema::<ClusterStatus>();
    assert_to_schema::<BrokerCountersBody>();
    assert_to_schema::<AllocStatusResponse>();
    assert_to_schema::<AllocStatusRowBody>();
    assert_to_schema::<NodeList>();
    assert_to_schema::<NodeRowBody>();
    assert_to_schema::<ErrorBody>();
    assert_to_schema::<IdempotencyOutcome>();
    // Step 04-02 — wire-shape input twins per ADR-0031 §8 / DWD-8.
    // `JobSpecInput` already carried `ToSchema` (Step 02-03); the 3
    // supporting types land here now that they are registered in
    // `OverdriveApi`'s `components(schemas(...))`.
    assert_to_schema::<JobSpecInput>();
    assert_to_schema::<ResourcesInput>();
    assert_to_schema::<ExecInput>();
    assert_to_schema::<DriverInput>();
}
