//! Host guest-flow owner surface — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Owner and provisioner" (`QuiescenceHold`, `QuiescenceLedger`,
//! `QuiescenceRestoreError`, `FlowCloseCause`,
//! `GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION`, "Conformance observation (owner
//! accessors)"), § DG-3 ("Owner construction": `GuestFlowOwnerPorts`) and
//! § "Conformance observation surface" QPROJ-2
//! (`GuestAddressPool::lease_observation`), QPROJ-3
//! (`HostSharedGuestNetworkOwner::flow_observation`) and QPROJ-8
//! (`forwarder_generation`, `forwarder_attach_ended` and the `generation` of
//! each `Paired` phase). Every item below is the one those sections pin; the
//! items are re-exported from `overdrive_control_plane::guest_network`.
//!
//! The owner's constructors (`new(GuestFlowOwnerPorts)`, `with_flow_io`) live
//! beside its struct in `guest_network`. The accessors and the ledger's
//! operations are RED scaffolds; the owner and pool they observe land in the
//! attachment-cut step (7). The observation types are plain data.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock attachment cut, step 7).

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ipnet::Ipv4Net;
use overdrive_core::guest_transport::dial::{DialPurpose, GuestStreamEnd};
use overdrive_core::guest_transport::forwarder::{
    ForwardEnd, ForwardId, ForwarderGeneration, ForwarderInventory,
};
use overdrive_core::guest_transport::wire::FlowId;
use overdrive_core::guest_transport::{GuestCid, LaunchToken, VsockListenPort};
use overdrive_core::id::AllocationId;
use overdrive_core::traits::GuestAttachmentLease;
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::mtls_resolve::MtlsResolve;
use overdrive_core::vm::config::VsockPort;
use overdrive_dataplane::allocators::VipRange;
use overdrive_worker::mtls_intercept_worker::MtlsInterceptWorker;

use super::{GuestAddressPool, HostSharedGuestNetworkOwner};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// The per-allocation flow quota (ADR-0208, 4,096). It counts every flow of
/// the allocation from its registration until the host releases it — at the
/// flow's end record, or for a dial at its stream's drop or the owner's abort
/// (EVMOD-AB): guest-opened TCP flows, datagram associations, and
/// `DialPurpose::Relay` host-opened flows (an in-flight dial, then its
/// `GuestStream`), half-closed flows included. A `DialPurpose::Probe` dial
/// takes a node flow slot and no quota slot (PROBE-NOSAMPLE).
pub use overdrive_core::guest_transport::GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION;

// ---------------------------------------------------------------------------
// Owner construction (DG-3)
// ---------------------------------------------------------------------------

/// Every input the owner needs; no field has a default (`development.md`
/// § "Port-trait dependencies").
pub struct GuestFlowOwnerPorts {
    /// Every deadline and the audit cadence read this clock.
    pub clock: Arc<dyn Clock>,
    /// The owner's single resolution of a guest TCP destination, with the VIP
    /// branch (D24a).
    pub resolve: Arc<dyn MtlsResolve>,
    /// The mTLS worker handle (mesh `register_forwarded_outbound`, D9).
    pub mtls: Arc<MtlsInterceptWorker>,
    /// The configured service VIP range — the same value the VIP allocator
    /// and `ServiceBackendsResolve::new` receive (D24a, D8-PREFIX-VIP).
    pub vip_range: VipRange,
    /// `GUEST_NODE_PREFIX` in production (DG-9).
    pub node_prefix: Ipv4Net,
    /// `GUEST_NODE_GATEWAY` in production (DG-9).
    pub gateway: Ipv4Addr,
    /// PORTS-CONFIG: `config.vsock_ports.flow_stream()`.
    pub stream_port: VsockListenPort,
    /// PORTS-CONFIG: `config.vsock_ports.flow_control()`.
    pub control_port: VsockListenPort,
}

impl HostSharedGuestNetworkOwner {
    /// QPROJ-3: an owned snapshot of the owner's per-CID tables, the hold and
    /// leaked-hold counts, the listener-task state and the forwarder facts the
    /// owner holds.
    ///
    /// - **Preconditions:** none. Infallible; callable on a started owner at
    ///   any time, including while quiesced and after `teardown` of every
    ///   allocation.
    /// - **Postconditions:** an owned copy of the owner's per-CID tables, the
    ///   hold and leaked-hold counts and the listener-task state. No lock or
    ///   queue slot is held after return. The call starts no step: it
    ///   registers no flow, sends no message and changes no state. Dropping
    ///   the future before it completes has no effect on the owner
    ///   (cancellation-safe).
    /// - **Consistency point:** between two steps of every CID's serialized
    ///   order (M-7) and of quiescence. It never reports a flow
    ///   half-registered or a count that disagrees with `admission` because a
    ///   quiesce is mid-way. The attach and the `Paired` send of one dispatch,
    ///   and the release of one end record, are reported after the dispatch,
    ///   never inside it.
    /// - **Edge cases:** an owner with no allocation returns empty `cids`,
    ///   both counts 0, `admission = Open` and `flow_listeners_running =
    ///   true`. A flow, dial, association or CID the owner no longer holds is
    ///   absent; an aborted or closed flow is absent. A CID in `Retiring`
    ///   stays until `teardown` returns `Ok`. A dial that returned an error is
    ///   absent. A dropped `GuestStream` is absent. `leaked_quiescence_holds`
    ///   starts at 0 in every owner instance (a `serve` restart starts at 0).
    /// - **Observable invariants:** `admission = Quiesced` iff
    ///   `quiescence_holds > 0` (`LatchIffCount`); `leaked_quiescence_holds`
    ///   never decreases, and `leaked_quiescence_holds > 0` implies
    ///   `quiescence_holds > 0`; `held_flows` equals the number of the CID's
    ///   entries in `guest_flows` and `datagrams` plus its
    ///   `DialPurpose::Relay` entries in `dials` (D-UP-17) and never exceeds
    ///   `GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION` (`QuotaBound`); while
    ///   `admission = Quiesced` every CID's `guest_flows`, `dials` and
    ///   `datagrams` are empty (`QuiescedHoldsNoFlow`); every `ForwardId` in
    ///   a phase names a pair or watch of the owner's current forwarder
    ///   context whose end record the owner has not yet read, and every
    ///   `generation` in a `Paired` phase equals `forwarder_generation`
    ///   (QPROJ-8); `dials` is empty for a CID that is not Active; each
    ///   `GuestConnKey` appears in at most one of a CID's `unread`,
    ///   `guest_flows` and `session`; a repeated call with no intervening
    ///   owner step returns an equal value.
    #[expect(
        clippy::unused_async,
        reason = "RED scaffold body panics; the attachment-cut step lands the observation"
    )]
    pub async fn flow_observation(&self) -> GuestOwnerObservation {
        panic!("{SCAFFOLD}")
    }
}

// ---------------------------------------------------------------------------
// QPROJ-3 / QPROJ-8 — host guest-flow owner observation
// ---------------------------------------------------------------------------

/// QPROJ-3 snapshot of the host guest-flow owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestOwnerObservation {
    /// QHOLD: outstanding holds, leaked holds included.
    pub quiescence_holds: u32,
    /// QHOLD: holds of this owner's ledger dropped unredeemed since the owner
    /// started (`QuiescenceLedger::leaked`, D-UP-4).
    pub leaked_quiescence_holds: u32,
    /// The admission latch, read apart from the count so a divergence
    /// between them is visible.
    pub admission: GuestAdmission,
    /// The flow and control listeners' accept tasks are all running.
    pub flow_listeners_running: bool,
    /// The generation of the forwarder context the owner holds:
    /// `ForwarderGeneration::FIRST` after boot step 9's open, + 1 at each
    /// `Forwarder` repair's successful open (EVMOD-CTL; QPROJ-8, U-EV-8). The
    /// count of serving contexts this owner has opened, never decremented: a
    /// repair's close does not change it, and phase 1's probe context
    /// (`probe_forwarder`, D-8) is not one of them.
    pub forwarder_generation: ForwarderGeneration,
    /// `FlowForwarder::inventory()` of the context the owner holds, read in
    /// the same snapshot (D-15; the owner's audit compares the same value
    /// with its flow table).
    pub forwarder_inventory: ForwarderInventoryRead,
    /// Cumulative attaches and watches the forwarder refused `HalfEnded`,
    /// each ended by the owner as a flow abort (EVMOD-ATTACH-END; QPROJ-8).
    /// Never decremented.
    pub forwarder_attach_ended: u64,
    /// Cumulative attaches and watches the forwarder accepted (`Ok`), raised
    /// in the dispatch that made the call, in every context this owner held
    /// (QPROJ-8, `attRes`). Never decremented.
    pub forwarder_attached: u64,
    /// The last end record this owner consumed (`take_end` in its dispatch,
    /// EVMOD-REC), from any context it held: depth 1, replaced at each
    /// consumption, `None` until the first (D-11). A consumed flow has left
    /// `cids`, so this is the only surface that still shows how it ended.
    pub last_consumed_end: Option<ForwardEnd>,
    /// Cumulative end records this owner consumed. Never decremented.
    pub consumed_ends: u64,
    /// Per-CID tables.
    pub cids: BTreeMap<GuestCid, CidObservation>,
}

/// The forwarder inventory as one snapshot found it. Two distinct
/// non-inventory states, so neither is a sentinel (D-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwarderInventoryRead {
    /// The context the owner holds answered `inventory()`.
    Read(ForwarderInventory),
    /// The owner holds no context: between a `Forwarder` repair's close and
    /// its reopen, or after a failed reopen.
    NoContext,
    /// The held context's `inventory()` failed; the same failure is the
    /// `Forwarder` component's damage, which the audit reports.
    Unreadable,
}

/// The owner's admission latch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestAdmission {
    /// New flows are admitted and dials served.
    Open,
    /// New flows are refused `Quiesced`; dials answered `Quiesced` (QHOLD).
    Quiesced,
}

/// One CID's state at the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CidObservation {
    /// The allocation the CID is leased to.
    pub alloc: AllocationId,
    /// The lease's launch token (ATTR-TOKEN).
    pub launch: LaunchToken,
    /// Provisioned | Active | Retiring.
    pub phase: CidPhase,
    /// Holder of the CID's `ClaimSet<GuestCid>` session claim.
    pub session: Option<GuestConnKey>,
    /// Control connections accepted on 1243 whose `ControlOpen` is not yet
    /// read.
    pub unread: BTreeSet<GuestConnKey>,
    /// Registered guest-opened TCP flows (U-1), by connection.
    pub guest_flows: BTreeMap<GuestConnKey, GuestFlowPhase>,
    /// Host-opened flows: in-flight dials and open `GuestStream`s, by flow
    /// id, each with its purpose (D-UP-17).
    pub dials: BTreeMap<FlowId, DialObservation>,
    /// Datagram associations, by guest-origin flow id.
    pub datagrams: BTreeMap<FlowId, AssociationObservation>,
    /// Flows counted against the allocation's quota, half-closed ones
    /// included: every entry of `guest_flows` and `datagrams`, and the
    /// `DialPurpose::Relay` entries of `dials`. A `DialPurpose::Probe` dial
    /// is not counted (PROBE-NOSAMPLE, D-UP-17).
    pub held_flows: u32,
}

/// A host-opened flow at the owner, with the purpose it was dialled for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialObservation {
    /// `Relay` (leg-S) or `Probe` (the VM probe runner).
    pub purpose: DialPurpose,
    /// Awaiting the guest's reply, or paired and held by its stream.
    pub phase: DialPhase,
}

/// The owner's per-CID lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CidPhase {
    /// Registered by `provision`; not yet activated.
    Provisioned,
    /// Activated: flows from this CID are admitted, dials served.
    Active,
    /// Teardown in progress.
    Retiring,
}

/// Kernel-reported identity of an accepted guest-opened host vsock
/// connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuestConnKey {
    /// The host vsock listener port that accepted it.
    pub listener: VsockPort,
    /// The guest CID that opened it.
    pub peer_cid: GuestCid,
    /// The guest's source vsock port.
    pub peer_port: VsockPort,
}

/// A registered guest-opened TCP flow's phase at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFlowPhase {
    /// Admitted and registered (U-1); the request is not yet complete. Its
    /// completion and the token comparison are one step.
    Registered {
        /// How much of the request the owner has read.
        request: RequestRead,
    },
    /// The request's token matched the live token.
    Attributed,
    /// The non-blocking connect is issued; its completion is not processed.
    Connecting,
    /// The pair (`V_h`, D or L) is attached and `Paired` sent (EVMOD-PAIR).
    /// Its per-direction state is `FlowForwarder::pair_state(pair)`; the flow
    /// leaves the table, and `held_flows` drops, in the dispatch that reads
    /// the pair's end record (EVMOD-AB). `generation` is the context
    /// generation the pair was attached in (QPROJ-8).
    Paired {
        /// The pair's id in the owner's forwarder context.
        pair: ForwardId,
        /// The context generation the pair was attached in.
        generation: ForwarderGeneration,
    },
}

/// How much of a flow request the owner has read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestRead {
    /// No byte read.
    Nothing,
    /// Some, but not all, bytes read.
    Partial,
}

/// A host-opened flow's phase at the owner (INB-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialPhase {
    /// `V_h` connected and the TcpAccept request written; no reply processed.
    AwaitingReply,
    /// `Paired` received; the flow is held by its `GuestStream`. `watch` is
    /// the forwarder watch on `V_h`; `guest_end` is its end record's class
    /// once the owner has read it; `host_shut` the holder's `SHUT_WR`;
    /// `generation` the context generation the watch was made in.
    Paired {
        /// The watch on `V_h` (EVMOD-END).
        watch: ForwardId,
        /// The context generation the watch was made in.
        generation: ForwarderGeneration,
        /// The class of the watch's end record, once the owner read it.
        guest_end: Option<GuestStreamEnd>,
        /// The holder shut its direction down (`shutdown_write`).
        host_shut: bool,
    },
}

/// A datagram association at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssociationObservation {
    /// Requested or paired.
    pub phase: AssociationPhase,
}

/// A datagram association's phase at the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationPhase {
    /// Requested; `Paired` not yet sent.
    AwaitingPaired,
    /// The datagram pair (`V_h`, H) is attached and `Paired` sent.
    Paired {
        /// The pair's id in the owner's forwarder context.
        pair: ForwardId,
        /// The context generation the pair was attached in.
        generation: ForwarderGeneration,
    },
}

// ---------------------------------------------------------------------------
// QHOLD — the counted quiescence hold and its ledger (D-UP-4, PD-1, PD-2)
// ---------------------------------------------------------------------------

/// The ledger's shared state: the count of its holds dropped unredeemed.
#[derive(Debug, Default)]
struct LedgerInner {
    leaked: AtomicU32,
}

/// QHOLD: one outstanding quiescence of node forwarding. Not `Clone`; built
/// only by `QuiescenceLedger::issue`; consumed only by the `redeem` of the
/// ledger that issued it (reached through `restore`). It holds a shared
/// reference to its issuing ledger's state, which is how a drop reaches that
/// ledger: dropped unredeemed, it increments that ledger's leaked count (one
/// atomic increment; no I/O, no lock, nothing awaited). The owner's hold
/// count stays raised, so forwarding stays quiesced until the process ends.
#[must_use]
#[derive(Debug)]
pub struct QuiescenceHold {
    ledger: Arc<LedgerInner>,
    redeemed: bool,
}

impl Drop for QuiescenceHold {
    fn drop(&mut self) {
        if !self.redeemed {
            self.ledger.leaked.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// QHOLD capability (D-UP-4 and PD-2; public so the sim owner uses it).
/// Issues and redeems the `QuiescenceHold`s of one owner and counts those
/// dropped without a redeem. Not `Clone`: each owner constructs and holds
/// exactly one; a ledger is never shared between owners. The ledger keeps
/// its inner state alive as long as any hold of it exists, so a hold dropped
/// after its owner is dropped still increments a valid counter.
#[derive(Debug)]
pub struct QuiescenceLedger {
    #[allow(
        dead_code,
        reason = "RED scaffold — `issue` / `redeem` / `leaked` read it in step 09-01"
    )]
    inner: Arc<LedgerInner>,
}

impl QuiescenceLedger {
    /// A ledger with no hold issued and a leaked count of 0.
    #[must_use]
    pub fn new() -> Self {
        Self { inner: Arc::new(LedgerInner::default()) }
    }

    /// A new hold bound to this ledger (it clones this ledger's inner `Arc`).
    ///
    /// RED scaffold: lands with the counted quiescence (step 09-01 rework).
    #[expect(clippy::unused_self, reason = "RED scaffold body panics")]
    pub fn issue(&self) -> QuiescenceHold {
        panic!("{SCAFFOLD}")
    }

    /// Consumes `hold` without counting a leak when this ledger issued it
    /// (identity: `Arc::ptr_eq` of the hold's inner and this ledger's);
    /// returns it unchanged otherwise.
    ///
    /// RED scaffold: lands with the counted quiescence (step 09-01 rework).
    #[expect(
        clippy::unused_self,
        clippy::needless_pass_by_value,
        reason = "RED scaffold body panics"
    )]
    pub fn redeem(&self, hold: QuiescenceHold) -> Result<(), QuiescenceHold> {
        let _ = hold;
        panic!("{SCAFFOLD}")
    }

    /// Holds of this ledger dropped without a redeem; never decreases.
    ///
    /// RED scaffold: lands with the counted quiescence (step 09-01 rework).
    #[expect(clippy::unused_self, reason = "RED scaffold body panics")]
    pub fn leaked(&self) -> u32 {
        panic!("{SCAFFOLD}")
    }
}

impl Default for QuiescenceLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// Why `restore` refused a hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuiescenceRestoreError {
    /// The hold was issued by another owner's ledger (a composition defect:
    /// one supervisor is composed with one owner).
    #[error("quiescence hold was issued by another owner's ledger")]
    ForeignHold,
}

impl QuiescenceRestoreError {
    /// The error's telemetry label.
    ///
    /// RED scaffold: lands with the counted quiescence (step 09-01 rework).
    #[expect(clippy::unused_self, reason = "RED scaffold body panics")]
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// ---------------------------------------------------------------------------
// Flow end (EVMOD-REC, failure projection, D26-PINS)
// ---------------------------------------------------------------------------

/// Why the owner closed a flow (`guest_flow.closed { cause }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowCloseCause {
    /// The pair completed: both directions ended cleanly.
    PeerEof,
    /// The pair aborted on a peer's end: `PeerAbort`, a `SocketError` other
    /// than a datagram destination's, `Unclassified` (an alarm), or a half
    /// refused `HalfEnded` at attach (§ *[REF] vsock data path*, "Failure
    /// projection").
    PeerReset,
    /// The flow was refused.
    Refused,
    /// The peer owner sent `Abort`.
    AbortReceived,
    /// Forwarding quiesced (QHOLD).
    Quiesced,
    /// The allocation was torn down.
    Teardown,
    /// The owner aborted the flow.
    OwnerAbort,
    /// The request did not arrive within `FLOW_REQUEST_DEADLINE`.
    RequestTimeout,
    /// The flow did not pair within its pairing deadline.
    PairingTimeout,
    /// The control session was lost.
    ControlSessionLost,
    /// The control session carried a malformed message.
    ControlMalformed,
    /// The CID has no control session.
    NoControlSession,
    /// The record carried another launch's token (ATTR-TOKEN).
    StaleLaunch,
    /// D26-PINS: the host-internal output rule rejected the transmit of a
    /// forwarded datagram, or the destination refused it (the pair's
    /// `SocketError` on its UDP half).
    DestinationUnreachable,
    /// A datagram frame failed its magic or length check (`FrameInvalid`).
    FrameInvalid,
    /// A host-opened flow whose `GuestStream` was dropped with a direction
    /// open.
    StreamDropped,
}

impl FlowCloseCause {
    /// The cause's telemetry label.
    ///
    /// RED scaffold: the attachment-cut step lands the label vocabulary with
    /// the owner flows.
    #[expect(
        clippy::unused_self,
        reason = "RED scaffold body panics; the attachment-cut step lands it"
    )]
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// ---------------------------------------------------------------------------
// QPROJ-2 — pool lease observation
// ---------------------------------------------------------------------------

impl GuestAddressPool {
    /// QPROJ-2: an owned copy of the lease table and cursor.
    ///
    /// - **Preconditions:** none. Infallible; callable at any time on a live
    ///   pool.
    /// - **Postconditions:** an owned copy of the lease table and cursor,
    ///   taken under one acquisition of the pool lock, released before
    ///   return. `leases` holds exactly the Admitted and Retiring leases at
    ///   that instant. The call runs no `assign` walk and changes no state.
    /// - **Consistency point:** between two pool operations (`assign`,
    ///   `retire`, `release`).
    /// - **Edge cases:** an empty pool returns empty `leases` and the initial
    ///   cursor. A Retiring lease keeps its CID and launch token until
    ///   `release`, which runs only after the VMM has exited. A pool built
    ///   after a `serve` restart reports no lease of the previous process
    ///   (NO-SURVIVE).
    /// - **Invariants:** at most one lease per CID (`CidUnique`); a repeated
    ///   call with no intervening pool operation returns an equal value.
    #[expect(
        clippy::unused_self,
        reason = "RED scaffold body panics; the attachment-cut step lands the CID lease table"
    )]
    pub fn lease_observation(&self) -> GuestLeaseObservation {
        panic!("{SCAFFOLD}")
    }
}

/// QPROJ-2 snapshot of the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLeaseObservation {
    /// The CID at which the next `assign` walk starts (the next-fit cursor,
    /// M-9).
    pub next_fit: GuestCid,
    /// Every Admitted and Retiring lease.
    pub leases: BTreeMap<AllocationId, GuestLeaseView>,
}

/// One lease in a [`GuestLeaseObservation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestLeaseView {
    /// `GUEST_CID_BASE + lease offset`.
    pub cid: GuestCid,
    /// The existing lease state: Admitted | Retiring.
    pub state: GuestAttachmentLease,
    /// The lease's launch token (ATTR-TOKEN).
    pub launch: LaunchToken,
}
