//! Regression: `LocalIntentStore::open` must NOT refuse boot after a
//! workload's `workloads/<id>/generation` sub-key has been written.
//!
//! ADR-0073 added the monotonic `workloads/<id>/generation` sub-key,
//! bumped by `overdrive workload restart` via a `TxnOp::IncrementU64`
//! into the `entries` table (persistent, never deleted). The store's
//! `open()` boot-validation walk originally hand-enumerated its
//! sub-key skip set as `/stop` + `/kind` only, so it never learned to
//! skip `/generation`: on the next boot it decoded the 8-byte BE
//! generation value as a `WorkloadIntent` envelope, failed bytecheck,
//! and refused to start with `health.startup.refused`.
//!
//! The fix centralises "which workload keys carry an aggregate body"
//! in the shared `IntentKey::is_canonical_workload_record` predicate
//! (aggregate/mod.rs), whose generic `/`-exclusion skips `/generation`
//! (and every future `workloads/<id>/<subkey>` sibling) by construction.
//!
//! This test FAILS against the pre-fix code (reopen returns
//! `IntentStoreError::Envelope`) and PASSES after — it is the durable
//! lock on the bug.
//!
//! Port-to-port discipline: the store is exercised entirely through the
//! `IntentStore` trait surface (`put` / `txn`) and the `open()`
//! boundary; no internal redb types are inspected. Strategy C — real
//! redb, `tempfile::TempDir` backing path.

use bytes::Bytes;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, JobSpecInput, JobV2, ResourcesInput, WorkloadIntent,
};
use overdrive_core::id::WorkloadId;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn job_intent(id_str: &str) -> WorkloadIntent {
    let spec = JobSpecInput {
        id: id_str.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 500, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/true".to_string(), args: vec![] }),
    };
    WorkloadIntent::Job(JobV2::from_submit(spec).expect("canonical job spec must validate"))
}

fn workload_id(id_str: &str) -> WorkloadId {
    WorkloadId::new(id_str).expect("canonical id must validate")
}

#[tokio::test]
async fn reopen_after_generation_bump_does_not_refuse_boot() {
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().join("intent.redb");

    {
        // Given a store holding a VALID workload aggregate body AND the
        // ADR-0073 `workloads/<id>/generation` sub-key (an 8-byte BE u64,
        // written the way `overdrive workload restart` writes it).
        let store = LocalIntentStore::open(&path).expect("first open");
        let id = workload_id("payments");
        let intent = job_intent("payments");

        let key_body = IntentKey::for_workload(&id);
        store
            .put(key_body.as_bytes(), intent.archive_for_store().expect("archive").as_ref())
            .await
            .expect("put aggregate body");

        // Bump the generation sub-key through the same atomic
        // read-modify-write primitive the restart handler uses. Absent ⇒
        // 1, persisted as a canonical 8-byte big-endian value.
        let gen_key = IntentKey::for_workload_generation(&id);
        let outcome = store
            .txn(vec![TxnOp::IncrementU64 { key: Bytes::copy_from_slice(gen_key.as_bytes()) }])
            .await
            .expect("generation bump txn");
        assert!(
            matches!(outcome, TxnOutcome::Committed),
            "the generation bump must commit; got {outcome:?}",
        );

        // Drop the store handle so the redb file is closed before reopen.
    }

    // When the control-plane reboots against the SAME on-disk store, its
    // `open()` boot-validation walk sees `workloads/payments` (aggregate
    // body) AND `workloads/payments/generation` (8-byte BE scalar).
    let reopened = LocalIntentStore::open(&path);

    // Then the open SUCCEEDS — the walk decoded the aggregate body and
    // SKIPPED the `/generation` sub-key via
    // `IntentKey::is_canonical_workload_record`. Pre-fix, the 8-byte
    // generation value was routed through `WorkloadIntent::from_store_bytes`,
    // failed bytecheck, and surfaced as `IntentStoreError::Envelope` —
    // refusing boot.
    assert!(
        reopened.is_ok(),
        "reopen after a `/generation` bump must not refuse boot (ADR-0073 sub-key must be \
         skipped by the canonical-record predicate); reopen failed: {:?}",
        reopened.err(),
    );
}
