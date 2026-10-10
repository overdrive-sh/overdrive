//! Guest transport vocabulary — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Core vocabulary — `overdrive-core` (D2, D3, D5, D8, D8a-CAP, D16)"
//! (the `GuestCid` / `LaunchToken` / `GuestTransportAssignment` /
//! `GuestCidClaim` contract, the guest intake constants of D25 and the intake
//! steering and host-port constants of D8a-CAP, INTAKE-BIND and INTAKE-PORTS).
//! The wire codec lives in [`wire`].
//!
//! RED scaffold (DISTILL, Mandate 7): every signature below is the one the
//! design pins; every function and method body panics until DELIVER
//! (vsock new-step 2, "core types and wire codec", and new-step 15, the pool
//! claim) replaces it. Constants carry the values the design pins. No surface
//! the design does not pin is added here.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock new-steps 2 and 15).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lints self-trip when DELIVER lands the bodies"
)]

pub mod wire;

use std::fmt;
use std::net::Ipv4Addr;
use std::ops::RangeInclusive;
use std::os::fd::BorrowedFd;
use std::str::FromStr;
use std::time::Duration;

use ipnet::Ipv4Net;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::traits::entropy::Entropy;
use crate::vm::config::VsockPort;

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

// ---------------------------------------------------------------------------
// Guest CID
// ---------------------------------------------------------------------------

/// First guest CID handed out by admission. Every leased CID is
/// `GUEST_CID_BASE + lease_offset`.
pub const GUEST_CID_BASE: u32 = 0x0100_0000;
/// Number of CIDs in the leased range: one per offset of the /16 prefix.
pub const GUEST_CID_SPAN: u32 = 65_536;
/// Reserved CID used only by the forwarder startup probe; never leased.
pub const GUEST_CID_PROBE: u32 = GUEST_CID_BASE - 1;

/// Host-unique vsock context identifier of one allocation's VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuestCid(u32);

impl GuestCid {
    /// Accepts `GUEST_CID_BASE..GUEST_CID_BASE + GUEST_CID_SPAN` only. The
    /// edge cases 0, 1, 2, `u32::MAX` and `GUEST_CID_PROBE` are `OutOfRange`.
    pub fn new(_raw: u32) -> Result<Self, GuestCidError> {
        panic!("{SCAFFOLD}")
    }

    /// The raw CID.
    pub const fn get(self) -> u32 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Decimal form.
impl fmt::Display for GuestCid {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; validated by [`GuestCid::new`].
impl FromStr for GuestCid {
    type Err = GuestCidError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

impl Serialize for GuestCid {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

impl<'de> Deserialize<'de> for GuestCid {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a raw value is not a [`GuestCid`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuestCidError {
    /// Outside `GUEST_CID_BASE..GUEST_CID_BASE + GUEST_CID_SPAN`.
    #[error("guest CID {raw} is outside the leased range")]
    OutOfRange {
        /// The rejected value.
        raw: u32,
    },
    /// Not a decimal integer.
    #[error("guest CID is not a decimal integer")]
    Unparseable,
}

// ---------------------------------------------------------------------------
// vsock ports (host listeners bind `VMADDR_CID_ANY` and check the peer CID)
// ---------------------------------------------------------------------------

/// Host listens, `SOCK_STREAM`: guest-opened TCP and datagram-stream flows.
pub const GUEST_FLOW_STREAM_PORT: VsockPort = VsockPort::new(1240);
/// Host listens, `SOCK_SEQPACKET`: guest-opened datagram records.
pub const GUEST_FLOW_DATAGRAM_PORT: VsockPort = VsockPort::new(1241);
/// Guest listens, `SOCK_STREAM`: host-opened `TcpAccept` flows.
pub const GUEST_INBOUND_PORT: VsockPort = VsockPort::new(1242);
/// Host listens, `SOCK_STREAM`: the per-VM control session (D18a).
pub const GUEST_FLOW_CONTROL_PORT: VsockPort = VsockPort::new(1243);

// ---------------------------------------------------------------------------
// Launch token (ATTR-TOKEN, ADR-0177)
// ---------------------------------------------------------------------------

/// Identifies one launch of one allocation's VM — the lease, its claimed CID
/// and the one VMM that runs on it. 16 bytes drawn from the injected
/// [`Entropy`] port by the pool's `assign`; never all zero. Not a secret and
/// not persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaunchToken([u8; 16]);

impl LaunchToken {
    /// `Err(LaunchTokenError::Zero)` for sixteen zero bytes.
    pub fn from_bytes(_bytes: [u8; 16]) -> Result<Self, LaunchTokenError> {
        panic!("{SCAFFOLD}")
    }

    /// Draws a token; redraws until non-zero. Called only by the pool.
    pub fn generate(_entropy: &dyn Entropy) -> Self {
        panic!("{SCAFFOLD}")
    }

    /// The token's bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        panic!("Not yet implemented -- RED scaffold")
    }
}

/// Exactly 32 lowercase hex characters.
impl fmt::Display for LaunchToken {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Exactly 32 lowercase hex characters, case-sensitive; all zero is `Zero`.
impl FromStr for LaunchToken {
    type Err = LaunchTokenError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

impl Serialize for LaunchToken {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

impl<'de> Deserialize<'de> for LaunchToken {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Why a value is not a [`LaunchToken`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchTokenError {
    /// Sixteen zero bytes: the invalid encoding.
    #[error("launch token is all zero")]
    Zero,
    /// Not exactly 32 lowercase hex characters.
    #[error("launch token is not 32 lowercase hex characters")]
    Unparseable,
}

// ---------------------------------------------------------------------------
// Allocation → driver handoff (D16-CLAIM)
// ---------------------------------------------------------------------------

/// Allocation → driver handoff. Constructed only by the pool's `assign`: the
/// CID is the one the carried claim holds. `Clone` shares one take-once claim
/// slot; equality compares `workload_addr`, `gateway`, `cid` and `launch`.
#[derive(Debug, Clone)]
pub struct GuestTransportAssignment {
    _scaffold: (),
}

impl GuestTransportAssignment {
    /// Builds the handoff over a held claim.
    pub fn new(
        _workload_addr: Ipv4Addr,
        _gateway: Ipv4Addr,
        _launch: LaunchToken,
        _claim: ClaimedGuestCid,
    ) -> Self {
        panic!("{SCAFFOLD}")
    }

    /// The allocation's workload address.
    pub fn workload_addr(&self) -> Ipv4Addr {
        panic!("{SCAFFOLD}")
    }

    /// The node's guest gateway: preferred source of the host `lo` route and
    /// the guest's DNS server address.
    pub fn gateway(&self) -> Ipv4Addr {
        panic!("{SCAFFOLD}")
    }

    /// The claimed CID; available after the claim was taken.
    pub fn cid(&self) -> GuestCid {
        panic!("{SCAFFOLD}")
    }

    /// This launch's token.
    pub fn launch(&self) -> LaunchToken {
        panic!("{SCAFFOLD}")
    }

    /// Takes the claim out of the shared slot. Atomic: exactly one caller
    /// across all clones receives `Some`; every later call returns `None`.
    pub fn take_claim(&self) -> Option<ClaimedGuestCid> {
        panic!("{SCAFFOLD}")
    }
}

/// Compares `workload_addr`, `gateway`, `cid` and `launch` only.
impl PartialEq for GuestTransportAssignment {
    fn eq(&self, _other: &Self) -> bool {
        panic!("{SCAFFOLD}")
    }
}

impl Eq for GuestTransportAssignment {}

/// Claims guest CIDs on the host kernel (D16-CLAIM). Driven port; host adapter
/// `overdrive-host::VhostVsockCidClaim`, sim adapter
/// `overdrive-sim::SimGuestCidClaim`. A required constructor argument of the
/// pool.
pub trait GuestCidClaim: Send + Sync {
    /// Claims `cid` for a new device instance. `Ok`: the kernel holds `cid`
    /// for the returned handle and refuses it to every other instance until
    /// the handle (or the VMM it was handed to) releases it. `Err(InUse)`:
    /// another holder had it at the moment of the call; nothing is held. Any
    /// other `Err`: nothing is held; the cause is not "in use". Only
    /// `EADDRINUSE` is `InUse`; every other errno is `Kernel`. Each call opens
    /// a new device instance and issues `VHOST_VSOCK_SET_GUEST_CID` on it
    /// exactly once; no type offers a re-claim.
    fn claim(&self, cid: GuestCid) -> Result<ClaimedGuestCid, GuestCidClaimError>;

    /// Earned Trust: claim `GUEST_CID_PROBE`; a second claim of it must fail
    /// `InUse`; release; a third claim must succeed; release. Any deviation is
    /// `Err`. Leaves nothing held.
    fn probe(&self) -> Result<(), GuestCidClaimError>;
}

/// A CID held on the host kernel for one device instance. Move-only. Dropping
/// it releases the claim, unless the device was handed to a VMM.
#[must_use]
#[derive(Debug)]
pub struct ClaimedGuestCid {
    _scaffold: (),
}

impl ClaimedGuestCid {
    /// Wraps the device instance holding the claim of `cid`.
    pub fn new(_cid: GuestCid, _device: Box<dyn ClaimedVsockDevice>) -> Self {
        panic!("{SCAFFOLD}")
    }

    /// The claimed CID.
    pub fn cid(&self) -> GuestCid {
        panic!("{SCAFFOLD}")
    }

    /// Consumes the claim for handoff to the VMM.
    pub fn into_device(self) -> Box<dyn ClaimedVsockDevice> {
        panic!("{SCAFFOLD}")
    }
}

/// The device instance that holds a claim. It has no claim or re-claim method.
pub trait ClaimedVsockDevice: Send + fmt::Debug {
    /// The `/dev/vhost-vsock` file holding the claim, for descriptor handoff;
    /// `None` for a simulated claim. The handoff passes this open file
    /// description itself; reopening `/proc/self/fd/<n>` would create a new,
    /// unclaimed instance.
    fn vhost_fd(&self) -> Option<BorrowedFd<'_>>;
}

/// Why a CID claim or the claim probe failed.
#[derive(Debug, thiserror::Error)]
pub enum GuestCidClaimError {
    /// Another device instance held `cid` at the moment of the call.
    #[error("guest CID {cid} is held by another vhost-vsock device instance")]
    InUse {
        /// The CID that was refused.
        cid: GuestCid,
    },
    /// `/dev/vhost-vsock` could not be opened.
    #[error("/dev/vhost-vsock cannot be opened: {source}")]
    DeviceUnavailable {
        /// The open error.
        source: std::io::Error,
    },
    /// The claim failed for a cause other than "in use".
    #[error("claiming guest CID {cid} failed: {source}")]
    Kernel {
        /// The CID whose claim failed.
        cid: GuestCid,
        /// The kernel's error.
        source: std::io::Error,
    },
    /// The claim probe observed a deviation from the kernel contract.
    #[error("claim probe: {stage} did not behave as the kernel contract requires", stage = stage.as_str())]
    ProbeMismatch {
        /// The probe stage that deviated.
        stage: GuestCidClaimProbeStage,
    },
}

/// The claim probe's stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestCidClaimProbeStage {
    /// The first claim of `GUEST_CID_PROBE`.
    FirstClaim,
    /// The second claim, which must be refused `InUse`.
    SecondClaimRefused,
    /// The claim after release, which must succeed.
    ReclaimAfterRelease,
}

impl GuestCidClaimProbeStage {
    /// The stage's canonical label.
    pub const fn as_str(self) -> &'static str {
        panic!("Not yet implemented -- RED scaffold")
    }
}

// ---------------------------------------------------------------------------
// Guest intake constants (D25)
// ---------------------------------------------------------------------------

/// The guest TCP intake port on `workload_addr`.
pub const GUEST_TCP_INTAKE_PORT: u16 = 61_000;
/// Slot `n` binds `GUEST_UDP_SLOT_BASE + n`.
pub const GUEST_UDP_SLOT_BASE: u16 = 61_001;
/// Number of guest UDP slots (61,001..=62,024).
pub const GUEST_UDP_SLOTS: u16 = 1_024;
/// Guest parking cells (= the per-allocation flow quota).
pub const GUEST_TCP_PARK_CELLS: u32 = 4_096;
/// A UDP slot idle in both directions this long is released.
pub const GUEST_DATAGRAM_SLOT_IDLE: Duration = Duration::from_secs(120);

/// The reserved guest port range, inclusive: `GUEST_TCP_INTAKE_PORT ..=
/// GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1`.
pub fn guest_reserved_ports() -> RangeInclusive<u16> {
    panic!("{SCAFFOLD}")
}

// ---------------------------------------------------------------------------
// Node guest prefix and its route tag (DG-14)
// ---------------------------------------------------------------------------

/// The node's guest prefix (G-PL-7, DG-14). Composition step 4 and
/// `GuestFlowOwnerPorts` read it; the literals in `lib.rs` and the sims are
/// replaced by it.
pub const GUEST_NODE_PREFIX: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 0), 16);

/// The guest prefix's gateway (G-PL-7, DG-14): the `src` of the prefix's
/// `local` route and the guests' resolver.
pub const GUEST_NODE_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);

/// The route protocol tag `converge_prefix_route` writes on the prefix's
/// `local` route (G-PL-2, DG-14; unassigned in the kernel's `RTPROT_*` table).
/// The `prefix_landing` reader queries `ip route show proto 247`.
pub const GUEST_PREFIX_ROUTE_PROTOCOL: u8 = 247;

// ---------------------------------------------------------------------------
// Intake steering capacity (D8a-CAP) and intake host ports (INTAKE-BIND,
// INTAKE-PORTS)
// ---------------------------------------------------------------------------

/// Node capacity of steered intake listeners: the guest-prefix steering map's
/// `max_entries`, and the bound admission reserves declared TCP listener
/// ports against.
pub const GUEST_INTAKE_STEERING_MAX: u32 = 262_144;

/// First port of the host block intake listeners bind on their workload
/// address.
pub const GUEST_INTAKE_HOST_PORT_BASE: u16 = 20_480;

/// The most TCP listener ports one workload may declare, and the size of the
/// intake host-port block.
pub const GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX: u16 = 4_096;

/// The intake host-port block, inclusive (20,480..=24,575).
pub fn guest_intake_host_ports() -> RangeInclusive<u16> {
    panic!("{SCAFFOLD}")
}

/// The host port the intake listener for the declared TCP port at `index`
/// binds on its workload address: `GUEST_INTAKE_HOST_PORT_BASE + index`.
/// `None` for `index >= GUEST_INTAKE_PORTS_PER_ALLOCATION_MAX`.
pub fn guest_intake_host_port(_index: u16) -> Option<u16> {
    panic!("{SCAFFOLD}")
}
