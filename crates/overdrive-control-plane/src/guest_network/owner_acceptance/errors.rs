//! C6c — the owner's error set is closed: every owner failure path returns
//! exactly one variant of § *Typed error taxonomy and who branches*, the
//! forwarder's failures carried as `Forwarder { source: ForwarderError }`
//! (one `#[from]` variant, never flattened); the boot-time failures each
//! refuse at their own operation, before the effects that follow them.
//!
//! Also the GW-LOCAL boot cells (ADR-0152, DELIVER step 13): `converge_shared`
//! converges the one Overdrive-tagged gateway `/32` route after the
//! forwarder context and the listeners, idempotently and over a route a
//! previous boot left behind (H-4); `sweep_stale` never reports it; a failed
//! write refuses boot with `Netlink { LocalRouteConverge }`.
//!
//! The closure is structural: [`taxonomy_name`] matches `GuestNetworkError`
//! exhaustively with no wildcard, so a variant added outside the taxonomy
//! fails to compile here until the taxonomy names it.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::forwarder::{
    ForwarderError, ForwarderProbeObservation, ForwarderProbeStage,
};
use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX};
use overdrive_core::net::InterfaceIndex;
use overdrive_core::traits::mtls_resolve::MtlsResolution;
use overdrive_sim::adapters::SimMtlsResolve;

use super::support::{Composition, vip_range};
use crate::guest_network::flow_io::sim::{SimEffectKind, SimFailure, SimForwarderFailure};
use crate::guest_network::{
    GuestNetworkError, GuestNetworkOperation, GuestNetworkProvisioner, SharedGuestNetworkOwner,
};

/// The taxonomy's name for `error`; exhaustive, no wildcard (C6c).
const fn taxonomy_name(error: &GuestNetworkError) -> &'static str {
    match error {
        GuestNetworkError::PoolExhausted { .. } => "PoolExhausted",
        GuestNetworkError::ExecGateNotBootClosed => "ExecGateNotBootClosed",
        GuestNetworkError::Netlink { .. } => "Netlink",
        GuestNetworkError::Forwarder { .. } => "Forwarder",
        GuestNetworkError::Wire { .. } => "Wire",
        GuestNetworkError::Io { .. } => "Io",
        GuestNetworkError::PostconditionMismatch { .. } => "PostconditionMismatch",
        GuestNetworkError::AdmissionCapReached { .. } => "AdmissionCapReached",
        GuestNetworkError::LeaseRetiring { .. } => "LeaseRetiring",
        GuestNetworkError::GuestPrefixOverlapsNodeAddress { .. } => {
            "GuestPrefixOverlapsNodeAddress"
        }
        GuestNetworkError::GuestPrefixOverlapsVipRange { .. } => "GuestPrefixOverlapsVipRange",
        GuestNetworkError::FdBudget { .. } => "FdBudget",
        GuestNetworkError::OwnerCgroupPlacement { .. } => "OwnerCgroupPlacement",
    }
}

fn inside_prefix() -> Ipv4Net {
    Ipv4Net::new(GUEST_NODE_PREFIX.network(), 24).expect("a /24 inside the prefix")
}

fn non_mesh() -> SimMtlsResolve {
    SimMtlsResolve::new(std::collections::BTreeMap::new(), MtlsResolution::NonMesh)
}

/// The boot sequence of § *Composition* on the owner, stopping at the first
/// refusal.
async fn boot_until_refused(c: &Composition) -> Result<(), GuestNetworkError> {
    c.owner.probe_startup().await?;
    c.owner.sweep_stale().await?;
    c.owner.converge_shared().await
}

/// Boot effects a refused boot step must not have issued after it.
fn listener_or_route_effects(c: &Composition) -> usize {
    c.io.issued()
        .into_iter()
        .filter(|effect| {
            matches!(effect.kind, SimEffectKind::BindListener | SimEffectKind::GatewayRoute)
        })
        .count()
}

/// `probe_startup` refusals (D8-PREFIX-VIP checked first, D8-PREFIX-CFG,
/// fd budget, D24 placement, an unreadable fact) each return their own
/// variant, change nothing and issue no boot effect.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn every_probe_startup_refusal_is_its_own_variant_and_changes_nothing() {
    let node_address = |c: &Composition| {
        c.io.update_host_facts(|facts| {
            facts.interfaces.insert(
                InterfaceIndex::new(2).expect("non-zero"),
                BTreeSet::from([inside_prefix()]),
            );
        });
    };

    let both = Composition::with(non_mesh(), inside_prefix(), 1);
    node_address(&both);
    let outcome = both.owner.probe_startup().await;
    assert!(
        matches!(outcome, Err(GuestNetworkError::GuestPrefixOverlapsVipRange { prefix, vip_range })
            if prefix == GUEST_NODE_PREFIX && vip_range == inside_prefix()),
        "the VIP range is checked first: {outcome:?}"
    );

    let address = Composition::with(non_mesh(), vip_range(), 2);
    node_address(&address);
    assert!(matches!(
        address.owner.probe_startup().await,
        Err(GuestNetworkError::GuestPrefixOverlapsNodeAddress { address, .. }) if address == inside_prefix()
    ));

    let budget = Composition::non_mesh(3);
    budget.io.update_host_facts(|facts| facts.nofile_limit = 1_024);
    assert!(matches!(
        budget.owner.probe_startup().await,
        Err(GuestNetworkError::FdBudget { available: 1_024, .. })
    ));

    let placement = Composition::non_mesh(4);
    placement.io.update_host_facts(|facts| facts.owner_cgroup = PathBuf::from("/sys/fs/cgroup"));
    assert!(matches!(
        placement.owner.probe_startup().await,
        Err(GuestNetworkError::OwnerCgroupPlacement { .. })
    ));

    let unreadable = Composition::non_mesh(6);
    unreadable
        .io
        .script_failure(SimEffectKind::HostFacts, SimFailure::Io(io::ErrorKind::PermissionDenied));
    assert!(matches!(
        unreadable.owner.probe_startup().await,
        Err(GuestNetworkError::Io { operation: GuestNetworkOperation::StartupProbe, .. })
    ));

    for c in [&both, &address, &budget, &placement, &unreadable] {
        assert_eq!(listener_or_route_effects(c), 0, "a refused probe issues no boot effect");
        assert_eq!(c.io.gateway_route(), None);
    }
}

/// G-V1 row 2 / Earned Trust: a forwarder context that cannot be opened or
/// fails its probe — the module absent, another ABI, a module that is not
/// permanent, a probe stage that observed the wrong behaviour — refuses boot
/// as `GuestNetworkError::Forwarder { source }` carrying the forwarder's own
/// typed error (never flattened); no listener is bound, no route written,
/// and the seam holds no context.
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_forwarder_that_cannot_be_opened_or_probed_refuses_boot_typed() {
    let failures = [
        SimForwarderFailure::DeviceAbsent,
        SimForwarderFailure::AbiMismatch,
        SimForwarderFailure::ProbeMismatch(
            ForwarderProbeStage::PeerAbortClassified,
            ForwarderProbeObservation::WrongOutcome,
        ),
    ];
    for (seed, failure) in (20_u64..).zip(failures) {
        let c = Composition::non_mesh(seed);
        c.io.script_failure(SimEffectKind::OpenForwarder, SimFailure::Forwarder(failure));
        let refused = boot_until_refused(&c).await.expect_err("boot refuses");
        assert_eq!(taxonomy_name(&refused), "Forwarder");
        let matches_failure = match (&refused, failure) {
            (
                GuestNetworkError::Forwarder { source: ForwarderError::DeviceAbsent { .. } },
                SimForwarderFailure::DeviceAbsent,
            )
            | (
                GuestNetworkError::Forwarder { source: ForwarderError::AbiMismatch { .. } },
                SimForwarderFailure::AbiMismatch,
            ) => true,
            (
                GuestNetworkError::Forwarder {
                    source: ForwarderError::ProbeMismatch { stage, observed },
                },
                SimForwarderFailure::ProbeMismatch(want_stage, want_observed),
            ) => *stage == want_stage && *observed == want_observed,
            _ => false,
        };
        assert!(matches_failure, "{failure:?} refuses with its own typed source: {refused:?}");
        assert_eq!(
            listener_or_route_effects(&c),
            0,
            "nothing is bound or written after the refusal"
        );
        assert_eq!(
            c.io.forwarder_inventory(),
            overdrive_core::guest_transport::forwarder::ForwarderInventory::default(),
            "the seam holds no context"
        );
    }
}

/// `converge_shared` failures refuse boot at their own operation: the flow
/// and control listener binds (`Io { FlowListenerBind | ControlListenerBind }`).
///
/// CONTRACT_SHAPE: unbounded-preservation.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_listener_bind_failure_refuses_at_its_own_operation() {
    let bind = Composition::non_mesh(11);
    bind.io.script_failure(SimEffectKind::BindListener, SimFailure::Io(io::ErrorKind::AddrInUse));
    let outcome = boot_until_refused(&bind).await;
    assert!(matches!(
        outcome,
        Err(GuestNetworkError::Io {
            operation: GuestNetworkOperation::FlowListenerBind
                | GuestNetworkOperation::ControlListenerBind,
            ..
        })
    ));
    assert_eq!(taxonomy_name(&outcome.expect_err("refused")), "Io");
    assert_eq!(bind.io.gateway_route(), None, "the route follows the listeners");
}

/// GW-LOCAL (ADR-0152): boot `converge_shared` converges the gateway route
/// for `GUEST_NODE_GATEWAY` after the forwarder context and the listeners; a
/// failed route write refuses boot with `Netlink { LocalRouteConverge }`,
/// the route as it was.
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn boot_converges_the_gateway_route_last_and_a_failed_write_refuses() {
    let c = Composition::booted(12).await;
    assert_eq!(c.io.gateway_route(), Some(GUEST_NODE_GATEWAY));
    let kinds: Vec<_> = c.io.issued().into_iter().map(|effect| effect.kind).collect();
    let route =
        kinds.iter().position(|kind| *kind == SimEffectKind::GatewayRoute).expect("written");
    let open = kinds.iter().position(|kind| *kind == SimEffectKind::OpenForwarder).expect("opened");
    let last_bind =
        kinds.iter().rposition(|kind| *kind == SimEffectKind::BindListener).expect("bound");
    assert!(
        open < route && last_bind < route,
        "the route is converged after the forwarder and listeners"
    );

    let refused = Composition::non_mesh(13);
    refused.io.script_failure(SimEffectKind::GatewayRoute, SimFailure::Netlink);
    let outcome = boot_until_refused(&refused).await;
    assert!(matches!(
        outcome,
        Err(GuestNetworkError::Netlink {
            operation: GuestNetworkOperation::LocalRouteConverge,
            ..
        })
    ));
    assert_eq!(refused.io.gateway_route(), None, "the route is as it was");
}

/// GW-LOCAL / H-4: a gateway route a previous boot left behind is not a
/// stale object (`sweep_stale` passes) and boot converges over it; a second
/// `converge_shared` at boot reaches the same state (idempotent).
///
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn boot_converges_over_a_route_left_behind_and_is_idempotent() {
    let c = Composition::non_mesh(15);
    c.io.set_gateway_route(Some(GUEST_NODE_GATEWAY));
    c.owner.probe_startup().await.expect("probe passes");
    c.owner.sweep_stale().await.expect("the gateway route is not a stale owner object");
    c.owner.converge_shared().await.expect("boot converges");
    assert_eq!(c.io.gateway_route(), Some(GUEST_NODE_GATEWAY));
    let writes = c.io.issued_count(SimEffectKind::GatewayRoute);
    assert_eq!(writes, 1, "one convergence of the one route");
}

/// C6c across the owner's runtime failure paths: an unknown or Retiring CID
/// on `activate` is `PostconditionMismatch { ForwardingActivate }`.
///
/// CONTRACT_SHAPE: pure-function.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn runtime_owner_failures_are_taxonomy_variants() {
    let c = Composition::booted(14).await;
    let (lease, _session) = c.active_lease("c6c", &[]).await;
    c.owner.teardown(&lease.plan).await.expect("teardown");
    let error = c.owner.activate(&lease.plan).await.expect_err("activation of a torn-down CID");
    assert_eq!(taxonomy_name(&error), "PostconditionMismatch");
    assert!(matches!(
        error,
        GuestNetworkError::PostconditionMismatch {
            operation: GuestNetworkOperation::ForwardingActivate,
            ..
        }
    ));
}
