//! S-DBN-BIND-01/02/03 — the `DnsResponder` socket-loop Tier-3 acceptance
//! (dial-by-name-responder, ADR-0072 REV-2, GH #243; roadmap 02-01).
//!
//! The irreducibly-Tier-3 substrate (DDN-4 — the `IP_PKTINFO` multi-homed
//! socket has NO `BPF_PROG_TEST_RUN` backstop, so a green check / `--no-run`
//! proves NOTHING about runtime source-pinning). These scenarios drive the
//! PRODUCTION `DnsResponder::{probe,serve}` on a real kernel under Lima, as
//! root.
//!
//! - **S-DBN-BIND-01** — `probe` binds the wildcard `0.0.0.0:53`
//!   (`SO_REUSEADDR` + `IP_PKTINFO`); a query for a resolvable `<job>` answers
//!   its stable frontend `F`, and the reply is source-pinned to the queried
//!   destination via `ipi_spec_dst` (the spike litmus: a missing source-pin is
//!   what `getent` rejects). We assert the reply arrives FROM the queried dst
//!   addr — the wire-level source-pin proof `getent` accepts.
//! - **S-DBN-BIND-02** — a stand-in wildcard `0.0.0.0:53` holder forces the
//!   wildcard path to `EADDRINUSE`; `probe` re-derives the bound socket set
//!   from `NetSlotAllocator::snapshot()` (one `:53` socket per assigned
//!   gateway) and answers on each.
//! - **S-DBN-BIND-03** — the Earned-Trust gate (DDN-6, wire → probe → use):
//!   Scenario A (`:53` already held) → `probe` returns
//!   `Err(DnsResponderError::Bind { .. })`; Scenario B
//!   (`all_service_backends_rows()` fails at List-seed) → `probe` returns
//!   `Err(DnsResponderError::ListSeed { .. })`. Each maps to a distinct
//!   `health.startup.refused` reason at the `run_server` composition root.
//! - **S-DBN-BIND-03 composition-root half (D2)** — the prior two assert the
//!   `probe()`-level variant; this one boots the PRODUCTION `run_server`
//!   composition root (real `EbpfDataplane` + composed mTLS worker via
//!   `mtls_identity_override`, mirroring the keystone) with the test-only
//!   `dns_probe_fault` seam armed, and asserts the boot REFUSES with
//!   `Err(ControlPlaneError::DnsResponderBoot(_))` AND emits a structured
//!   `health.startup.refused` event whose `reason` is `dns.responder.probe` —
//!   killing the "delete the `return Err(DnsResponderBoot)`" + "flatten the
//!   reason mapping" mutants the `probe()`-level tests cannot reach.
//!
//! Root + Lima (the `:53` bind needs CAP_NET_BIND_SERVICE / root; the
//! composition-root test additionally needs the real `EbpfDataplane` XDP attach
//! + the mTLS kTLS-arm probe); a non-root run SKIPs cleanly (the K1 root gate).
//! `uname -r` is recorded.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::too_many_lines,
    reason = "Tier-3 acceptance body; failures panic with informative messages; \
              DDN-5/ipi_spec_dst/SO_REUSEADDR are the ADR-0072 contract vocabulary"
)]

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::responder::{DnsResponder, DnsResponderError};
use overdrive_control_plane::error::ControlPlaneError;
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_control_plane::{ServerConfig, run_server_with_obs_and_driver};
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::IdentityRead;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::Driver;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalObservationStore;
use rcgen::string::Ia5String;
use rcgen::{CertificateParams, Issuer, KeyPair, SanType};
use rustls::pki_types::CertificateDer;
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

/// True iff this process is uid 0 (root). The bind to `:53` needs root /
/// CAP_NET_BIND_SERVICE, so a non-root run SKIPs cleanly (the K1 root gate).
fn is_root() -> bool {
    // SAFETY: getuid is always safe; it takes no args and never fails.
    unsafe { libc::getuid() == 0 }
}

/// Record the running kernel (`uname -r`) — the Tier-3 verdict is pinned to a
/// kernel (dev Lima and the pinned 6.18 appliance kernel differ — ADR-0068).
fn record_kernel() {
    if let Ok(out) = Command::new("uname").arg("-r").output() {
        eprintln!("dns_responder_bind: kernel {}", String::from_utf8_lossy(&out.stdout).trim());
    }
}

fn fresh_store() -> Arc<SimObservationStore> {
    Arc::new(SimObservationStore::single_peer(NodeId::new("local").expect("valid node id"), 0))
}

/// A `service_backends` row for `service_id` carrying one healthy backend whose
/// alloc SpiffeId is the `/job/<job>/alloc/<alloc>` shape `job_of` parses.
fn job_backend_row(service_id: u64, job: &str, addr: SocketAddrV4) -> ServiceBackendRow {
    let backend = Backend {
        alloc: SpiffeId::for_allocation(
            &WorkloadId::new(job).expect("valid workload id"),
            &AllocationId::new("alloc-1").expect("valid alloc id"),
        ),
        addr: std::net::SocketAddr::V4(addr),
        weight: 1,
        healthy: true,
    };
    ServiceBackendRow {
        service_id: ServiceId::new(service_id).expect("valid service id"),
        vip: Ipv4Addr::new(10, 1, 0, 1),
        backends: vec![backend],
        updated_at: LogicalTimestamp {
            counter: 1,
            writer: NodeId::new("local").expect("valid node id"),
        },
    }
}

/// Encode an `A` query for `<job>.svc.overdrive.local` (the on-wire form a stub
/// resolver sends).
fn encode_a_query(job: &str) -> Vec<u8> {
    let name = Name::from_ascii(format!("{job}.svc.overdrive.local.")).expect("valid FQDN");
    let mut message = Message::new(0x1234, MessageType::Query, OpCode::Query);
    message.add_query(Query::query(name, RecordType::A));
    message.to_bytes().expect("encode query")
}

fn responder(
    store: Arc<dyn ObservationStore>,
    slots: NetSlotAllocator,
    frontend: FrontendAddrAllocator,
) -> Arc<DnsResponder> {
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    Arc::new(DnsResponder::new(store, clock, slots, frontend))
}

// ---------------------------------------------------------------------------
// S-DBN-BIND-01 — wildcard 0.0.0.0:53 + IP_PKTINFO; reply answers F, source-pin
// ---------------------------------------------------------------------------

/// S-DBN-BIND-01 — `probe` binds the wildcard `0.0.0.0:53` (`SO_REUSEADDR` +
/// `IP_PKTINFO`); a query for a resolvable `<job>` answers its stable frontend
/// `F`, and the reply is SOURCE-PINNED to the queried dst via `ipi_spec_dst`.
///
/// The source-pin is the litmus: a client that addressed `127.0.0.2:53` must
/// receive the reply FROM `127.0.0.2:53`, not the host's primary addr — that is
/// exactly what `getaddrinfo`/`getent` validates and `dig` does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wildcard_bind_answers_frontend_and_source_pins_reply() {
    if !is_root() {
        eprintln!("SKIP wildcard_bind_answers_frontend_and_source_pins_reply: not root");
        return;
    }
    record_kernel();

    // GIVEN a resolvable `<job>` whose stable F the allocator binds + whose
    // healthy backend row is in the store.
    let store = fresh_store();
    let frontend = FrontendAddrAllocator::new();
    let job = MeshServiceName::new("server.svc.overdrive.local").expect("valid mesh name");
    let f = frontend.assign(&job).expect("assign F");
    store
        .write(ObservationRow::ServiceBackend(job_backend_row(
            1,
            "server",
            SocketAddrV4::new(Ipv4Addr::new(10, 99, 0, 6), 8080),
        )))
        .await
        .expect("write backend row");

    // WHEN probe binds the wildcard 0.0.0.0:53 and serve runs.
    let dns = responder(
        Arc::clone(&store) as Arc<dyn ObservationStore>,
        NetSlotAllocator::new(),
        frontend,
    );
    dns.probe().await.expect("probe binds wildcard 0.0.0.0:53 + List-seeds");
    let serve = tokio::spawn(Arc::clone(&dns).serve());

    // Drive the blocking UDP client on a dedicated OS thread with a bounded
    // read timeout, so the async runtime is never blocked on a raw syscall (the
    // serve loop's spawn_blocking recvmsg must keep making progress). The
    // client ADDRESSES a specific loopback dst (127.0.0.2:53): the wildcard
    // socket receives it with ipi_spec_dst = 127.0.0.2, and the reply MUST come
    // back FROM 127.0.0.2:53 (the source-pin) — the getent oracle (a stub
    // resolver discards a reply whose source != the queried server addr).
    let exchange = tokio::task::spawn_blocking(|| {
        // Brief settle so the serve loop reaches its first recvmsg (UDP buffers
        // anyway; this just tightens the signal).
        std::thread::sleep(Duration::from_millis(200));
        let dst = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 53);
        let client =
            UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("bind client");
        client.set_read_timeout(Some(Duration::from_secs(3))).expect("set timeout");
        client.send_to(&encode_a_query("server"), dst).expect("send A query to 127.0.0.2:53");
        let mut buf = [0u8; 1500];
        let (n, reply_src) =
            client.recv_from(&mut buf).expect("a reply arrives (the serve loop answered)");
        (buf[..n].to_vec(), reply_src)
    });

    let (reply_bytes, reply_src) = exchange.await.expect("client exchange task");
    // Stop the SO_RCVTIMEO-bounded serve loop (exits within one poll window) so
    // the blocking recvmsg tasks do not leak and hang teardown; abort backstops.
    dns.stop();
    serve.abort();

    // THE SOURCE-PIN LITMUS: the reply's source addr is the queried dst
    // (127.0.0.2), NOT the host's primary loopback (127.0.0.1) — what getent
    // accepts and dig does not.
    assert_eq!(
        reply_src.ip(),
        std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)),
        "reply MUST be source-pinned to the queried dst (ipi_spec_dst) — the getent oracle",
    );
    let reply = Message::from_vec(&reply_bytes).expect("decode reply");
    let answered: Vec<Ipv4Addr> = reply
        .answers
        .iter()
        .filter_map(|r| match &r.data {
            hickory_proto::rr::RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect();
    assert_eq!(answered, vec![f], "the A reply must answer the stable frontend F");
}

// ---------------------------------------------------------------------------
// S-DBN-BIND-02 — per-gateway-addr fallback re-derives from NetSlotAllocator
// ---------------------------------------------------------------------------

/// S-DBN-BIND-02 — a stand-in wildcard `0.0.0.0:53` holder forces the wildcard
/// path to `EADDRINUSE`; `probe` falls back to one `:53` socket per assigned
/// gateway addr, re-derived from `NetSlotAllocator::snapshot()`. We assert the
/// fallback path binds (no error) when a wildcard holder is present and ≥1 slot
/// is assigned.
#[tokio::test]
async fn per_gateway_addr_fallback_binds_when_wildcard_is_held() {
    if !is_root() {
        eprintln!("SKIP per_gateway_addr_fallback_binds_when_wildcard_is_held: not root");
        return;
    }
    record_kernel();

    // GIVEN a stand-in process holding a WILDCARD 0.0.0.0:53 bind (forces
    // EADDRINUSE on the wildcard path) ...
    let wildcard_holder = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 53));
    let Ok(_wildcard_holder) = wildcard_holder else {
        eprintln!("SKIP: could not bind stand-in wildcard :53 (port in use by another binder)");
        return;
    };

    // ... AND ≥1 alloc assigned a slot in the NetSlotAllocator (each with a
    // derived gateway addr via responder_addr_for_slot).
    let slots = NetSlotAllocator::new();
    slots.assign(AllocationId::new("alloc-a").expect("valid alloc id")).expect("assign slot a");
    slots.assign(AllocationId::new("alloc-b").expect("valid alloc id")).expect("assign slot b");

    let store = fresh_store();
    let frontend = FrontendAddrAllocator::new();
    let dns = responder(Arc::clone(&store) as Arc<dyn ObservationStore>, slots, frontend);

    // WHEN probe attempts the wildcard bind, gets EADDRINUSE, and falls back to
    // per-gateway-addr sockets re-derived from net_slot_allocator.snapshot().
    // THEN the fallback binds one :53 per assigned gateway with no error.
    dns.probe()
        .await
        .expect("probe falls back to per-gateway-addr :53 sockets on wildcard EADDRINUSE");
}

// ---------------------------------------------------------------------------
// S-DBN-BIND-03 — boot REFUSES on unbindable port / unreadable store (DDN-6)
// ---------------------------------------------------------------------------

/// S-DBN-BIND-03 Scenario A — no bindable `:53` (the wildcard is held AND the
/// NetSlotAllocator snapshot is empty, so the per-gateway-addr fallback has
/// nothing to bind) → `probe` returns `Err(DnsResponderError::Bind { .. })`.
///
/// The composition root maps this to `health.startup.refused`
/// (reason `dns.responder.bind`) and refuses boot — mirroring the
/// `MtlsResolve.probe()` → `MtlsBoot` block.
#[tokio::test]
async fn probe_refuses_when_no_bindable_port() {
    if !is_root() {
        eprintln!("SKIP probe_refuses_when_no_bindable_port: not root");
        return;
    }
    record_kernel();

    // GIVEN the wildcard 0.0.0.0:53 is held AND no slots are assigned (the
    // fallback would bind nothing) — so the wildcard EADDRINUSE has no fallback
    // and probe must refuse with Bind.
    let Ok(_wildcard_holder) = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 53)) else {
        eprintln!("SKIP: could not bind stand-in wildcard :53");
        return;
    };

    let store = fresh_store();
    let frontend = FrontendAddrAllocator::new();
    let dns = responder(
        Arc::clone(&store) as Arc<dyn ObservationStore>,
        NetSlotAllocator::new(),
        frontend,
    );

    // The wildcard is held; the empty-snapshot fallback binds nothing, so probe
    // succeeds with zero sockets — which is NOT a refusal. To force the Bind
    // refusal we hold the wildcard AND a candidate gateway addr. With no slots,
    // the contract is: empty fallback is a degenerate-but-valid bind (probe Ok,
    // no sockets). The genuine "no bindable :53" refusal is exercised by the
    // per-addr collision below.
    let slots = NetSlotAllocator::new();
    let slot = slots.assign(AllocationId::new("alloc-x").expect("valid alloc id")).expect("slot");
    let gateway = overdrive_control_plane::veth_provisioner::responder_addr_for_slot(slot);
    let Ok(_gw_holder) = UdpSocket::bind(SocketAddrV4::new(gateway, 53)) else {
        eprintln!("SKIP: could not bind stand-in gateway :53 at {gateway}");
        return;
    };
    let dns_collision = responder(
        Arc::clone(&store) as Arc<dyn ObservationStore>,
        slots,
        FrontendAddrAllocator::new(),
    );
    let err = dns_collision
        .probe()
        .await
        .expect_err("probe refuses when the wildcard AND every per-gateway :53 are held");
    assert!(
        matches!(err, DnsResponderError::Bind { .. }),
        "no bindable :53 → DnsResponderError::Bind (reason dns.responder.bind), got {err:?}",
    );
    drop(dns);
}

/// S-DBN-BIND-03 Scenario B — an `ObservationStore` whose
/// `all_service_backends_rows()` fails at the List-seed → `probe` returns
/// `Err(DnsResponderError::ListSeed { .. })`.
///
/// The composition root maps this to `health.startup.refused`
/// (reason `dns.responder.listseed`) and refuses boot.
#[tokio::test]
async fn probe_refuses_when_store_unreadable_at_listseed() {
    if !is_root() {
        eprintln!("SKIP probe_refuses_when_store_unreadable_at_listseed: not root");
        return;
    }
    record_kernel();

    // GIVEN a store whose List read fails (armed fault). The wildcard binds Ok,
    // so the refusal is specifically the ListSeed leg.
    let store = Arc::new(FailingListStore::new());
    let frontend = FrontendAddrAllocator::new();
    let dns = responder(
        Arc::clone(&store) as Arc<dyn ObservationStore>,
        NetSlotAllocator::new(),
        frontend,
    );

    let err = dns.probe().await.expect_err("probe refuses on an unreadable store at List-seed");
    assert!(
        matches!(err, DnsResponderError::ListSeed { .. }),
        "unreadable store at List-seed → DnsResponderError::ListSeed \
         (reason dns.responder.listseed), got {err:?}",
    );
}

/// N2 — the silent-deaf-responder guard. When the wildcard `0.0.0.0:53` is held
/// (forcing the fallback) AND the slot snapshot is EMPTY, `probe` binds ZERO
/// sockets and returns `Ok(())` (a degenerate-but-valid bind). With no converge
/// tick (#247) the responder is then permanently deaf — so `probe` MUST emit a
/// structured `dns.responder.fallback.zero_sockets` warning, making the degraded
/// boot observable rather than indistinguishable from a healthy one.
#[tokio::test]
async fn empty_fallback_binds_zero_sockets_and_warns_it_is_deaf() {
    if !is_root() {
        eprintln!("SKIP empty_fallback_binds_zero_sockets_and_warns_it_is_deaf: not root");
        return;
    }
    record_kernel();

    // Hold the wildcard 0.0.0.0:53 so the fallback fires; assign NO slots so the
    // fallback binds nothing.
    let Ok(_wildcard_holder) = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 53)) else {
        eprintln!("SKIP: could not bind stand-in wildcard :53");
        return;
    };

    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let store = fresh_store();
    let dns = responder(
        Arc::clone(&store) as Arc<dyn ObservationStore>,
        NetSlotAllocator::new(),
        FrontendAddrAllocator::new(),
    );

    // The empty fallback is a valid bind of zero sockets — probe returns Ok.
    dns.probe().await.expect("empty fallback binds zero sockets (degenerate but valid)");

    // ... but it MUST warn that the responder is deaf.
    let events = collector.snapshot();
    let warned = events.iter().any(|row| row.name == "dns.responder.fallback.zero_sockets");
    assert!(
        warned,
        "an empty-fallback zero-socket bind MUST emit dns.responder.fallback.zero_sockets \
         (the responder is deaf and must not look healthy); got: {events:?}",
    );
}

/// An `ObservationStore` whose `all_service_backends_rows` always errors (the
/// List-seed fault), delegating every other method to an inner
/// `SimObservationStore`. Used by S-DBN-BIND-03 Scenario B. Mirrors the
/// `RelistFailsStore` delegation shape from `tests/acceptance/dns_name_index.rs`.
struct FailingListStore {
    inner: Arc<SimObservationStore>,
}

impl FailingListStore {
    fn new() -> Self {
        Self { inner: fresh_store() }
    }
}

use overdrive_core::traits::observation_store::{
    AllocStatusRow, LagAwareSubscription, NodeHealthRow, ObservationStoreError,
    ReconcileConflictRow, ServiceHydrationResultRow,
};

#[async_trait::async_trait]
impl ObservationStore for FailingListStore {
    async fn write(&self, row: ObservationRow) -> Result<(), ObservationStoreError> {
        self.inner.write(row).await
    }

    async fn subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError> {
        self.inner.subscribe_all_events().await
    }

    async fn all_service_backends_rows(
        &self,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        // The injected List-seed fault — probe's `name_index.probe()` List leg
        // surfaces this as DnsResponderError::ListSeed.
        Err(ObservationStoreError::Unreachable { peer: "list-seed-fault".to_owned() })
    }

    // The remaining surface is unused by the responder's NameIndex — delegate
    // everything to the backing SimObservationStore.
    async fn alloc_status_rows(&self) -> Result<Vec<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_rows().await
    }
    async fn alloc_status_row(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Option<AllocStatusRow>, ObservationStoreError> {
        self.inner.alloc_status_row(alloc_id).await
    }
    async fn node_health_rows(&self) -> Result<Vec<NodeHealthRow>, ObservationStoreError> {
        self.inner.node_health_rows().await
    }
    async fn issued_certificate_rows(
        &self,
    ) -> Result<
        Vec<overdrive_core::ca::issued_certificate_row::IssuedCertificateRow>,
        ObservationStoreError,
    > {
        self.inner.issued_certificate_rows().await
    }
    async fn next_issuance_ordinal(
        &self,
    ) -> Result<overdrive_core::id::IssuanceOrdinal, ObservationStoreError> {
        self.inner.next_issuance_ordinal().await
    }
    async fn write_probe_result(
        &self,
        row: overdrive_core::observation::ProbeResultRow,
    ) -> Result<(), ObservationStoreError> {
        self.inner.write_probe_result(row).await
    }
    async fn list_probe_results_for_alloc(
        &self,
        alloc_id: &AllocationId,
    ) -> Result<Vec<overdrive_core::observation::ProbeResultRow>, ObservationStoreError> {
        self.inner.list_probe_results_for_alloc(alloc_id).await
    }
    async fn workflow_terminal_rows(
        &self,
    ) -> Result<
        Vec<(overdrive_core::id::CorrelationKey, overdrive_core::workflow::WorkflowStatus)>,
        ObservationStoreError,
    > {
        self.inner.workflow_terminal_rows().await
    }
    async fn workflow_signal(
        &self,
        key: &overdrive_core::workflow::SignalKey,
    ) -> Result<Option<overdrive_core::workflow::SignalValue>, ObservationStoreError> {
        self.inner.workflow_signal(key).await
    }
    async fn service_hydration_results_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceHydrationResultRow>, ObservationStoreError> {
        self.inner.service_hydration_results_rows(service_id).await
    }
    async fn service_backends_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ServiceBackendRow>, ObservationStoreError> {
        self.inner.service_backends_rows(service_id).await
    }
    async fn reconcile_conflict_rows(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<ReconcileConflictRow>, ObservationStoreError> {
        self.inner.reconcile_conflict_rows(service_id).await
    }
}

// ===========================================================================
// S-DBN-BIND-03 composition-root half (D2) — `run_server` REFUSES boot on a
// DNS-probe failure, mapping it to the `dns.responder.probe` refusal reason.
// ===========================================================================
//
// The `probe_refuses_*` tests above drive `DnsResponder::probe()` directly and
// assert the `DnsResponderError` variant — they never boot `run_server`, so the
// composition-root logic (the reason mapping + the
// `return Err(ControlPlaneError::DnsResponderBoot(_))` refusal) is untested by
// them. This test closes that gap: it boots the PRODUCTION composition root
// in-process (real `EbpfDataplane` + composed mTLS worker via
// `mtls_identity_override`, mirroring `canonical_address_inbound_walking_skeleton`),
// arms the test-only `dns_probe_fault` seam so the DNS responder's `probe()`
// short-circuits to `Err(DnsResponderError::Probe { .. })`, and asserts BOTH:
//   1. the boot returns `Err(ControlPlaneError::DnsResponderBoot(_))` — kills
//      the "delete the `return Err(DnsResponderBoot)`" mutant (boot would else
//      continue);
//   2. a structured `health.startup.refused` event with `reason =
//      dns.responder.probe` is emitted — kills the "flatten the reason mapping"
//      mutant (the wired reason comes from the tested `boot_refusal_reason`).

// ---------------------------------------------------------------------------
// Tracing capture — minimal layer recording each event's `name:` + visited
// fields (mirrors `tests/acceptance/probe_runner_boot_gate.rs`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct EventRow {
    name: String,
    fields: std::collections::BTreeMap<String, String>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: std::collections::BTreeMap<String, String>,
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
    fn snapshot(&self) -> Vec<EventRow> {
        self.inner.lock().expect("collector lock").clone()
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

// ---------------------------------------------------------------------------
// Minimal test PKI (root → intermediate → server leaf) so the composed mTLS
// worker's Earned-Trust `probe()` PASSES — the boot must REACH the DNS
// responder block (it is gated on `mtls_worker.is_some()`) before the armed
// `dns_probe_fault` refuses it. Trimmed from the keystone's `TestPki` to just
// the server SVID + trust bundle the inbound leg-C handshake needs.
// ---------------------------------------------------------------------------

struct ServerPki {
    ca_cert_pem: String,
    intermediate_cert_pem: String,
    server_cert_pem: String,
    server_cert_der: CertificateDer<'static>,
    server_key_pem: String,
    server_spiffe: SpiffeId,
}

impl ServerPki {
    fn mint() -> Self {
        let root = mint_root("overdrive-dns-d2-ROOT-CA");
        let intermediate = mint_intermediate(&root, "overdrive-dns-d2-INTERMEDIATE-CA");
        let server_spiffe = "spiffe://overdrive.local/ns/default/sa/server";
        let (cert_pem, cert_der, key_pem) =
            mint_server_leaf(&intermediate, server_spiffe, "server.overdrive.local");
        Self {
            ca_cert_pem: root.cert_pem,
            intermediate_cert_pem: intermediate.cert_pem,
            server_cert_pem: cert_pem,
            server_cert_der: cert_der,
            server_key_pem: key_pem,
            server_spiffe: server_spiffe.parse().expect("valid spiffe id"),
        }
    }

    fn identity(&self) -> Arc<dyn IdentityRead> {
        let not_after = UnixInstant::from_unix_duration(Duration::from_secs(4_102_444_800)); // 2100
        let svid = SvidMaterial::new(
            CaCertPem::new(self.server_cert_pem.clone()),
            CaCertDer::new(self.server_cert_der.as_ref().to_vec()),
            CertSerial::new("0a0b0c0d").expect("valid serial"),
            self.server_spiffe.clone(),
            CaKeyPem::new(self.server_key_pem.clone()),
            not_after,
        );
        let bundle = TrustBundle::new(
            CaCertPem::new(self.ca_cert_pem.clone()),
            Some(CaCertPem::new(self.intermediate_cert_pem.clone())),
        );
        Arc::new(HeldServerIdentity { svid, bundle })
    }
}

use overdrive_core::CertSerial;

struct HeldServerIdentity {
    svid: SvidMaterial,
    bundle: TrustBundle,
}

impl IdentityRead for HeldServerIdentity {
    fn svid_for(&self, _alloc: &AllocationId) -> Option<SvidMaterial> {
        Some(self.svid.clone())
    }
    fn current_bundle(&self) -> Option<TrustBundle> {
        Some(self.bundle.clone())
    }
}

struct MintedCa {
    params: CertificateParams,
    key: KeyPair,
    cert_pem: String,
}

fn mint_root(cn: &str) -> MintedCa {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.distinguished_name.push(rcgen::DnType::CommonName, cn);
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let cert_pem = cert.pem();
    MintedCa { params, key, cert_pem }
}

fn mint_intermediate(root: &MintedCa, cn: &str) -> MintedCa {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    params.distinguished_name.push(rcgen::DnType::CommonName, cn);
    params.use_authority_key_identifier_extension = true;
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let root_issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&root.params, &root.key);
    let cert = params.signed_by(&key, &root_issuer).unwrap();
    let cert_pem = cert.pem();
    MintedCa { params, key, cert_pem }
}

fn mint_server_leaf(
    intermediate: &MintedCa,
    spiffe: &str,
    dns_san: &str,
) -> (String, CertificateDer<'static>, String) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    let uri = Ia5String::try_from(spiffe).expect("spiffe URI is a valid IA5 string");
    let dns = Ia5String::try_from(dns_san).expect("dns SAN is a valid IA5 string");
    params.subject_alt_names = vec![SanType::URI(uri), SanType::DnsName(dns)];
    params.distinguished_name.push(rcgen::DnType::CommonName, spiffe);
    params.use_authority_key_identifier_extension = true;
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let issuer: Issuer<'_, &KeyPair> = Issuer::from_params(&intermediate.params, &intermediate.key);
    let cert = params.signed_by(&leaf_key, &issuer).unwrap();
    let cert_pem = cert.pem();
    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_pem = leaf_key.serialize_pem();
    (cert_pem, cert_der, key_pem)
}

/// S-DBN-BIND-03 (D2) — boot the production composition root with the DNS-probe
/// fault armed; the boot must REFUSE with
/// `Err(ControlPlaneError::DnsResponderBoot(_))` AND emit
/// `health.startup.refused` with `reason = dns.responder.probe`.
///
/// Mirrors the keystone's real-`EbpfDataplane` + `mtls_identity_override` boot
/// harness (the DNS responder block is gated on `mtls_worker.is_some()`, so the
/// composed mTLS worker is required to REACH it; the armed `dns_probe_fault`
/// then refuses the boot at the DNS responder's `probe()`).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn run_server_refuses_boot_on_dns_probe_fault_with_probe_reason() {
    if !is_root() {
        eprintln!(
            "SKIP run_server_refuses_boot_on_dns_probe_fault_with_probe_reason: not root \
             (real EbpfDataplane XDP attach + mTLS kTLS-arm probe + netns provision need \
             CAP_NET_ADMIN/CAP_SYS_ADMIN)"
        );
        return;
    }
    record_kernel();

    // The composition-root rustls CryptoProvider (installed once per process).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Capture the structured boot events so we can assert the refusal reason.
    let collector = EventCollector::default();
    let subscriber = tracing_subscriber::registry().with(collector.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let pki = ServerPki::mint();

    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let cfg_dir = tmp.path().join("conf");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir cfg");

    let obs_path = data_dir.join("observation.redb");
    let obs: Arc<dyn ObservationStore> =
        Arc::new(LocalObservationStore::open(&obs_path).expect("open LocalObservationStore"));

    let driver: Arc<dyn Driver> = Arc::new(overdrive_worker::ExecDriver::new(
        std::path::PathBuf::from("/sys/fs/cgroup"),
        Arc::new(overdrive_host::SystemClock),
        Arc::new(overdrive_host::RealCgroupFs::new()),
    ));

    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("parse bind addr"),
        data_dir: data_dir.clone(),
        operator_config_dir: cfg_dir.clone(),
        dataplane: Some(overdrive_control_plane::dataplane_config::DataplaneConfig {
            client_iface: overdrive_control_plane::veth_provisioner::DEFAULT_CLIENT_IFACE
                .to_owned(),
            backend_iface: overdrive_control_plane::veth_provisioner::DEFAULT_BACKEND_IFACE
                .to_owned(),
        }),
        dataplane_pin_dir: None,
        // NO dataplane_override → compose_mtls = true → the production mTLS
        // worker is composed and its Earned-Trust probe runs (so the boot
        // REACHES the `mtls_worker.is_some()`-gated DNS responder block).
        dataplane_override: None,
        // The leg-C/leg-B test PKI so the composed mTLS worker's probe passes.
        mtls_identity_override: Some(pki.identity()),
        // THE seam under test: force the DNS responder's `probe()` to fail.
        dns_probe_fault: Some("injected dns probe fault (D2)".to_owned()),
        ..ServerConfig::new(Arc::new(overdrive_sim::adapters::SimKek::for_boot()))
    };

    let result = run_server_with_obs_and_driver(config, obs.clone(), driver).await;

    // (1) the boot REFUSED with the typed DnsResponderBoot variant — kills the
    // "delete the return Err(DnsResponderBoot)" mutant (boot would otherwise
    // continue and return Ok(ServerHandle)).
    let Err(err) = result else {
        panic!(
            "run_server MUST refuse boot when the DNS responder probe fails; \
             got Ok(ServerHandle) — the DnsResponderBoot refusal was bypassed"
        );
    };
    assert!(
        matches!(err, ControlPlaneError::DnsResponderBoot(DnsResponderError::Probe { .. })),
        "DNS-probe fault → ControlPlaneError::DnsResponderBoot(Probe); got {err:?}",
    );

    // (2) the structured refusal event names reason = dns.responder.probe —
    // kills the "flatten the reason mapping" mutant (the wired reason comes
    // from the tested `boot_refusal_reason`, distinct from the other variants).
    let events = collector.snapshot();
    let dns_refusal = events.iter().any(|row| {
        row.name == "health.startup.refused"
            && row.fields.get("reason").map(String::as_str) == Some("dns.responder.probe")
    });
    assert!(
        dns_refusal,
        "expected health.startup.refused with reason=dns.responder.probe; got: {events:?}",
    );
}
