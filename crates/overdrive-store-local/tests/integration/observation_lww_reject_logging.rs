//! `observation.lww.rejected` diagnostics on `LocalObservationStore`.
//!
//! The LWW reject path used to be completely silent: `apply_alloc_status_lww`
//! returns `false`, `write` commits anyway and returns `Ok(())`, and nothing
//! was logged — so a dropped observation write was indistinguishable from a
//! successful one at every surface. See
//! `docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`
//! § 2.3, where a full control-plane boot log across a 29-second window in
//! which *every* stop write was rejected contained two INFO lines and no
//! warning at all.
//!
//! This pins the diagnostic half of that fix (§ 8.2 fix 1) on the host
//! adapter:
//!
//! * the event FIRES when a write loses to the stored row, carrying both the
//!   incoming and the stored `(counter, writer)` so an operator can see which
//!   stamp lost to what — the pair that makes the cross-restart counter reset
//!   diagnosable rather than invisible;
//! * the event does NOT fire when a write is accepted (fresh key, or a
//!   dominating stamp) — a diagnostic that fires on the happy path is noise,
//!   not signal.
//!
//! # Level
//!
//! The host adapter emits at `warn!`, its sim sibling at `debug!`. That
//! divergence is deliberate — see the `log_lww_reject` docstring in
//! `overdrive-store-local/src/observation_backend.rs` for the full rationale.
//! The short version: `LocalObservationStore` is single-node with no gossip
//! surface, so an LWW reject here is always anomalous; the sim has a
//! `GossipRouter` where a re-delivered row losing IS the expected LWW
//! idempotency case.
//!
//! # Why a GLOBAL subscriber, not the thread-local `set_default` used by
//! the sibling harness in `envelope_observation_skip.rs`
//!
//! `LocalObservationStore::write` runs its whole redb transaction — and
//! therefore the `apply_*_lww` call that emits this event — inside
//! `tokio::task::spawn_blocking`. The event fires on a blocking-pool thread,
//! not on the test's thread, so a thread-local `set_default` guard cannot
//! observe it. (The read path deliberately routes its `decode_failed` warnings
//! back to the async side for exactly this reason — see `log_decode_failures`
//! — but doing the same here would mean plumbing the reject verdict out of
//! `apply_*_lww`, i.e. changing its signature, which this step explicitly
//! must not do.)
//!
//! `set_global_default` can only be called once per process. nextest runs each
//! test in its own process, so this is safe — but it is also why this file
//! holds ONE test covering both the accept and the reject direction rather
//! than two.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_core::aggregate::WorkloadKind;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;
use tracing::subscriber::set_global_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

/// Records every event the subscriber sees as `"<name> | target=<t> f=v …"`.
/// Same shape as the `envelope_observation_skip` harness in this crate.
#[derive(Clone, Default)]
struct CapturedEvents {
    inner: Arc<Mutex<Vec<String>>>,
}

impl CapturedEvents {
    fn entries(&self) -> Vec<String> {
        self.inner.lock().expect("captured events mutex").clone()
    }

    /// Only the `observation.lww.rejected` events, in emission order.
    fn lww_rejects(&self) -> Vec<String> {
        self.entries().into_iter().filter(|e| e.contains("observation.lww.rejected")).collect()
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

fn writer_node() -> NodeId {
    NodeId::new("local").expect("valid node id")
}

fn alloc_row(alloc: &str, counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::new(alloc).expect("valid alloc id"),
        workload_id: WorkloadId::new("payments").expect("valid workload id"),
        node_id: writer_node(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: writer_node() },
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

#[tokio::test]
async fn losing_alloc_status_write_emits_lww_rejected_and_winning_writes_do_not() {
    let captured = CapturedEvents::default();
    set_global_default(Registry::default().with(captured.clone()))
        .expect("global subscriber installed exactly once per test process");

    let tmp = TempDir::new().expect("tempdir");
    let store =
        LocalObservationStore::open(tmp.path().join("observation.redb")).expect("open store");
    let alloc = "alloc-lww-reject-01";

    // ACCEPT #1 — fresh key, no prior to lose to.
    store
        .write(ObservationRow::AllocStatus(Box::new(alloc_row(alloc, 5))))
        .await
        .expect("fresh write succeeds");
    assert!(
        captured.lww_rejects().is_empty(),
        "a write on a fresh key must not emit a reject event; got {:?}",
        captured.lww_rejects()
    );

    // ACCEPT #2 — dominating stamp.
    store
        .write(ObservationRow::AllocStatus(Box::new(alloc_row(alloc, 9))))
        .await
        .expect("dominating write succeeds");
    assert!(
        captured.lww_rejects().is_empty(),
        "a dominating write must not emit a reject event; got {:?}",
        captured.lww_rejects()
    );

    // REJECT — counter 3 loses to the stored counter 9. This is the RCA's
    // cross-restart shape in miniature: a writer whose tick counter reset
    // below the surviving row's high-water mark.
    store
        .write(ObservationRow::AllocStatus(Box::new(alloc_row(alloc, 3))))
        .await
        .expect("losing write still returns Ok(()) — the silent-drop shape this event exposes");

    let rejects = captured.lww_rejects();
    assert_eq!(rejects.len(), 1, "exactly one reject event expected; got {rejects:?}");

    let event = &rejects[0];
    for expected in [
        "row_kind=\"alloc_status\"",
        "key=\"alloc-lww-reject-01\"",
        "incoming_counter=3",
        "incoming_writer=local",
        "stored_counter=9",
        "stored_writer=local",
    ] {
        assert!(event.contains(expected), "reject event must carry {expected}; got {event:?}");
    }

    // The observable contract is untouched by the diagnostics: the losing
    // row did not mutate state.
    let rows = store.alloc_status_rows().await.expect("read rows");
    assert_eq!(rows.len(), 1, "one row expected; got {rows:?}");
    assert_eq!(rows[0].updated_at.counter, 9, "the stored row must still be the counter-9 winner");
}
