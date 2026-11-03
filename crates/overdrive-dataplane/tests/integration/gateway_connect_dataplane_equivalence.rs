//! Host/Sim `GatewayConnectDataplane` equivalence (S-PIG-67).
//!
//! `EbpfDataplane` (the real cgroup hook, bpffs maps and commit-guarded
//! `update_service`) and `SimGatewayConnectDataplane` over a shared
//! `SimDataplane` are driven through the same ports with the same call sequence
//! and must agree on every observable: the receipt kind, the applied identity of
//! the selected id, the effect of a rejected conflicting backend set, historical
//! identities after the Service empties, residue counts, and the instance
//! restart. Only the observable is compared, never the numeric `BackendId`
//! (the Sim's numbering is its own): selection is observed as the allocation the
//! selected id denotes, with exactness only for singleton generations and
//! membership for a generation of two backends (the real Maglev choice is
//! Tier-3 only).
//!
//! Every phase also carries its expected observation as a literal, so two
//! adapters that were wrong in the same way cannot pass: the comparison guards
//! drift, the literals are the oracle.
//!
//! Native metal only (the Host side loads the BPF object).

#![cfg(target_os = "linux")]
#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    reason = "CONTRACT_SHAPE metadata and fixture preconditions"
)]

use std::sync::Arc;
use std::time::Duration;

use overdrive_core::dataplane::ServiceFrontend;
use overdrive_core::id::SpiffeId;
use overdrive_core::traits::dataplane::{Backend, Dataplane, DataplaneError};
use overdrive_gateway::ports::{
    GatewayConnectDataplane, GatewayConnectDataplaneError, GatewaySelectionReceipt,
};
use overdrive_sim::adapters::dataplane::SimDataplane;
use overdrive_sim::adapters::gateway::SimGatewayConnectDataplane;

use super::helpers::gateway_connect_rig::{
    HostRig, alloc, backend, dial, frontend, frontend_target, service_key,
};

/// What one registered socket observed, backend-neutrally.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Observed {
    /// The receipt was `NoBackend`.
    NoBackend,
    /// The receipt was `Selected` and the selected id resolved to this identity.
    Selected(SpiffeId),
    /// The receipt was `Selected` but its id resolved to no identity.
    SelectedUnresolved,
}

/// One owner under test: the `Dataplane` and `GatewayConnectDataplane` ports of
/// the same instance.
trait Owner {
    fn update(
        &self,
        frontend: ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError>;
    fn gateway(&self) -> &dyn GatewayConnectDataplane;
    /// Re-create the instance (a serve restart): nothing survives.
    fn restart(&mut self);
}

struct Host {
    rig: Option<HostRig>,
    rt: tokio::runtime::Runtime,
}

impl Host {
    fn new() -> Self {
        Self {
            rig: Some(HostRig::new("h67")),
            rt: tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt"),
        }
    }

    fn rig(&self) -> &HostRig {
        self.rig.as_ref().expect("the rig exists between restarts")
    }
}

impl Owner for Host {
    fn update(
        &self,
        frontend: ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        self.rt.block_on(self.rig().dataplane.update_service(frontend, backends))
    }
    fn gateway(&self) -> &dyn GatewayConnectDataplane {
        &self.rig().dataplane
    }
    fn restart(&mut self) {
        let rig = self.rig.take().expect("the rig exists between restarts");
        self.rig = Some(rig.recreate());
    }
}

struct Sim {
    dataplane: Arc<SimDataplane>,
    gateway: SimGatewayConnectDataplane,
    rt: tokio::runtime::Runtime,
}

impl Sim {
    fn fresh() -> (Arc<SimDataplane>, SimGatewayConnectDataplane) {
        let dataplane = Arc::new(SimDataplane::new());
        let gateway = SimGatewayConnectDataplane::new(Arc::clone(&dataplane));
        (dataplane, gateway)
    }

    fn new() -> Self {
        let (dataplane, gateway) = Self::fresh();
        Self {
            dataplane,
            gateway,
            rt: tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt"),
        }
    }
}

impl Owner for Sim {
    fn update(
        &self,
        frontend: ServiceFrontend,
        backends: Vec<Backend>,
    ) -> Result<(), DataplaneError> {
        self.rt.block_on(self.dataplane.update_service(frontend, backends))
    }
    fn gateway(&self) -> &dyn GatewayConnectDataplane {
        &self.gateway
    }
    fn restart(&mut self) {
        let (dataplane, gateway) = Self::fresh();
        self.dataplane = dataplane;
        self.gateway = gateway;
    }
}

/// Register one socket for `key`, dial the Service frontend (bounded; the dial's
/// own result is irrelevant, the receipt is what the owner recorded), take the
/// receipt, resolve its identity and clean up. Returns what was observed and the
/// selected id, if any.
fn select_once(
    rt: &tokio::runtime::Runtime,
    owner: &dyn Owner,
    n: u8,
) -> (Observed, Option<overdrive_core::id::BackendId>) {
    let gateway = owner.gateway();
    let _guard = rt.enter();
    let (socket, intent) =
        gateway.open_upstream(service_key(n)).expect("open_upstream").into_parts();
    drop(rt.block_on(dial(socket, frontend_target(n), Duration::from_millis(300))));
    let receipt = gateway.take_receipt(&intent).expect("the receipt of this socket");
    let observed = match &receipt {
        GatewaySelectionReceipt::NoBackend { socket_cookie, service_key: key } => {
            assert_eq!(*socket_cookie, intent.socket_cookie, "receipt bound to the cookie");
            assert_eq!(*key, intent.service_key, "receipt bound to the ServiceKey");
            (Observed::NoBackend, None)
        }
        GatewaySelectionReceipt::Selected { socket_cookie, service_key: key, backend_id } => {
            assert_eq!(*socket_cookie, intent.socket_cookie, "receipt bound to the cookie");
            assert_eq!(*key, intent.service_key, "receipt bound to the ServiceKey");
            let observed = gateway
                .selected_backend_identity(*backend_id)
                .map_or(Observed::SelectedUnresolved, Observed::Selected);
            (observed, Some(*backend_id))
        }
    };
    gateway.cleanup(&intent).expect("cleanup");
    observed
}

fn counts(owner: &dyn Owner) -> (u32, u32) {
    let gateway = owner.gateway();
    (
        gateway.live_intent_count().expect("live intent count"),
        u32::try_from(gateway.live_receipts().expect("live receipts").len())
            .expect("receipt count fits u32"),
    )
}

/// One scripted sequence; returns the backend-neutral observation trace.
fn script(owner: &mut dyn Owner, rt: &tokio::runtime::Runtime) -> Vec<String> {
    let mut trace = Vec::new();
    let mut note = |label: &str, value: String| trace.push(format!("{label}: {value}"));

    // 1. No outer entry for the key: NoBackend.
    let (observed, _) = select_once(rt, owner, 1);
    assert_eq!(observed, Observed::NoBackend, "an unprogrammed key is NoBackend");
    note("unprogrammed", format!("{observed:?} counts={:?}", counts(owner)));

    // 2. A singleton generation: Selected, exact identity.
    owner.update(frontend(1), vec![backend(1, 1, true)]).expect("programmed");
    let (observed, first_id) = select_once(rt, owner, 1);
    assert_eq!(observed, Observed::Selected(alloc(1)), "the singleton generation is selected");
    note("singleton", format!("{observed:?} counts={:?}", counts(owner)));
    let first_id = first_id.expect("a Selected receipt names an id");

    // 3. A conflicting backend set (the same address re-leased to another
    //    allocation) is rejected before any mutation: the prior generation and
    //    its identity are untouched.
    let rejected = owner.update(frontend(1), vec![backend(1, 2, true)]);
    assert!(
        matches!(rejected, Err(DataplaneError::BackendIdentityConflict { .. })),
        "a re-leased address must be rejected with BackendIdentityConflict: {rejected:?}"
    );
    let (observed, id_after) = select_once(rt, owner, 1);
    assert_eq!(
        observed,
        Observed::Selected(alloc(1)),
        "the prior generation survives the rejection"
    );
    assert_eq!(id_after, Some(first_id), "no new id was minted by the rejected update");
    note("rejected", format!("{observed:?} rejected=true counts={:?}", counts(owner)));

    // 4. Two backends: membership only (the real Maglev choice is Tier-3 only).
    owner.update(frontend(1), vec![backend(1, 1, true), backend(2, 2, true)]).expect("programmed");
    let (observed, _) = select_once(rt, owner, 1);
    let member = matches!(&observed, Observed::Selected(identity) if *identity == alloc(1) || *identity == alloc(2));
    assert!(member, "the selection belongs to the committed generation, got {observed:?}");
    note("pair", format!("member={member} counts={:?}", counts(owner)));

    // 5. The Service empties: NoBackend again; the id a previous receipt named
    //    still resolves (historical identities are retained for old receipts).
    owner.update(frontend(1), Vec::new()).expect("emptied");
    let (observed, _) = select_once(rt, owner, 1);
    assert_eq!(observed, Observed::NoBackend, "an emptied key is NoBackend");
    assert_eq!(
        owner.gateway().selected_backend_identity(first_id),
        Ok(alloc(1)),
        "an old receipt's identity stays readable after the Service empties",
    );
    note("emptied", format!("{observed:?} historical=ok counts={:?}", counts(owner)));

    // 6. Restart: nothing survives, and the key is unprogrammed until re-hydrated.
    owner.restart();
    assert_eq!(
        owner.gateway().selected_backend_identity(first_id),
        Err(GatewayConnectDataplaneError::BackendIdentityMissing { backend_id: first_id }),
        "no applied identity survives a restart",
    );
    let (observed, _) = select_once(rt, owner, 1);
    assert_eq!(observed, Observed::NoBackend, "the restarted instance holds no Service");
    note("restarted", format!("{observed:?} historical=missing counts={:?}", counts(owner)));

    assert_eq!(counts(owner), (0, 0), "no intent and no receipt remain");
    trace
}

/// S-PIG-67. Host and Sim agree through the ports over the same call sequence.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER selection-receipt-and-applied-identity"]
fn host_and_sim_agree_on_selection_identity_rejection_residue_and_restart() {
    let mut host = Host::new();
    let mut sim = Sim::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");

    let host_trace = script(&mut host, &rt);
    let sim_trace = script(&mut sim, &rt);

    assert_eq!(host_trace, sim_trace, "Host and Sim observe the same trace");
}
