//! Slice 01 / US-WP-2 AC1 (O6) / AC2 (ordering) — `@real-io`: a completed
//! step is recorded in the **real redb** journal before the run suspends,
//! and no libSQL journal table exists.
//!
//! Scenario S-WP-01-04. **K5 (O6).** This is the one journal-persistence
//! scenario that exercises a REAL redb file (the `RedbJournalStore`
//! sharing the reconciler redb file + `Arc<Database>`), per
//! `.claude/rules/testing.md` § "Integration vs unit gating": real
//! filesystem I/O (opening a real redb file) MUST be gated behind the
//! `integration-tests` feature and live under `tests/integration/`. The
//! recorded `RunResult` entry is present in the redb journal when read
//! back through the journal handle (the bytes written are the bytes read
//! — `journal_checkpoint` consistency, journey steps 2↔3), and a
//! grep/dep-graph check confirms no libSQL journal table. ADR-0066 §1/§3.
//!
//! Per Mandate 11, this layer-3 sad path / persistence scenario is
//! example-based (one representative real-redb roundtrip), NOT PBT-
//! generated.

use std::sync::Arc;

use redb::Database;

use overdrive_control_plane::journal::redb::RedbJournalStore;
use overdrive_control_plane::journal::{JournalCommand, JournalStore, LoadedEntry, WorkflowId};
use overdrive_core::id::ContentHash;

/// The shared `ProvisionRecord` fixture's spec name + start-input bytes,
/// inlined here (byte-identical to
/// `overdrive_core::testing::workflow::ProvisionRecord`'s `WORKFLOW_NAME` and
/// the CBOR of its unit `Input`) to keep this integration test within the
/// step's `files_to_modify` scope — pulling the `test-utils`-gated
/// `overdrive_core::testing` module into the integration binary would require
/// a `Cargo.toml` dev-dependency edit outside this step. The digests are
/// derived from these INPUTS exactly as the engine will (ADR-0066 §2).
const PROVISION_RECORD_WORKFLOW_NAME: &str = "provision-record";
/// The opaque CBOR start-input bytes — `ciborium::into_writer(&())` yields a
/// single-byte CBOR `null` (`0xf6`). Post-#217 the `Started { input_digest }`
/// hashes THESE start-input bytes (`spec.input`), NOT the transport STEP
/// payload `b"provision-record"` it coincidentally equalled before the typed
/// `WorkflowStart { name, input }` surface existed (ADR-0065 §5, D5).
const PROVISION_RECORD_START_INPUT: &[u8] = &[0xf6];

/// Build the `Started` entry's `spec_digest` from the fixture's spec —
/// the INPUT the journal records, mirroring what the engine derives
/// (ADR-0066 §2: hash the spec's canonical identity).
fn spec_digest_of(spec_name: &str) -> ContentHash {
    ContentHash::of(spec_name.as_bytes())
}

/// S-WP-01-04 / K5 (O6) / US-WP-2 AC1 + AC2.
///
/// Drive the production `RedbJournalStore` over a REAL redb file (the
/// SAME `Arc<Database>` shape `RedbViewStore` shares — ADR-0066 §1
/// one-file-two-layouts): append a `ProvisionRecord`-derived
/// `RunResult`, read it back byte-equal via `load_journal`, and confirm
/// the persisted run is ordered + survives a close/reopen of the real
/// redb file. The "no libSQL journal table" half of K5/O6 is asserted
/// structurally: the journal module source references no second storage
/// engine — the journal rides the existing redb substrate.
#[tokio::test]
async fn call_result_is_present_in_the_real_redb_journal_and_no_libsql_table_exists() {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let db_path = tmp.path().join("memory.redb");

    let workflow_id = WorkflowId::new("wf-provision-0001").expect("valid workflow id");

    // The journal's first entry records the workflow's INPUTS (ADR-0066
    // §2 `Started`), derived from the shared `ProvisionRecord` fixture.
    let started = LoadedEntry::Command(JournalCommand::Started {
        spec_digest: spec_digest_of(PROVISION_RECORD_WORKFLOW_NAME),
        input_digest: ContentHash::of(PROVISION_RECORD_START_INPUT),
    });
    // The `ctx.run` step result is recorded as its CBOR bytes + a RESULT
    // DIGEST (US-WP-2 AC1 — the RunResult under audit). No in-entry `step`
    // (D5 — identity is positional).
    let result_bytes = b"provision-write-response".to_vec();
    let run_result = LoadedEntry::Command(JournalCommand::RunResult {
        name: "provision-write".to_string(),
        result_digest: ContentHash::of(&result_bytes),
        result_bytes,
    });

    // --- Append through the production adapter on a REAL redb file. ---
    {
        // Share an `Arc<Database>` exactly as the boot composition root
        // will (one redb file backs both ViewStore + JournalStore —
        // ADR-0066 §1). `Database::create` here proves the shared-handle
        // construction path; `begin_read`/`begin_write` both take `&self`
        // so the same `Arc` is safe across both stores.
        let db = Arc::new(Database::create(&db_path).expect("create real redb"));
        let journal = RedbJournalStore::new(Arc::clone(&db));

        // Earned-Trust probe must succeed on a healthy fs and leave no
        // residue (ADR-0066 §4) before the run starts.
        journal.probe().await.expect("probe ok on healthy redb");

        journal.append(&workflow_id, &started).await.expect("append Started durably");
        journal.append(&workflow_id, &run_result).await.expect("append RunResult durably");
        // Drop `journal` + `db` to release the redb file lock and ensure
        // the `Durability::Immediate` commits hit disk before reopen.
    }

    // --- Reopen the SAME redb file and read the run back. ---
    let db = Arc::new(Database::create(&db_path).expect("reopen real redb"));
    let journal = RedbJournalStore::new(db);
    let loaded = journal.load_journal(&workflow_id).await.expect("load_journal from real redb");

    // Observable outcome 1 — the recorded run round-trips byte-equal,
    // across a real close/reopen, in append (== ascending step) order
    // (US-WP-2 AC2). bytes-written == bytes-read.
    assert_eq!(
        loaded,
        vec![started, run_result.clone()],
        "the real redb journal must return the appended run byte-equal and in order"
    );

    // Observable outcome 2 — the RunResult under audit is present with
    // its recorded inputs intact (US-WP-2 AC1, the journey 2↔3 consistency
    // check).
    let found_run_result = loaded
        .iter()
        .find(|e| matches!(e, LoadedEntry::Command(JournalCommand::RunResult { .. })))
        .expect("the RunResult entry must be present in the real redb journal");
    assert_eq!(
        *found_run_result, run_result,
        "the read-back RunResult must equal the recorded one (result_digest + name + bytes)"
    );

    // Observable outcome 3 (K5 / O6) — NO second storage engine: the
    // journal module rides the existing redb substrate, NOT libSQL. Assert
    // structurally over the journal module source — `redb::` is present,
    // no libSQL / rusqlite / sqlite symbol is referenced anywhere in the
    // journal path. This is the "no libSQL journal table" half of K5/O6
    // (US-WP-2 AC1); a per-step dep-graph grep would false-positive
    // because the control-plane crate depends on libSQL for *other*
    // per-primitive memory, so we scope the check to the journal source.
    let journal_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/journal");
    let mut saw_redb = false;
    for entry in std::fs::read_dir(journal_dir).expect("read journal src dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read journal source file");
        let lower = src.to_lowercase();
        assert!(
            !lower.contains("libsql") && !lower.contains("rusqlite") && !lower.contains("sqlite"),
            "K5/O6 violation: the journal source {path:?} references a second storage engine \
             (libSQL/rusqlite/sqlite) — the journal MUST ride the existing redb substrate"
        );
        if lower.contains("redb") {
            saw_redb = true;
        }
    }
    assert!(
        saw_redb,
        "the journal module must use redb (the shared substrate) — no redb reference found in src/journal/"
    );
}
