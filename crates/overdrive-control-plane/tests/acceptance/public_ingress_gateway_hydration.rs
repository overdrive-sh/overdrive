//! Public-ingress gateway — the identity hydration boundary (S-PIG-65,
//! hydration half; ADR-0225, ADR-0086).
//!
//! `GatewaySvidLifecycle` owns its hydration: it reads the checked desired
//! epoch and the held non-secret facts through the one `GatewayIdentityView`
//! read-port that `HydrationContext.gateway_identity` lends, and derives
//! `ever_issued` only from the existing issued-certificate audit rows that name
//! the gateway SPIFFE subject. A disabled `serve` supplies the canonical empty
//! read (epoch one, no subject, no current facts).
//!
//! Driving port: `AnyReconciler::hydrate_desired` / `hydrate_actual`, the
//! reconciler's own hydration entry (the boundary the runtime calls each tick).
//! The identity view is a test-local `GatewayIdentityView`; the audit rows are
//! written through the production `issue_and_audit` path, so the oracle reads
//! rows production authored.
//!
//! The reconciler's `hydrate_*` bodies are DELIVER's; the bodies below are
//! complete and run against the landed hydration. The disabled-composition
//! body runs against the existing total composition.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::panic,
    reason = "CONTRACT_SHAPE is required scenario metadata; fixture preconditions panic with their cause"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::GatewayAppStateComposition;
use overdrive_control_plane::ca_issuance::issue_and_audit;
use overdrive_core::gateway_identity::{
    GatewayIdentityDesired, GatewayIdentityEpoch, GatewayIdentityFacts, GatewayIdentityView,
};
use overdrive_core::guest_network::GuestAttachmentOccupancy;
use overdrive_core::id::{CertSerial, NodeId, SpiffeId};
use overdrive_core::reconcilers::{HydrationContext, TargetResource};
use overdrive_core::traits::driver::DriverRegistry;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::{AnyReconciler, AnyState, GatewaySvidLifecycle};
use overdrive_sim::adapters::SimGuestAttachmentView;
use overdrive_sim::adapters::ca::SimCa;
use overdrive_sim::adapters::clock::SimClock;
use overdrive_sim::adapters::entropy::SimEntropy;
use overdrive_sim::adapters::observation_store::SimObservationStore;
use overdrive_sim::adapters::read_ports::{
    SimHeldSvidView, SimListenerFacts, SimServiceVipView, SimWorkflowLiveSet,
};
use overdrive_sim::adapters::vm_host_state::SimVmHostState;
use overdrive_store_local::LocalIntentStore;
use tempfile::TempDir;

fn node() -> NodeId {
    NodeId::from_str("gw-node").expect("fixture node id")
}

/// A fixed identity read: what the slot (or the disabled composition) answers.
struct FixedIdentityView {
    desired: GatewayIdentityDesired,
    current: Option<GatewayIdentityFacts>,
}

impl GatewayIdentityView for FixedIdentityView {
    fn desired(&self) -> GatewayIdentityDesired {
        self.desired.clone()
    }

    fn current(&self) -> Option<GatewayIdentityFacts> {
        self.current.clone()
    }
}

fn held_facts(epoch: GatewayIdentityEpoch, subject: SpiffeId) -> GatewayIdentityFacts {
    GatewayIdentityFacts {
        epoch,
        spiffe_id: subject,
        serial: CertSerial::new("0a").expect("fixture serial"),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(7_200)),
    }
}

/// The world a hydration reads: stores and the borrowed read-ports.
struct World {
    _dir: TempDir,
    intent_path: std::path::PathBuf,
    intent: LocalIntentStore,
    observation: SimObservationStore,
    drivers: DriverRegistry,
    vm_host: SimVmHostState,
    listener_facts: SimListenerFacts,
    service_vips: SimServiceVipView,
    workflows: SimWorkflowLiveSet,
    held_svids: SimHeldSvidView,
    guest_attachments: SimGuestAttachmentView,
    node_id: NodeId,
    /// One CA for the whole world, so successive issuances draw successive
    /// serials from its entropy stream as production does; the audit index is
    /// keyed on the serial.
    ca: SimCa,
}

impl World {
    fn new() -> Self {
        let dir = TempDir::new().expect("intent store directory");
        let intent_path = dir.path().join("intent.redb");
        let intent = LocalIntentStore::open(&intent_path).expect("LocalIntentStore::open");
        Self {
            _dir: dir,
            intent_path,
            intent,
            observation: SimObservationStore::single_peer(node(), 0x61),
            drivers: DriverRegistry::new(),
            vm_host: SimVmHostState::new(),
            listener_facts: SimListenerFacts::new(BTreeMap::new()),
            service_vips: SimServiceVipView::new(BTreeMap::new()),
            workflows: SimWorkflowLiveSet::new(BTreeSet::new()),
            held_svids: SimHeldSvidView::new(BTreeMap::new()),
            guest_attachments: SimGuestAttachmentView::new(
                GuestAttachmentOccupancy { held: 0, retiring: 0 },
                BTreeMap::new(),
            ),
            node_id: node(),
            ca: SimCa::new(Arc::new(SimEntropy::new(0x61))),
        }
    }

    fn context<'a>(&'a self, identity: &'a dyn GatewayIdentityView) -> HydrationContext<'a> {
        HydrationContext {
            intent_store: &self.intent,
            observation_store: &self.observation,
            drivers: &self.drivers,
            vm_host_state: &self.vm_host,
            listener_facts: &self.listener_facts,
            service_vip_view: &self.service_vips,
            workflow_live_set: &self.workflows,
            held_svid_view: &self.held_svids,
            guest_attachments: &self.guest_attachments,
            gateway_identity: identity,
            node_id: &self.node_id,
            host_ipv4: std::net::Ipv4Addr::LOCALHOST,
            intent_redb_path: &self.intent_path,
        }
    }

    /// Write one issued-certificate audit row for `subject` through the
    /// production issuance path.
    async fn audit_issuance(&self, subject: &SpiffeId) {
        issue_and_audit(&self.ca, &self.observation, &SimClock::new(), &self.node_id, subject)
            .await
            .expect("issue_and_audit writes the audit row");
    }
}

fn target(raw: &str) -> TargetResource {
    TargetResource::new(raw).expect("fixture target")
}

fn reconciler() -> AnyReconciler {
    AnyReconciler::GatewaySvidLifecycle(GatewaySvidLifecycle::canonical())
}

fn gateway_state(state: AnyState) -> overdrive_reconcilers::GatewaySvidLifecycleState {
    match state {
        AnyState::GatewaySvidLifecycle(state) => state,
        other => panic!("the gateway reconciler hydrates its own state variant, got {other:?}"),
    }
}

/// S-PIG-65 (disabled `serve`): the total composition's identity read is the
/// canonical empty one. No existing non-gateway caller changes: epoch one, no
/// subject, never any current facts, and no control handle.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[test]
fn disabled_composition_supplies_the_canonical_empty_identity_read() {
    let disabled = GatewayAppStateComposition::disabled();
    let view = disabled.identity_view();

    assert_eq!(
        view.desired(),
        GatewayIdentityDesired { epoch: GatewayIdentityEpoch::first(), spiffe_id: None },
        "a disabled gateway desires epoch one with no subject"
    );
    assert_eq!(view.current(), None, "a disabled gateway never holds an identity");
    assert!(disabled.routes().is_none(), "a disabled gateway has no Route command port");
    assert!(disabled.status().is_none(), "a disabled gateway has no status read port");
}

/// S-PIG-65: hydration reads the desired epoch and subject from the identity
/// view for `desired`, and the held non-secret facts for `actual`, for an
/// enabled gateway on its own node target.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn hydration_projects_the_desired_epoch_and_the_held_facts_of_the_identity_view() {
    let world = World::new();
    let epoch = GatewayIdentityEpoch::first();
    let subject = SpiffeId::for_gateway(&node());
    let held = held_facts(epoch, subject.clone());
    let view = FixedIdentityView {
        desired: GatewayIdentityDesired { epoch, spiffe_id: Some(subject) },
        current: Some(held.clone()),
    };
    let ctx = world.context(&view);
    let node_target = target("node/gw-node");

    let desired = gateway_state(
        reconciler().hydrate_desired(&ctx, &node_target).await.expect("hydrate_desired"),
    );
    let actual = gateway_state(
        reconciler().hydrate_actual(&ctx, &node_target).await.expect("hydrate_actual"),
    );

    assert_eq!(desired.desired, view.desired(), "desired is the view's checked epoch and subject");
    assert_eq!(actual.actual, Some(held), "actual is the view's held non-secret facts");
}

/// S-PIG-65 (empty slot): with nothing held, `actual` carries no facts and the
/// desired identity is still projected.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn hydration_of_an_empty_slot_projects_no_held_facts() {
    let world = World::new();
    let view = FixedIdentityView {
        desired: GatewayIdentityDesired {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: Some(SpiffeId::for_gateway(&node())),
        },
        current: None,
    };
    let ctx = world.context(&view);
    let node_target = target("node/gw-node");

    let actual = gateway_state(
        reconciler().hydrate_actual(&ctx, &node_target).await.expect("hydrate_actual"),
    );

    assert_eq!(actual.actual, None, "an empty slot hydrates no held facts");
}

/// S-PIG-65 (restart marker): `ever_issued` is derived only from the audit rows
/// that name the gateway SPIFFE subject. No rows, and rows for another subject,
/// leave it false; a gateway issuance row sets it, so a restart (empty volatile
/// slot, durable rows) reissues immediately.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn ever_issued_is_derived_only_from_audit_rows_of_the_gateway_subject() {
    let world = World::new();
    let gateway = SpiffeId::for_gateway(&node());
    let view = FixedIdentityView {
        desired: GatewayIdentityDesired {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: Some(gateway.clone()),
        },
        current: None,
    };
    let node_target = target("node/gw-node");
    let ever_issued = || async {
        let ctx = world.context(&view);
        gateway_state(
            reconciler().hydrate_actual(&ctx, &node_target).await.expect("hydrate_actual"),
        )
        .ever_issued
    };

    assert!(!ever_issued().await, "no audit rows: never issued");

    let workload = SpiffeId::from_str("spiffe://overdrive.local/workload/payments/alloc/a1")
        .expect("fixture workload SPIFFE id");
    world.audit_issuance(&workload).await;
    assert!(!ever_issued().await, "another subject's row is not the gateway's restart marker");

    world.audit_issuance(&gateway).await;
    assert!(ever_issued().await, "a gateway issuance row marks that the gateway was issued");
}

/// S-PIG-65 (target shape): a target that is not the configured
/// `node/<node-id>` is refused by both hydration entries; neither reads the
/// identity view or the audit rows for a stranger's target.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test]
async fn a_target_other_than_the_configured_node_is_refused_by_both_hydration_entries() {
    let world = World::new();
    let view = FixedIdentityView {
        desired: GatewayIdentityDesired {
            epoch: GatewayIdentityEpoch::first(),
            spiffe_id: Some(SpiffeId::for_gateway(&node())),
        },
        current: None,
    };
    let ctx = world.context(&view);

    for stranger in ["node/other-node", "workload/payments"] {
        let stranger = target(stranger);
        assert!(
            reconciler().hydrate_desired(&ctx, &stranger).await.is_err(),
            "hydrate_desired refuses {stranger}"
        );
        assert!(
            reconciler().hydrate_actual(&ctx, &stranger).await.is_err(),
            "hydrate_actual refuses {stranger}"
        );
    }
}

/// S-PIG-65 (disabled desired): a disabled gateway (no desired subject) hydrates
/// the epoch with no subject, so the reconciler has nothing to issue.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test]
async fn a_disabled_desired_identity_hydrates_the_epoch_with_no_subject() {
    let world = World::new();
    let epoch = GatewayIdentityEpoch::new(2).expect("epoch two is nonzero");
    let view = FixedIdentityView {
        desired: GatewayIdentityDesired { epoch, spiffe_id: None },
        current: None,
    };
    let ctx = world.context(&view);
    let node_target = target("node/gw-node");

    let desired = gateway_state(
        reconciler().hydrate_desired(&ctx, &node_target).await.expect("hydrate_desired"),
    );

    assert_eq!(desired.desired.epoch, epoch);
    assert_eq!(desired.desired.spiffe_id, None, "disable hydrates no subject");
}
