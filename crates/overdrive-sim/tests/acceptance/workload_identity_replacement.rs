//! A replacement allocation of a workload is accepted by the dataplane, and a
//! different workload at a still-memoised endpoint takes over its `BackendId`
//! (workload-spiffe-identity D4 / DR-1(b), ADR-0347; GH regression of the
//! public ingress gateway 502 after a `serve` restart).
//!
//! Two layers, both in-memory (`SimDataplane`, `SimGatewayConnectDataplane`):
//!
//! 1. The one `Dataplane::update_service` contract sequence of the approved
//!    host-vs-Sim equivalence case whose Sim half has no other default-lane test:
//!    (c) one call naming an endpoint twice is `LoadFailed`. Sequences (a) (same
//!    workload, replacement allocation: nothing changes) and (b) (a different
//!    workload: same `BackendId`, association follows) are exercised by the seeded
//!    schedules below and, on both adapters, by
//!    `crates/overdrive-dataplane/tests/integration/gateway_connect_dataplane_equivalence.rs`.
//! 2. The seeded interleaving invariant through the REAL `ServiceMapHydrator`
//!    plus `SimDataplane`: a `serve` restart (the dataplane, its memo and its
//!    receipts rebuilt) before the replacement, the replacement being another
//!    allocation of the same workload or a different workload, with idle ticks
//!    interleaved. The oracle is the dataplane's own accessors: the programmed
//!    fingerprint and `selected_backend_identity`. Every failure names the seed.
//!
//! The backend identity is built the way production builds it: from the
//! `AllocationSpec` the workload lifecycle produces for a live intent
//! (`allocation_spec_for_live_intent`). Today that is a per-allocation
//! identity, so two allocations of one workload differ at one endpoint and the
//! dataplane refuses the replacement: RED. After the feature it is the workload
//! identity and the same bodies pass unchanged.
//!
//! CONTRACT_SHAPE: bounded-change.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::{Duration, Instant};

use overdrive_core::aggregate::{
    DriverInput, Job, JobSpecInput, ResourcesInput, VmInput, WorkloadIntent,
};
use overdrive_core::byte_size::ByteSize;
use overdrive_core::cpu_quantity::CpuQuantity;
use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::dataplane::fingerprint::{BackendSetFingerprint, fingerprint};
use overdrive_core::id::{AllocationId, BackendId, ServiceId, ServiceVip, SpiffeId};
use overdrive_core::public_ingress::ServiceKey;
use overdrive_core::reconcilers::{Action, TickContext};
use overdrive_core::traits::ProgrammedServiceView;
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_gateway::ports::{
    GatewayConnectCleanup, GatewayConnectDataplane, GatewaySelectionReceipt,
};
use overdrive_reconcilers::workload_lifecycle::allocation_spec_for_live_intent;
use overdrive_reconcilers::{
    AnyReconciler, AnyReconcilerView, AnyState, ServiceDesired, ServiceMapHydrator,
    ServiceMapHydratorState, ServiceMapHydratorView,
};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::gateway::SimGatewayConnectDataplane;
use proptest::prelude::*;
use rand::{Rng, SeedableRng, rngs::StdRng};

use overdrive_testing::placement;

placement!(host_pure);

/// The convergence budget in hydrator ticks. A replacement converges in one.
const TICK_BUDGET: u32 = 8;
const BACKEND_PORT: u16 = 8080;
const FRONTEND_PORT: u16 = 8080;

fn intent_of(workload: &str) -> WorkloadIntent {
    WorkloadIntent::Job(
        Job::from_submit(JobSpecInput {
            id: workload.to_owned(),
            replicas: 1,
            resources: ResourcesInput {
                cpu: CpuQuantity::from_millis(500),
                memory: ByteSize::new(134_217_728),
            },
            driver: DriverInput::Vm(VmInput {
                volume: None,
                command: "/usr/local/bin/app".to_owned(),
                args: vec![],
                kernel: "/kernel".to_owned(),
                rootfs: "/rootfs".to_owned(),
            }),
        })
        .expect("a valid job"),
    )
}

/// The identity production gives allocation `alloc` of `intent`.
fn identity_of(intent: &WorkloadIntent, alloc: &str) -> SpiffeId {
    let alloc = AllocationId::new(alloc).expect("valid allocation id");
    allocation_spec_for_live_intent(intent, &alloc).expect("a job has a live allocation").identity
}

/// One backend at `endpoint` presenting `identity`. The single place the
/// `Backend` field holding the identity is named.
fn backend_at(identity: &SpiffeId, endpoint: u8) -> Backend {
    Backend {
        workload_identity: identity.clone(),
        addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 96, 0, endpoint)), BACKEND_PORT),
        weight: 1,
        healthy: true,
    }
}

fn frontend() -> ServiceFrontend {
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("IPv4 VIP");
    ServiceFrontend::new(vip, NonZeroU16::new(FRONTEND_PORT).expect("non-zero"), Proto::Tcp)
        .expect("IPv4 frontend")
}

struct World {
    dataplane: Arc<SimDataplane>,
    gateway: SimGatewayConnectDataplane,
}

impl World {
    fn new() -> Self {
        let dataplane = Arc::new(SimDataplane::new());
        let gateway = SimGatewayConnectDataplane::new(Arc::clone(&dataplane));
        Self { dataplane, gateway }
    }

    /// What the gateway reads for the Service now: the selected `BackendId` and
    /// the identity the dataplane resolves it to.
    fn selection(&self) -> Option<(BackendId, SpiffeId)> {
        let _runtime = tokio::runtime::Handle::current().enter();
        let (_socket, intent) = self
            .gateway
            .open_upstream(ServiceKey::from_frontend(frontend()))
            .expect("open_upstream")
            .into_parts();
        let receipt = self.gateway.take_receipt(&intent).expect("the receipt of this socket");
        let selected = match receipt {
            GatewaySelectionReceipt::Selected { backend_id, .. } => Some((
                backend_id,
                self.gateway.selected_backend_identity(backend_id).expect("an applied identity"),
            )),
            GatewaySelectionReceipt::NoBackend { .. } => None,
        };
        self.gateway.cleanup(&intent).expect("cleanup");
        selected
    }
}

// ---------------------------------------------------------------------------
// 1. The Dataplane::update_service contract sequences (Sim half of the
//    approved host-vs-Sim equivalence case).
// ---------------------------------------------------------------------------

/// (c) One `update_service` call naming one endpoint twice is `LoadFailed`
/// regardless of identity (the same workload twice, or two workloads), and
/// changes nothing.
#[tokio::test]
async fn one_call_naming_an_endpoint_twice_is_refused_and_changes_nothing() {
    let (payments, inventory) = (intent_of("payments"), intent_of("inventory"));
    let first = identity_of(&payments, "alloc-payments-0");
    let other = identity_of(&inventory, "alloc-inventory-0");
    let world = World::new();
    world
        .dataplane
        .update_service(frontend(), vec![backend_at(&first, 50)])
        .await
        .expect("programmed");
    let before = (
        world.selection(),
        world.dataplane.programmed_fingerprint(&frontend()).await.expect("read"),
    );

    for (label, backends) in [
        ("the same workload twice", vec![backend_at(&first, 50), backend_at(&first, 50)]),
        ("two workloads", vec![backend_at(&first, 50), backend_at(&other, 50)]),
    ] {
        let outcome = world.dataplane.update_service(frontend(), backends).await;
        assert!(
            matches!(outcome, Err(DataplaneError::LoadFailed(_))),
            "{label} at one endpoint must be LoadFailed, got {outcome:?}"
        );
        let after = (
            world.selection(),
            world.dataplane.programmed_fingerprint(&frontend()).await.expect("read"),
        );
        assert_eq!(after, before, "{label}: a refused call changes nothing");
    }
}

// ---------------------------------------------------------------------------
// 2. The seeded interleaving invariant through ServiceMapHydrator.
// ---------------------------------------------------------------------------

fn make_tick(index: u32) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(u64::from(index) * 60)),
        tick: u64::from(index),
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

/// Drive the REAL hydrator against `world` until the Service is programmed with
/// `backends` (the hydrator's own `actual` read back from the dataplane through
/// `ProgrammedServiceView`, the shim's write modelled by `update_service`).
/// Fails on any refusal, on a refusal that changed the programmed fingerprint, or
/// on a budget overrun; `idle_ticks` extra ticks run first against the new
/// desired state with nothing dispatched (the decision is observed before the shim
/// runs).
async fn converge(
    world: &World,
    backends: &[Backend],
    idle_ticks: u32,
) -> Result<BackendSetFingerprint, String> {
    let service_id = ServiceId::new(42).expect("ServiceId");
    let frontend = frontend();
    let vip = ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("VIP");
    let desired_fp = fingerprint(&vip, backends);
    let reconciler = AnyReconciler::ServiceMapHydrator(ServiceMapHydrator::canonical(
        overdrive_control_plane::veth_provisioner::WORKLOAD_SUBNET_BASE,
    ));
    let mut view = ServiceMapHydratorView::default();
    let desired = BTreeMap::from([(
        service_id,
        ServiceDesired {
            vip,
            port: NonZeroU16::new(FRONTEND_PORT).expect("non-zero"),
            proto: Proto::Tcp,
            backends: backends.to_vec(),
            fingerprint: desired_fp,
        },
    )]);

    for index in 0..TICK_BUDGET + idle_ticks {
        let mut actual = BTreeMap::new();
        if let Some(programmed) =
            world.dataplane.programmed_fingerprint(&frontend).await.map_err(|e| e.to_string())?
        {
            actual.insert(service_id, programmed);
        }
        let state = ServiceMapHydratorState { desired: desired.clone(), actual };
        let (actions, next_view) = reconciler.reconcile(
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyState::ServiceMapHydrator(state.clone()),
            &AnyReconcilerView::ServiceMapHydrator(view.clone()),
            &make_tick(index),
        );
        let AnyReconcilerView::ServiceMapHydrator(next_view) = next_view else {
            return Err("the hydrator returned another reconciler's view".to_owned());
        };
        view = next_view;
        if actions.is_empty() && state.actual.get(&service_id) == Some(&desired_fp) {
            return Ok(desired_fp);
        }
        if index < idle_ticks {
            // An idle tick: the decision is observed, the shim has not run yet.
            continue;
        }
        for action in actions {
            if let Action::DataplaneUpdateService { backends, .. } = action {
                let before = state.actual.get(&service_id).copied();
                if let Err(error) = world.dataplane.update_service(frontend, backends).await {
                    let after = world
                        .dataplane
                        .programmed_fingerprint(&frontend)
                        .await
                        .map_err(|e| e.to_string())?;
                    if after != before {
                        return Err(format!(
                            "a refused update changed the programmed fingerprint: {before:?} -> {after:?}"
                        ));
                    }
                    return Err(format!("the dataplane refused the update: {error}"));
                }
            }
        }
    }
    Err(format!("did not converge within {TICK_BUDGET} ticks"))
}

/// One seeded schedule. Returns a description of the first violation.
async fn run_schedule(seed: u64) -> Result<(), String> {
    let mut rng = StdRng::seed_from_u64(seed);
    let same_workload = rng.gen_bool(0.5);
    let restart_first = rng.gen_bool(0.5);
    let idle_ticks = rng.gen_range(0..=2_u32);
    let endpoint = rng.gen_range(10..=200_u8);

    let payments = intent_of("payments");
    let original = identity_of(&payments, "alloc-payments-0");
    let (replacement_intent, replacement_alloc) = if same_workload {
        (payments, "alloc-payments-1")
    } else {
        (intent_of("inventory"), "alloc-inventory-0")
    };
    let replacement = identity_of(&replacement_intent, replacement_alloc);

    let mut world = World::new();
    converge(&world, &[backend_at(&original, endpoint)], 0).await?;
    let (id_before, _) = world.selection().ok_or("the first generation selects its backend")?;

    if restart_first {
        // A `serve` restart: the dataplane, its memo and its receipts are rebuilt;
        // the boot resync programs the stored row again.
        world = World::new();
        converge(&world, &[backend_at(&original, endpoint)], 0).await?;
    }
    let (id_boot, _) = world.selection().ok_or("the boot resync selects its backend")?;

    let desired = [backend_at(&replacement, endpoint)];
    let programmed = converge(&world, &desired, idle_ticks).await?;

    let (id_after, identity_after) =
        world.selection().ok_or("the replacement generation selects its backend")?;
    if programmed
        != fingerprint(
            &ServiceVip::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))).expect("VIP"),
            &desired,
        )
    {
        return Err("the programmed fingerprint is not the desired one".to_owned());
    }
    if id_after != id_boot {
        return Err(format!("the endpoint changed BackendId: {id_boot} -> {id_after}"));
    }
    if !restart_first && id_before != id_after {
        return Err(format!("the endpoint changed BackendId: {id_before} -> {id_after}"));
    }
    if identity_after != replacement {
        return Err(format!(
            "the association is {identity_after}, not the programmed {replacement}"
        ));
    }
    Ok(())
}

proptest! {
    // The seed is the whole schedule; a failure names it and proptest minimises
    // it. Persistence is off so a pending (RED) property writes no seed file.
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    /// Replacing an allocation at the same endpoint, across a serve restart and
    /// any idle-tick interleaving, converges without refusal: the endpoint keeps
    /// its `BackendId` and the association equals the workload now programmed
    /// there (the same workload: nothing changes; a different workload: the
    /// association follows).
    #[test]
    fn a_replacement_converges_without_refusal_across_restarts_and_schedules(seed in any::<u64>()) {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
        let outcome = runtime.block_on(run_schedule(seed));
        prop_assert!(
            outcome.is_ok(),
            "seed {}: replacement must converge without refusal: {:?}",
            seed,
            outcome
        );
    }
}
