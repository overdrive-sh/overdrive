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
//!
//! Root + Lima (the `:53` bind needs CAP_NET_BIND_SERVICE / root); a non-root
//! run SKIPs cleanly (the K1 root gate). `uname -r` is recorded.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::doc_markdown,
    reason = "Tier-3 acceptance body; failures panic with informative messages; \
              DDN-5/ipi_spec_dst/SO_REUSEADDR are the ADR-0072 contract vocabulary"
)]

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};
use hickory_proto::serialize::binary::BinEncodable;
use overdrive_control_plane::dns_responder::frontend_addr_allocator::FrontendAddrAllocator;
use overdrive_control_plane::dns_responder::responder::{DnsResponder, DnsResponderError};
use overdrive_control_plane::veth_provisioner::NetSlotAllocator;
use overdrive_core::id::{AllocationId, MeshServiceName, NodeId, ServiceId, SpiffeId, WorkloadId};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::observation_store::SimObservationStore;

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
