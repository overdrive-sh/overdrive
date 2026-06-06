//! Slice 01 / US-WP-2 AC3 — the journal records step inputs/results,
//! not a derived deadline cache.
//!
//! Scenario S-WP-01-05. O6. Per `development.md` "Persist inputs, not
//! derived state": the recorded `LoadedEntry` carries the step's
//! inputs/result digest and no derived-deadline / "remaining" field —
//! and, post the command/notification split (ADR-0063 §2 / ADR-0064 §3,
//! D5), no in-entry `step` either (position is structural, not a
//! persisted cache of "my own position"). ADR-0063 §2
//! (`RunResult { name, result_digest, result_bytes }`,
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
use overdrive_core::workflow::{SignalKey, SignalValue, WorkflowStatus};

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};
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
    let started = LoadedEntry::Command(JournalCommand::Started { spec_digest, input_digest });

    // The `ctx.run` step result is recorded as its CBOR bytes + a RESULT
    // DIGEST (inputs to replay-equivalence) — not as a derived
    // "next_attempt_at" / "remaining wait" field, and (D5) not keyed by a
    // persisted `step`: identity is the command's POSITION in the run.
    let result_bytes = b"provision-write-response".to_vec();
    let result_digest = ContentHash::of(&result_bytes);
    let run_result = LoadedEntry::Command(JournalCommand::RunResult {
        name: "provision-write".to_string(),
        result_digest,
        result_bytes: result_bytes.clone(),
    });

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

    // Observable outcome 2 — the recorded commands carry INPUT/RESULT
    // digests, never a derived deadline/remaining cache AND never an
    // in-entry `step` (D5 — the variants have no such field by
    // construction; identity is positional). We assert positively on the
    // digests we recorded.
    match &loaded[0] {
        LoadedEntry::Command(JournalCommand::Started {
            spec_digest: got_spec,
            input_digest: got_input,
        }) => {
            assert_eq!(*got_spec, spec_digest, "Started records the spec_digest input");
            assert_eq!(*got_input, input_digest, "Started records the input_digest input");
        }
        other => panic!("first entry must be Started, got {other:?}"),
    }
    match &loaded[1] {
        LoadedEntry::Command(JournalCommand::RunResult {
            name,
            result_digest: got_digest,
            result_bytes: got_bytes,
        }) => {
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

/// D2/D3 dumb-store contract — the `JournalStore` is a dumb ordered log
/// over `LoadedEntry`: commands and notifications **interleave** in one
/// ordered run, the store never classifies, append order == load order,
/// and the storage append-position (which `next_step` counts) advances
/// for BOTH classes — a notification is counted exactly like a command.
///
/// Port-to-port: drives the `JournalStore` driving port (`append` /
/// `load_journal`) of the `SimJournalStore` adapter. The observable
/// outcome is the flat ordered `Vec<LoadedEntry>` returned by
/// `load_journal` — byte-equal to what was appended, with the interleaved
/// `SignalSeen` notification preserved in its append position (not
/// partitioned, not dropped, not reordered ahead of the commands). This
/// pins the contract the cursor (step 01-03) relies on: the partition is
/// the cursor's job; the store must hand it the verbatim interleave.
///
/// This is a characterization test — the store already behaves this way
/// (the plumbing shipped in step 01-01); the test pins the dumb-store
/// contract going forward so a future HA adapter (#205) cannot quietly
/// start classifying / partitioning in the store layer.
#[tokio::test]
async fn loaded_entry_run_round_trips_with_interleaved_command_and_notification() {
    let store = SimJournalStore::new();
    let workflow_id = WorkflowId::new("wf-interleave-0001").expect("valid workflow id");

    // A run that INTERLEAVES a Command and a Notification: the
    // `SignalAwaited` command (the workflow blocked on a key) sits BETWEEN
    // a `Started` command and the satisfying `SignalSeen` notification,
    // and a closing `Terminal` command follows. The store must preserve
    // every entry in append position, never hoisting the notification out
    // of the ordered run.
    let signal_key = SignalKey::new("provision-ready").expect("valid signal key");

    let started = LoadedEntry::Command(JournalCommand::Started {
        spec_digest: ContentHash::of(ProvisionRecord::WORKFLOW_NAME.as_bytes()),
        input_digest: ContentHash::of(ProvisionRecord::PAYLOAD),
    });
    let awaited =
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: signal_key.clone() });
    let seen = LoadedEntry::Notification(JournalNotification::SignalSeen {
        signal_key: signal_key.clone(),
        value_digest: ContentHash::of(b"ready"),
        value: SignalValue::new("ready"),
    });
    let terminal = LoadedEntry::Command(JournalCommand::Terminal {
        status: WorkflowStatus::Completed { output: Vec::new() },
    });

    // The run as appended — a Command, a Command, a NOTIFICATION, a Command.
    let appended = vec![started, awaited, seen, terminal];
    for entry in &appended {
        store.append(&workflow_id, entry).await.expect("append interleaved entry");
    }

    // Observable outcome 1 — the flat ordered run round-trips byte-equal,
    // interleave preserved. The store did NOT partition the notification
    // out of the command sequence (that is the cursor's job, D2).
    let loaded = store.load_journal(&workflow_id).await.expect("load journal");
    assert_eq!(
        loaded, appended,
        "the dumb store returns the interleaved command/notification run \
         byte-equal in append order — it never partitions",
    );

    // Observable outcome 2 — the storage append-position counted BOTH
    // classes. Four entries were appended (3 commands + 1 notification);
    // load returns four. The notification occupies an append-position
    // exactly like a command — `next_step` (count-all over the run) would
    // have advanced for it. If the store classified, the notification
    // would be absent here and the length would be 3.
    assert_eq!(
        loaded.len(),
        4,
        "append-position / next_step counts every entry — the notification \
         is counted, not skipped (count-all parity, D3)",
    );

    // The interleaved notification sits at its append position (index 2),
    // BETWEEN the SignalAwaited command and the Terminal command — proving
    // the store preserved ordering across the class boundary rather than
    // grouping notifications separately.
    assert!(
        matches!(loaded[2], LoadedEntry::Notification(JournalNotification::SignalSeen { .. })),
        "the notification stays at its interleaved append position (index 2), got {:?}",
        loaded[2],
    );
    assert!(
        matches!(loaded[1], LoadedEntry::Command(JournalCommand::SignalAwaited { .. })),
        "the SignalAwaited command precedes the notification, got {:?}",
        loaded[1],
    );
    assert!(
        matches!(loaded[3], LoadedEntry::Command(JournalCommand::Terminal { .. })),
        "the Terminal command follows the interleaved notification, got {:?}",
        loaded[3],
    );
}
