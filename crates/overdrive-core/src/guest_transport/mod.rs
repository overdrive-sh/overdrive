//! Guest transport vocabulary — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Core vocabulary — `overdrive-core`" (the `GuestCid` /
//! `LaunchToken` / `GuestTransportAssignment` contract, CID-NOCLAIM and
//! ATTR-TOKEN, the guest intake constants of D25, the per-allocation quota
//! and probe cap of D-UP-12 / PD-4, the guest kit's module path of
//! EVMOD-LOAD) and DG-9 (the node guest prefix and gateway). The wire codec
//! lives in [`wire`]; the guest stream dial driving port (INB-1) in
//! [`dial`]; the flow forwarder's vocabulary and driven port (EVMOD-PORT) in
//! [`forwarder`].
//!
//! Core transport values, launch attribution and the wire/dial/forwarder
//! contracts are pinned by the feature delta. Remaining bridge-era items are
//! marked for removal with their last users at the attachment cut.
//!
//! Items marked **D-list** or **E-list** are on the feature-delta's
//! "Bridge-era code for DELIVER to delete" inventory (§ D, § E): they are not
//! part of the pinned contract, no test defends them, and DELIVER deletes
//! them together with their remaining users.

// SCAFFOLD: true — netns-density-295 DISTILL (vsock DELIVER step 2).
pub mod dial;
pub mod forwarder;
pub mod wire;

use std::fmt;
use std::net::Ipv4Addr;
use std::ops::RangeInclusive;
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

/// Host-unique vsock context identifier of one allocation's VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuestCid(u32);

impl GuestCid {
    /// Accepts `GUEST_CID_BASE..GUEST_CID_BASE + GUEST_CID_SPAN` only. The
    /// edge cases 0, 1, 2 and `u32::MAX` are `OutOfRange`.
    pub fn new(raw: u32) -> Result<Self, GuestCidError> {
        let end = GUEST_CID_BASE + GUEST_CID_SPAN;
        if (GUEST_CID_BASE..end).contains(&raw) {
            Ok(Self(raw))
        } else {
            Err(GuestCidError::OutOfRange { raw })
        }
    }

    /// The raw CID.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Decimal form.
impl fmt::Display for GuestCid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decimal form; validated by [`GuestCid::new`].
impl FromStr for GuestCid {
    type Err = GuestCidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !is_decimal(s) {
            return Err(GuestCidError::Unparseable);
        }
        let raw = s.parse::<u32>().map_err(|_| GuestCidError::Unparseable)?;
        Self::new(raw)
    }
}

impl Serialize for GuestCid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for GuestCid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
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

/// Host listens, `SOCK_STREAM`: guest-opened TCP flows and datagram
/// associations (EVMOD-DGRAM: one STREAM per association).
pub const GUEST_FLOW_STREAM_PORT: VsockPort = VsockPort::new(1240);
/// Guest listens, `SOCK_STREAM`: host-opened `TcpAccept` flows.
pub const GUEST_INBOUND_PORT: VsockPort = VsockPort::new(1242);
/// Host listens, `SOCK_STREAM`: the per-VM control session (D18a).
pub const GUEST_FLOW_CONTROL_PORT: VsockPort = VsockPort::new(1243);

// ---------------------------------------------------------------------------
// Node vsock listener ports (PORTS-CONFIG, ADR-0218)
// ---------------------------------------------------------------------------

/// How one node-global host vsock listener's port is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VsockListenPort {
    /// Bind exactly this port. Valid only above `LAST_RESERVED_PORT` (1023)
    /// and below `VMADDR_PORT_ANY` (`u32::MAX`); checked by
    /// [`NodeVsockPortConfig::new`].
    Fixed(VsockPort),
    /// Bind `VMADDR_PORT_ANY` (`-1U`, `include/uapi/linux/vm_sockets.h:109
    /// @ v7.0`): the kernel picks a port above `LAST_RESERVED_PORT` from a
    /// per-network-namespace counter seeded at random
    /// (`net/vmw_vsock/af_vsock.c:787-788 @ v7.0`, shared with outbound
    /// auto-bind, `:251-259`): it tries up to `MAX_PORT_RETRIES` (24, `:229`)
    /// consecutive candidates and fails `EADDRNOTAVAIL` when all of them are
    /// bound (`:792-810 @ v7.0`). The caller never assumes which port it
    /// gets; it reads it back from the owner.
    KernelAssigned,
}

/// The three node-global listeners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeVsockListener {
    /// The VM driver's beacon intake.
    Beacon,
    /// The guest-flow owner's stream listener.
    FlowStream,
    /// The guest-flow owner's control listener.
    FlowControl,
}

impl NodeVsockListener {
    /// `"beacon"` | `"flow_stream"` | `"flow_control"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Beacon => "beacon",
            Self::FlowStream => "flow_stream",
            Self::FlowControl => "flow_control",
        }
    }
}

impl fmt::Display for NodeVsockListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The node's configured vsock listener ports. Validated at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeVsockPortConfig {
    beacon: VsockListenPort,
    flow_stream: VsockListenPort,
    flow_control: VsockListenPort,
}

impl NodeVsockPortConfig {
    /// Err, first match in this order: a fixed port in 0..=1023 →
    /// `Reserved`; a fixed `u32::MAX` → `PortAny`; fixed and kernel-assigned
    /// ports mixed → `Mixed`; two equal fixed ports → `Collision` (listeners
    /// in [`NodeVsockListener`] order).
    pub fn new(
        _beacon: VsockListenPort,
        _flow_stream: VsockListenPort,
        _flow_control: VsockListenPort,
    ) -> Result<Self, NodeVsockPortConfigError> {
        panic!("{SCAFFOLD}")
    }

    /// All three `KernelAssigned` (what tests configure).
    pub const fn kernel_assigned() -> Self {
        Self {
            beacon: VsockListenPort::KernelAssigned,
            flow_stream: VsockListenPort::KernelAssigned,
            flow_control: VsockListenPort::KernelAssigned,
        }
    }

    /// The beacon listener's port.
    pub const fn beacon(&self) -> VsockListenPort {
        self.beacon
    }

    /// The guest-flow stream listener's port.
    pub const fn flow_stream(&self) -> VsockListenPort {
        self.flow_stream
    }

    /// The guest-flow control listener's port.
    pub const fn flow_control(&self) -> VsockListenPort {
        self.flow_control
    }
}

/// `Fixed(1234)`, `Fixed(1240)`, `Fixed(1243)`.
impl Default for NodeVsockPortConfig {
    fn default() -> Self {
        Self {
            beacon: VsockListenPort::Fixed(crate::vm::beacon::BEACON_VSOCK_PORT),
            flow_stream: VsockListenPort::Fixed(GUEST_FLOW_STREAM_PORT),
            flow_control: VsockListenPort::Fixed(GUEST_FLOW_CONTROL_PORT),
        }
    }
}

/// Why a port set is not a [`NodeVsockPortConfig`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeVsockPortConfigError {
    /// A fixed port in the reserved range.
    #[error("vsock port {port} for the {listener} listener is in the reserved range 0-1023")]
    Reserved {
        /// The listener configured with it.
        listener: NodeVsockListener,
        /// The rejected port.
        port: u32,
    },
    /// A fixed `VMADDR_PORT_ANY`.
    #[error(
        "vsock port 4294967295 (VMADDR_PORT_ANY) is not a fixed port for the {listener} listener; use 0 for a kernel-assigned port"
    )]
    PortAny {
        /// The listener configured with it.
        listener: NodeVsockListener,
    },
    /// Fixed and kernel-assigned ports mixed.
    #[error("vsock listener ports must be all fixed or all kernel-assigned")]
    Mixed,
    /// Two listeners on one fixed port.
    #[error("the {first} and {second} listeners are both configured on vsock port {port}")]
    Collision {
        /// The first listener, in [`NodeVsockListener`] order.
        first: NodeVsockListener,
        /// The second listener.
        second: NodeVsockListener,
        /// The shared port.
        port: u32,
    },
}

/// The ports the guest-flow owner's listeners are bound on (built by the
/// owner from its two binds' results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestFlowPorts {
    stream: VsockPort,
    control: VsockPort,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "the accessors' `&self` receivers are the pinned #295 interface contract"
)]
impl GuestFlowPorts {
    /// Err: either port in 0..=1023 or `u32::MAX` (`OutOfRange`), or the two
    /// equal (`Equal`). A bound port always passes (the kernel never assigns
    /// those values and the configuration refuses them).
    pub fn new(stream: VsockPort, control: VsockPort) -> Result<Self, GuestFlowPortsError> {
        for port in [stream, control] {
            let raw = port.as_u32();
            if raw <= 1_023 || raw == u32::MAX {
                return Err(GuestFlowPortsError::OutOfRange { port: raw });
            }
        }
        if stream == control {
            return Err(GuestFlowPortsError::Equal { port: stream.as_u32() });
        }
        Ok(Self { stream, control })
    }

    /// The stream listener's bound port.
    pub const fn stream(&self) -> VsockPort {
        self.stream
    }

    /// The control listener's bound port.
    pub const fn control(&self) -> VsockPort {
        self.control
    }
}

/// Why two ports are not a [`GuestFlowPorts`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuestFlowPortsError {
    /// In 0..=1023 or `u32::MAX`.
    #[error("vsock port {port} is not a valid guest-flow listener port")]
    OutOfRange {
        /// The rejected port.
        port: u32,
    },
    /// Stream and control equal.
    #[error("the guest-flow stream and control ports are both {port}")]
    Equal {
        /// The shared port.
        port: u32,
    },
}

// ---------------------------------------------------------------------------
// Launch token (ATTR-TOKEN, ADR-0177)
// ---------------------------------------------------------------------------

/// ATTR-TOKEN (ADR-0177): identifies one launch of one allocation's VM — the
/// lease, its CID and the one VMM that runs on it. 16 bytes drawn from the
/// injected [`Entropy`] port by `GuestAddressPool::assign`; never all zero
/// (all zero is the invalid encoding). Not a secret and not persisted. It
/// names one guest boot only because a guest reset ends the VMM (RESET-EXIT,
/// ADR-0182).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaunchToken([u8; 16]);

impl LaunchToken {
    /// `Err(LaunchTokenError::Zero)` for sixteen zero bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, LaunchTokenError> {
        if bytes == [0; 16] { Err(LaunchTokenError::Zero) } else { Ok(Self(bytes)) }
    }

    /// Draws a token; redraws until non-zero. Called only by the pool.
    pub fn generate(entropy: &dyn Entropy) -> Self {
        loop {
            let mut bytes = [0; 16];
            entropy.fill(&mut bytes);
            if let Ok(token) = Self::from_bytes(bytes) {
                return token;
            }
        }
    }

    /// The token's bytes.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Exactly 32 lowercase hex characters.
impl fmt::Display for LaunchToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Exactly 32 lowercase hex characters, case-sensitive; all zero is `Zero`.
impl FromStr for LaunchToken {
    type Err = LaunchTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let input = s.as_bytes();
        if input.len() != 32 {
            return Err(LaunchTokenError::Unparseable);
        }

        let mut bytes = [0; 16];
        for (index, pair) in input.chunks_exact(2).enumerate() {
            let Some(high_byte) = pair.first().copied() else {
                return Err(LaunchTokenError::Unparseable);
            };
            let Some(low_byte) = pair.get(1).copied() else {
                return Err(LaunchTokenError::Unparseable);
            };
            let Some(high) = lowercase_hex_digit(high_byte) else {
                return Err(LaunchTokenError::Unparseable);
            };
            let Some(low) = lowercase_hex_digit(low_byte) else {
                return Err(LaunchTokenError::Unparseable);
            };
            bytes[index] = (high << 4) | low;
        }
        Self::from_bytes(bytes)
    }
}

impl Serialize for LaunchToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for LaunchToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
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
// Allocation → driver handoff (CID-NOCLAIM, ATTR-TOKEN)
// ---------------------------------------------------------------------------

/// Allocation → driver handoff. Replaces `GuestNetworkAssignment`.
/// Constructed only by the pool's `assign`. Plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestTransportAssignment {
    workload_addr: Ipv4Addr,
    gateway: Ipv4Addr,
    cid: GuestCid,
    launch: LaunchToken,
    flow_ports: GuestFlowPorts,
}

impl GuestTransportAssignment {
    /// Builds the handoff.
    pub const fn new(
        workload_addr: Ipv4Addr,
        gateway: Ipv4Addr,
        cid: GuestCid,
        launch: LaunchToken,
        flow_ports: GuestFlowPorts,
    ) -> Self {
        Self { workload_addr, gateway, cid, launch, flow_ports }
    }

    /// GUEST-PORTS (ADR-0217): the guest-flow owner's bound stream and
    /// control ports; the VM driver writes them on the kernel command line
    /// beside its own intake's beacon port.
    pub const fn flow_ports(&self) -> GuestFlowPorts {
        self.flow_ports
    }

    /// The allocation's workload address.
    pub const fn workload_addr(&self) -> Ipv4Addr {
        self.workload_addr
    }

    /// The node's guest gateway: the guest's DNS server address, and the peer
    /// address every host-opened flow presents to the guest application
    /// (ADR-0174).
    pub const fn gateway(&self) -> Ipv4Addr {
        self.gateway
    }

    /// The leased CID, `GUEST_CID_BASE + lease_offset`.
    pub const fn cid(&self) -> GuestCid {
        self.cid
    }

    /// ATTR-TOKEN: this launch's token. The VM driver puts it on the kernel
    /// command line and checks it on READY; the owner checks it on every
    /// guest-opened connection of this CID and writes it into every flow
    /// request it opens to this CID.
    pub const fn launch(&self) -> LaunchToken {
        self.launch
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
/// EVMOD-PAIR: the most guest TcpConnect flows the guest owner holds before
/// `Paired` (accepted intake children not yet attached). An intake child
/// accepted beyond it is reset (the application sees `ECONNRESET`), counted
/// `guest_tcp_pending_exhausted`.
pub const GUEST_TCP_PENDING_MAX: u32 = 4_096;
/// A UDP slot idle in both directions this long is released.
pub const GUEST_DATAGRAM_SLOT_IDLE: Duration = Duration::from_secs(120);
/// The guest flow owner's audit tick (DG-5-A).
pub const GUEST_OWNER_AUDIT_PERIOD: Duration = Duration::from_secs(1);
/// The per-allocation flow quota (ADR-0208). Declared here, not in the host
/// owner crate, so that `overdrive-init` and the host owner share one SSOT
/// without the guest binary linking the control plane (D-UP-12, PD-4).
pub const GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION: u32 = 4_096;
/// The most probes one workload may declare across startup, readiness and
/// liveness together (D-UP-12, PD-4, EVMOD-AB). Each probe task holds at most
/// one dial in flight, and a probe dial is exempt from the allocation's flow
/// quota (PROBE-NOSAMPLE), so this bounds the node flow slots one allocation
/// holds outside its quota (a node-capacity bulkhead). A spec with more
/// probes is refused at deploy (`ParseError::ProbeCountExceeded`).
pub const GUEST_PROBE_SLOTS: u32 = 8;
/// EVMOD-LOAD: the fixed path in a Linux guest's root filesystem from which
/// `overdrive-init` loads the forwarder module of its guest kit.
pub const GUEST_FORWARDER_MODULE_PATH: &str = "/lib/overdrive/ovd_fwd.ko";

/// The reserved guest port range, inclusive: `GUEST_TCP_INTAKE_PORT ..=
/// GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1`.
#[expect(
    clippy::missing_const_for_fn,
    reason = "the feature delta pins this as a non-const public function"
)]
pub fn guest_reserved_ports() -> RangeInclusive<u16> {
    GUEST_TCP_INTAKE_PORT..=GUEST_UDP_SLOT_BASE + GUEST_UDP_SLOTS - 1
}

fn is_decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

const fn lowercase_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Node guest prefix, its gateway and the gateway route tag (DG-9, GW-LOCAL)
// ---------------------------------------------------------------------------

/// The node's guest prefix (G-PL-7, DG-9). Composition step 4 and
/// `GuestFlowOwnerPorts` read it; the literals in `lib.rs` and the sims are
/// replaced by it.
pub const GUEST_NODE_PREFIX: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(100, 95, 0, 0), 16);

/// The guest prefix's gateway (G-PL-7, DG-9): the only host-local address of
/// the prefix (its `local /32` route, GW-LOCAL), the guests' resolver, and
/// the peer every host-opened flow presents to the guest application.
pub const GUEST_NODE_GATEWAY: Ipv4Addr = Ipv4Addr::new(100, 95, 0, 1);

/// The route protocol tag on the gateway's `local /32` route (GW-LOCAL); boot
/// converges only a route carrying it. No other route covers the guest
/// prefix (LEGB-DIAL). Unassigned in the kernel's `RTPROT_*` table.
pub const GUEST_PREFIX_ROUTE_PROTOCOL: u8 = 247;

// ---------------------------------------------------------------------------
// D-list: intake steering capacity (D8a-CAP) and intake host ports
// (INTAKE-BIND, INTAKE-PORTS), removed by INB-1. Kept only while the
// `overdrive-dataplane` guest steering and the control-plane intake code
// still name them; DELIVER deletes them with those users.
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
