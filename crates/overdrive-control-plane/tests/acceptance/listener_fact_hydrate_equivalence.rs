//! Behavior-equivalence pins for the `ServiceMapHydrator` read-path
//! switch (ADR-0062 § Decision (3); feature-delta sub-decisions 3-5).
//!
//! The hydrator's source of the per-listener `(port, protocol)` fact
//! moves from the per-tick cluster scan
//! (`gather_service_listener_facts`, deleted in step 01-04) to the
//! in-memory keyed [`ListenerFactStore`]. These tests pin the OBSERVABLE
//! behavior across the switch so the source change is provably
//! semantics-preserving:
//!
//! * BE-1 — a `service_backends` row WHOSE `ServiceId` has a keyed fact
//!   projects a desired carrying the right `(port, protocol)`, identical
//!   to the pre-change projection.
//! * BE-2 — a `service_backends` row WHOSE `ServiceId` has NO keyed fact
//!   is skipped: NO `ServiceDesired` is produced, and crucially no
//!   silently-defaulted `Proto::Tcp` entry leaks (ADR-0060 C3 verbatim).
//! * BE-3 — distinct VIPs derive distinct `ServiceId`s with no
//!   collision: the keyed store's primary-entry count equals the total
//!   listener count across all services.
//!
//! Tier 1, default lane — pure in-process `SimObservationStore` +
//! `LocalIntentStore` over a `TempDir`.

// `doc_markdown` / `unused_async` cover the appended ADR-0086 read-port
// injectability scaffolds (S-ROH-B-05..B-08): their GIVEN/WHEN/THEN + CONTRACT
// docstrings carry un-backticked spec prose, and each is an `async fn` scaffold
// whose `panic!` body has no `.await` until 02-05 fills in the real DST body.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown, clippy::unused_async)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::str::FromStr;
use std::sync::Arc;

use overdrive_control_plane::AppState;
use overdrive_control_plane::reconciler_runtime::ReconcilerRuntime;
use overdrive_core::SpiffeId;
use overdrive_core::aggregate::{
    DriverInput, ExecInput, IntentKey, Listener, ResourcesInput, ServiceV2, WorkloadIntent,
    WorkloadKind,
};
use overdrive_core::api::submit::{ListenerInput, ServiceSpecInput};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{NodeId, ServiceId, ServiceVip, WorkloadId};
use overdrive_core::reconcilers::{AnyReconciler, AnyState, ServiceMapHydrator, TargetResource};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::driver::{Driver, DriverType};
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    LogicalTimestamp, ObservationRow, ObservationStore, ServiceBackendRow,
};
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::driver::SimDriver;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;

const SERVICE_MAP_PURPOSE: &str = "service-map";

fn node_id(name: &str) -> NodeId {
    NodeId::from_str(name).expect("valid NodeId")
}

fn hydrator_reconciler() -> AnyReconciler {
    AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        std::net::Ipv4Addr::UNSPECIFIED,
        overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
    ))
}

const fn proto_str(p: Proto) -> &'static str {
    match p {
        Proto::Tcp => "tcp",
        Proto::Udp => "udp",
    }
}

fn build_app_state(tmp: &TempDir, obs: Arc<dyn ObservationStore>) -> AppState {
    let runtime =
        ReconcilerRuntime::new_with_redb_view_store_for_test(tmp.path()).expect("runtime::new");
    let store_path = tmp.path().join("intent.redb");
    let store = Arc::new(LocalIntentStore::open(&store_path).expect("LocalIntentStore::open"));
    let driver: Arc<dyn Driver> = Arc::new(SimDriver::new(DriverType::Exec));
    let allocator =
        overdrive_control_plane::test_default_allocator(Arc::clone(&store) as Arc<dyn IntentStore>);
    AppState::new(
        store,
        store_path,
        obs,
        Arc::new(runtime),
        driver,
        Arc::new(SimClock::new()),
        Arc::new(SimDataplane::new()),
        Arc::new(overdrive_sim::adapters::ca::SimCa::new(Arc::new(
            overdrive_sim::adapters::entropy::SimEntropy::new(0),
        ))),
        Arc::new(overdrive_control_plane::identity_mgr::IdentityMgr::new(None)),
        node_id("writer-1"),
        allocator,
        overdrive_control_plane::test_empty_listener_facts(),
        std::net::Ipv4Addr::LOCALHOST,
    )
}

async fn persist_and_allocate(
    state: &AppState,
    workload: &str,
    listeners: &[Listener],
) -> ServiceVip {
    let listener_inputs: Vec<ListenerInput> = listeners
        .iter()
        .map(|l| ListenerInput { port: l.port.get(), protocol: proto_str(l.protocol).to_string() })
        .collect();
    let svc = ServiceV2::from_submit(ServiceSpecInput {
        id: workload.to_string(),
        replicas: 1,
        resources: ResourcesInput { cpu_milli: 100, memory_bytes: 128 * 1024 * 1024 },
        driver: DriverInput::Exec(ExecInput { command: "/bin/serve".to_string(), args: vec![] }),
        listeners: listener_inputs,
        startup_probes: vec![],
        readiness_probes: vec![],
        liveness_probes: vec![],
    })
    .expect("valid service spec");
    let intent = WorkloadIntent::Service(svc.clone());
    let key = IntentKey::for_workload(&svc.id);
    let archived = intent.archive_for_store().expect("rkyv archive");
    state.store.put(key.as_bytes(), archived.as_ref()).await.expect("put intent");
    let kind_key = IntentKey::for_workload_kind(&svc.id);
    state
        .store
        .put(kind_key.as_bytes(), &[WorkloadKind::Service.discriminator_byte()])
        .await
        .expect("put kind");

    let digest = intent.spec_digest().expect("spec_digest");
    let bytes: [u8; 32] = *digest.as_bytes();
    let mut guard = state.allocator.lock().await;
    let vip = guard.allocate(bytes).await.expect("allocate vip");
    drop(guard);
    vip
}

fn one_backend(workload: &str) -> Backend {
    Backend {
        alloc: SpiffeId::from_str(&format!(
            "spiffe://overdrive.local/workload/{workload}/alloc/a1"
        ))
        .expect("spiffe"),
        addr: SocketAddr::from_str("10.9.9.9:8080").expect("addr"),
        weight: 100,
        healthy: true,
    }
}

fn proto_strategy() -> impl Strategy<Value = Proto> {
    prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
}

prop_compose! {
    fn listener_strategy()(port in 1u16..=65535, protocol in proto_strategy()) -> Listener {
        Listener { port: NonZeroU16::new(port).expect("port 1..=65535 is non-zero"), protocol }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// BE-1 — a `service_backends` row whose `ServiceId` has a keyed
    /// fact projects a desired carrying exactly that `(port, protocol)`.
    /// This is the post-switch equivalent of the pre-change projection
    /// (which sourced the same `(port, protocol)` from the cluster
    /// scan's `ListenerRow`).
    #[test]
    fn hydrate_desired_with_fact_matches_pre_change_projection(listener in listener_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().expect("rt");
        rt.block_on(async move {
            let tmp = TempDir::new().expect("tmpdir");
            let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
            let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

            let workload = "be1";
            let wid = WorkloadId::new(workload).expect("wid");
            let listeners = vec![listener];
            let vip = persist_and_allocate(&state, workload, &listeners).await;
            let vip_addr = vip.try_as_ipv4().expect("ipv4");

            {
                let mut facts = state.listener_facts.lock().await;
                facts.upsert(wid.clone(), &vip, &listeners);
            }

            let sid = ServiceId::derive(&vip, listener.port, listener.protocol, SERVICE_MAP_PURPOSE);
            let backends = vec![one_backend(workload)];
            let row = ServiceBackendRow {
                service_id: sid,
                vip: vip_addr,
                backends: backends.clone(),
                updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
            };
            obs.write(ObservationRow::ServiceBackend(row)).await.expect("write row");

            let target = TargetResource::new(&format!("service/{sid}")).expect("target");
            let hydrated =
                overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
                    &hydrator_reconciler(), &target, &state,
                ).await.expect("hydrate ok");
            let smh = match hydrated {
                AnyState::ServiceMapHydrator(s) => s,
                other => panic!("expected ServiceMapHydrator, got {other:?}"),
            };

            prop_assert_eq!(smh.desired.len(), 1, "exactly one service projected");
            let desired = smh.desired.get(&sid).expect("desired entry");
            prop_assert_eq!(desired.port, listener.port, "port from keyed fact");
            prop_assert_eq!(desired.proto, listener.protocol, "proto from keyed fact (C3)");
            prop_assert_eq!(desired.vip, vip, "vip matches allocator-issued");
            prop_assert_eq!(&desired.backends, &backends, "backends from the obs row");
            let expected_fp =
                overdrive_core::dataplane::fingerprint::fingerprint(&vip, &backends);
            prop_assert_eq!(desired.fingerprint, expected_fp, "fingerprint canonical");
            Ok(())
        })?;
    }
}

/// BE-2 — a `service_backends` row whose `ServiceId` has NO keyed fact
/// is SKIPPED: hydrate produces an EMPTY desired. Crucially no
/// silently-defaulted `Proto::Tcp` entry leaks — the C3 guard is
/// preserved verbatim across the read-path switch. Single-example
/// (the contract is the absence of an entry, not a quantified range).
#[tokio::test]
async fn hydrate_desired_unresolvable_proto_skips_and_emits_no_tcp_default() {
    let tmp = TempDir::new().expect("tmpdir");
    let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
    let state = build_app_state(&tmp, obs.clone() as Arc<dyn ObservationStore>);

    // Allocate a VIP + derive the ServiceId, write the service_backends
    // row — but DO NOT populate the listener_facts store. The keyed read
    // resolves None ⇒ the service must be skipped (no Tcp default).
    let workload = "be2";
    let listeners =
        vec![Listener { port: NonZeroU16::new(443).expect("nz"), protocol: Proto::Udp }];
    let vip = persist_and_allocate(&state, workload, &listeners).await;
    let vip_addr = vip.try_as_ipv4().expect("ipv4");
    let sid =
        ServiceId::derive(&vip, listeners[0].port, listeners[0].protocol, SERVICE_MAP_PURPOSE);
    let row = ServiceBackendRow {
        service_id: sid,
        vip: vip_addr,
        backends: vec![one_backend(workload)],
        updated_at: LogicalTimestamp { counter: 1, writer: node_id("writer-1") },
    };
    obs.write(ObservationRow::ServiceBackend(row)).await.expect("write row");

    let target = TargetResource::new(&format!("service/{sid}")).expect("target");
    let hydrated = overdrive_control_plane::reconciler_runtime::hydrate_desired_for_test(
        &hydrator_reconciler(),
        &target,
        &state,
    )
    .await
    .expect("hydrate ok");
    let smh = match hydrated {
        AnyState::ServiceMapHydrator(s) => s,
        other => panic!("expected ServiceMapHydrator, got {other:?}"),
    };

    assert!(
        smh.desired.is_empty(),
        "a row with no keyed listener fact must be skipped — never defaulted to Tcp (C3)"
    );
}

prop_compose! {
    fn listeners_strategy()(
        listeners in prop::collection::vec(listener_strategy(), 1..=3),
    ) -> Vec<Listener> {
        let mut seen = BTreeSet::new();
        listeners.into_iter().filter(|l| seen.insert(l.port)).collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// BE-3 — distinct service VIPs derive distinct `ServiceId`s with no
    /// collision: after upserting S services (each with its own
    /// allocator-issued VIP + L listeners), the keyed store's primary
    /// entry count equals the total listener count across all services.
    /// A `ServiceId` collision (two listeners deriving the same id)
    /// would shrink the primary count below the listener total.
    #[test]
    fn distinct_service_vips_derive_distinct_service_ids_no_collision(
        services in prop::collection::vec(listeners_strategy(), 1..=6),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().expect("rt");
        rt.block_on(async move {
            let tmp = TempDir::new().expect("tmpdir");
            let obs = Arc::new(SimObservationStore::single_peer(node_id("local"), 42));
            let state = build_app_state(&tmp, obs as Arc<dyn ObservationStore>);

            let mut total_listeners = 0usize;
            let mut all_ids: BTreeSet<ServiceId> = BTreeSet::new();
            for (si, listeners) in services.iter().enumerate() {
                let workload = format!("be3-{si}");
                let wid = WorkloadId::new(&workload).expect("wid");
                let vip = persist_and_allocate(&state, &workload, listeners).await;
                {
                    let mut facts = state.listener_facts.lock().await;
                    facts.upsert(wid, &vip, listeners);
                }
                for l in listeners {
                    all_ids.insert(ServiceId::derive(&vip, l.port, l.protocol, SERVICE_MAP_PURPOSE));
                    total_listeners += 1;
                }
            }

            // The allocator issues a distinct VIP per service, so every
            // (vip, port) pair is unique ⇒ no ServiceId collision.
            prop_assert_eq!(
                all_ids.len(),
                total_listeners,
                "distinct VIPs ⇒ distinct ServiceIds (no collision)"
            );

            // The keyed store holds exactly one primary entry per listener.
            let primary_count = {
                let facts = state.listener_facts.lock().await;
                let mut n = 0usize;
                for id in &all_ids {
                    if facts.fact_for(*id).is_some() {
                        n += 1;
                    }
                }
                n
            };
            prop_assert_eq!(
                primary_count,
                total_listeners,
                "primary entry count == total listener count"
            );
            Ok(())
        })?;
    }
}

// ===========================================================================
// ADR-0086 read-port injectability-edge DST scaffolds — S-ROH-B-05..B-08.
//
// Authored NOW as `#[ignore]`-blocked scaffolds (step 02-01). They are the
// net-new DST coverage ADR-0086 D8 unlocks: for the first time the hydration
// boundary is injectable, so a DST scenario can seed a stale/empty/absent/global
// read-port and drive the owning reconciler through an edge the concrete
// `AppState` field never allowed. B-06 (`ListenerFacts` miss → skip, never
// default `Proto::Tcp`) is the ListenerFacts-port restatement of BE-2 above,
// which is why the four anchor to this file; B-05 (`WorkflowLiveSet`), B-07
// (`ServiceVipView`) and B-08 (`HeldSvidView`) are colocated because all four
// share the SAME unblock — the 02-05 `Sim*` read-port impls.
//
// The real DST bodies inject `SimWorkflowLiveSet` / `SimListenerFacts` /
// `SimServiceVipView` / `SimHeldSvidView` through a `HydrationContext` — types
// that do not exist until step 02-05 (`overdrive-sim`). A body referencing them
// cannot compile at 02-01, so each scaffold carries the full port contract in
// its docstring and a `panic!` marker 02-05 replaces. Un-blocking is: drop the
// `#[ignore]`, write the DST body the docstring specifies. These are NOT a
// 02-01 pass/fail bar (they are `#[ignore]`d). Discover via
// `grep -rn 'RED scaffold' crates/overdrive-control-plane/tests/`.
// ===========================================================================

/// S-ROH-B-05 — Injectable crash-resume: empty/stale `SimWorkflowLiveSet`
/// triggers convergence (ADR-0086 D5 `WorkflowLiveSet` edge + D8).
///
/// **CONTRACT_SHAPE: Tier-1 DST edge (injectability WIN).**
///
/// `WorkflowLiveSet::live_instances()` returns a point-in-time snapshot of the
/// engine's live-task correlation keys — ephemeral runtime state, NOT
/// intent/observation. An EMPTY set after a restart is LEGITIMATE, not an error
/// (ADR-0064 §5): an instance running-in-intent, with no live task and no
/// terminal observation row, IS the crash-resume trigger.
///
/// ```text
/// GIVEN a SimWorkflowLiveSet returning an EMPTY live-instance set
///   AND a workflow instance running-in-intent with no terminal observation row
/// WHEN WorkflowLifecycle hydrates via HydrationContext and reconciles
/// THEN the empty set is treated as legitimate (not an error)
///   AND running-in-intent + no-live-task + no-terminal ⇒ re-emit StartWorkflow
///   AND this DST case was impossible under the pre-move concrete AppState
/// ```
#[tokio::test]
#[ignore = "blocked on 02-05 Sim read-ports (SimWorkflowLiveSet); un-ignore + inject once they land"]
async fn empty_workflow_live_set_triggers_crash_resume_start_workflow() {
    // 02-05 replaces this body: seed an EMPTY SimWorkflowLiveSet on the
    // HydrationContext, hydrate + reconcile WorkflowLifecycle, assert a single
    // Action::StartWorkflow is re-emitted for the running-in-intent instance.
    panic!(
        "RED scaffold: S-ROH-B-05 empty SimWorkflowLiveSet crash-resume \
         (blocked on 02-05 Sim read-ports)"
    );
}

/// S-ROH-B-06 — Injectable `ListenerFacts` miss: hydrator SKIPS, never defaults
/// `Proto::Tcp` (ADR-0086 D5 `ListenerFacts` edge, ADR-0060 C3).
///
/// **CONTRACT_SHAPE: Tier-1 DST edge / error path.**
///
/// `ListenerFacts::fact_for(service_id)` returns the boot-rebuilt +
/// edge-maintained listener fact, or `None` when unknown. `None` MUST cause the
/// hydrator to SKIP the service — it may NEVER default the protocol to
/// `Proto::Tcp` (ADR-0060 C3). This is the port-injectable restatement of BE-2
/// above (which pins the same skip via the concrete `ListenerFactStore`).
///
/// ```text
/// GIVEN a SimListenerFacts returning None for a given ServiceId
/// WHEN the service-map hydrator hydrates its State via ListenerFacts::fact_for
/// THEN the service is SKIPPED (no listener fact), NEVER defaulting to Proto::Tcp
///   AND a subsequent seeding of the fact makes the same service hydrate normally
/// ```
#[tokio::test]
#[ignore = "blocked on 02-05 Sim read-ports (SimListenerFacts); un-ignore + inject once they land"]
async fn listener_facts_miss_skips_service_never_defaults_tcp_via_port() {
    // 02-05 replaces this body: seed SimListenerFacts to return None for a
    // ServiceId, hydrate the ServiceMapHydrator State via the port, assert the
    // service is absent (skipped) and NO update carries a defaulted Proto::Tcp;
    // then seed the fact and assert the service hydrates.
    panic!(
        "RED scaffold: S-ROH-B-06 SimListenerFacts miss skips service, never \
         defaults Proto::Tcp (blocked on 02-05 Sim read-ports)"
    );
}

/// S-ROH-B-07 — Injectable `ServiceVipView` memo-absent: defer the tick, log
/// `allocator_memo_absent` (ADR-0086 D5 `ServiceVipView` edge, ADR-0049 §4).
///
/// **CONTRACT_SHAPE: Tier-1 DST edge / error path.**
///
/// `ServiceVipView::assigned_vip(spec_digest)` returns the allocator-issued VIP
/// for the content-addressed spec digest, or `None` when no VIP is memoised.
/// `None` on a persisted Service intent is the ADR-0049 §4 structural-invariant
/// violation signal: DEFER the tick (do not hydrate the service, emit no Action,
/// never panic, never default a VIP) and log `allocator_memo_absent`. The adapter
/// maps the core `ContentHash` to the allocator's `ServiceSpecDigest`.
///
/// ```text
/// GIVEN a persisted Service intent whose spec digest has no memoised VIP
///   AND a SimServiceVipView returning None for that ContentHash
/// WHEN the hydrator hydrates via ServiceVipView::assigned_vip
/// THEN (PRIMARY) the tick is DEFERRED: no State hydrated, no Action emitted
///   AND (secondary) `allocator_memo_absent` is logged (a supporting check)
/// ```
#[tokio::test]
#[ignore = "blocked on 02-05 Sim read-ports (SimServiceVipView); un-ignore + inject once they land"]
async fn service_vip_view_memo_absent_defers_tick_and_logs() {
    // 02-05 replaces this body: seed SimServiceVipView to return None for the
    // Service intent's ContentHash, hydrate, assert NO State for the service and
    // NO emitted Action (deferred), plus the `allocator_memo_absent` log signal.
    panic!(
        "RED scaffold: S-ROH-B-07 SimServiceVipView memo-absent defers tick \
         (blocked on 02-05 Sim read-ports)"
    );
}

/// S-ROH-B-08 — `HeldSvidView` returns the GLOBAL set; the hydrator filters to
/// the target workload (ADR-0086 D5 `HeldSvidView` edge, ADR-0067 D5b).
///
/// **CONTRACT_SHAPE: Tier-1 DST edge / equivalence.**
///
/// `HeldSvidView::held_snapshot()` returns the GLOBAL node-held SVID map (every
/// workload's held leaves), keyed by `AllocationId`; presence == "held". The
/// trait returns the UNFILTERED global set by contract; filtering to the target
/// workload by `SpiffeId::for_allocation` equality is the HYDRATOR's job
/// (ADR-0067 D5b).
///
/// ```text
/// GIVEN a SimHeldSvidView returning the unfiltered GLOBAL node-held SVID map
///       (several workloads present)
/// WHEN the svid-lifecycle reconciler hydrates its State
/// THEN the hydrator filters the global set to the TARGET workload by
///      SpiffeId::for_allocation equality
///   AND presence in the (filtered) set means "held"
/// ```
#[tokio::test]
#[ignore = "blocked on 02-05 Sim read-ports (SimHeldSvidView); un-ignore + inject once they land"]
async fn held_svid_view_global_set_is_filtered_to_target_workload() {
    // 02-05 replaces this body: seed SimHeldSvidView with a multi-workload
    // global held map, hydrate the SvidLifecycle State for one target, assert
    // `actual` reflects ONLY the target workload's held facts.
    panic!(
        "RED scaffold: S-ROH-B-08 SimHeldSvidView global set filtered to target \
         (blocked on 02-05 Sim read-ports)"
    );
}
