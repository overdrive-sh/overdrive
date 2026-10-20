//! Quint conformance driver — `udp_slots`, conformance main `slots_conf`,
//! host side: the host owner's datagram associations (EVMOD-DGRAM; QPROJ-1,
//! QPROJ-3, QPROJ-7; § *Per-module projection and action map* "`udp_slots`,
//! host side"; D-UP-1).
//!
//! Replays `specs/quint/guest-flow-owner/udp_slots.qnt` (referenced in place)
//! on its recording main `slots_conf` against the production owner over
//! `SimGuestFlowIo` and its `SimFlowForwarder` arbiter. The guest slot pool
//! is the other side — stimulus taken from the trace (its owner,
//! `overdrive-init`, has its own driver). The driver keeps, from the trace's
//! guest steps, only the stimulus it must replay: each slot's destination
//! (from `AppClaim`) and the next association id; it never projects them.
//! The step record is read through
//! `Config::nondet = &["slots_conf::udp_slots::conf_step"]`; the state is
//! projected by qualified names ([`Qualified`]).
//!
//! | Action | Production call or fault |
//! |---|---|
//! | `init` | a fresh composition boots; one lease is provisioned, its guest's session opened, and activated |
//! | `GuestReady(s)` (starts association `i`) | seam queues its one 1240 stream with its `DatagramStream` request for the slot's destination; the owner admits and reads it; its local-delivery route check is held |
//! | `HostPair(i)` | the seam answers the held route check `Unicast`: the owner connects H, checks its kernel peer, attaches `V_h`–H in datagram mode and sends `Paired` |
//! | `HostRefuse(i)` | the destination's route class is scripted `Local` and the held check answered: the owner refuses `HostInternal`, `V_h` closed |
//! | `HostAbortAssoc(i)` | seam: H reports `EPERM` (the D26 rule): the pair aborts `SocketError` and the owner ends the association (`DestinationUnreachable`) |
//! | `HostRecvEnd(i)` | the guest's end reaches the host: its `Abort { flow }` on the session for an association the host has not paired, the close of the association's vsock (`0x3`, the guest's slot release or its `Abort` of a paired one) otherwise |
//! | guest actions (`AppSendOnSlot`, `AppClaim`, `GuestRecvPaired`, `GuestRecvEnd`, `GuestPairingTimeout`, `SockClose`, `SockCloseReportLost`, `GuestReleaseSock`, `GuestAuditIdle`) | stimulus: the guest's own state moves; the owner is run to quiescence and the step compares the host unchanged |
//! | `AppPick`, `AppTake` | hazard-only: an error |
//!
//! Projection: `host(i).st` ← O `cids[cid].datagrams[i].phase`
//! (`AwaitingPaired` → `req`, `Paired` → `paired`); `closed` ⇔ S
//! `closed_by_owner` for its connection; `none` ⇔ never started. Every guest
//! variable, `host(i).sock`, `.dest`, `.slot` → stimulus; the messages in
//! flight → model-only; the rest ghost or hazard-only; `conf_step`
//! model-only. Identity: model association id `i` ↔ guest-origin `FlowId`
//! with sequence `i`. The owner is held directly (QPROJ-3's
//! `flow_observation`); `closed_by_owner` is the seam sim's, the named
//! arbiter (QPROJ-7).

#![allow(
    clippy::needless_pass_by_ref_mut,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::print_stderr,
    reason = "a conformance driver asserts its own preconditions and prints its seed; protocol names are prose"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, and handler signatures follow the action map"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use itf::Value;
use overdrive_core::guest_transport::wire::{ControlMessage, KernelErrno};
use quint_connect::{Config, Driver, Result, State, Step, quint_run, quint_test, switch};
use serde::Deserialize;

use super::flow_io::HostRouteType;
use super::flow_io::sim::{SimEffectKind, SimEffectTarget};
use super::owner_acceptance::support::{Composition, Lease, guest_flow, paused_runtime};
use super::quint_itf::{self, ItfError, Qualified};
use super::{AssociationPhase, GuestConnKey, HostSharedGuestNetworkOwner};

const MAIN: &str = "slots_conf";
const MODULE: &str = "udp_slots";
/// `MAXFID` of the conformance instance (`CONF_STEP_BOUND + 1`).
const MAXFID: i64 = 33;
const SEED: u64 = 0x0F50;

/// The destination of model destination `d` (one address per destination,
/// so a route class scripted for one association touches no other).
fn destination(d: i64) -> SocketAddrV4 {
    let last = u8::try_from(10 + d).expect("small model ids");
    SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, last), 5_353)
}

// ----- the compared state ---------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SlotsState {
    host: BTreeMap<i64, String>,
}

impl SlotsState {
    fn from_value(value: &Value) -> std::result::Result<Self, ItfError> {
        let q = Qualified::of(value, MAIN, MODULE)?;
        let host = quint_itf::int_map(q.var("host")?, |assoc| {
            quint_itf::string(quint_itf::field(assoc, "st")?)
        })?;
        Ok(Self { host })
    }
}

impl<'de> Deserialize<'de> for SlotsState {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(&value).map_err(serde::de::Error::custom)
    }
}

impl State<SlotsDriver> for SlotsState {
    /// The raw ITF state, decoded by the shared ITF helpers (the serde
    /// path renders `#bigint` as a string and `#map` as a record, QPROJ-1).
    fn from_spec(value: Value) -> Result<Self> {
        Ok(Self::from_value(&value)?)
    }

    fn from_driver(driver: &SlotsDriver) -> Result<Self> {
        driver.project()
    }
}

#[derive(Debug, thiserror::Error)]
enum DriverError {
    #[error("action `{0}` is hazard-only: it is never enabled in a design instance")]
    HazardOnly(&'static str),
    #[error("action `{action}` cannot be played: {why}")]
    NotPlayable { action: &'static str, why: String },
}

fn not_playable(action: &'static str, why: impl Into<String>) -> anyhow::Error {
    DriverError::NotPlayable { action, why: why.into() }.into()
}

pub(super) struct SlotsDriver {
    rt: tokio::runtime::Runtime,
    /// The production owner under test (QPROJ-3), the composition's own.
    owner: Arc<HostSharedGuestNetworkOwner>,
    /// The composition the owner stands on (its seam sim and pool).
    c: Composition,
    lease: Option<Lease>,
    session: Option<GuestConnKey>,
    /// Stimulus: each guest slot's destination, from `AppClaim`.
    slot_dest: BTreeMap<i64, i64>,
    /// Stimulus: the next association id the guest mints (`nextFid`).
    next_fid: i64,
    /// Association id ↦ its 1240 connection and destination.
    assocs: BTreeMap<i64, (GuestConnKey, SocketAddrV4)>,
    next_port: u32,
}

impl SlotsDriver {
    fn new() -> Self {
        let rt = paused_runtime();
        let c = {
            let _runtime = rt.enter();
            Composition::non_mesh(SEED)
        };
        Self {
            rt,
            owner: Arc::clone(&c.owner),
            c,
            lease: None,
            session: None,
            slot_dest: BTreeMap::new(),
            next_fid: 1,
            assocs: BTreeMap::new(),
            next_port: 6_000,
        }
    }

    fn composition(&self) -> &Composition {
        &self.c
    }

    fn settle(&self) {
        self.rt.block_on(self.composition().settle());
    }

    fn init(&mut self) {
        *self = Self::new();
        self.rt.block_on(self.c.boot());
        // The owner's local-delivery check is the host step the trace places.
        self.c.io.hold(SimEffectKind::RouteType);
        let (lease, session) = self.rt.block_on(self.c.active_lease("udp-slots", &[]));
        self.lease = Some(lease);
        self.session = Some(session);
    }

    fn lease(&self) -> &Lease {
        self.lease.as_ref().expect("init provisioned the lease")
    }

    fn assoc(&self, i: i64, action: &'static str) -> Result<(GuestConnKey, SocketAddrV4)> {
        self.assocs
            .get(&i)
            .copied()
            .ok_or_else(|| not_playable(action, format!("association {i} never started")))
    }

    fn flow_of(
        &self,
        i: i64,
        action: &'static str,
    ) -> Result<overdrive_core::guest_transport::wire::FlowId> {
        let seq =
            u32::try_from(i).map_err(|_| not_playable(action, "a negative association id"))?;
        Ok(guest_flow(seq))
    }

    fn phase(&self, i: i64, action: &'static str) -> Result<Option<AssociationPhase>> {
        let flow = self.flow_of(i, action)?;
        let observed = self.rt.block_on(self.composition().observe());
        Ok(observed
            .cids
            .get(&self.lease().cid)
            .and_then(|cid| cid.datagrams.get(&flow))
            .map(|assoc| assoc.phase))
    }

    // ----- guest stimulus -----

    fn app_claim(&mut self, d: i64, s: i64) {
        self.slot_dest.insert(s, d);
        self.settle();
    }

    fn guest_ready(&mut self, s: i64) -> Result {
        let d = *self
            .slot_dest
            .get(&s)
            .ok_or_else(|| not_playable("GuestReady", format!("slot {s} was never claimed")))?;
        let i = self.next_fid;
        if i > MAXFID {
            return Err(not_playable("GuestReady", "MAXFID reached"));
        }
        self.next_fid += 1;
        let seq = u32::try_from(i).map_err(|_| not_playable("GuestReady", "a negative id"))?;
        self.next_port += 1;
        let target = destination(d);
        let lease = self.lease().clone();
        let (key, _flow) = self.composition().open_datagram(&lease, seq, target, self.next_port);
        self.assocs.insert(i, (key, target));
        self.settle();
        Ok(())
    }

    fn guest_stimulus(&mut self) {
        self.settle();
    }

    // ----- host steps -----

    fn release_route_check(&self, target: SocketAddrV4, action: &'static str) -> Result {
        let c = self.composition();
        let id =
            c.io.held()
                .into_iter()
                .find(|(_, effect)| {
                    effect.kind == SimEffectKind::RouteType
                        && effect.target == SimEffectTarget::Ip(*target.ip())
                })
                .map(|(id, _)| id)
                .ok_or_else(|| not_playable(action, format!("no held route check for {target}")))?;
        c.io.release(id);
        self.settle();
        Ok(())
    }

    fn host_pair(&mut self, i: i64) -> Result {
        let (_key, target) = self.assoc(i, "HostPair")?;
        self.composition().io.set_route_type(*target.ip(), HostRouteType::Unicast);
        self.release_route_check(target, "HostPair")
    }

    fn host_refuse(&mut self, i: i64) -> Result {
        let (_key, target) = self.assoc(i, "HostRefuse")?;
        self.composition().io.set_route_type(*target.ip(), HostRouteType::Local);
        self.release_route_check(target, "HostRefuse")
    }

    fn host_abort_assoc(&mut self, i: i64) -> Result {
        let flow = self.flow_of(i, "HostAbortAssoc")?;
        let eperm = KernelErrno::new(1)
            .map_err(|error| not_playable("HostAbortAssoc", error.to_string()))?;
        self.composition().io.destination_socket_error(flow, eperm);
        self.settle();
        Ok(())
    }

    fn host_recv_end(&mut self, i: i64) -> Result {
        let flow = self.flow_of(i, "HostRecvEnd")?;
        match self.phase(i, "HostRecvEnd")? {
            Some(AssociationPhase::AwaitingPaired) => {
                let session =
                    self.session.ok_or_else(|| not_playable("HostRecvEnd", "no session"))?;
                self.composition().guest_sends(session, ControlMessage::Abort { flow });
            }
            Some(AssociationPhase::Paired { .. }) => self.composition().io.guest_closes(flow),
            // The association already ended at the host: the guest's end
            // finds nothing (U-5).
            None => {}
        }
        self.settle();
        Ok(())
    }

    // ----- projection -----

    fn project(&self) -> Result<SlotsState> {
        let mut host: BTreeMap<i64, String> =
            (1..=MAXFID).map(|i| (i, "none".to_owned())).collect();
        self.rt.block_on(self.c.settle());
        let observed = self.rt.block_on(self.owner.flow_observation());
        // conformance-lint: sim-arbiter-ok QPROJ-7 owner-seam socket arbiter (closed_by_owner), Tier-3 pair R5-16 / R5-41 and R5-59(a)
        let closed: BTreeSet<GuestConnKey> = self.c.io.socket_observation().closed_by_owner;
        let table = self.lease.as_ref().and_then(|lease| observed.cids.get(&lease.cid));
        for (i, (key, _)) in &self.assocs {
            let flow = self.flow_of(*i, "project")?;
            let st = match table.and_then(|cid| cid.datagrams.get(&flow)).map(|assoc| assoc.phase) {
                Some(AssociationPhase::AwaitingPaired) => "req",
                Some(AssociationPhase::Paired { .. }) => "paired",
                None if closed.contains(key) => "closed",
                None => anyhow::bail!("association {i} ({key:?}) has no model state"),
            };
            host.insert(*i, st.to_owned());
        }
        Ok(SlotsState { host })
    }
}

impl Driver for SlotsDriver {
    type State = SlotsState;

    fn config() -> Config {
        Config { state: &[], nondet: &["slots_conf::udp_slots::conf_step"] }
    }

    fn step(&mut self, step: &Step) -> Result {
        switch!(step {
            init => self.init(),
            AppSendOnSlot => self.guest_stimulus(),
            AppClaim(k: i64, d: i64, s: i64) => {
                let _ = k;
                self.app_claim(d, s);
            }
            AppPick => return Err(DriverError::HazardOnly("AppPick").into()),
            AppTake => return Err(DriverError::HazardOnly("AppTake").into()),
            GuestReady(s: i64) => self.guest_ready(s)?,
            HostPair(i: i64) => self.host_pair(i)?,
            HostRefuse(i: i64) => self.host_refuse(i)?,
            HostAbortAssoc(i: i64) => self.host_abort_assoc(i)?,
            GuestRecvPaired => self.guest_stimulus(),
            GuestRecvEnd => self.guest_stimulus(),
            GuestPairingTimeout => self.guest_stimulus(),
            HostRecvEnd(i: i64) => self.host_recv_end(i)?,
            SockClose => self.guest_stimulus(),
            SockCloseReportLost => self.guest_stimulus(),
            GuestReleaseSock => self.guest_stimulus(),
            GuestAuditIdle => self.guest_stimulus(),
        })
    }
}

// ----- named runs (`checks.toml`, `udp_slots`) and randomized traces -----

/// Named run `DatagramAssociationClaimsPairsAndRoutes` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "DatagramAssociationClaimsPairsAndRoutes"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_datagram_association_claims_pairs_and_routes() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `TwoSocketsClaimDistinctSlots` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "TwoSocketsClaimDistinctSlots"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_two_sockets_claim_distinct_slots() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "RefusedDiscardsParkedFramesAndOnlyNewDatagramReassociates"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_refused_discards_parked_frames_and_only_new_datagram_reassociates() -> impl Driver
{
    SlotsDriver::new()
}

/// Named run `HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "HostAbortOfLiveAssociationEndsItAndKeepsSlotOwned"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_host_abort_of_live_association_ends_it_and_keeps_slot_owned() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "DatagramQueuedBeforeAnEndIsDiscardedWithTheQueue"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_datagram_queued_before_an_end_is_discarded_with_the_queue() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `LatePairedForReleasedSlotIsAnsweredWithAbort` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "LatePairedForReleasedSlotIsAnsweredWithAbort"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_late_paired_for_released_slot_is_answered_with_abort() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `LostReleaseReportIsFoundByAudit` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "LostReleaseReportIsFoundByAudit"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_lost_release_report_is_found_by_audit() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `SocketCloseReleasesEverySlotItOwns` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "SocketCloseReleasesEverySlotItOwns"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_socket_close_releases_every_slot_it_owns() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `IdleSlotReleaseEndsAssociation` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "IdleSlotReleaseEndsAssociation"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_idle_slot_release_ends_association() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `AuditTickReleasesOrphansAndIdleSlotsInOneStep` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "AuditTickReleasesOrphansAndIdleSlotsInOneStep"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_audit_tick_releases_orphans_and_idle_slots_in_one_step() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `PairingDeadlineAbortsAndLatePairedIsDiscarded` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "PairingDeadlineAbortsAndLatePairedIsDiscarded"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_pairing_deadline_aborts_and_late_paired_is_discarded() -> impl Driver {
    SlotsDriver::new()
}

/// Named run `RecycledSlotNeverDeliversPreviousOwnersFrames` (`slots_conf`), host side.
#[quint_test(
    spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt",
    main = "slots_conf",
    max_samples = 1,
    test = "RecycledSlotNeverDeliversPreviousOwnersFrames"
)]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn quint_udp_slots_recycled_slot_never_delivers_previous_owners_frames() -> impl Driver {
    SlotsDriver::new()
}

/// Randomized `slots_conf` traces; the seed is printed on failure.
#[quint_run(spec = "../../specs/quint/guest-flow-owner/udp_slots.qnt", main = "slots_conf")]
#[ignore = "pending DELIVER attachment-cut step (7)"]
fn slots_conf_randomized_traces() -> impl Driver {
    SlotsDriver::new()
}
