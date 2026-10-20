//! The host guest-flow owner and mTLS-worker composition (D-UP-16, shaped
//! by LEGS-DIAL).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § *Owner and mTLS-worker composition (D-UP-16)*. The owner takes the mTLS
//! worker (`GuestFlowOwnerPorts::mtls`, forwarded-outbound registration); the
//! inbound enforcement the worker is built over reaches the owner as its
//! `Arc<dyn GuestStreamDial>` (`guest_dial`, LEGS-DIAL) through the boot's one
//! [`OwnerDial`]. [`compose_guest_network`] is the one construction of that
//! pair (phase 2 of the owner's boot, D-8): `run_server` calls it over
//! `HostGuestFlowIo`; in-crate tests and conformance drivers call it over
//! `SimGuestFlowIo`, so a test composition runs the production construction
//! over the same worker instance the owner registers flows with.
//!
//! The cycle is closed with [`OwnerDial`], a delegating `GuestStreamDial`
//! that holds the owner weakly: the owner holds the worker, which holds the
//! enforcement, strongly, so a strong handle back would keep all three alive
//! after the server drops them (and with them the forwarder context the seam
//! holds).

#![allow(
    clippy::redundant_pub_crate,
    reason = "D-UP-16 pins crate-private visibility for the composition's items"
)]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, OnceLock, Weak};

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::guest_transport::dial::{
    DialPurpose, GuestDialError, GuestStream, GuestStreamDial,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_enforcement::MtlsEnforcement;
use overdrive_core::traits::mtls_resolve::MtlsResolve;
use overdrive_dataplane::allocators::VipRange;
use overdrive_worker::mtls_intercept_port::MtlsIntercept;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use crate::guest_network::flow_io::GuestFlowIo;
use crate::guest_network::{GuestFlowOwnerPorts, HostSharedGuestNetworkOwner};

/// Every input of the owner and the mTLS worker; no field has a default
/// (`development.md` § "Port-trait dependencies").
pub(crate) struct GuestNetworkCompositionPorts {
    /// The worker's clock and the owner's (every deadline, the audit cadence).
    pub(crate) clock: Arc<dyn Clock>,
    /// The one resolution port both the owner and the worker read.
    pub(crate) resolve: Arc<dyn MtlsResolve>,
    /// The worker's enforcement port.
    pub(crate) enforcement: Arc<dyn MtlsEnforcement>,
    /// The worker's intercept port.
    pub(crate) intercept: Arc<dyn MtlsIntercept>,
    /// The configured service VIP range (D24a, D8-PREFIX-VIP).
    pub(crate) vip_range: VipRange,
    /// `GUEST_NODE_PREFIX` in production (DG-9).
    pub(crate) node_prefix: Ipv4Net,
    /// `GUEST_NODE_GATEWAY` in production (DG-9).
    pub(crate) gateway: Ipv4Addr,
    /// PORTS-CONFIG: `config.vsock_ports.flow_stream()`.
    pub(crate) stream_port: VsockListenPort,
    /// PORTS-CONFIG: `config.vsock_ports.flow_control()`.
    pub(crate) control_port: VsockListenPort,
}

/// Phase 2 of the owner's boot (D-8). `io` is the seam phase 1
/// (`probe_guest_flow_startup`) checked; `dial` is the boot's one unbound
/// [`OwnerDial`] — the same `Arc` `ports.enforcement` was constructed with as
/// its `guest_dial` (LEGS-DIAL) — which this binds to the returned owner.
/// Builds the worker over `ports.enforcement`, `resolve`, `clock` and
/// `intercept`, and the owner over `io` with that worker.
///
/// Postcondition: `dial.owner()` is the returned owner; the owner's
/// `GuestFlowOwnerPorts::mtls` is the returned worker; no other owner, worker
/// or enforcement is constructed.
#[allow(
    clippy::needless_pass_by_value,
    reason = "D-UP-16 / LEGS-DIAL pin the signature: the boot hands its one `OwnerDial` to the composition"
)]
pub(crate) fn compose_guest_network(
    ports: GuestNetworkCompositionPorts,
    io: Arc<dyn GuestFlowIo>,
    dial: Arc<OwnerDial>,
) -> (Arc<HostSharedGuestNetworkOwner>, Arc<MtlsInterceptWorker>) {
    let GuestNetworkCompositionPorts {
        clock,
        resolve,
        enforcement,
        intercept,
        vip_range,
        node_prefix,
        gateway,
        stream_port,
        control_port,
    } = ports;
    let worker = Arc::new(MtlsInterceptWorker::new(
        enforcement,
        Arc::clone(&resolve),
        Arc::clone(&clock),
        intercept,
    ));
    let owner = Arc::new(HostSharedGuestNetworkOwner::with_flow_io(
        GuestFlowOwnerPorts {
            clock,
            resolve,
            mtls: Arc::clone(&worker),
            vip_range,
            node_prefix,
            gateway,
            stream_port,
            control_port,
        },
        io,
    ));
    dial.bind(&owner);
    (owner, worker)
}

/// The owner's `GuestStreamDial`, reached through a weak handle (the owner
/// holds the worker, which holds the enforcement, strongly, so the dial back
/// must not).
///
/// One per boot: `run_server` creates it unbound before the probe-runner gate,
/// and every holder of a guest dial in the boot — the VM probe runner and the
/// inbound enforcement (leg S, LEGS-DIAL) — holds this same `Arc`;
/// [`compose_guest_network`] binds it. No VM is served before that (DG-2 row
/// 19).
#[derive(Debug, Default)]
pub(crate) struct OwnerDial {
    owner: OnceLock<Weak<HostSharedGuestNetworkOwner>>,
}

impl OwnerDial {
    /// A dial with no owner yet; every dial is `NotServing` until [`Self::bind`].
    pub(crate) fn unbound() -> Self {
        Self::default()
    }

    /// Binds the boot's one owner; called once, by [`compose_guest_network`].
    /// A second bind is a logic error: a boot composes one owner.
    pub(crate) fn bind(&self, owner: &Arc<HostSharedGuestNetworkOwner>) {
        if self.owner.set(Arc::downgrade(owner)).is_err() {
            unreachable!("a boot composes and binds exactly one guest-flow owner");
        }
    }

    /// The bound owner while it is alive (D-13); `None` before [`Self::bind`]
    /// or after the owner is dropped.
    pub(crate) fn owner(&self) -> Option<Arc<HostSharedGuestNetworkOwner>> {
        self.owner.get().and_then(Weak::upgrade)
    }
}

#[async_trait::async_trait]
impl GuestStreamDial for OwnerDial {
    /// Delegates to the owner. Before the owner is composed, or after it is
    /// dropped, no Active VM allocation is served: `NotServing`.
    async fn dial(
        &self,
        target: SocketAddrV4,
        purpose: DialPurpose,
    ) -> Result<GuestStream, GuestDialError> {
        let owner = self.owner().ok_or(GuestDialError::NotServing)?;
        owner.dial(target, purpose).await
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test fixtures assert their own preconditions")]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::Arc;

    use overdrive_core::guest_transport::dial::{DialPurpose, GuestDialError, GuestStreamDial};
    use overdrive_core::guest_transport::{GUEST_NODE_GATEWAY, GUEST_NODE_PREFIX, VsockListenPort};
    use overdrive_core::traits::mtls_enforcement::MtlsLimits;
    use overdrive_core::traits::mtls_resolve::MtlsResolution;
    use overdrive_dataplane::allocators::VipRange;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::mtls_enforcement::SimMtlsEnforcement;
    use overdrive_sim::adapters::{SimIdentityRead, SimMtlsIntercept, SimMtlsResolve};

    use super::{GuestNetworkCompositionPorts, OwnerDial, compose_guest_network};
    use crate::guest_network::flow_io::GuestFlowIo;
    use crate::guest_network::flow_io::sim::SimGuestFlowIo;

    fn ports(dial: &Arc<OwnerDial>) -> GuestNetworkCompositionPorts {
        GuestNetworkCompositionPorts {
            clock: Arc::new(SimClock::new()),
            resolve: Arc::new(SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)),
            enforcement: Arc::new(SimMtlsEnforcement::new(
                Arc::new(SimIdentityRead::new(BTreeMap::new(), None)),
                MtlsLimits::default(),
                Arc::clone(dial) as Arc<dyn GuestStreamDial>,
            )),
            intercept: Arc::new(SimMtlsIntercept::new()),
            vip_range: VipRange::new(
                vec!["10.96.0.0/20".parse().expect("a valid CIDR")],
                BTreeSet::new(),
            )
            .expect("valid VIP range"),
            node_prefix: GUEST_NODE_PREFIX,
            gateway: GUEST_NODE_GATEWAY,
            stream_port: VsockListenPort::KernelAssigned,
            control_port: VsockListenPort::KernelAssigned,
        }
    }

    fn target() -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(100, 95, 0, 6), 8080)
    }

    /// Before a boot composes the owner, the VM probe runner's dial serves
    /// no allocation.
    #[tokio::test]
    async fn a_dial_before_the_owner_is_composed_is_not_serving() {
        let dial = OwnerDial::unbound();
        let outcome = dial.dial(target(), DialPurpose::Probe).await;
        assert!(
            matches!(outcome, Err(GuestDialError::NotServing)),
            "an unbound owner dial must answer NotServing, got {:?}",
            outcome.err()
        );
    }

    /// D-UP-16 (LEGS-DIAL shape): the owner, the worker and the enforcement
    /// hold each other without a strong cycle — dropping the two handles the
    /// composition returns releases the owner (and the seam it holds), and
    /// the boot's dial, bound to it, then serves nothing.
    #[tokio::test]
    async fn the_composed_owner_is_released_with_its_worker() {
        let io = SimGuestFlowIo::new(7);
        let probe_dial = Arc::new(OwnerDial::unbound());
        let (owner, worker) = compose_guest_network(
            ports(&probe_dial),
            io as Arc<dyn GuestFlowIo>,
            Arc::clone(&probe_dial),
        );
        assert!(
            probe_dial.owner().is_some_and(|bound| Arc::ptr_eq(&bound, &owner)),
            "compose_guest_network binds the boot's one dial to the owner it returns"
        );
        let released = Arc::downgrade(&owner);
        drop(worker);
        drop(owner);
        assert!(
            released.upgrade().is_none(),
            "the worker must not keep the owner alive (no strong owner/worker cycle)"
        );
        let outcome = probe_dial.dial(target(), DialPurpose::Probe).await;
        assert!(
            matches!(outcome, Err(GuestDialError::NotServing)),
            "a dial bound to a released owner must answer NotServing, got {:?}",
            outcome.err()
        );
    }
}
