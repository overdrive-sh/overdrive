//! Permit derivation at boot (GH #382, ADR-0362): the forwarding in-flight bound `serve`
//! derives equals `(nofile_soft - reserve) / FDS_PER_PERMIT`, saturating, where
//! `reserve = guest_flow_fd_requirement() + NODE_FD_RESERVE`.
//!
//! Drives the production `run_server` composition in-process (the host bundle the
//! non-VM fixtures use, `tests/common/node_adapters.rs`) and reads the pinned boot
//! event `dns.forward.permits` (fields `permits`, `nofile_soft`, `reserve`) keyed on
//! `meta.name()`. The soft limit is the one the bundle injects, not the process's:
//! DELIVER's fourth `NodeAdapters::host` argument is passed
//! [`INJECTED_NOFILE_SOFT`] at the common helper.
//!
//! Pending: the event is not emitted today (`forward_permits` is the scaffold `0`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use overdrive_control_plane::dns_responder::forwarder::FDS_PER_PERMIT;
use overdrive_control_plane::guest_network::{NODE_FD_RESERVE, guest_flow_fd_requirement};
use overdrive_testing::overdrive_test;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

/// The soft descriptor limit the test composition injects: the number the sim guest-flow
/// seam reports (`flow_io/sim.rs`), far above the reserve.
const INJECTED_NOFILE_SOFT: u64 = 1_048_576;

#[derive(Debug, Clone, Default)]
struct EventRow {
    name: String,
    fields: BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}").trim_matches('"').to_owned());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.insert(field.name().to_owned(), value.to_owned());
    }
}

#[derive(Clone, Default)]
struct EventCollector {
    inner: Arc<Mutex<Vec<EventRow>>>,
}

impl EventCollector {
    fn named(&self, name: &str) -> Vec<EventRow> {
        self.inner
            .lock()
            .expect("collector lock")
            .iter()
            .filter(|row| row.name == name)
            .cloned()
            .collect()
    }
}

impl<S> Layer<S> for EventCollector
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.inner
            .lock()
            .expect("collector lock")
            .push(EventRow { name: event.metadata().name().to_owned(), fields: visitor.fields });
    }
}

fn field_u64(row: &EventRow, field: &str) -> u64 {
    row.fields
        .get(field)
        .unwrap_or_else(|| panic!("dns.forward.permits carries `{field}`; got {:?}", row.fields))
        .parse()
        .unwrap_or_else(|error| panic!("`{field}` is an unsigned integer: {error}"))
}

/// Outcome anchor: guest-public-dns-resolution ADR-0362 (permit bound derived from the
/// descriptor limit). `CONTRACT_SHAPE`: pure-function.
///
/// `serve` derives the forward in-flight bound once at boot from the soft descriptor
/// limit and reports it: exactly one `dns.forward.permits` event whose `reserve` is the
/// guest-flow requirement plus the node reserve, whose `nofile_soft` is the injected
/// limit, and whose `permits` is `(nofile_soft - reserve) / FDS_PER_PERMIT`.
#[overdrive_test(guest = "cgroup-default")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[should_panic(expected = "serve emits exactly one dns.forward.permits event per boot")]
async fn boot_derives_the_forward_permit_bound_from_the_descriptor_limit() {
    // GIVEN a node whose composition is told its soft descriptor limit.
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");
    let config = super::node_adapters::test_config(data_dir, cfg_dir);

    // WHEN the production composition boots.
    let handle = super::node_adapters::boot(config).await.expect("serve boots");
    handle.shutdown(Duration::from_secs(10)).await.expect("serve shuts down");

    // THEN it reports the bound exactly once.
    let events = collector.named("dns.forward.permits");
    assert_eq!(
        events.len(),
        1,
        "serve emits exactly one dns.forward.permits event per boot; got {events:?}"
    );
    let event = events.first().expect("one event");

    // AND the reserve is the guest-flow requirement plus the node reserve.
    let reserve = guest_flow_fd_requirement() + NODE_FD_RESERVE;
    assert_eq!(
        field_u64(event, "reserve"),
        reserve,
        "the reported reserve is guest_flow_fd_requirement() + NODE_FD_RESERVE"
    );

    // AND the limit is the injected one and the bound follows from both.
    let nofile_soft = field_u64(event, "nofile_soft");
    assert_eq!(nofile_soft, INJECTED_NOFILE_SOFT, "the reported soft limit is the injected one");
    assert_eq!(
        field_u64(event, "permits"),
        nofile_soft.saturating_sub(reserve) / FDS_PER_PERMIT,
        "permits == (nofile_soft - reserve) / FDS_PER_PERMIT"
    );
}
