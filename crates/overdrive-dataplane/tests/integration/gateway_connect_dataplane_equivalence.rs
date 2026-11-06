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
//! Known limitation of the Sim pairing (receipt timing): the Sim decides a
//! selection receipt at `take_receipt` or read time, not at connect time as the
//! real cgroup hook does. A comparison here therefore proves the observable
//! receipt kind and identity, never when the receipt was decided relative to a
//! connect, an outer swap or a concurrent update; that ordering is proven only
//! at Tier 3 (S-PIG-72) and by the selection-receipt conformance driver.
//! The call script consequently never interleaves an `update_service` between a
//! dial and the `take_receipt` that reads its receipt.
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
fn host_and_sim_agree_on_selection_identity_rejection_residue_and_restart() {
    let mut host = Host::new();
    let mut sim = Sim::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");

    let host_trace = script(&mut host, &rt);
    let sim_trace = script(&mut sim, &rt);

    assert_eq!(host_trace, sim_trace, "Host and Sim observe the same trace");
}

// ---------------------------------------------------------------------------
// S-PIG-131 (R-59): a reverse-NAT key owned by another Service's VIP.
// ---------------------------------------------------------------------------

use overdrive_core::dataplane::{BackendKey, Proto};

use super::helpers::gateway_connect_rig::backend_target;

/// The map state both adapters expose through their own inspection accessors,
/// backend-neutrally: whether each Service is programmed, how many reverse-NAT
/// entries exist, and whether the contested key is present.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MapState {
    service_one_programmed: bool,
    service_two_programmed: bool,
    reverse_nat_entries: usize,
    contested_key_present: bool,
}

/// An owner whose map state can be observed through its own accessors.
trait Observe {
    fn map_state(&self, contested: BackendKey) -> MapState;
}

impl Host {
    /// A rig with its own tag (names the veth pair and the pin directory).
    fn new_tagged(tag: &str) -> Self {
        Self {
            rig: Some(HostRig::new(tag)),
            rt: tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt"),
        }
    }
}

impl Observe for Host {
    fn map_state(&self, contested: BackendKey) -> MapState {
        let dp = &self.rig().dataplane;
        let programmed = |n: u8| {
            dp.service_map_contains(frontend(n).vip_v4(), frontend(n).port().get(), Proto::Tcp)
                .expect("SERVICE_MAP read")
        };
        MapState {
            service_one_programmed: programmed(1),
            service_two_programmed: programmed(2),
            reverse_nat_entries: dp.reverse_nat_map_size().expect("REVERSE_NAT_MAP size"),
            contested_key_present: dp
                .reverse_nat_map_has_backend_proto(contested.ip, contested.port, contested.proto)
                .expect("REVERSE_NAT_MAP read"),
        }
    }
}

impl Observe for Sim {
    fn map_state(&self, contested: BackendKey) -> MapState {
        let programmed = |n: u8| {
            self.dataplane
                .service_backends_for(frontend(n).vip_v4(), frontend(n).port().get(), Proto::Tcp)
                .is_some()
        };
        MapState {
            service_one_programmed: programmed(1),
            service_two_programmed: programmed(2),
            reverse_nat_entries: self.dataplane.reverse_nat_entries().len(),
            contested_key_present: self.dataplane.reverse_nat_lookup(contested).is_some(),
        }
    }
}

/// One scripted R-59 sequence over an owner; returns the backend-neutral trace.
fn conflict_script<O: Owner + Observe>(owner: &O, rt: &tokio::runtime::Runtime) -> Vec<String> {
    let mut trace = Vec::new();

    // Service one owns backend address 1 (reverse-NAT key -> VIP one); service
    // two owns backend address 2. The contested key is address 1's.
    owner.update(frontend(1), vec![backend(1, 1, true)]).expect("service one programmed");
    owner.update(frontend(2), vec![backend(2, 2, true)]).expect("service two programmed");
    let contested = BackendKey::new(*backend_target(1).ip(), backend_target(1).port(), Proto::Tcp);

    let (one_before, one_id) = select_once(rt, owner, 1);
    let (two_before, two_id) = select_once(rt, owner, 2);
    assert_eq!(one_before, Observed::Selected(alloc(1)), "service one selects its backend");
    assert_eq!(two_before, Observed::Selected(alloc(2)), "service two selects its backend");
    let before = owner.map_state(contested);
    assert_eq!(
        before,
        MapState {
            service_one_programmed: true,
            service_two_programmed: true,
            reverse_nat_entries: 2,
            contested_key_present: true,
        },
        "the baseline holds exactly the two Services' entries",
    );

    // Service two now names service one's backend address: the contested
    // reverse-NAT key already maps to VIP one, so planning rejects it.
    let rejected = owner.update(frontend(2), vec![backend(2, 2, true), backend(1, 1, true)]);
    match rejected {
        Err(DataplaneError::ReverseNatConflict { key, current_vip, attempted_vip }) => {
            assert_eq!(key, contested, "the conflict names the contested reverse-NAT key");
            assert_eq!(current_vip, frontend(1).vip(), "the key is owned by VIP one");
            assert_eq!(attempted_vip, frontend(2).vip(), "VIP two attempted to take it");
        }
        other => panic!("expected ReverseNatConflict, got {other:?}"),
    }

    // Nothing was mutated: both Services' map state and both selections are as
    // before, and no new BackendId was minted by the rejected update.
    let after = owner.map_state(contested);
    assert_eq!(after, before, "a rejected update leaves both Services' map state unchanged");
    let (one_after, one_id_after) = select_once(rt, owner, 1);
    let (two_after, two_id_after) = select_once(rt, owner, 2);
    assert_eq!(one_after, one_before, "service one still selects its backend");
    assert_eq!(two_after, two_before, "service two keeps its prior generation");
    assert_eq!(one_id_after, one_id, "service one's id is unchanged");
    assert_eq!(two_id_after, two_id, "service two's id is unchanged");
    trace.push(format!("rejected: {after:?} one={one_after:?} two={two_after:?}"));

    assert_eq!(counts(owner), (0, 0), "no intent and no receipt remain");
    trace
}

/// S-PIG-131 (R-59). An update whose reverse-NAT key already maps to another
/// Service's VIP is rejected in planning with `ReverseNatConflict`, and both
/// Services' map state is unchanged; Host and Sim agree.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn host_and_sim_reject_an_update_whose_reverse_nat_key_belongs_to_another_service() {
    let host = Host::new_tagged("h131");
    let sim = Sim::new();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("rt");

    let host_trace = conflict_script(&host, &rt);
    let sim_trace = conflict_script(&sim, &rt);

    assert_eq!(host_trace, sim_trace, "Host and Sim observe the same trace");
}
