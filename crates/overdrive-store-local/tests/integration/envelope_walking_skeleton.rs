//! Walking-skeleton integration test for `rkyv-envelope-evolution`.
//!
//! Operator scenario per `docs/feature/rkyv-envelope-evolution/distill/
//! walking-skeleton.md`:
//!
//!   "An operator restarts the control-plane and observes yesterday's
//!    in-flight allocation status without `subtree pointer overran
//!    range` log entries corrupting the convergence tick."
//!
//! The test drives `LocalObservationStore::write_alloc_status` +
//! reopen + `alloc_status_rows()` end-to-end through real redb, asserts
//! the row reads back as the latest envelope shape, and asserts the
//! tracing-event capture contains zero `subtree pointer overran range`
//! or `observation.envelope.decode_failed` entries on the happy path.

use std::sync::{Arc, Mutex};

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// Captures `tracing::Event` records as `(name, message)` so the test
/// can assert on emitted event names from a single thread.
#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn entries(&self) -> Vec<String> {
        self.inner.lock().expect("captured events mutex").clone()
    }
}

/// Minimal field visitor that renders every `Debug` field into the
/// shared `String` buffer. Lives at module scope so the items appear
/// before any statement in `Layer::on_event` (clippy
/// `items_after_statements`).
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
        // Record both the event metadata name and a Debug rendering of
        // its fields — the assertion below matches on substrings of
        // either, so we don't have to commit to one shape upfront.
        let mut buf = String::new();
        buf.push_str(event.metadata().name());
        buf.push_str(" | target=");
        buf.push_str(event.metadata().target());
        event.record(&mut V(&mut buf));
        self.inner.lock().expect("captured events mutex").push(buf);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn operator_restart_observes_yesterday_alloc_status_without_subtree_overrun() {
    let captured = CapturedEvents::default();
    let subscriber = Registry::default().with(captured.clone());

    let tmp = TempDir::new().expect("tempdir for redb file");
    let redb_path = tmp.path().join("observation.redb");

    // Pre-restart: write the "yesterday" Running alloc.
    let alloc_id = AllocationId::new("alloc-walking-01").expect("valid alloc id");
    let row = AllocStatusRow {
        alloc_id: alloc_id.clone(),
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
    };

    let _guard = set_default(subscriber);

    let store = LocalObservationStore::open(&redb_path).expect("open #1");
    store
        .write(ObservationRow::AllocStatus(Box::new(row.clone())))
        .await
        .expect("write yesterday alloc-status");
    drop(store);

    // Post-restart: re-open the store and read.
    let store_after = LocalObservationStore::open(&redb_path).expect("open #2");
    let rows = store_after.alloc_status_rows().await.expect("read after restart");

    assert_eq!(rows.len(), 1, "exactly one alloc row must survive restart");
    assert_eq!(rows[0].alloc_id, row.alloc_id);
    assert_eq!(rows[0].state, AllocState::Running, "state must read as Running");

    // No structural decode error events from envelope or rkyv.
    let entries = captured.entries();
    let bad: Vec<&String> = entries
        .iter()
        .filter(|e| {
            e.contains("subtree pointer overran range")
                || e.contains("observation.envelope.decode_failed")
        })
        .collect();
    assert!(bad.is_empty(), "no decode-failure events expected on the happy path; got {bad:?}");
}
