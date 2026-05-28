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
use overdrive_core::dataplane::fingerprint::BackendSetFingerprint;
use overdrive_core::id::{AllocationId, NodeId, Region, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, NodeHealthRow, ObservationRow, ObservationStore,
    ServiceBackendRow, ServiceHydrationResultRow, ServiceHydrationStatus,
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
    write_raw_bytes_to_alloc_status_table, write_raw_bytes_to_node_health_table,
    write_raw_bytes_to_service_backends_table, write_raw_bytes_to_service_hydration_results_table,
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
        started_at_unix_ms: Some(1_700_000_000_000),
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
        store.write(ObservationRow::AllocStatus(Box::new(k1_row.clone()))).await.expect("write K1");
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
        store.write(ObservationRow::AllocStatus(Box::new(k1_row.clone()))).await.expect("write K1");
    }

    // K2: malformed.
    let k2_alloc = AllocationId::new("alloc-malformed-02").expect("valid alloc id");
    write_raw_bytes_to_alloc_status_table(&redb_path, &k2_alloc, &[0xFF; 16]);

    let _guard = set_default(subscriber);

    // First read: 1 row + 1 decode event.
    let store = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store.alloc_status_rows().await.expect("read #1");
    assert_eq!(rows.len(), 1);

    let first_pass_decodes = captured
        .entries()
        .iter()
        .filter(|e| e.contains("observation.envelope.decode_failed"))
        .count();
    assert_eq!(first_pass_decodes, 1, "first read must emit one decode event");

    // Re-write K2 with a valid envelope through the typed write
    // path — overwrites the garbage bytes.
    let mut k2_row = k1_row.clone();
    k2_row.alloc_id = k2_alloc.clone();
    store
        .write(ObservationRow::AllocStatus(Box::new(k2_row.clone())))
        .await
        .expect("re-write K2 valid");

    // Second read: 2 rows, no additional decode event.
    let rows2 = store.alloc_status_rows().await.expect("read #2");
    assert_eq!(rows2.len(), 2, "both rows must surface after recovery; got {rows2:?}");

    let second_pass_decodes = captured
        .entries()
        .iter()
        .filter(|e| e.contains("observation.envelope.decode_failed"))
        .count();
    assert_eq!(
        second_pass_decodes, 1,
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
        store.write(ObservationRow::NodeHealth(k1_row.clone())).await.expect("write K1");
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
        store.write(ObservationRow::ServiceHydration(k1_row.clone())).await.expect("write K1");
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
        store.write(ObservationRow::ServiceBackend(k1_row.clone())).await.expect("write K1");
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
