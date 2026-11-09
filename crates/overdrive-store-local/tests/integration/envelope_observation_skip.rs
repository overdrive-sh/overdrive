//! Observation log + skip self-heal — S-EV-04.1 + S-EV-04.2 + S-EV-04.3.
//!
//! Per ADR-0048 § 3 (observation layer): on envelope decode failure
//! the reader emits `tracing::warn!(name: "observation.envelope.
//! decode_failed", table = ?, key = ?, source = ?)` and skips the
//! row. Valid rows continue to surface. Re-writing the malformed key
//! with a valid envelope recovers reads (the bad bytes are
//! overwritten through the typed write path).
//!
//! The bytes-injection back-door lives in `envelope_helpers.rs` —
//! production code MUST NOT have a raw-bytes write path.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::net::Ipv4Addr;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::ca::issue_failure::{SvidIssueFailure, SvidIssueFailureCause};
use overdrive_core::codec::VersionedEnvelope;
use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{AllocationId, NodeId, Region, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    AllocLifecyclePredecessor, AllocLifecycleUnreadable, AllocState, AllocStatusRow,
    AllocStatusRowEnvelope, LogicalTimestamp, NodeHealthRow, ObservationStore, ServiceBackendRow,
    ServiceHydrationResultRow, ServiceHydrationStatus, TransitionSource,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use super::envelope_helpers::{
    write_raw_bytes_to_alloc_status_table, write_raw_bytes_to_gateway_application_status_table,
    write_raw_bytes_to_node_health_table, write_raw_bytes_to_service_backends_table,
    write_raw_bytes_to_service_hydration_results_table,
    write_raw_bytes_to_workload_svid_issue_status_table,
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

struct V<'a>(&'a mut String);

impl tracing::field::Visit for V<'_> {
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
        event.record(&mut V(&mut buf));
        self.inner.lock().expect("captured events mutex").push(buf);
    }
}

fn make_alloc_row(alloc_id_str: &str) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(alloc_id_str).expect("valid alloc id"),
        workload_id: WorkloadId::new("svc-payments").expect("valid workload id"),
        node_id: NodeId::new("node-001").expect("valid node id"),
        state: AllocState::Running,
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("node-001").expect("valid writer node id"),
        },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: WorkloadKind::Service,
        listeners: Vec::new(),
        // GAP-1 subsidiary: Running state carries fixed wall-clock.
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        // Host-netns fixture — no canonical workload address
        // (AllocStatusRowV2 additive field, GH #241).
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_alloc_status_row_is_logged_and_skipped_but_valid_row_surfaces() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");

    // K1: valid row through the typed write path.
    let k1_row = make_alloc_row("alloc-01");
    {
        let store = LocalObservationStore::open(&redb_path).expect("open #1");
        store
            .write_alloc_lifecycle(
                k1_row.clone(),
                overdrive_core::traits::observation_store::TransitionSource::Reconciler,
            )
            .await
            .expect("write K1");
    } // store dropped; redb lock released

    // K2: 16 bytes of 0xFF — does not decode through the envelope.
    let k2_alloc = AllocationId::new("alloc-malformed-02").expect("valid alloc id");
    write_raw_bytes_to_alloc_status_table(&redb_path, &k2_alloc, &[0xFF; 16]);

    let _guard = set_default(subscriber);

    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store.alloc_status_rows().await.expect("read alloc rows");

    assert_eq!(rows.len(), 1, "exactly one valid row must survive; got {rows:?}");
    assert_eq!(rows[0].alloc_id, k1_row.alloc_id);

    let entries = captured.entries();
    let decode_events: Vec<&String> =
        entries.iter().filter(|e| e.contains("observation.envelope.decode_failed")).collect();
    assert_eq!(
        decode_events.len(),
        1,
        "exactly one decode_failed event expected; got {decode_events:?}"
    );
    assert!(
        decode_events[0].contains("observation_alloc_status"),
        "event must name the table 'observation_alloc_status'; got {:?}",
        decode_events[0]
    );

    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert!(
        refused.is_empty(),
        "observation layer must NOT emit health.startup.refused; got {refused:?}"
    );
}

/// Outcome anchor: DISCUSS Elevator Pitch
/// CONTRACT_SHAPE: bounded-change.
#[allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "the two unreadable predecessor partitions share one store-reopen and diagnostic capture journey"
)]
#[tokio::test(flavor = "current_thread")]
async fn rewriting_malformed_key_with_valid_envelope_recovers_reads() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");

    // K1: valid row.
    let k1_row = make_alloc_row("alloc-01");
    {
        let store = LocalObservationStore::open(&redb_path).expect("open #1");
        store
            .write_alloc_lifecycle(
                k1_row.clone(),
                overdrive_core::traits::observation_store::TransitionSource::Reconciler,
            )
            .await
            .expect("write K1");
    }

    // K2: malformed.
    let k2_alloc = AllocationId::new("alloc-malformed-02").expect("valid alloc id");
    write_raw_bytes_to_alloc_status_table(&redb_path, &k2_alloc, &[0xFF; 16]);

    // K3: structurally valid current envelope with an unknown future
    // discriminant. The offset is the envelope's own schema-evolution
    // contract rather than a second test-owned layout constant.
    let mut k3_row = k1_row.clone();
    k3_row.alloc_id = AllocationId::new("alloc-unknown-03").expect("valid alloc id");
    let envelope = AllocStatusRowEnvelope::latest(k3_row.clone());
    let mut unknown_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
        .expect("archive valid current envelope")
        .to_vec();
    let discriminant_offset = AllocStatusRowEnvelope::discriminant_offset_from_end()
        .expect("alloc current envelope pins its discriminant offset");
    let discriminant_idx = unknown_bytes.len() - discriminant_offset;
    unknown_bytes[discriminant_idx] = 99;
    write_raw_bytes_to_alloc_status_table(&redb_path, &k3_row.alloc_id, &unknown_bytes);

    let _guard = set_default(subscriber);

    // First read: 1 row + one decode event for each unreadable key.
    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store.alloc_status_rows().await.expect("read #1");
    assert_eq!(rows.len(), 1);

    let first_pass_decodes = captured
        .entries()
        .iter()
        .filter(|e| e.contains("observation.envelope.decode_failed"))
        .count();
    assert_eq!(first_pass_decodes, 2, "first read must emit one decode event per bad key");

    // Re-write K2 with a valid envelope through the typed write
    // path — overwrites the garbage bytes.
    let mut k2_row = k1_row.clone();
    k2_row.alloc_id = k2_alloc.clone();
    let occurrence = store
        .write_alloc_lifecycle(k2_row.clone(), TransitionSource::Reconciler)
        .await
        .expect("re-write K2 valid")
        .expect("unreadable prior is unconditionally self-healed");
    assert_eq!(
        occurrence.from,
        AllocLifecyclePredecessor::Unreadable(AllocLifecycleUnreadable::Malformed),
        "the repair occurrence truthfully distinguishes malformed bytes from an absent key"
    );
    assert_eq!(occurrence.to, k2_row.state);
    assert_eq!(occurrence.source, TransitionSource::Reconciler);
    assert_eq!(
        store.alloc_lifecycle_occurrences(&k2_alloc).await.expect("read repair occurrence"),
        vec![occurrence],
        "current replacement and its unreadable-predecessor occurrence commit together"
    );

    let unknown_occurrence = store
        .write_alloc_lifecycle(
            k3_row.clone(),
            TransitionSource::Driver(overdrive_core::traits::driver::DriverType::Vm),
        )
        .await
        .expect("re-write K3 valid")
        .expect("unknown future prior is unconditionally self-healed");
    assert_eq!(
        unknown_occurrence.from,
        AllocLifecyclePredecessor::Unreadable(AllocLifecycleUnreadable::UnknownVersion {
            observed: 99,
            supported_max: 0,
        })
    );
    assert_eq!(unknown_occurrence.to, k3_row.state);
    assert_eq!(
        unknown_occurrence.source,
        TransitionSource::Driver(overdrive_core::traits::driver::DriverType::Vm)
    );
    assert_eq!(
        store
            .alloc_lifecycle_occurrences(&k3_row.alloc_id)
            .await
            .expect("read unknown-version repair occurrence"),
        vec![unknown_occurrence]
    );

    assert_eq!(
        store
            .write_alloc_lifecycle(k2_row.clone(), TransitionSource::Reconciler)
            .await
            .expect("replay repaired current"),
        None,
        "an exact replay after self-heal is an ordinary LWW no-op"
    );

    // Second read: all 3 rows, no additional decode event.
    let rows2 = store.alloc_status_rows().await.expect("read #2");
    assert_eq!(rows2.len(), 3, "all rows must surface after recovery; got {rows2:?}");

    let second_pass_decodes = captured
        .entries()
        .iter()
        .filter(|e| e.contains("observation.envelope.decode_failed"))
        .count();
    assert_eq!(
        second_pass_decodes, 2,
        "no additional decode event expected on the recovery read; total {second_pass_decodes}"
    );
}

// ---------------------------------------------------------------------
// S-EV-04.3 — NodeHealthRow malformed-row skip + valid-row survives.
// Mirrors the AllocStatusRow sub-tests above against the node_health
// table. Per ADR-0048 § 3 observation policy is log+skip — identical
// shape, distinct table = "observation_node_health" tag in the warn
// event.
// ---------------------------------------------------------------------

fn make_node_health_row(node_id_str: &str, counter: u64) -> NodeHealthRow {
    let node_id = NodeId::new(node_id_str).expect("valid node id");
    NodeHealthRow {
        node_id: node_id.clone(),
        region: Region::new("us-east-1").expect("valid region"),
        last_heartbeat: LogicalTimestamp { counter, writer: node_id },
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_node_health_row_is_logged_and_skipped_but_valid_row_surfaces() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");

    // K1: valid row through the typed write path.
    let k1_row = make_node_health_row("node-alpha", 1);
    {
        let store = LocalObservationStore::open(&redb_path).expect("open #1");
        store
            .write(overdrive_core::traits::observation_store::ObservationWrite::NodeHealth(
                k1_row.clone(),
            ))
            .await
            .expect("write K1");
    } // store dropped; redb lock released

    // K2: 16 bytes of 0xFF — does not decode through the envelope.
    let k2_node = NodeId::new("node-malformed-02").expect("valid node id");
    write_raw_bytes_to_node_health_table(&redb_path, &k2_node, &[0xFF; 16]);

    let _guard = set_default(subscriber);

    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store.node_health_rows().await.expect("read node_health rows");

    assert_eq!(rows.len(), 1, "exactly one valid row must survive; got {rows:?}");
    assert_eq!(rows[0].node_id, k1_row.node_id);

    let entries = captured.entries();
    let decode_events: Vec<&String> =
        entries.iter().filter(|e| e.contains("observation.envelope.decode_failed")).collect();
    assert_eq!(
        decode_events.len(),
        1,
        "exactly one decode_failed event expected; got {decode_events:?}"
    );
    assert!(
        decode_events[0].contains("observation_node_health"),
        "event must name the table 'observation_node_health'; got {:?}",
        decode_events[0]
    );

    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert!(
        refused.is_empty(),
        "observation layer must NOT emit health.startup.refused; got {refused:?}"
    );
}

// ---------------------------------------------------------------------
// S-EV-04.4 — ServiceHydrationResultRow malformed-row skip + valid-row
// survives. Mirrors the AllocStatusRow / NodeHealthRow sub-tests above
// against the service_hydration_results table. Per ADR-0048 § 3
// observation policy is log+skip — identical shape, distinct table =
// "observation_service_hydration_results" tag in the warn event.
// ---------------------------------------------------------------------

fn make_service_hydration_row(
    service_id_value: u64,
    fingerprint_value: BackendSetFingerprint,
    counter: u64,
) -> ServiceHydrationResultRow {
    let writer = NodeId::new("node-001").expect("valid writer node id");
    ServiceHydrationResultRow {
        service_id: ServiceId::new(service_id_value).expect("valid service id"),
        fingerprint: fingerprint_value,
        status: ServiceHydrationStatus::Completed {
            fingerprint: fingerprint_value,
            applied_at: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
        },
        updated_at: LogicalTimestamp { counter, writer },
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_service_hydration_row_is_logged_and_skipped_but_valid_row_surfaces() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");

    let service_id = ServiceId::new(42).expect("valid service id");

    // K1: valid row through the typed write path.
    let k1_row = make_service_hydration_row(42, 100, 1);
    {
        let store = LocalObservationStore::open(&redb_path).expect("open #1");
        store
            .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceHydration(
                k1_row.clone(),
            ))
            .await
            .expect("write K1");
    } // store dropped; redb lock released

    // K2: 16 bytes of 0xFF — does not decode through the envelope.
    // The composite key matches the table's `(service_id, fingerprint)`
    // 16-byte LE encoding so the range-scan over service_id == 42 picks
    // it up.
    write_raw_bytes_to_service_hydration_results_table(&redb_path, service_id, 999, &[0xFF; 16]);

    let _guard = set_default(subscriber);

    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store
        .service_hydration_results_rows(&service_id)
        .await
        .expect("read service_hydration rows");

    assert_eq!(rows.len(), 1, "exactly one valid row must survive; got {rows:?}");
    assert_eq!(rows[0].service_id, k1_row.service_id);
    assert_eq!(rows[0].fingerprint, k1_row.fingerprint);

    let entries = captured.entries();
    let decode_events: Vec<&String> =
        entries.iter().filter(|e| e.contains("observation.envelope.decode_failed")).collect();
    assert_eq!(
        decode_events.len(),
        1,
        "exactly one decode_failed event expected; got {decode_events:?}"
    );
    assert!(
        decode_events[0].contains("observation_service_hydration_results"),
        "event must name the table 'observation_service_hydration_results'; got {:?}",
        decode_events[0]
    );

    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert!(
        refused.is_empty(),
        "observation layer must NOT emit health.startup.refused; got {refused:?}"
    );
}

// ---------------------------------------------------------------------
// S-EV-04.5 — ServiceBackendRow malformed-row skip + valid-row survives.
// Mirrors the AllocStatusRow / NodeHealthRow / ServiceHydrationResultRow
// sub-tests above against the service_backends table. Per ADR-0048 § 3
// observation policy is log+skip — identical shape, distinct table =
// "observation_service_backends" tag in the warn event.
// ---------------------------------------------------------------------

fn make_service_backend_row(service_id_value: u64, counter: u64) -> ServiceBackendRow {
    let writer = NodeId::new("node-001").expect("valid writer node id");
    let alloc =
        SpiffeId::new("spiffe://overdrive.sh/svc/payments/alloc-1").expect("valid spiffe id");
    ServiceBackendRow {
        service_id: ServiceId::new(service_id_value).expect("valid service id"),
        vip: Ipv4Addr::new(10, 0, 0, 1),
        backends: vec![Backend {
            alloc,
            addr: "10.0.1.1:8080".parse().expect("valid socket addr"),
            weight: 1,
            healthy: true,
        }],
        updated_at: LogicalTimestamp { counter, writer },
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_service_backend_row_is_logged_and_skipped_but_valid_row_surfaces() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");

    let service_id = ServiceId::new(42).expect("valid service id");

    // K1: valid row through the typed write path.
    let k1_row = make_service_backend_row(42, 1);
    {
        let store = LocalObservationStore::open(&redb_path).expect("open #1");
        store
            .write(overdrive_core::traits::observation_store::ObservationWrite::ServiceBackend(
                k1_row.clone(),
            ))
            .await
            .expect("write K1");
    } // store dropped; redb lock released

    // K2: 16 bytes of 0xFF on a DIFFERENT service_id — does not decode
    // through the envelope. We use a separate service_id so the scan
    // for service_id=42 surfaces K1, while a scan for service_id=999
    // exercises the decode_failed path.
    let malformed_service_id = ServiceId::new(999).expect("valid service id");
    write_raw_bytes_to_service_backends_table(&redb_path, malformed_service_id, &[0xFF; 16]);

    let _guard = set_default(subscriber);

    let store = LocalObservationStore::open(&redb_path).expect("open #2");

    // Read for the malformed key — exercises the decode_failed path.
    let rows_malformed = store
        .service_backends_rows(&malformed_service_id)
        .await
        .expect("read service_backends rows (malformed key)");
    assert!(rows_malformed.is_empty(), "malformed row must be skipped; got {rows_malformed:?}");

    // Read for the valid key — exercises the survivor path.
    let rows_valid = store
        .service_backends_rows(&service_id)
        .await
        .expect("read service_backends rows (valid key)");

    assert_eq!(rows_valid.len(), 1, "exactly one valid row must survive; got {rows_valid:?}");
    assert_eq!(rows_valid[0].service_id, k1_row.service_id);
    assert_eq!(rows_valid[0].vip, k1_row.vip);

    let entries = captured.entries();
    let decode_events: Vec<&String> =
        entries.iter().filter(|e| e.contains("observation.envelope.decode_failed")).collect();
    assert_eq!(
        decode_events.len(),
        1,
        "exactly one decode_failed event expected; got {decode_events:?}"
    );
    assert!(
        decode_events[0].contains("observation_service_backends"),
        "event must name the table 'observation_service_backends'; got {:?}",
        decode_events[0]
    );

    let refused: Vec<&String> =
        entries.iter().filter(|e| e.contains("health.startup.refused")).collect();
    assert!(
        refused.is_empty(),
        "observation layer must NOT emit health.startup.refused; got {refused:?}"
    );
}

// ---------------------------------------------------------------------
// SVID issuance failure occurrence (O-SIF-G3, O-SIF-W2): the two status rows
// follow the observation log-and-skip policy. An undecodable stored value reads
// as absent through the point read with exactly one `observation.row.skipped`
// warning whose separate `table` and `key` fields carry the redb table name and the row id; the workload record operation then restarts
// at one. The gateway owner's base-0 restart is the owner reading this `None`
// (its composition has no byte layer; see gaps).
// ---------------------------------------------------------------------

/// Value of the structured field `name` in a captured event line, with the
/// surrounding quotes a `Debug`-rendered string carries removed. Fields render
/// as ` name=value` tokens after the event name and target (see `V`), and table
/// names and row ids contain no whitespace, so a whole-token match is exact.
fn field_value(entry: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    entry
        .split_whitespace()
        .find_map(|token| token.strip_prefix(prefix.as_str()))
        .map(|value| value.trim_matches('"').to_owned())
}

/// Count of `observation.row.skipped` events whose SEPARATE `table` field is
/// exactly `table` and whose SEPARATE `key` field is exactly `key` (R-56: the
/// shape of the `observation.envelope.decode_failed` event, not a substring of
/// a message).
fn skipped_warnings_naming(captured: &CapturedEvents, table: &str, key: &str) -> usize {
    captured
        .entries()
        .iter()
        .filter(|entry| {
            entry.starts_with("observation.row.skipped | ")
                && field_value(entry, "table").as_deref() == Some(table)
                && field_value(entry, "key").as_deref() == Some(key)
        })
        .count()
}

/// O-SIF-G3 (stored-row half): garbage under the gateway status row's table and
/// key reads as no row, with one skipped-row warning carrying the exact table name
/// in its `table` field and the row id in its `key` field, and no startup
/// refusal.
///
/// CONTRACT_SHAPE: bounded-change.
#[allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required scenario metadata")]
#[tokio::test(flavor = "current_thread")]
async fn an_undecodable_gateway_status_row_reads_as_absent_with_one_skipped_warning() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());
    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");
    let node = NodeId::new("gateway-node").expect("valid node id");
    {
        // Create the file and its tables through the typed open.
        let _store = LocalObservationStore::open(&redb_path).expect("open #1");
    }
    write_raw_bytes_to_gateway_application_status_table(&redb_path, &node, &[0xFF; 16]);

    let _guard = set_default(subscriber);
    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let row = store.gateway_application_status_row(&node).await.expect("point read");

    assert_eq!(row, None, "an undecodable stored row reads as absent");
    assert_eq!(
        skipped_warnings_naming(
            &captured,
            "observation_gateway_application_status",
            &node.to_string()
        ),
        1,
        "exactly one skipped-row warning with table and key fields; got {:?}",
        captured.entries(),
    );
    let refused: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|entry| entry.contains("health.startup.refused"))
        .collect();
    assert!(refused.is_empty(), "the observation layer never refuses to start: {refused:?}");
}

/// O-SIF-G3, O-SIF-W2 (stored-row half): garbage under the workload failure
/// row's table and key reads as no row with one skipped-row warning; the record
/// operation then treats it as absent (one more warning) and restarts at
/// `issue_failures = 1`, logical counter 1.
///
/// CONTRACT_SHAPE: bounded-change.
#[allow(clippy::doc_markdown, reason = "CONTRACT_SHAPE is required scenario metadata")]
#[tokio::test(flavor = "current_thread")]
async fn an_undecodable_workload_issue_status_row_reads_as_absent_and_the_record_restarts_at_one() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());
    let tmp = TempDir::new().expect("tempdir");
    let redb_path = tmp.path().join("observation.redb");
    let node = NodeId::new("node-001").expect("valid node id");
    let alloc = AllocationId::new("alloc-garbled-0").expect("valid alloc id");
    {
        let _store = LocalObservationStore::open(&redb_path).expect("open #1");
    }
    write_raw_bytes_to_workload_svid_issue_status_table(&redb_path, &alloc, &[0xFF; 16]);

    let _guard = set_default(subscriber);
    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let table = "observation_workload_svid_issue_status";

    assert_eq!(store.workload_svid_issue_status_row(&alloc).await.expect("point read"), None);
    assert_eq!(
        skipped_warnings_naming(&captured, table, &alloc.to_string()),
        1,
        "exactly one skipped-row warning with table and key fields; got {:?}",
        captured.entries(),
    );

    let failure = SvidIssueFailure {
        cause: SvidIssueFailureCause::Ca,
        at: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
    };
    store
        .record_workload_svid_issue_failure(&alloc, &node, failure)
        .await
        .expect("record over an undecodable prior row");
    assert_eq!(
        skipped_warnings_naming(&captured, table, &alloc.to_string()),
        2,
        "the record operation logs the same warning once for the undecodable prior row",
    );
    let row = store
        .workload_svid_issue_status_row(&alloc)
        .await
        .expect("point read")
        .expect("the record installed a row");
    assert_eq!(row.issue_failures, 1, "the count restarts at one");
    assert_eq!(row.updated_at.counter, 1, "the logical counter restarts at one");
    assert_eq!(row.last_issue_failure, failure);
}
