//! Slice 01 / US-WP-2 AC3 — the journal records step inputs/results,
//! not a derived deadline cache.
//!
//! Scenario S-WP-01-05. O6. Per `development.md` "Persist inputs, not
//! derived state": the recorded `JournalEntry` carries the step's
//! inputs/result digest and no derived-deadline / "remaining" field.
//! ADR-0063 §2 (`RunResult { step, name, result_digest, result_bytes }`,
//! `Started { spec_digest, input_digest }`).
//!
//! Port-to-port: the test drives the `JournalStore` driving port
//! (`append` / `load_journal`) of the `SimJournalStore` adapter, using
//! the shared `ProvisionRecord` fixture (promoted to
//! `overdrive-core::testing::workflow` in this step) to derive the
//! `Started { spec_digest }` input. The observable outcome is the
//! ordered run returned by `load_journal` — its variants carry
//! input/result digests and expose NO derived-deadline/remaining slot.

use overdrive_core::id::ContentHash;
use overdrive_core::testing::workflow::ProvisionRecord;

use overdrive_control_plane::journal::{JournalEntry, JournalStore, WorkflowId};
use overdrive_sim::adapters::journal::SimJournalStore;

/// Build the `Started` entry's `spec_digest` from the fixture's spec —
/// the INPUT the journal records, not a derived cache. Mirrors what the
/// engine will do (ADR-0063 §2): hash the spec's canonical identity.
fn spec_digest_of(spec_name: &str) -> ContentHash {
    ContentHash::of(spec_name.as_bytes())
}

#[tokio::test]
async fn provision_record_journal_entry_records_inputs_not_a_derived_cache() {
    let store = SimJournalStore::new();
    let workflow_id = WorkflowId::new("wf-provision-0001").expect("valid workflow id");

    // The journal's first entry records the workflow's INPUTS: the spec
    // digest + the input digest (ADR-0063 §2 `Started`). Derived from
    // the shared `ProvisionRecord` fixture's spec — no pre-computed
    // deadline/remaining cache is involved.
    let spec_digest = spec_digest_of(ProvisionRecord::WORKFLOW_NAME);
    let input_digest = ContentHash::of(ProvisionRecord::PAYLOAD);
    let started = JournalEntry::Started { spec_digest, input_digest };

    // The `ctx.run` step result is recorded as its CBOR bytes + a RESULT
    // DIGEST (inputs to replay-equivalence), keyed by the await-point step
    // index — not as a derived "next_attempt_at" / "remaining wait" field.
    let result_bytes = b"provision-write-response".to_vec();
    let result_digest = ContentHash::of(&result_bytes);
    let run_result = JournalEntry::RunResult {
        step: 0,
        name: "provision-write".to_string(),
        result_digest,
        result_bytes: result_bytes.clone(),
    };

    store.append(&workflow_id, &started).await.expect("append Started");
    store.append(&workflow_id, &run_result).await.expect("append RunResult");

    // Drive the read port: the ordered run for this instance.
    let loaded = store.load_journal(&workflow_id).await.expect("load journal");

    // Observable outcome 1 — the run round-trips losslessly and in order.
    assert_eq!(
        loaded,
        vec![started.clone(), run_result.clone()],
        "load_journal must return the appended entries byte-equal and in append order",
    );

    // Observable outcome 2 — the recorded entries carry INPUT/RESULT
    // digests, never a derived deadline/remaining cache. The
    // `JournalEntry` enum has no such variant/field by construction; we
    // assert positively on the digests we recorded.
    match &loaded[0] {
        JournalEntry::Started { spec_digest: got_spec, input_digest: got_input } => {
            assert_eq!(*got_spec, spec_digest, "Started records the spec_digest input");
            assert_eq!(*got_input, input_digest, "Started records the input_digest input");
        }
        other => panic!("first entry must be Started, got {other:?}"),
    }
    match &loaded[1] {
        JournalEntry::RunResult {
            step,
            name,
            result_digest: got_digest,
            result_bytes: got_bytes,
        } => {
            assert_eq!(*step, 0, "RunResult records the await-point step index (an input)");
            assert_eq!(name, "provision-write", "RunResult records the ctx.run step name");
            assert_eq!(
                *got_digest, result_digest,
                "RunResult records the result digest, not a derived cache"
            );
            assert_eq!(*got_bytes, result_bytes, "RunResult records the CBOR result bytes");
        }
        other => panic!("second entry must be RunResult, got {other:?}"),
    }
}
