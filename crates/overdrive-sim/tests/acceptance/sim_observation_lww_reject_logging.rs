//! `observation.lww.rejected` diagnostics on `SimObservationStore`.
//!
//! The LWW reject path used to be completely silent: `apply_alloc_status`
//! returns `false`, `write` returns `Ok(())`, and nothing was logged — so
//! a dropped observation write was indistinguishable from a successful
//! one at every surface (see
//! `docs/analysis/root-cause-analysis-cross-restart-lww-counter-regression.md`
//! § 2.3, where a full control-plane boot log across a 29-second outage
//! window was two INFO lines).
//!
//! This pins the diagnostic half of that fix on the sim adapter:
//!
//! * the event FIRES when a write loses to the stored row, carrying both
//!   the incoming and the stored `(counter, writer)` so the operator can
//!   see *which* stamp lost to *what*;
//! * the event does NOT fire when a write is accepted (fresh key, or a
//!   dominating stamp) — a diagnostic that fires on the happy path is
//!   noise, not signal.
//!
//! # Level
//!
//! The sim emits at `debug!`, its host sibling at `warn!`. That
//! divergence is deliberate — see the `log_lww_reject` docstring in
//! `overdrive-sim/src/adapters/observation_store.rs` for the full
//! rationale. The short version: this adapter has a `GossipRouter`, so a
//! re-delivered row losing IS the expected LWW idempotency case, whereas
//! the single-node host adapter has no gossip surface and a reject there
//! is always anomalous.
//!
//! # Capture mechanism
//!
//! Thread-local `tracing::subscriber::set_default`, mirroring the host
//! adapter's existing capture harness in
//! `overdrive-store-local/tests/integration/envelope_observation_skip.rs`.
//! Thread-local is sufficient here because `SimObservationStore::write`
//! applies the row synchronously on the caller's thread — no
//! `spawn_blocking`, no task spawn. (The host adapter runs its redb
//! transaction inside `tokio::task::spawn_blocking`, so its own test
//! needs a global subscriber instead.)

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::id::{AllocationId, NodeId, WorkloadId};
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, LogicalTimestamp, ObservationStore,
};
use overdrive_sim::adapters::observation_store::SimObservationStore;
use tracing::subscriber::set_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

const STEP_SEED: u64 = 0x1A_11_DE_AD_BE_EF_00_01;

/// Records every event the subscriber sees as `"<name> | target=<t> f=v …"`.
/// Same shape as the host adapter's harness so a reader comparing the two
/// suites sees one pattern, not two.
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

fn peer_node() -> NodeId {
    NodeId::from_str("node-a").expect("valid node id")
}

fn alloc_row(alloc: &str, counter: u64) -> AllocStatusRow {
    AllocStatusRow {
        alloc_id: AllocationId::from_str(alloc).expect("valid alloc id"),
        workload_id: WorkloadId::from_str("payments").expect("valid workload id"),
        node_id: peer_node(),
        state: AllocState::Running,
        updated_at: LogicalTimestamp { counter, writer: peer_node() },
        reason: None,
        detail: None,
        terminal: None,
        stderr_tail: None,
        kind: overdrive_core::aggregate::WorkloadKind::Service,
        listeners: Vec::new(),
        started_at: Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
        workload_addr: None,
        last_terminated: None,
        restart_count: 0,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn losing_alloc_status_write_emits_lww_rejected_and_winning_writes_do_not() {
    let captured = CapturedEvents::default();
    let _guard = set_default(Registry::default().with(captured.clone()));

    let store = SimObservationStore::single_peer(peer_node(), STEP_SEED);
    let alloc = "alloc-lww-reject-01";

    // ACCEPT #1 — fresh key, no prior to lose to.
    store
        .write_alloc_lifecycle(
            alloc_row(alloc, 5),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("fresh write succeeds");
    assert!(
        captured.lww_rejects().is_empty(),
        "a write on a fresh key must not emit a reject event; got {:?}",
        captured.lww_rejects()
    );

    // ACCEPT #2 — dominating stamp.
    store
        .write_alloc_lifecycle(
            alloc_row(alloc, 9),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("dominating write succeeds");
    assert!(
        captured.lww_rejects().is_empty(),
        "a dominating write must not emit a reject event; got {:?}",
        captured.lww_rejects()
    );

    // REJECT — counter 3 loses to the stored counter 9. This is the
    // cross-restart shape from the RCA in miniature: a writer whose
    // counter reset below the surviving row's high-water mark.
    store
        .write_alloc_lifecycle(
            alloc_row(alloc, 3),
            overdrive_core::traits::observation_store::TransitionSource::Reconciler,
        )
        .await
        .expect("losing write still returns Ok(()) — the silent-drop shape");

    let rejects = captured.lww_rejects();
    assert_eq!(rejects.len(), 1, "exactly one reject event expected; got {rejects:?}");

    let event = &rejects[0];
    for expected in [
        "row_kind=\"alloc_status\"",
        "key=\"alloc-lww-reject-01\"",
        "incoming_counter=3",
        "incoming_writer=node-a",
        "stored_counter=9",
        "stored_writer=node-a",
    ] {
        assert!(event.contains(expected), "reject event must carry {expected}; got {event:?}");
    }

    // The observable contract is untouched by the diagnostics: the
    // losing row did not mutate state.
    let winner = store.latest_alloc_status(&AllocationId::from_str(alloc).expect("valid alloc id"));
    assert_eq!(
        winner.expect("stored row present").updated_at.counter,
        9,
        "the stored row must still be the counter-9 winner"
    );
}
