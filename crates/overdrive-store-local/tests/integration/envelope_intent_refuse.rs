//! Intent fail-fast on unknown / malformed envelope — S-EV-03.1 + S-EV-03.2.
//!
//! Per ADR-0048 § 3 (intent layer) and § 6 (operator remediation): on
//! envelope decode failure the intent path refuses to start. The
//! driving port (UI-03 amendment) is `Job::from_store_bytes(bytes,
//! redb_path) -> Result<Job, IntentStoreError>`; `LocalIntentStore::
//! open` calls it per `jobs/`-prefixed entry during its recovery
//! walk. The expected error is `IntentStoreError::Envelope {
//! redb_path, source: EnvelopeError::* }` with a structured
//! `health.startup.refused` tracing event emitted BEFORE the `Err`
//! return.
//!
//! Synthesised bytes are written via the back-door in
//! `envelope_helpers.rs` — production code MUST NOT have a raw-bytes
//! write surface for intent.

use std::sync::{Arc, Mutex};

use overdrive_core::aggregate::{DriverInput, ExecInput, IntentKey, ResourcesInput};
use overdrive_core::aggregate::{Job, JobEnvelope, JobSpecInput};
use overdrive_core::codec::{EnvelopeError, VersionedEnvelope};
use overdrive_core::traits::intent_store::IntentStoreError;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use super::envelope_helpers::{
    synthesise_unknown_job_envelope_variant_tag, write_raw_bytes_to_entries_table,
};

#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn entries(&self) -> Vec<String> {
        self.inner.lock().expect("captured events mutex").clone()
    }
}

struct FieldVisitor<'a>(&'a mut String);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl<S> Layer<S> for CapturedEvents
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut buf = String::new();
        buf.push_str(event.metadata().name());
        buf.push_str(" | target=");
        buf.push_str(event.metadata().target());
        event.record(&mut FieldVisitor(&mut buf));
        self.inner.lock().expect("captured events mutex").push(buf);
    }
}

fn sample_job_spec(id: &str) -> JobSpecInput {
    JobSpecInput {
        id: id.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 256 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput {
            command: "/bin/sleep".to_string(),
            args: vec!["3600".to_string()],
        }),
    }
}

#[test]
fn malformed_intent_bytes_cause_refuse_to_start() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("intent.redb");

    // Materialise the `entries` table by opening + dropping a store.
    {
        let _ = LocalIntentStore::open(&redb_path).expect("first open");
    }

    // Inject malformed bytes at a `jobs/<id>` key — the recovery walk
    // in `LocalIntentStore::open` will pick this up. Bytes are
    // deliberately shorter than
    // `JobEnvelope::discriminant_offset_from_end()` (= 64) so the
    // pre-decode probe in `Job::from_store_bytes` returns Ok (the
    // trailing root region isn't reachable) and the rkyv bytecheck
    // layer is the one that classifies the bytes as `Malformed` —
    // this test exercises the "structurally garbage" branch, not
    // the "unknown future variant" branch (which lives in
    // `unknown_future_variant_tag_causes_refuse_to_start`).
    let job_id = "job-malformed-01";
    let job = Job::from_spec(sample_job_spec(job_id)).expect("valid job");
    let key = IntentKey::for_job(&job.id);
    let garbage: &[u8] = b"\xff\xfe\xfd\xfc not rkyv";
    assert!(
        garbage.len() < 64,
        "test precondition: garbage must be shorter than JobEnvelope::discriminant_offset_from_end (= 64) so the probe falls through to rkyv classification",
    );
    write_raw_bytes_to_entries_table(&redb_path, key.as_bytes(), garbage);

    // Install the subscriber AFTER bytes injection so the tracing
    // event from `from_store_bytes` is captured.
    let _guard = set_default(subscriber);

    let err = LocalIntentStore::open(&redb_path)
        .err()
        .expect("LocalIntentStore::open must refuse to start on malformed intent bytes");

    match &err {
        IntentStoreError::Envelope { redb_path: err_path, source } => {
            assert_eq!(err_path, &redb_path, "redb_path field must name the injected file");
            assert!(
                matches!(source, EnvelopeError::Malformed { .. }),
                "expected EnvelopeError::Malformed for garbage bytes; got {source:?}",
            );
        }
        other => panic!("expected IntentStoreError::Envelope; got {other:?}"),
    }

    // Operator-facing remediation per ADR-0048 § 6 — Display form
    // contains the redb path and the literal "delete".
    let display = format!("{err}");
    let path_str = redb_path.display().to_string();
    assert!(
        display.contains(&path_str),
        "Display form must name the redb path '{path_str}'; got {display:?}",
    );
    assert!(
        display.contains("delete"),
        "Display form must contain 'delete' (operator remediation); got {display:?}",
    );

    // The structured event MUST fire before Err is returned.
    let entries = captured.entries();
    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert_eq!(
        refused.len(),
        1,
        "exactly one health.startup.refused event expected; got {refused:?}",
    );
    assert!(
        refused[0].contains(&path_str),
        "event must include redb_path={path_str}; got {:?}",
        refused[0],
    );
}

#[test]
fn unknown_future_variant_tag_causes_refuse_to_start() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("intent.redb");

    {
        let _ = LocalIntentStore::open(&redb_path).expect("first open");
    }

    // Take a valid envelope's archived bytes and corrupt the
    // discriminant byte at the empirically-pinned offset (32 for
    // `JobEnvelope` per
    // `JobEnvelope::discriminant_offset`) to value 99. The
    // pre-decode probe in `Job::from_store_bytes` surfaces this as
    // `EnvelopeError::UnknownVersion` with the observed byte and
    // the envelope's `type_name` — the structured surface ADR-0048
    // § 3 calls for, distinguishing "future binary's V<N+1>" from
    // "bytes don't decode at all".
    let job_id = "job-unknown-future-99";
    let job = Job::from_spec(sample_job_spec(job_id)).expect("valid job");
    let key = IntentKey::for_job(&job.id);
    let valid_archive: rkyv::util::AlignedVec =
        rkyv::to_bytes::<rkyv::rancor::Error>(&JobEnvelope::latest(job))
            .expect("rkyv archive of valid envelope");
    let synthesised = synthesise_unknown_job_envelope_variant_tag(valid_archive.as_ref());
    write_raw_bytes_to_entries_table(&redb_path, key.as_bytes(), &synthesised);

    let _guard = set_default(subscriber);

    let err = LocalIntentStore::open(&redb_path)
        .err()
        .expect("LocalIntentStore::open must refuse to start on unknown-tag bytes");

    match &err {
        IntentStoreError::Envelope { redb_path: err_path, source } => {
            assert_eq!(err_path, &redb_path);
            match source {
                EnvelopeError::UnknownVersion { observed, type_name, supported_max } => {
                    assert_eq!(
                        *observed, 99,
                        "probe must surface the observed discriminant byte verbatim",
                    );
                    assert_eq!(
                        *type_name, "JobEnvelope",
                        "probe must name the envelope whose decode path surfaced the unknown tag",
                    );
                    assert_eq!(
                        *supported_max, 0,
                        "JobEnvelope today supports only V1 (rkyv discriminant 0)",
                    );
                }
                other @ EnvelopeError::Malformed { .. } => panic!(
                    "expected EnvelopeError::UnknownVersion {{ observed: 99, type_name: \"JobEnvelope\", .. }} for synthesised unknown-tag bytes; got {other:?}",
                ),
            }
        }
        other => panic!("expected IntentStoreError::Envelope; got {other:?}"),
    }

    let entries = captured.entries();
    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert_eq!(
        refused.len(),
        1,
        "exactly one health.startup.refused event expected for the synthesised entry; got {refused:?}",
    );
}

#[test]
fn well_formed_intent_bytes_do_not_emit_refused_event() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("intent.redb");

    {
        let _ = LocalIntentStore::open(&redb_path).expect("first open");
    }

    // Write a valid envelope through the typed codec — the recovery
    // walk must observe it on re-open without emitting any
    // `health.startup.refused` event.
    let job = Job::from_spec(sample_job_spec("job-ok-01")).expect("valid job");
    let archived = job.archive_for_store().expect("typed codec archive");
    let key = IntentKey::for_job(&job.id);
    write_raw_bytes_to_entries_table(&redb_path, key.as_bytes(), archived.as_ref());

    let _guard = set_default(subscriber);

    LocalIntentStore::open(&redb_path).expect("recovery walk must accept valid envelope bytes");

    let entries = captured.entries();
    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert!(
        refused.is_empty(),
        "well-formed entries must NOT trigger health.startup.refused; got {refused:?}",
    );
}
