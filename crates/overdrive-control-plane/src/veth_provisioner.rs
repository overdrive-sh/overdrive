//! Single-node veth provisioner (adapter-host).
//!
//! Stands up the single-node veth pair in the **host** netns at serve
//! boot per ADR-0061 § 3. Unlike the Tier-3-only
//! [`crate::netns`-equivalent] `ThreeIfaceTopology` fixture in
//! `overdrive-testing` (which shells `ip netns add` and is dev-dep-only),
//! this is **production** code: the `overdrive serve` binary calls
//! [`provision`] before [`EbpfDataplane::new`] in the non-override boot
//! branch (wired in step 01-03). It therefore lives here in
//! `overdrive-control-plane` (`crate_class = "adapter-host"`), NOT in
//! `overdrive-testing`.
//!
//! Two surfaces:
//!
//! - [`derive_veth_plan`] — **pure** derivation (default lane, compiles
//!   on every platform). Computes the on-link gateway IP for the
//!   client-side veth (the first usable host address of the first VIP
//!   range, e.g. `10.96.0.0/24` → `10.96.0.1`) and the route
//!   `<vip_range> dev <client_iface>`. Per
//!   `.claude/rules/development.md` § "Persist inputs not derived
//!   state": the plan is derived at provision time from the range and is
//!   never persisted.
//! - [`converge_steps`] — **pure** per-resource desired-vs-actual diff
//!   (default lane, no I/O). Maps an [`ObservedVeth`] snapshot of actual
//!   kernel state to the minimal ordered [`VethStep`] set that converges
//!   the pair to its desired complete shape per ADR-0061 § 3.1 / § 3.2.
//! - [`provision`] — the real `ip(8)` shell-out (`#[cfg(target_os =
//!   "linux")]` production). **Idempotent converge-on-boot** per ADR-0061
//!   § 3.1: OBSERVE actual kernel state, compute [`converge_steps`], then
//!   EXECUTE each step idempotently (swallowing `EEXIST` / `File exists`
//!   on address/route add). A complete pair converges to all-noop; a
//!   half-provisioned pair (the crash-mid-provision case) is COMPLETED in
//!   place; a corrupted pair (client present, peer absent — § 3.2) is
//!   RECREATED. Never tears down a usable pair (DQ-4 leave-and-reuse).
//!
//! Two topologies live in this module:
//!
//! - **Single-node host-netns pair** ([`derive_veth_plan`] /
//!   [`converge_steps`] / [`provision`], ADR-0061 § 3) — the boot-time pair
//!   stood up directly in the **host** netns. No per-allocation namespace is
//!   involved on this path.
//! - **Per-allocation netns + veth pair** ([`derive_workload_netns_plan`] /
//!   [`workload_converge_steps`], transparent-mTLS / Path A, ADR-0071) — each
//!   live allocation gets its own Linux network namespace and a slot-keyed
//!   veth pair, so the agent has an agent-controlled routing point per
//!   workload (the nft-TPROXY PREROUTING hook fires on the host-side veth
//!   ingress). This path DOES use netns machinery: `workload_converge_steps`
//!   emits [`WorkloadVethStep::CreateNetns`] (`ip netns add`) and
//!   [`WorkloadVethStep::MoveWorkloadEndIntoNetns`]
//!   (`ip link set <if> netns <ns>`), among others.
//!
//! `CAP_NET_ADMIN` is already a precondition of serve boot (XDP attach +
//! cgroup delegation), so neither path adds a new privilege.

use ipnet::{IpAdd, Ipv4Net};
use std::net::Ipv4Addr;

/// Default client-facing veth name for the single-node host-netns pair
/// (ADR-0061 § 1). This is the SSOT consumed BOTH by
/// [`crate::dataplane_config::DataplaneConfig::single_node_veth`] (the
/// boot/test default config) AND by the serve-boot provision gate in
/// [`crate::run_server_with_obs_and_driver`] (step 01-03): provision
/// fires only when the configured ifaces equal these two names, so an
/// operator who names real NICs skips provision entirely. Both sites
/// reference these consts so the config default and the gate cannot
/// drift.
pub const DEFAULT_CLIENT_IFACE: &str = "ovd-veth-cli";

/// Default backend-facing veth peer name for the single-node host-netns
/// pair (ADR-0061 § 1). SSOT — see [`DEFAULT_CLIENT_IFACE`]. Distinct
/// from `DEFAULT_CLIENT_IFACE` by construction: a veth pair's two ends
/// MUST have different names, which is what makes the `EBUSY`
/// "attach two XDP programs to the same iface" failure structurally
/// unreachable (feature-delta § 6.4).
pub const DEFAULT_BACKEND_IFACE: &str = "ovd-veth-bk";

/// Derived plan for the single-node veth pair. A plain value object —
/// carries the literal interface names from config (not hardcoded), the
/// client-side on-link gateway address, the optional backend-side
/// gateway address, and the route CIDR (the VIP range made on-link on
/// the client veth).
///
/// Per § "Persist inputs not derived state" this plan is recomputed at
/// every provision from the config range; it is never persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VethProvisionPlan {
    /// Client-facing veth name (e.g. `ovd-veth-cli`) — from config.
    pub client_iface: String,
    /// Backend-facing veth peer name (e.g. `ovd-veth-bk`) — from config.
    pub backend_iface: String,
    /// On-link gateway address assigned to `client_iface`. This is the
    /// FIRST USABLE host of the VIP range, which makes every VIP in the
    /// range on-link from the host route the provisioner populates so
    /// `bpf_fib_lookup` resolves egress (ADR-0061 § 4).
    pub client_gateway: Ipv4Addr,
    /// Optional gateway address assigned to `backend_iface`. Derived as
    /// the SECOND usable host of the same VIP range for Phase-1
    /// single-range configs (the smallest honest rule — the e2e
    /// steering correctness is proven in step 01-04; this step proves
    /// derivation + idempotent provision). `None` only when the range
    /// has no second usable host (e.g. a `/31`).
    pub backend_gateway: Option<Ipv4Addr>,
    /// The VIP range, installed as an on-link route
    /// `<route_cidr> dev <client_iface>` so the VIPs are reachable.
    pub route_cidr: Ipv4Net,
}

/// Distinct failure modes of [`provision`]. One variant per `ip(8)`
/// invocation site per `.claude/rules/development.md` § Errors — never
/// collapse to a single `String` variant, so a caller can branch on
/// which step failed (and the operator gets a cause-specific message).
#[derive(Debug, thiserror::Error)]
pub enum VethProvisionError {
    /// `ip link show <cli>` itself failed to spawn or returned an error
    /// that is neither "present" nor "absent" (e.g. permission denied).
    #[error("`ip link show {iface}` failed (status={status:?}): {stderr}")]
    LinkShowFailed { iface: String, stderr: String, status: Option<i32> },
    /// `ip link add <cli> type veth peer name <bk>` failed.
    #[error(
        "`ip link add {client_iface} type veth peer name {backend_iface}` failed (status={status:?}): {stderr}"
    )]
    LinkAddFailed {
        client_iface: String,
        backend_iface: String,
        stderr: String,
        status: Option<i32>,
    },
    /// `ip addr add <cidr> dev <iface>` failed.
    #[error("`ip addr add {cidr} dev {iface}` failed (status={status:?}): {stderr}")]
    AddrAddFailed { iface: String, cidr: String, stderr: String, status: Option<i32> },
    /// `ip link del <iface>` failed (the § 3.2 RecreatePair teardown of a
    /// corrupted, Overdrive-owned half-pair). An "absent" failure is
    /// benign (already gone) and is swallowed before this surfaces.
    #[error("`ip link del {iface}` failed (status={status:?}): {stderr}")]
    LinkDelFailed { iface: String, stderr: String, status: Option<i32> },
    /// `ip link set <iface> up` failed.
    #[error("`ip link set {iface} up` failed (status={status:?}): {stderr}")]
    LinkUpFailed { iface: String, stderr: String, status: Option<i32> },
    /// `ip route add <cidr> dev <iface>` failed.
    #[error("`ip route add {cidr} dev {iface}` failed (status={status:?}): {stderr}")]
    RouteAddFailed { cidr: String, iface: String, stderr: String, status: Option<i32> },
    /// `ethtool -K <iface> tx off` failed for a non-benign reason. A
    /// "feature is fixed" / "not supported" non-zero exit is benign (the
    /// iface delivers a FULL checksum already, no disable needed) and is
    /// swallowed before this surfaces; a genuine failure (EPERM, the
    /// `ethtool` binary missing on a feature-bearing veth) is fatal —
    /// booting with TX offload still ON would corrupt every NAT'd packet
    /// (commit 62fa6be2), so refuse to boot rather than silently ship the
    /// landmine.
    #[error("`ethtool -K {iface} tx off` failed (status={status:?}): {stderr}")]
    TxOffloadDisableFailed { iface: String, stderr: String, status: Option<i32> },
    /// Spawning `ip(8)` itself failed (binary missing, etc.).
    #[error("spawning `ip(8)` failed: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Derive the [`VethProvisionPlan`] for the single-node veth pair from
/// the operator-supplied interface names and the first VIP range.
///
/// Pure — performs no I/O, deterministic (same inputs → same plan).
///
/// - `client_gateway` = first usable host of `vip_range`
///   (`10.96.0.0/24` → `10.96.0.1`).
/// - `backend_gateway` = second usable host of `vip_range`
///   (`10.96.0.0/24` → `10.96.0.2`), or `None` when the range has no
///   second usable host. This is the smallest honest Phase-1 rule per
///   ADR-0061 § 3; a second VIP range, when present, will supersede it
///   in a later phase.
/// - `route_cidr` = `vip_range` itself, installed as
///   `<vip_range> dev <client_iface>`.
#[must_use]
pub fn derive_veth_plan(
    client_iface: &str,
    backend_iface: &str,
    vip_range: Ipv4Net,
) -> VethProvisionPlan {
    let mut hosts = vip_range.hosts();
    // For every /24../30 range `hosts()` yields network()+1 first; for a
    // /31 it yields the two literal addresses; for a /32 it yields the
    // single host. `next()` is therefore the first usable host in every
    // case the allocator admits.
    let client_gateway = hosts.next().unwrap_or_else(|| vip_range.network());
    let backend_gateway = hosts.next();

    VethProvisionPlan {
        client_iface: client_iface.to_owned(),
        backend_iface: backend_iface.to_owned(),
        client_gateway,
        backend_gateway,
        route_cidr: vip_range,
    }
}

// =============================================================================
// Per-allocation netns + veth surface (transparent-mTLS enrollment, D-TME-2)
// =============================================================================
//
// Path A (ADR-0071) moves v1 OFF the single-node host-netns pair above ONTO a
// per-allocation Linux network namespace + veth pair, so the agent has an
// agent-controlled routing point per workload (the nft-TPROXY PREROUTING hook
// fires on the host-side veth ingress — spike `findings-egress-tproxy.md`).
//
// This is the parallel per-alloc surface to the host-netns surface above:
// `WorkloadNetnsPlan` ↔ `VethProvisionPlan`, `ObservedWorkloadVeth` ↔
// `ObservedVeth`, `WorkloadVethStep` ↔ `VethStep`, `workload_converge_steps`
// ↔ `converge_steps`. The host-netns surface STAYS — it is not retired here.
//
// The four spike-proven converge-on-boot host prerequisites
// (`findings-egress-tproxy.md` § "Design implications" 4 + § "Edge cases")
// are modeled as steps the provisioner OWNS: `ip_forward=1`, `rp_filter`
// relaxation on the host-side ingress veth + `all` + `lo`, and `tx off` on
// BOTH ends (the incremental-L4-csum invariant, `bpf.md` Rule 2). The
// leg-dial `SO_MARK` is NOT here — it belongs to the agent dial (step 03-03).

/// Per-allocation network-namespace prefix for the workload netns name
/// (`ovd-ns-<4hex-slot>`). SLOT-keyed, NOT alloc-id-keyed (B3): combined with a
/// 4-char hex [`NetSlot`] this yields an 11-char name, bounded ≤ NAME_MAX (255)
/// AND ≤ IFNAMSIZ (15) BY CONSTRUCTION — the identical shape to the two veth
/// names. An alloc-id-keyed netns would overflow NAME_MAX at 260 chars for a
/// 253-char [`overdrive_core::AllocationId`] (`ip netns add` → `ENAMETOOLONG`),
/// the same pigeonhole/ceiling class as the IFNAMSIZ veth-name overflow B1
/// closed. `ip netns list` shows `ovd-ns-<4hex>`; the human-readable alloc
/// identity is rendered by tooling against the 02-04 slot↔alloc map (the Cilium
/// `lxc<hex>` + `cilium endpoint list` model).
const WORKLOAD_NETNS_PREFIX: &str = "ovd-ns-";
/// Host-side veth-end name prefix (`ovd-hv-<4hex-slot>`). This is the end that
/// stays in the host netns, where nft-TPROXY PREROUTING intercepts the
/// workload's egress (now ingressing the host veth) and inbound traffic.
/// Combined with a 4-char hex [`NetSlot`] this yields an 11-char iface name,
/// inside the 15-char IFNAMSIZ limit BY CONSTRUCTION (D-TME-12).
const WORKLOAD_HOST_VETH_PREFIX: &str = "ovd-hv-";
/// In-netns veth-end name prefix (`ovd-wl-<4hex-slot>`). This end is moved
/// into the workload netns; the workload is born behind it. Same 11-char,
/// IFNAMSIZ-safe shape as [`WORKLOAD_HOST_VETH_PREFIX`].
const WORKLOAD_VETH_PREFIX: &str = "ovd-wl-";

/// The maximum [`NetSlot`] value: 4096 slots (`0..=4095`) carve 4096 contiguous
/// /30s (16384 addresses = a `/18`) out of the front of the
/// [`WORKLOAD_SUBNET_BASE`] /16 — `10.99.0.0`–`10.99.63.255` (the /16 has room
/// for far more; only the first /18 is allocated). The ceiling is the
/// pigeonhole companion to the 4-char hex name segment — a `u16` slot below
/// `0x1000` always renders as exactly 4 lowercase hex chars.
pub const NET_SLOT_MAX: u16 = 4095;

/// Per-host base block all per-allocation /30s are carved from. The full
/// `0..=NET_SLOT_MAX` slot space carves 4096 contiguous /30s (`base + slot*4`)
/// — a `/18` (`10.99.0.0`–`10.99.63.255`) out of the front of this /16; the
/// remainder of the /16 is unallocated headroom.
///
/// Fixed for Phase-1 single-node; making it operator-configurable is tracked
/// in <https://github.com/overdrive-sh/overdrive/issues/239> (do NOT make it
/// tunable here). `Ipv4Net::new_assert` is `const` in `ipnet` 2.x, so the
/// base is a compile-time constant; the `/16` prefix is statically valid.
pub const WORKLOAD_SUBNET_BASE: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(10, 99, 0, 0), 16);

/// A bounded per-allocation network slot in the range `0..=NET_SLOT_MAX`
/// (see [`NET_SLOT_MAX`]).
///
/// This is the host-unique, collision-free-BY-CONSTRUCTION index a stateful
/// allocator (step 02-04, NOT here) assigns to each live allocation. It is the
/// answer to the pigeonhole problem D-TME-12 / B1 raises: no pure function of a
/// 253-char [`overdrive_core::AllocationId`] can collision-free-map into a
/// 15-char (IFNAMSIZ) iface name, so the veth names are derived from this
/// bounded slot — rendered as a 4-char hex segment ([`Self::to_hex4`]) — NOT
/// from the alloc id. Distinct slots yield distinct iface names AND distinct
/// /30 subnets by construction, never by hash.
///
/// Construction is validating ([`Self::new`] / [`std::str::FromStr`] reject
/// `> NET_SLOT_MAX`); [`std::fmt::Display`] is the canonical DECIMAL form and
/// serde matches `Display` / `FromStr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetSlot(u16);

/// The error returned when a [`NetSlot`] value is out of range or unparseable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetSlotError {
    /// The value exceeds [`NET_SLOT_MAX`].
    #[error("net slot {value} exceeds maximum {max}")]
    OutOfRange { value: u16, max: u16 },
    /// The string is not a base-10 `u16` (the canonical [`NetSlot`] form).
    #[error("net slot {raw:?} is not a base-10 integer")]
    NotAnInteger { raw: String },
}

impl NetSlot {
    /// Construct a [`NetSlot`], rejecting any value beyond [`NET_SLOT_MAX`].
    ///
    /// # Errors
    ///
    /// Returns [`NetSlotError::OutOfRange`] when `value > NET_SLOT_MAX`.
    pub const fn new(value: u16) -> Result<Self, NetSlotError> {
        if value > NET_SLOT_MAX {
            return Err(NetSlotError::OutOfRange { value, max: NET_SLOT_MAX });
        }
        Ok(Self(value))
    }

    /// The IFNAMSIZ-bounded 4-char lowercase hex name segment for this slot
    /// (`0` → `"0000"`, `4095` → `"0fff"`). Because the slot is bounded below
    /// `0x1000`, this is ALWAYS exactly 4 chars, which — combined with the
    /// 7-char `ovd-ns-` / `ovd-hv-` / `ovd-wl-` prefix — yields an 11-char
    /// name, inside the 15-char IFNAMSIZ limit. The `{:04x}` zero-pad keeps
    /// every slot's name the same length so a future prefix change that would
    /// overflow fails the build-time const assertion just below this `impl` (a
    /// `cargo check` failure, not a runtime `ip link add`).
    #[must_use]
    pub fn to_hex4(self) -> String {
        format!("{:04x}", self.0)
    }
}

/// Build-time proof (N5) that every slot-keyed name fits IFNAMSIZ BY
/// CONSTRUCTION — a compile-time `const` assertion, so an overflowing prefix
/// fails `cargo check`, not a runtime `ip link add` / `ip netns add`.
///
/// [`NetSlot::to_hex4`] always renders exactly 4 chars (the slot is bounded
/// below `0x1000`), so the longest name any prefix produces is
/// `<prefix>.len() + 4`. IFNAMSIZ (15) is the tightest of the IFNAMSIZ-vs-
/// NAME_MAX ceilings, so satisfying it satisfies NAME_MAX (255) for the netns
/// too. All three prefixes — `ovd-ns-` (netns), `ovd-hv-`, `ovd-wl-` — are
/// asserted independently so a change to any ONE that overflowed would be
/// caught even if the three stopped being equal-length.
///
/// The fourth slot-derived axis — the /30 subnet — gets the symmetric guard
/// (S6): the full `0..=NET_SLOT_MAX` slot space carves /30s at
/// `base + slot*4`, so the TOP slot's /30 broadcast sits at
/// `NET_SLOT_MAX*4 + 3`. That offset must stay strictly inside
/// [`WORKLOAD_SUBNET_BASE`]'s address span (`2^(32 - prefix_len)`), or a future
/// `NET_SLOT_MAX` raise (or the #239 tunable base) would silently carve /30s
/// OUTSIDE the base — an out-of-base address-collision class the name-axis
/// guards cannot catch. `Ipv4Net::prefix_len()` is `const` in `ipnet` 2.x, so
/// this is pure const integer arithmetic and overflows fail `cargo check`, not
/// `ip addr add`.
const _: () = {
    const IFNAMSIZ: usize = 15;
    assert!(WORKLOAD_NETNS_PREFIX.len() + 4 <= IFNAMSIZ, "netns prefix + 4 hex must fit IFNAMSIZ");
    assert!(
        WORKLOAD_HOST_VETH_PREFIX.len() + 4 <= IFNAMSIZ,
        "host-veth prefix + 4 hex must fit IFNAMSIZ"
    );
    assert!(
        WORKLOAD_VETH_PREFIX.len() + 4 <= IFNAMSIZ,
        "workload-veth prefix + 4 hex must fit IFNAMSIZ"
    );

    // S6: the top slot's /30 broadcast must fall strictly inside the base's
    // address span. `base_span = 2^(32 - prefix_len)` is the count of
    // addresses in WORKLOAD_SUBNET_BASE; the highest offset any slot's /30
    // reaches is `NET_SLOT_MAX*4 + 3` (the top /30's broadcast). Keeping that
    // `< base_span` proves the whole slot space tiles WITHIN the base.
    let base_span: u32 = 1u32 << (32 - WORKLOAD_SUBNET_BASE.prefix_len() as u32);
    assert!(
        (NET_SLOT_MAX as u32 * 4 + 3) < base_span,
        "every slot's /30 must tile inside WORKLOAD_SUBNET_BASE (NET_SLOT_MAX*4+3 < base span)"
    );
};

impl std::fmt::Display for NetSlot {
    /// Canonical DECIMAL form — matches the serde representation and the
    /// [`std::str::FromStr`] parse.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for NetSlot {
    type Err = NetSlotError;

    /// Parse the canonical DECIMAL form, rejecting non-integers and any value
    /// beyond [`NET_SLOT_MAX`].
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let value: u16 =
            raw.parse().map_err(|_| NetSlotError::NotAnInteger { raw: raw.to_owned() })?;
        Self::new(value)
    }
}

impl serde::Serialize for NetSlot {
    /// Serialise as the canonical DECIMAL string (matches `Display` /
    /// `FromStr`).
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for NetSlot {
    /// Deserialise from the canonical DECIMAL string, enforcing the
    /// [`NET_SLOT_MAX`] bound (matches `FromStr`).
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Derived plan for a single allocation's netns + veth pair. A plain value
/// object — carries the per-alloc netns name, the two slot-derived veth-end
/// names, the host-side and in-netns addresses, the in-netns default-route
/// gateway (= the host-side address), the slot-derived /30 subnet, and the
/// node-local DNS responder address (an INPUT carried for the later
/// resolv.conf-injection step, D-TME-9 / Q5a; it is NOT derived state).
///
/// Per § "Persist inputs not derived state" this plan is recomputed at every
/// provision from `(slot, responder_addr)`; it is never persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadNetnsPlan {
    /// Per-allocation network-namespace name (`ovd-ns-<4hex-slot>`). SLOT-keyed
    /// (B3), so 11 chars ≤ NAME_MAX (255) and ≤ IFNAMSIZ (15) by construction —
    /// the identical shape to the two veth names.
    pub netns: String,
    /// Host-side veth-end name (`ovd-hv-<4hex-slot>`) — stays in the host
    /// netns; the nft-TPROXY PREROUTING interception point. SLOT-derived
    /// (not alloc-id-derived) so it fits IFNAMSIZ by construction (D-TME-12).
    pub host_veth: String,
    /// In-netns veth-end name (`ovd-wl-<4hex-slot>`) — moved into `netns`; the
    /// workload is born behind it. SLOT-derived, IFNAMSIZ-safe.
    pub workload_veth: String,
    /// Address assigned to the host-side end (`host_veth`). The FIRST usable
    /// host of `subnet`; also the in-netns default-route gateway.
    pub host_addr: Ipv4Addr,
    /// Address assigned to the in-netns end (`workload_veth`). The SECOND
    /// usable host of `subnet`.
    pub workload_addr: Ipv4Addr,
    /// In-netns default-route gateway — the host-side address, so the
    /// workload's egress leaves via the veth and ingresses the host-side end
    /// (`default via <host_addr> dev <workload_veth>`).
    pub gateway: Ipv4Addr,
    /// The per-allocation point-to-point /30 the two ends are addressed from
    /// (e.g. `10.99.0.0/30` for slot 0). Carved from [`WORKLOAD_SUBNET_BASE`]
    /// at `base + slot*4`; its prefix length (always 30) sizes the
    /// `ip addr add` CIDRs. Derived from the slot — never a caller parameter.
    pub subnet: Ipv4Net,
    /// Node-local DNS responder address (D-TME-9 / Q5a) written into the
    /// netns's `resolv.conf` by a LATER step. Carried as a plan INPUT — not
    /// derived state.
    pub responder_addr: Ipv4Addr,
}

/// Derive the [`WorkloadNetnsPlan`] for one allocation's netns + veth pair
/// from the host-unique network [`NetSlot`] and the node-local DNS responder
/// address (D-TME-12).
///
/// Pure — performs no I/O, deterministic (same inputs → same plan), total
/// (the /30 ALWAYS has two distinct usable hosts, so there is no fallback).
///
/// Every name and the subnet are SLOT-derived; the allocation id is NOT a
/// parameter (B3). With the slot keying all three names and the subnet, the
/// alloc id derives nothing here — the alloc↔slot binding lives in the 02-04
/// allocator map, not in this pure derivation.
///
/// - `netns` = `ovd-ns-<4hex-slot>` — SLOT-keyed 11-char name, ≤ NAME_MAX (255)
///   AND ≤ IFNAMSIZ (15) by construction (B3; an alloc-id-keyed netns would
///   overflow NAME_MAX at 260 chars for a 253-char alloc id).
/// - `host_veth` = `ovd-hv-<4hex-slot>`, `workload_veth` = `ovd-wl-<4hex-slot>`
///   — SLOT-derived 11-char names, IFNAMSIZ-safe and collision-free BY
///   CONSTRUCTION (distinct slots ⇒ distinct names; B1).
/// - `subnet` = the /30 at `WORKLOAD_SUBNET_BASE.network() + slot*4` — the
///   slot carves a /18 of contiguous /30s out of the /16; distinct slots ⇒
///   distinct /30s (S1, the derivation owns slot→/30; the subnet is NOT a
///   caller parameter).
/// - `host_addr` = `subnet.network() + 1` (first usable),
///   `workload_addr` = `subnet.network() + 2` (second usable). A /30 always
///   has exactly two usable hosts, so neither is an `Option` / `network()`
///   fallback (S2).
/// - `gateway` = `host_addr` (the in-netns default route points back at the
///   host-side end).
/// - `responder_addr` flows through verbatim (carried for D-TME-9; an INPUT,
///   not derived state).
#[must_use]
pub fn derive_workload_netns_plan(slot: NetSlot, responder_addr: Ipv4Addr) -> WorkloadNetnsPlan {
    let hex = slot.to_hex4();

    // Carve the per-allocation /30 from the fixed base: slot N owns the four
    // contiguous addresses at base + N*4. A /30 always has exactly two usable
    // hosts (net+1, net+2), so the addressing is total — no Option / fallback.
    let network = WORKLOAD_SUBNET_BASE.network().saturating_add(u32::from(slot.0) * 4);
    let subnet = Ipv4Net::new(network, 30)
        .unwrap_or_else(|_| unreachable!("/30 is a statically-valid prefix; new() cannot fail"));
    let host_addr = network.saturating_add(1);
    let workload_addr = network.saturating_add(2);

    WorkloadNetnsPlan {
        netns: format!("{WORKLOAD_NETNS_PREFIX}{hex}"),
        host_veth: format!("{WORKLOAD_HOST_VETH_PREFIX}{hex}"),
        workload_veth: format!("{WORKLOAD_VETH_PREFIX}{hex}"),
        host_addr,
        workload_addr,
        gateway: host_addr,
        subnet,
        responder_addr,
    }
}

/// Observed actual kernel state of one allocation's netns + veth pair — the
/// input to the pure [`workload_converge_steps`] diff. Each field is a single
/// observable fact a thin observer reads from the kernel (`ip netns list`,
/// `ip -n <ns> link/addr/route`, `sysctl`, `ethtool -k`) per the
/// converge-on-boot model (ADR-0061 § 3.1). Modeling actual state as a plain
/// value object keeps the converge diff pure and exhaustively unit-testable
/// in the default lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "fifteen independent observed kernel facts (netns presence, host-veth/workload-veth \
              presence, in-netns move, per-end addr, host-end up, in-netns-end up, netns lo up, \
              default route, per-end tx-offload, ip_forward, global rp_filter, host-veth \
              rp_filter); a flag-per-fact value object is the clearest model of the converge input \
              and mirrors the host-netns ObservedVeth shape ADR-0061 § 3.1 prescribes"
)]
pub struct ObservedWorkloadVeth {
    /// The per-alloc netns (`ovd-ns-<4hex-slot>`) exists.
    pub netns_present: bool,
    /// The host-side veth end (`ovd-hv-<4hex-slot>`) exists in the host netns.
    pub host_veth_present: bool,
    /// The in-netns veth end (`ovd-wl-<4hex-slot>`) exists (in either netns).
    pub workload_veth_present: bool,
    /// The in-netns veth end has been MOVED into the workload netns (it is
    /// no longer in the host netns).
    pub workload_veth_in_netns: bool,
    /// The host-side end carries the desired host address.
    pub host_addr_present: bool,
    /// The in-netns end carries the desired in-netns address.
    pub workload_addr_present: bool,
    /// The host-side end is administratively UP.
    pub host_veth_up: bool,
    /// The in-netns end is administratively UP. Without it the netns cannot
    /// carry a packet (B2); ordered AFTER the in-netns move.
    pub workload_veth_up: bool,
    /// The netns loopback (`lo`) is administratively UP. A netns is born with
    /// `lo` DOWN; without bringing it up the netns cannot carry a packet (B2).
    pub lo_up: bool,
    /// The in-netns default route (`default via <host_addr>`) is present.
    pub default_route_present: bool,
    /// The host-side end still has TX-checksum-offload ON.
    pub host_tx_offload_on: bool,
    /// The in-netns end still has TX-checksum-offload ON.
    pub workload_tx_offload_on: bool,
    /// Host `net.ipv4.ip_forward` is `1` (the spike-proven egress-routing
    /// prerequisite — without forwarding the host won't route to the
    /// lo-bound backend).
    pub ip_forward_enabled: bool,
    /// The GLOBAL `rp_filter` relaxation is in place (`net.ipv4.conf.all` +
    /// `net.ipv4.conf.lo`). Host-global; survives a per-alloc veth rebuild
    /// (the spike-proven asymmetric-ingress prerequisite — without it the
    /// in-via-veth / local-table-reinject-via-lo path is dropped as a false
    /// "no fire"). Split from the per-host-veth relaxation below (S3).
    pub rp_filter_global_relaxed: bool,
    /// The PER-HOST-VETH `rp_filter` relaxation is in place
    /// (`net.ipv4.conf.<host_veth>`). A freshly created veth defaults STRICT,
    /// so a rebuilt pair always re-needs this (independent of the global
    /// relaxation above — S3, the lossy single bool is replaced by these two).
    pub host_veth_rp_filter_relaxed: bool,
}

/// A single idempotent convergence action the executor applies (via `ip
/// netns` / `ip -n <ns> …` / `sysctl` / `ethtool`). The ordered
/// `Vec<WorkloadVethStep>` from [`workload_converge_steps`] is the minimal
/// set of steps that brings an [`ObservedWorkloadVeth`] to the desired
/// complete shape. Ordering is load-bearing: the netns and pair must exist
/// before the in-netns end is moved; the move must precede in-netns
/// addressing and the default route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadVethStep {
    /// `ip netns add <netns>` — the per-alloc netns is absent.
    CreateNetns,
    /// `ip link add <workload_veth> type veth peer name <host_veth>` — the
    /// pair is absent.
    CreateVethPair,
    /// `ip link set <workload_veth> netns <netns>` — move the in-netns end
    /// into the workload netns.
    MoveWorkloadEndIntoNetns,
    /// `ip addr add <host_addr>/<prefix> dev <host_veth>` (host netns).
    AddHostAddr,
    /// `ip -n <netns> addr add <workload_addr>/<prefix> dev <workload_veth>`.
    AddWorkloadAddr,
    /// `ip link set <host_veth> up` (host netns).
    SetHostVethUp,
    /// `ip -n <netns> link set <workload_veth> up` — bring the in-netns end
    /// administratively UP (B2). Ordered AFTER `MoveWorkloadEndIntoNetns`: the
    /// end must be inside the netns before it can be brought up there. Without
    /// it the in-netns end stays DOWN and the netns cannot carry a packet.
    SetWorkloadVethUp,
    /// `ip -n <netns> link set lo up` — bring the netns loopback UP (B2). A
    /// netns is born with `lo` DOWN; the local-table reinject (and any
    /// loopback-bound service) needs it up, so a netns provisioned from the
    /// plan can carry a packet.
    SetLoopbackUp,
    /// `ip -n <netns> route add default via <gateway> dev <workload_veth>`.
    AddDefaultRoute,
    /// `sysctl -w net.ipv4.ip_forward=1` — the spike-proven egress-routing
    /// prerequisite.
    EnableIpForward,
    /// Relax the GLOBAL `rp_filter` (`net.ipv4.conf.all` + `net.ipv4.conf.lo`)
    /// — the spike-proven asymmetric-ingress prerequisite. Host-global;
    /// emitted when the global relaxation is missing (S3, split from the
    /// per-host-veth relax below).
    RelaxGlobalRpFilter,
    /// Relax the PER-HOST-VETH `rp_filter` (`net.ipv4.conf.<host_veth>`). A
    /// freshly created veth defaults STRICT, so this is re-emitted on every
    /// pair rebuild — independent of the global relaxation (S3).
    RelaxHostVethRpFilter,
    /// `ethtool -K <host_veth> tx off` — disable TX-checksum-offload on the
    /// host-side end (the incremental-L4-csum invariant, `bpf.md` Rule 2 /
    /// commit 62fa6be2).
    DisableHostTxOffload,
    /// `ethtool -K <workload_veth> tx off` (in-netns end) — same invariant
    /// for the in-netns end.
    DisableWorkloadTxOffload,
}

/// Compute the minimal ordered set of [`WorkloadVethStep`]s that converges
/// one allocation's netns + veth pair from its `observed` actual state to the
/// desired complete shape the `plan` describes (ADR-0061 § 3.1 Bar-1,
/// per-allocation parallel of [`converge_steps`]).
///
/// PURE — no I/O, deterministic (same inputs → same step vec).
///
/// Convergence rules (idempotent converge-on-boot, ADR-0061 § 3.1):
///
/// - **Complete** (every fact satisfied) → empty step set (all-noop): a
///   re-provision over a good alloc does nothing.
/// - **Netns absent** → `CreateNetns` first; a fresh netns implies the pair
///   must be (re)built and every veth-dependent step re-run.
/// - **Pair absent** (netns may be present) → `CreateVethPair`, then the move
///   + every veth-dependent step. The netns is NEVER torn down to rebuild the
///   pair — a present netns is usable and survives (never tear down a usable
///   resource).
/// - **Present netns + pair** → emit only the MISSING resources:
///   `MoveWorkloadEndIntoNetns` when the in-netns end has not been moved,
///   `AddHostAddr` / `AddWorkloadAddr` when an address is absent,
///   `SetHostVethUp` when the host end is down, `SetWorkloadVethUp` when the
///   in-netns end is down (B2), `SetLoopbackUp` when the netns `lo` is down
///   (B2), `AddDefaultRoute` when the in-netns default route is absent. The
///   two up-steps are ordered AFTER `MoveWorkloadEndIntoNetns` — a netns
///   provisioned from the plan must be able to carry a packet.
/// - **Host prerequisites** → `EnableIpForward` when `ip_forward` is off,
///   `RelaxGlobalRpFilter` when the GLOBAL `rp_filter` relaxation is missing
///   (host-global; survives a veth rebuild), `RelaxHostVethRpFilter` when the
///   pair was freshly (re)created (a fresh veth defaults STRICT — S3) OR when
///   the per-host-veth relaxation is missing, `DisableHostTxOffload` /
///   `DisableWorkloadTxOffload` when the respective end still has TX-offload
///   ON (or the pair was freshly (re)created — a new veth defaults to offload
///   ON). The global rp_filter / ip_forward facts are host-global and
///   converge independently of the netns/pair shape; the per-host-veth
///   rp_filter and the tx-offload facts are per-veth and re-emit on rebuild.
#[must_use]
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "the desired-vs-actual diff signature `(&plan, &observed)` is the reconciler-shaped \
              contract ADR-0061 § 3.1 prescribes (mirrors `converge_steps`); ObservedWorkloadVeth \
              is borrowed for symmetry with the plan and to stay stable if observed facts grow"
)]
pub fn workload_converge_steps(
    plan: &WorkloadNetnsPlan,
    observed: &ObservedWorkloadVeth,
) -> Vec<WorkloadVethStep> {
    // N1: the plan carries the names/addresses the 02-02 executor needs; the
    // pure diff below keys ONLY on the observed facts, so the body does not
    // read `plan`. It stays in the signature to mirror `converge_steps` and to
    // feed the executor — this is the contract, not a dead parameter.
    let _ = plan;
    let mut steps = Vec::new();

    // Netns first. A fresh netns means the pair must be (re)built and every
    // veth-dependent step re-run.
    if !observed.netns_present {
        steps.push(WorkloadVethStep::CreateNetns);
    }

    // Pair shape. A (re)create produces a clean pair, so the downstream
    // move/addr/up/route/tx-off steps are unconditionally needed afterwards.
    // The netns itself is never torn down to rebuild the pair (it is usable).
    let pair_rebuilt =
        !observed.netns_present || !observed.workload_veth_present || !observed.host_veth_present;
    if pair_rebuilt {
        steps.push(WorkloadVethStep::CreateVethPair);
    }

    // Move the in-netns end into the netns: needed when freshly (re)built OR
    // when it has not yet been moved.
    if pair_rebuilt || !observed.workload_veth_in_netns {
        steps.push(WorkloadVethStep::MoveWorkloadEndIntoNetns);
    }
    // Host-side address: needed when freshly (re)built OR when missing.
    if pair_rebuilt || !observed.host_addr_present {
        steps.push(WorkloadVethStep::AddHostAddr);
    }
    // In-netns address: needed when freshly (re)built OR when missing.
    if pair_rebuilt || !observed.workload_addr_present {
        steps.push(WorkloadVethStep::AddWorkloadAddr);
    }
    // Host-side end up: needed when freshly (re)built OR when down.
    if pair_rebuilt || !observed.host_veth_up {
        steps.push(WorkloadVethStep::SetHostVethUp);
    }
    // In-netns end up (B2): needed when freshly (re)built OR when down.
    // Ordered AFTER the move — the end must be inside the netns first.
    if pair_rebuilt || !observed.workload_veth_up {
        steps.push(WorkloadVethStep::SetWorkloadVethUp);
    }
    // Netns loopback up (B2): a netns is born with `lo` DOWN. A fresh netns
    // (CreateNetns) always re-needs it; otherwise emit only when `lo` is down.
    if !observed.netns_present || !observed.lo_up {
        steps.push(WorkloadVethStep::SetLoopbackUp);
    }
    // In-netns default route: needed when freshly (re)built OR when absent.
    if pair_rebuilt || !observed.default_route_present {
        steps.push(WorkloadVethStep::AddDefaultRoute);
    }

    // Spike-proven host prerequisites. ip_forward + the GLOBAL rp_filter are
    // host-global and converge independently of the netns/pair shape; the
    // per-host-veth rp_filter and tx-offload are per-veth and re-emit on a
    // rebuild (a fresh veth defaults strict rp_filter / offload ON).
    if !observed.ip_forward_enabled {
        steps.push(WorkloadVethStep::EnableIpForward);
    }
    if !observed.rp_filter_global_relaxed {
        steps.push(WorkloadVethStep::RelaxGlobalRpFilter);
    }
    // Per-host-veth rp_filter relaxation: emit when freshly (re)built (a new
    // veth defaults STRICT — S3) OR when the relaxation is missing.
    if pair_rebuilt || !observed.host_veth_rp_filter_relaxed {
        steps.push(WorkloadVethStep::RelaxHostVethRpFilter);
    }
    // TX-checksum-offload: emit when freshly (re)built (a new veth defaults
    // to offload ON) OR when the respective end still has it ON.
    if pair_rebuilt || observed.host_tx_offload_on {
        steps.push(WorkloadVethStep::DisableHostTxOffload);
    }
    if pair_rebuilt || observed.workload_tx_offload_on {
        steps.push(WorkloadVethStep::DisableWorkloadTxOffload);
    }

    steps
}

/// Observed actual kernel state of the single-node veth pair — the
/// input to the pure [`converge_steps`] diff. Each field is a single
/// observable fact the thin observer reads from the kernel
/// (`ip link show` for presence/up-state, `getifaddrs(3)` for address
/// presence) per ADR-0061 § 3.1. Modeling the actual state as a plain
/// value object keeps the converge diff pure and exhaustively
/// unit-testable in the default lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "eight independent observed kernel facts (presence/addr/up/tx-offload × client/peer); \
              a flag-per-fact value object is the clearest model of the converge input \
              and is the shape ADR-0061 § 3.1 prescribes"
)]
pub struct ObservedVeth {
    /// `<client_iface>` exists as a netdev.
    pub client_present: bool,
    /// `<backend_iface>` (the declared peer) exists as a netdev.
    pub peer_present: bool,
    /// `<client_iface>` carries the desired client gateway IPv4 address.
    pub client_addr_present: bool,
    /// `<backend_iface>` carries the desired backend gateway IPv4 address
    /// (only meaningful when the plan has a `backend_gateway`).
    pub backend_addr_present: bool,
    /// `<client_iface>` is administratively UP.
    pub client_up: bool,
    /// `<backend_iface>` is administratively UP.
    pub backend_up: bool,
    /// `<client_iface>` still has TX-checksum-offload ON (`ethtool -k
    /// <client>` reports `tx-checksumming: on`). When `true`, converge
    /// emits [`VethStep::DisableClientTxOffload`] to turn it off — the
    /// incremental L4-csum invariant (commit 62fa6be2) requires it OFF.
    pub client_tx_offload_on: bool,
    /// `<backend_iface>` still has TX-checksum-offload ON. When `true`,
    /// converge emits [`VethStep::DisableBackendTxOffload`].
    pub backend_tx_offload_on: bool,
}

/// A single idempotent convergence action the executor applies via
/// `ip(8)`. The ordered `Vec<VethStep>` from [`converge_steps`] is the
/// minimal set of steps that brings an [`ObservedVeth`] to the desired
/// complete shape. Ordering is load-bearing: the pair must exist before
/// addresses can be assigned, and (re)creating the pair subsumes every
/// downstream step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VethStep {
    /// Delete BOTH ends and recreate the pair from scratch — the corrupted
    /// edges where exactly one end of the declared pair is present (§ 3.2
    /// forward: client present, peer absent; OR inverse: client absent,
    /// peer present). The executor dels the client end then the backend
    /// end; `link_del` swallows "absent", so whichever end survives is
    /// reaped before `link_add` restores the atomic pair. Deleting both
    /// (rather than relying on "del one reaps both") is what clears a
    /// surviving/colliding peer on the inverse edge and avoids the
    /// `link_add` "File exists" boot refusal.
    RecreatePair,
    /// `ip link add <client> type veth peer name <backend>` — the pair
    /// is wholly absent (first boot).
    CreatePair,
    /// `ip addr add <client_gateway>/<prefix> dev <client>`.
    AddClientAddr,
    /// `ip addr add <backend_gateway>/<prefix> dev <backend>` (only when
    /// the plan derives a backend gateway).
    AddBackendAddr,
    /// `ip link set <client> up`.
    SetClientUp,
    /// `ip link set <backend> up`.
    SetBackendUp,
    /// `ethtool -K <client> tx off` — disable TX-checksum-offload on the
    /// client end. Emitted only when the client end's offload is still
    /// ON (or the pair was freshly (re)created, since a new veth defaults
    /// to offload ON). The dual-XDP NAT programs fix the L4 checksum
    /// INCREMENTALLY (RFC 1624 — `crates/overdrive-bpf/src/shared/csum.rs`,
    /// commit 62fa6be2), which requires the packet at the *receiving* XDP
    /// hook to carry a FULL L4 checksum. With TX offload ON, a
    /// locally-generated frame leaves the *sending* veth as
    /// `CHECKSUM_PARTIAL` (the on-wire field holds only the pseudo-header
    /// sum), and the incremental delta on that partial value is garbage —
    /// every packet's checksum is corrupted. Disabling offload forces the
    /// kernel to materialise the FULL checksum in software before the
    /// frame leaves the sender, restoring a valid base for the delta.
    DisableClientTxOffload,
    /// `ethtool -K <backend> tx off` — same as
    /// [`VethStep::DisableClientTxOffload`] for the backend end. Both ends
    /// send and receive (forward DNAT reads the client→backend direction;
    /// reverse SNAT reads backend→client), so the sender's tx-off on BOTH
    /// ends is what makes each direction's receive-side ingress checksum
    /// valid.
    DisableBackendTxOffload,
    /// `ip route add <route_cidr> dev <client>` — always attempted
    /// idempotently (the connected route the kernel auto-creates on
    /// address assignment legitimately collides with `File exists`).
    AddRoute,
}

/// Compute the minimal ordered set of [`VethStep`]s that converges the
/// veth pair from its `observed` actual state to the desired complete
/// shape the `plan` describes (ADR-0061 § 3.1 / § 3.2).
///
/// PURE — no I/O, deterministic (same inputs → same step vec). This is
/// the per-resource desired-vs-actual diff at the heart of the
/// converge-on-boot model; the thin executor in [`provision`] applies
/// the returned steps in order.
///
/// Convergence rules:
///
/// - **Pair wholly absent** → `[CreatePair, …]` then every downstream
///   step (a freshly created pair has no addresses and is down).
/// - **Client present but peer absent** (§ 3.2 corrupted edge) →
///   `[RecreatePair, …]` then every downstream step (the recreate
///   produces a clean pair that needs full address/up/route convergence).
/// - **Pair present** → add only the MISSING resources: `AddClientAddr`
///   when the client address is absent, `AddBackendAddr` when the plan
///   has a backend gateway and that address is absent, `SetClientUp` /
///   `SetBackendUp` when an end is down.
/// - **TX-checksum-offload** → `DisableClientTxOffload` /
///   `DisableBackendTxOffload` emitted only when the respective end still
///   has offload ON (or the pair was freshly (re)created — a new veth
///   defaults to offload ON). An already-offload-off end emits nothing,
///   so a re-run converges to a no-op. This completes the incremental
///   L4-checksum production-correctness invariant (commit 62fa6be2): the
///   receive-side XDP NAT hook needs a FULL (not `CHECKSUM_PARTIAL`)
///   ingress checksum as the base for its O(1) delta fixup.
/// - **Route** → `AddRoute` is ALWAYS emitted (the executor swallows the
///   `File exists` collision), so a complete pair still converges to
///   exactly `[AddRoute]` — a single idempotent noop — rather than an
///   empty vec. This keeps the route reachable even if a prior boot
///   created the addresses but not the explicit route.
#[must_use]
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "the desired-vs-actual diff signature `(&plan, &observed)` is the reconciler-shaped \
              contract ADR-0061 § 3.1 prescribes and a stepping-stone to the issue #197 port trait; \
              ObservedVeth is borrowed for symmetry with the plan and to stay stable if facts grow"
)]
pub fn converge_steps(plan: &VethProvisionPlan, observed: &ObservedVeth) -> Vec<VethStep> {
    let mut steps = Vec::new();

    // Pair-level shape first. A (re)create produces a clean pair, so the
    // downstream address/up steps are unconditionally needed afterwards.
    let recreated = match (observed.client_present, observed.peer_present) {
        (false, false) => {
            steps.push(VethStep::CreatePair);
            true
        }
        // Exactly one end present — a corrupted edge:
        //   (true, false): § 3.2 forward — client present, declared peer
        //                   absent (peer separately moved/deleted).
        //   (false, true): inverse — client absent, declared peer present
        //                   (client moved/renamed, or an unrelated iface
        //                   collides on the backend name).
        // Both must RecreatePair, which now dels BOTH ends (see
        // execute_step) so the surviving/colliding end is reaped before
        // link_add — avoiding the "File exists" boot refusal a bare
        // CreatePair would hit on the inverse edge.
        (true, false) | (false, true) => {
            steps.push(VethStep::RecreatePair);
            true
        }
        (true, true) => false,
    };

    // Client address: needed when freshly (re)created OR when missing.
    if recreated || !observed.client_addr_present {
        steps.push(VethStep::AddClientAddr);
    }
    // Backend address: only when the plan derives a backend gateway, and
    // needed when freshly (re)created OR when missing.
    if plan.backend_gateway.is_some() && (recreated || !observed.backend_addr_present) {
        steps.push(VethStep::AddBackendAddr);
    }
    // Bring ends up: needed when freshly (re)created OR when down.
    if recreated || !observed.client_up {
        steps.push(VethStep::SetClientUp);
    }
    if recreated || !observed.backend_up {
        steps.push(VethStep::SetBackendUp);
    }
    // Disable TX-checksum-offload: needed when freshly (re)created (a new
    // veth defaults to offload ON) OR when an existing iface still has it
    // ON. Mirrors the SetClientUp / SetBackendUp emit-only-when-needed
    // shape so a re-run over an already-offload-off pair converges to a
    // no-op (the converge-on-boot guarantee, ADR-0061 § 3.1). This is the
    // production equivalent of the Tier-3 fixture's `ethtool_tx_off`
    // (overdrive-testing `ThreeIfaceTopology::create`); it completes the
    // incremental-L4-csum production-correctness invariant from commit
    // 62fa6be2 (without offload OFF the receive-side XDP hook folds the
    // NAT delta into a CHECKSUM_PARTIAL base and corrupts every packet).
    if recreated || observed.client_tx_offload_on {
        steps.push(VethStep::DisableClientTxOffload);
    }
    if recreated || observed.backend_tx_offload_on {
        steps.push(VethStep::DisableBackendTxOffload);
    }
    // Route is always attempted idempotently.
    steps.push(VethStep::AddRoute);

    steps
}

/// Provision the single-node veth pair in the host netns from `plan`.
///
/// **Idempotent converge-on-boot** (ADR-0061 § 3.1 / § 3.2): OBSERVE the
/// actual kernel state ([`observe`]), compute the per-resource diff
/// ([`converge_steps`]), then EXECUTE each step idempotently. A complete
/// pair converges to an all-noop (`AddRoute` swallows `File exists`); a
/// half-provisioned pair — created by a serve boot that crashed after
/// `ip link add` but before address/up/route assignment — is COMPLETED
/// in place; a corrupted pair (client present, declared peer absent —
/// § 3.2) is RECREATED. Never tears down a *usable* pair (DQ-4
/// leave-and-reuse). The provisioner therefore tolerates being
/// interrupted at any point and re-run from the top across reboots
/// (research R7 self-heal).
///
/// Synchronous (`std::process::Command`) — provisioning is a boot-time
/// one-shot, so the sync shape (matching `ThreeIfaceTopology::create`)
/// is simplest and avoids dragging the `ip` shell-out into an `async fn`
/// (which the dst-lint async-fs gate would otherwise scrutinise).
///
/// # Errors
///
/// Returns a distinct [`VethProvisionError`] variant per failing `ip(8)`
/// step (link-show, link-add, link-del, addr-add, link-up, route-add) so
/// the caller can branch on which boot step failed. `EEXIST` /
/// `File exists` on address and route add is swallowed (already-present
/// is the success case, not a failure).
pub fn provision(plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    let observed = observe(plan)?;
    for step in converge_steps(plan, &observed) {
        execute_step(plan, step)?;
    }
    Ok(())
}

/// Read the actual kernel state of the pair into an [`ObservedVeth`].
///
/// Presence + up-state come from `ip link show <iface>` (exit 0 +
/// `state UP` / `UP` flag); address presence comes from
/// [`crate::iface::resolve_iface_ipv4`] matching the desired gateway —
/// the same `getifaddrs(3)` walk the downstream boot path uses, so the
/// observer and the consumer agree on what "address present" means.
fn observe(plan: &VethProvisionPlan) -> Result<ObservedVeth, VethProvisionError> {
    let (client_present, client_up) = link_state(&plan.client_iface)?;
    let (peer_present, backend_up) = link_state(&plan.backend_iface)?;

    let client_addr_present =
        client_present && iface_has_addr(&plan.client_iface, plan.client_gateway);
    // No backend gateway derived (e.g. /31) → the address is "present" by
    // vacuous truth so converge never emits AddBackendAddr.
    let backend_addr_present = plan
        .backend_gateway
        .is_none_or(|gw| peer_present && iface_has_addr(&plan.backend_iface, gw));

    // TX-offload is only meaningful for a present iface; an absent iface
    // reports `false` (off). When the iface is absent the pair is
    // (re)created, so the `recreated` path in converge_steps re-emits the
    // disable regardless — the false here never suppresses a needed step.
    //
    // The two `&&` short-circuits are an impure-observer I/O shim whose
    // `&&`→`||` mutant is end-state-INSENSITIVE: the downstream
    // DisableTxOffload step is idempotent, so whether observe reports on
    // or off, a second provision converges to offload-off either way and
    // no end-state assertion can distinguish the mutant. Same untestable
    // class as the sibling `client_present && iface_has_addr(...)` guard
    // above. The KILLABLE decision logic lives in the pure
    // `converge_steps` (fully mutation-covered).
    // mutants: skip — impure observer, `&&`→`||` is end-state-insensitive
    let client_tx_offload_on = client_present && iface_tx_offload_on(&plan.client_iface);
    // mutants: skip — impure observer, `&&`→`||` is end-state-insensitive
    let backend_tx_offload_on = peer_present && iface_tx_offload_on(&plan.backend_iface);

    Ok(ObservedVeth {
        client_present,
        peer_present,
        client_addr_present,
        backend_addr_present,
        client_up,
        backend_up,
        client_tx_offload_on,
        backend_tx_offload_on,
    })
}

/// `ip link show <iface>` → `(present, up)`. Absent (either iproute2
/// phrasing) → `(false, false)`; any other non-zero exit (e.g. EPERM)
/// → [`VethProvisionError::LinkShowFailed`].
fn link_state(iface: &str) -> Result<(bool, bool), VethProvisionError> {
    let show = std::process::Command::new("ip").args(["link", "show", iface]).output()?;
    if show.status.success() {
        let stdout = String::from_utf8_lossy(&show.stdout);
        // `ip link show` prints the admin flags between angle brackets,
        // e.g. `<BROADCAST,MULTICAST,UP,LOWER_UP>`, and `state UP`.
        let up = stdout.contains(",UP,")
            || stdout.contains("<UP,")
            || stdout.contains(",UP>")
            || stdout.contains("state UP");
        return Ok((true, up));
    }
    let stderr = String::from_utf8_lossy(&show.stderr);
    if link_absent(&stderr) {
        return Ok((false, false));
    }
    Err(VethProvisionError::LinkShowFailed {
        iface: iface.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: show.status.code(),
    })
}

/// True when `iface` carries `want` as a bound IPv4 address. Reuses the
/// production `getifaddrs(3)` walk so observer and consumer agree.
fn iface_has_addr(iface: &str, want: Ipv4Addr) -> bool {
    crate::iface::resolve_iface_ipv4(iface).is_ok_and(|got| got == want)
}

/// True when `iface` still has TX-checksum-offload ENABLED, read from
/// `ethtool -k <iface>` (lowercase `-k` queries features; uppercase `-K`
/// sets them). Parsed via [`tx_checksumming_on`].
///
/// Conservative on failure: if `ethtool` cannot be spawned, exits
/// non-zero, or does not report a `tx-checksumming:` line at all (a
/// virtual iface that does not expose the feature, or a missing
/// `ethtool` binary), this returns `false` ("offload not on"). That is
/// the correct default: an iface with no offload feature already
/// delivers a FULL checksum, so no disable step is needed, and emitting
/// one would be a wasted (harmless but noisy) `ethtool -K … tx off`. The
/// converge `recreated` path still re-emits the disable after a fresh
/// create regardless, so a transient read failure cannot leave a newly
/// created pair with offload silently on.
// mutants: skip — impure I/O shim: spawns real `ethtool -k` and reports
// the kernel feature bit. Its body mutants (`-> true` / `-> false` /
// delete `!`) are end-state-INSENSITIVE because the downstream disable is
// idempotent (a wrong observation only adds or skips a redundant
// `ethtool -K … tx off`; the converged offload-off end-state is the same).
// Same untestable class as the sibling `iface_has_addr` shim. The pure,
// KILLABLE parse logic is factored into `tx_checksumming_on` (unit-tested).
fn iface_tx_offload_on(iface: &str) -> bool {
    let Ok(out) = std::process::Command::new("ethtool").args(["-k", iface]).output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    tx_checksumming_on(&String::from_utf8_lossy(&out.stdout))
}

/// Parse `ethtool -k <iface>` output for the `tx-checksumming:` feature
/// line and return `true` iff it reports `on`. Pure (no I/O) so the
/// parse is unit-testable in the default lane without `ethtool`.
///
/// `ethtool -k` prints one feature per line, e.g.
/// `tx-checksumming: on` / `tx-checksumming: off [fixed]`. We match the
/// `tx-checksumming:` prefix and check the value token is exactly `on`
/// (so `off`, `off [fixed]`, and an absent line all read as "not on").
fn tx_checksumming_on(ethtool_output: &str) -> bool {
    ethtool_output.lines().any(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("tx-checksumming:")
            .is_some_and(|rest| rest.split_whitespace().next() == Some("on"))
    })
}

/// Apply a single [`VethStep`] via `ip(8)`. Idempotent: `EEXIST` /
/// `File exists` on address and route add is swallowed; `ip link set up`
/// is idempotent at the kernel.
fn execute_step(plan: &VethProvisionPlan, step: VethStep) -> Result<(), VethProvisionError> {
    match step {
        VethStep::RecreatePair => {
            link_del(&plan.client_iface)?;
            // Also reap the backend end. For the forward corrupted edge
            // (client present, peer absent) deleting the client reaps both,
            // so this is a no-op. For the inverse edge (client absent, peer
            // present) the client del is the no-op and THIS reaps the
            // surviving/colliding peer — without it, `link_add` would hit
            // the identical "File exists" failure on the peer name.
            link_del(&plan.backend_iface)?;
            link_add(plan)
        }
        VethStep::CreatePair => link_add(plan),
        VethStep::AddClientAddr => {
            let cidr = format!("{}/{}", plan.client_gateway, plan.route_cidr.prefix_len());
            addr_add(&plan.client_iface, &cidr)
        }
        VethStep::AddBackendAddr => {
            // Only emitted when backend_gateway is Some — unreachable
            // otherwise per converge_steps.
            let gw = plan.backend_gateway.unwrap_or_else(|| {
                unreachable!("AddBackendAddr emitted only when backend_gateway is Some")
            });
            let cidr = format!("{}/{}", gw, plan.route_cidr.prefix_len());
            addr_add(&plan.backend_iface, &cidr)
        }
        VethStep::SetClientUp => link_up(&plan.client_iface),
        VethStep::SetBackendUp => link_up(&plan.backend_iface),
        VethStep::DisableClientTxOffload => tx_offload_off(&plan.client_iface),
        VethStep::DisableBackendTxOffload => tx_offload_off(&plan.backend_iface),
        VethStep::AddRoute => add_route(plan),
    }
}

/// `ip link add <client> type veth peer name <backend>` (atomic pair
/// creation).
fn link_add(plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    let add = std::process::Command::new("ip")
        .args([
            "link",
            "add",
            &plan.client_iface,
            "type",
            "veth",
            "peer",
            "name",
            &plan.backend_iface,
        ])
        .output()?;
    if add.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::LinkAddFailed {
        client_iface: plan.client_iface.clone(),
        backend_iface: plan.backend_iface.clone(),
        stderr: String::from_utf8_lossy(&add.stderr).trim().to_owned(),
        status: add.status.code(),
    })
}

/// `ip link del <iface>` — deletes one named end of the veth pair.
/// Used only by [`VethStep::RecreatePair`] (§ 3.2), which calls it for
/// BOTH the client and the backend end so whichever end survived a
/// corrupted edge is reaped before recreate. A "does not exist" failure
/// is benign (already gone — the common case for the end that does not
/// exist on a given corrupted edge) and swallowed; any other failure
/// surfaces as [`VethProvisionError::LinkDelFailed`].
fn link_del(iface: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip").args(["link", "del", iface]).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if link_absent(&stderr) {
        // Already gone — recreate proceeds.
        return Ok(());
    }
    Err(VethProvisionError::LinkDelFailed {
        iface: iface.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// On-link route `<vip_range> dev <client_iface>`. Idempotent —
/// assigning the gateway address also auto-creates a kernel connected
/// route for the same /N, so `ip route add` here can legitimately
/// collide with `File exists`; that is the "already reachable" case,
/// not a failure (ADR-0061 § 3.1).
fn add_route(plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    let route_cidr = plan.route_cidr.to_string();
    let route = std::process::Command::new("ip")
        .args(["route", "add", &route_cidr, "dev", &plan.client_iface])
        .output()?;
    if route.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&route.stderr);
    if stderr.contains("File exists") {
        return Ok(());
    }
    Err(VethProvisionError::RouteAddFailed {
        cidr: route_cidr,
        iface: plan.client_iface.clone(),
        stderr: stderr.trim().to_owned(),
        status: route.status.code(),
    })
}

/// `ip addr add <cidr> dev <iface>`. Idempotent — swallows `EEXIST` /
/// `File exists` (already-assigned is the converge success case, not a
/// failure, per ADR-0061 § 3.1).
fn addr_add(iface: &str, cidr: &str) -> Result<(), VethProvisionError> {
    let out =
        std::process::Command::new("ip").args(["addr", "add", cidr, "dev", iface]).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("File exists") {
        // Already assigned — the idempotent converge success case.
        return Ok(());
    }
    Err(VethProvisionError::AddrAddFailed {
        iface: iface.to_owned(),
        cidr: cidr.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

fn link_up(iface: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip").args(["link", "set", iface, "up"]).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::LinkUpFailed {
        iface: iface.to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ethtool -K <iface> tx off` — disable TX-checksum-offload so the
/// kernel materialises the FULL L4 checksum in software before a frame
/// leaves this veth end, giving the receive-side XDP NAT hook a valid
/// base for its incremental delta (commit 62fa6be2, RFC 1624). Mirrors
/// the Tier-3 fixture's `NetNs::ethtool_tx_off` shape, but with
/// production typed-error discipline rather than best-effort `let _`.
///
/// A "feature is fixed" / "not supported" non-zero exit is BENIGN — such
/// an iface already delivers a FULL checksum, so the disable is a no-op
/// and is swallowed (idempotent converge success, ADR-0061 § 3.1). Any
/// other failure — EPERM, or a missing `ethtool` binary on a
/// feature-bearing veth — is FATAL: booting with offload still ON would
/// corrupt every NAT'd packet, so it surfaces as
/// [`VethProvisionError::TxOffloadDisableFailed`] and refuses the boot.
fn tx_offload_off(iface: &str) -> Result<(), VethProvisionError> {
    let out = match std::process::Command::new("ethtool").args(["-K", iface, "tx", "off"]).output()
    {
        Ok(out) => out,
        Err(err) => {
            return Err(VethProvisionError::TxOffloadDisableFailed {
                iface: iface.to_owned(),
                stderr: format!("spawning `ethtool` failed: {err}"),
                status: None,
            });
        }
    };
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if tx_offload_benign(&stderr) {
        // The iface does not expose a settable tx-checksumming feature;
        // it already delivers a FULL checksum, so no disable is needed.
        return Ok(());
    }
    Err(VethProvisionError::TxOffloadDisableFailed {
        iface: iface.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// True when an `ethtool -K … tx off` non-zero exit is BENIGN — the
/// iface's tx-checksumming feature is fixed/unsupported, so it already
/// delivers a FULL checksum and the disable is an idempotent no-op.
///
/// `ethtool` phrasing varies: a fixed feature prints `Cannot change ...`
/// (often with `... it is fixed`), and a feature absent on the device
/// prints `... not supported` / `Operation not supported`. Both mean
/// "nothing to disable here". A genuine permission failure
/// (`Operation not permitted`) is NOT benign and must surface.
fn tx_offload_benign(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    (lower.contains("cannot change") || lower.contains("not supported"))
        && !lower.contains("not permitted")
}

/// True when `ip link show <iface>` stderr indicates the interface is
/// simply ABSENT (the normal first-boot create path), as opposed to a
/// genuine failure (e.g. permission denied, `RTNETLINK answers: ...`).
///
/// iproute2 stderr phrasing is not stable across versions: newer
/// emits `Device "<iface>" does not exist.`, while older iproute2
/// (common in Alpine/minimal container images) emits
/// `Cannot find device "<iface>"`. Both mean the same thing — absent —
/// so the create path must accept either. Matching only the newer
/// phrase made first-boot provisioning fail with
/// [`VethProvisionError::LinkShowFailed`] on hosts shipping the older
/// iproute2.
fn link_absent(stderr: &str) -> bool {
    stderr.contains("does not exist") || stderr.contains("Cannot find device")
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use super::{
        NET_SLOT_MAX, NetSlot, ObservedVeth, ObservedWorkloadVeth, VethProvisionPlan, VethStep,
        WORKLOAD_SUBNET_BASE, WorkloadNetnsPlan, WorkloadVethStep, converge_steps,
        derive_veth_plan, derive_workload_netns_plan, link_absent, tx_checksumming_on,
        tx_offload_benign, workload_converge_steps,
    };
    use ipnet::{IpAdd, Ipv4Net};
    use proptest::prelude::*;
    use std::net::Ipv4Addr;
    use std::str::FromStr;

    /// A complete (all-present, both-up, offload-OFF) observation — the
    /// baseline the converge tests mutate one field at a time. "Complete"
    /// means fully CONVERGED, so TX-offload is already OFF on both ends
    /// (the desired post-converge state); a complete pair therefore emits
    /// no DisableTxOffload step (only the idempotent AddRoute noop).
    fn complete_observed() -> ObservedVeth {
        ObservedVeth {
            client_present: true,
            peer_present: true,
            client_addr_present: true,
            backend_addr_present: true,
            client_up: true,
            backend_up: true,
            client_tx_offload_on: false,
            backend_tx_offload_on: false,
        }
    }

    fn plan_24() -> VethProvisionPlan {
        let range: Ipv4Net = "10.96.0.0/24".parse().expect("valid /24");
        derive_veth_plan("ovd-veth-cli", "ovd-veth-bk", range)
    }

    /// REGRESSION (the bug this fix closes): a half-provisioned pair —
    /// both ends present but the client address ABSENT (a serve boot
    /// crashed after `ip link add` but before address assignment) —
    /// must converge by COMPLETING the missing address, NOT be adopted
    /// untouched. The old `provision` returned `Ok(())` here, leaving the
    /// pair incomplete; `converge_steps` must instead emit `AddClientAddr`
    /// (the address step the old path skipped).
    #[test]
    fn converge_completes_half_provisioned_pair_missing_client_addr() {
        let plan = plan_24();
        let observed = ObservedVeth { client_addr_present: false, ..complete_observed() };

        let steps = converge_steps(&plan, &observed);

        assert!(
            steps.contains(&VethStep::AddClientAddr),
            "half-provisioned pair (client addr absent) must emit AddClientAddr, got {steps:?}"
        );
        // It must NOT recreate or create — the pair is present, only the
        // address is missing.
        assert!(
            !steps.contains(&VethStep::CreatePair),
            "must not recreate a present pair: {steps:?}"
        );
        assert!(!steps.contains(&VethStep::RecreatePair), "peer present → no recreate: {steps:?}");
    }

    /// § 3.2 corrupted edge: client iface present but its declared peer
    /// ABSENT → recreate the pair from scratch, then converge fully.
    #[test]
    fn converge_recreates_pair_when_peer_absent() {
        let plan = plan_24();
        let observed = ObservedVeth { peer_present: false, ..complete_observed() };

        let steps = converge_steps(&plan, &observed);

        assert_eq!(
            steps,
            vec![
                VethStep::RecreatePair,
                VethStep::AddClientAddr,
                VethStep::AddBackendAddr,
                VethStep::SetClientUp,
                VethStep::SetBackendUp,
                VethStep::DisableClientTxOffload,
                VethStep::DisableBackendTxOffload,
                VethStep::AddRoute,
            ],
            "peer-absent corrupted pair must recreate then converge every downstream resource"
        );
    }

    /// REGRESSION (inverse corrupted edge): client iface ABSENT but its
    /// declared peer PRESENT — e.g. an unrelated interface collides on the
    /// backend name, or a veth peer survived its partner. The old `(false, _)`
    /// wildcard routed this to CreatePair, whose `ip link add ... peer name
    /// ovd-veth-bk` then failed with "File exists" → boot refusal. Must
    /// instead RecreatePair (which dels both ends, clearing the conflict)
    /// then converge every downstream resource.
    #[test]
    fn converge_recreates_pair_when_client_absent_but_peer_present() {
        let plan = plan_24();
        let observed =
            ObservedVeth { client_present: false, peer_present: true, ..complete_observed() };
        let steps = converge_steps(&plan, &observed);
        assert_eq!(
            steps,
            vec![
                VethStep::RecreatePair,
                VethStep::AddClientAddr,
                VethStep::AddBackendAddr,
                VethStep::SetClientUp,
                VethStep::SetBackendUp,
                VethStep::DisableClientTxOffload,
                VethStep::DisableBackendTxOffload,
                VethStep::AddRoute,
            ],
            "inverse corrupted edge must recreate then converge every downstream resource, got {steps:?}"
        );
        assert!(
            !steps.contains(&VethStep::CreatePair),
            "must NOT CreatePair over a present peer: {steps:?}"
        );
    }

    /// Wholly-absent pair (first boot) → create then converge everything.
    #[test]
    fn converge_creates_pair_when_wholly_absent() {
        let plan = plan_24();
        let observed = ObservedVeth {
            client_present: false,
            peer_present: false,
            client_addr_present: false,
            backend_addr_present: false,
            client_up: false,
            backend_up: false,
            // Absent ifaces report offload off; the `recreated` path
            // re-emits the disable after the fresh create regardless.
            client_tx_offload_on: false,
            backend_tx_offload_on: false,
        };

        let steps = converge_steps(&plan, &observed);

        assert_eq!(
            steps,
            vec![
                VethStep::CreatePair,
                VethStep::AddClientAddr,
                VethStep::AddBackendAddr,
                VethStep::SetClientUp,
                VethStep::SetBackendUp,
                VethStep::DisableClientTxOffload,
                VethStep::DisableBackendTxOffload,
                VethStep::AddRoute,
            ],
            "absent pair must create then converge every downstream resource"
        );
    }

    /// A fully-complete pair converges to a single idempotent
    /// `AddRoute` noop — never re-creating, re-addressing, or re-upping
    /// (guards against the converge falsely re-doing work on a good pair).
    #[test]
    fn converge_complete_pair_is_route_only_noop() {
        let plan = plan_24();
        let steps = converge_steps(&plan, &complete_observed());
        assert_eq!(
            steps,
            vec![VethStep::AddRoute],
            "complete pair must converge to exactly [AddRoute] (idempotent noop), got {steps:?}"
        );
    }

    /// The production TX-offload-off invariant (commit 62fa6be2): a
    /// present, otherwise-complete pair whose ends STILL have TX-offload
    /// ON must emit `DisableClientTxOffload` AND `DisableBackendTxOffload`
    /// (and nothing else but the idempotent `AddRoute`). Without offload
    /// off, the incremental-L4-csum XDP NAT hook folds its delta into a
    /// `CHECKSUM_PARTIAL` base and corrupts every packet — so the disable
    /// is mandatory, not cosmetic.
    #[test]
    fn converge_disables_tx_offload_when_still_on_both_ends() {
        let plan = plan_24();
        let observed = ObservedVeth {
            client_tx_offload_on: true,
            backend_tx_offload_on: true,
            ..complete_observed()
        };

        let steps = converge_steps(&plan, &observed);

        assert_eq!(
            steps,
            vec![
                VethStep::DisableClientTxOffload,
                VethStep::DisableBackendTxOffload,
                VethStep::AddRoute,
            ],
            "offload-on present pair must disable BOTH ends then route, got {steps:?}"
        );
    }

    /// Idempotency (the converge-on-boot no-op guarantee, ADR-0061 § 3.1):
    /// a present, complete pair whose offload is ALREADY OFF on both ends
    /// must emit NEITHER disable step — a second `provision()` re-observes
    /// offload off and converges to the single `[AddRoute]` noop. This is
    /// the mirror of [`converge_disables_tx_offload_when_still_on_both_ends`]
    /// and the property the conditional-emit predicate exists to satisfy:
    /// emit the disable ONLY when offload is on.
    #[test]
    fn converge_omits_tx_offload_disable_when_already_off() {
        let plan = plan_24();
        // complete_observed() already has both *_tx_offload_on = false.
        let steps = converge_steps(&plan, &complete_observed());

        assert!(
            !steps.contains(&VethStep::DisableClientTxOffload),
            "offload-off client must NOT emit a disable (idempotent re-run): {steps:?}"
        );
        assert!(
            !steps.contains(&VethStep::DisableBackendTxOffload),
            "offload-off backend must NOT emit a disable (idempotent re-run): {steps:?}"
        );
    }

    /// One disable per end, independently: only the end whose offload is
    /// still ON gets a disable (guards a both-or-neither collapse — the
    /// per-iface conditional must key on the per-iface fact).
    #[test]
    fn converge_disables_tx_offload_per_end_independently() {
        let plan = plan_24();

        let client_only = ObservedVeth { client_tx_offload_on: true, ..complete_observed() };
        let steps = converge_steps(&plan, &client_only);
        assert!(
            steps.contains(&VethStep::DisableClientTxOffload),
            "client on → disable: {steps:?}"
        );
        assert!(
            !steps.contains(&VethStep::DisableBackendTxOffload),
            "backend off → no disable: {steps:?}"
        );

        let backend_only = ObservedVeth { backend_tx_offload_on: true, ..complete_observed() };
        let steps = converge_steps(&plan, &backend_only);
        assert!(
            !steps.contains(&VethStep::DisableClientTxOffload),
            "client off → no disable: {steps:?}"
        );
        assert!(
            steps.contains(&VethStep::DisableBackendTxOffload),
            "backend on → disable: {steps:?}"
        );
    }

    /// The pure `ethtool -k` parser: `tx-checksumming: on` reads as ON;
    /// `off`, `off [fixed]`, and an absent line read as NOT on. Input
    /// variations of one classification behaviour (Mandate 5) — one
    /// parametrised assertion over the table.
    #[test]
    fn tx_checksumming_parse_classifies_ethtool_k_output() {
        let on = "Features for ovd-veth-cli:\n\
                  rx-checksumming: on\n\
                  tx-checksumming: on\n\
                  scatter-gather: on\n";
        let off = "Features for ovd-veth-cli:\n\
                   tx-checksumming: off\n";
        let off_fixed = "tx-checksumming: off [fixed]\n";
        // A device whose feature is fixed-ON still reports `on` and must
        // read as on (converge will then try the disable, which the
        // executor swallows as benign on a fixed feature).
        let on_fixed = "tx-checksumming: on [fixed]\n";
        let absent = "rx-checksumming: on\nscatter-gather: on\n";

        let cases: &[(&str, bool)] =
            &[(on, true), (off, false), (off_fixed, false), (on_fixed, true), (absent, false)];
        for (output, expected) in cases {
            assert_eq!(
                tx_checksumming_on(output),
                *expected,
                "tx_checksumming_on({output:?}) should be {expected}",
            );
        }
    }

    /// The executor's benign-failure classifier: a fixed / unsupported
    /// `ethtool -K … tx off` stderr is benign (idempotent no-op), but a
    /// genuine permission failure is NOT (must surface as
    /// `TxOffloadDisableFailed`).
    #[test]
    fn tx_offload_benign_classifies_ethtool_set_stderr() {
        let cases: &[(&str, bool)] = &[
            ("Cannot change tx-checksumming", true),
            ("Cannot get device feature names: Operation not supported", true),
            ("rx-checksumming: Operation not supported", true),
            // EPERM is NOT benign — booting with offload on corrupts packets.
            ("Cannot change tx-checksumming: Operation not permitted", false),
            ("netlink error: Operation not permitted", false),
        ];
        for (stderr, expected) in cases {
            assert_eq!(
                tx_offload_benign(stderr),
                *expected,
                "tx_offload_benign({stderr:?}) should be {expected}",
            );
        }
    }

    /// Acceptance anchor (readable golden): the provisioner derives the
    /// on-link gateway + route from the first VIP range. `10.96.0.0/24`
    /// → gateway `10.96.0.1`, route `10.96.0.0/24 dev ovd-veth-cli`, and
    /// the plan carries the config interface NAMES (not hardcoded).
    #[test]
    fn derives_on_link_gateway_and_route_from_first_vip_range() {
        let range: Ipv4Net = "10.96.0.0/24".parse().expect("valid /24");
        let plan = derive_veth_plan("ovd-veth-cli", "ovd-veth-bk", range);

        assert_eq!(
            plan,
            VethProvisionPlan {
                client_iface: "ovd-veth-cli".to_owned(),
                backend_iface: "ovd-veth-bk".to_owned(),
                client_gateway: Ipv4Addr::new(10, 96, 0, 1),
                backend_gateway: Some(Ipv4Addr::new(10, 96, 0, 2)),
                route_cidr: range,
            }
        );
    }

    /// The plan carries the literal config iface names verbatim — a
    /// non-default pair must flow through unmodified (guards against a
    /// hardcoded `ovd-veth-*`).
    #[test]
    fn plan_carries_config_iface_names_verbatim() {
        let range: Ipv4Net = "10.96.0.0/24".parse().expect("valid /24");
        let plan = derive_veth_plan("client0", "backend0", range);
        assert_eq!(plan.client_iface, "client0");
        assert_eq!(plan.backend_iface, "backend0");
    }

    /// Regression: `link_absent` must classify BOTH iproute2 absence
    /// phrasings as "absent" (the normal create path) while still
    /// rejecting genuine errors so they surface as
    /// [`super::VethProvisionError::LinkShowFailed`]. iproute2 phrasing
    /// varies across versions — newer prints `... does not exist`, older
    /// (Alpine/minimal images) prints `Cannot find device "..."`. The
    /// single-phrase predecessor accepted only the former, which made
    /// first-boot provisioning fail on the older phrasing.
    ///
    /// Input variations of the same behaviour (Mandate 5) — one
    /// parametrised assertion over the classification table.
    #[test]
    fn link_absent_accepts_both_iproute2_phrasings_and_rejects_real_errors() {
        let cases: &[(&str, bool)] = &[
            // newer iproute2 — absent
            (r#"Device "ovd-veth-cli" does not exist."#, true),
            // older iproute2 (Alpine/minimal images) — absent; the case
            // the single-phrase predecessor regressed.
            (r#"Cannot find device "ovd-veth-cli""#, true),
            // a genuine unrelated failure — must NOT be treated as absent,
            // so it still surfaces as LinkShowFailed (no real-error swallow).
            ("RTNETLINK answers: Operation not permitted", false),
        ];
        for (stderr, expected_absent) in cases {
            assert_eq!(
                link_absent(stderr),
                *expected_absent,
                "link_absent({stderr:?}) should be {expected_absent}",
            );
        }
    }

    proptest! {
        /// Property: over the full present-pair partial-state space
        /// (each of the six converge-relevant facts independently
        /// present/absent), `converge_steps`
        ///   (a) never emits Create/Recreate for a present pair with a
        ///       present peer;
        ///   (b) emits `AddClientAddr` iff the client addr is absent;
        ///   (c) emits `AddBackendAddr` iff the backend addr is absent
        ///       (the plan_24 backend gateway is always Some);
        ///   (d) emits `SetClientUp` / `SetBackendUp` iff the respective
        ///       end is down;
        ///   (e) emits `DisableClientTxOffload` / `DisableBackendTxOffload`
        ///       iff the respective end still has TX-offload ON — the
        ///       emit-only-when-needed predicate that makes a re-run over
        ///       an offload-off pair a no-op (the converge-on-boot
        ///       idempotency guarantee, ADR-0061 § 3.1);
        ///   (f) always ends with `AddRoute`.
        /// This is the exhaustive desired-vs-actual invariant for the
        /// completion path — the regression class the old adopt-untouched
        /// branch violated for every absent sub-resource, now including
        /// the production TX-offload-off invariant (commit 62fa6be2).
        #[test]
        fn converge_present_pair_emits_exactly_the_missing_resources(
            client_addr in any::<bool>(),
            backend_addr in any::<bool>(),
            client_up in any::<bool>(),
            backend_up in any::<bool>(),
            client_tx_on in any::<bool>(),
            backend_tx_on in any::<bool>(),
        ) {
            let plan = plan_24();
            let observed = ObservedVeth {
                client_present: true,
                peer_present: true,
                client_addr_present: client_addr,
                backend_addr_present: backend_addr,
                client_up,
                backend_up,
                client_tx_offload_on: client_tx_on,
                backend_tx_offload_on: backend_tx_on,
            };
            let steps = converge_steps(&plan, &observed);

            prop_assert!(!steps.contains(&VethStep::CreatePair));
            prop_assert!(!steps.contains(&VethStep::RecreatePair));
            prop_assert_eq!(steps.contains(&VethStep::AddClientAddr), !client_addr);
            prop_assert_eq!(steps.contains(&VethStep::AddBackendAddr), !backend_addr);
            prop_assert_eq!(steps.contains(&VethStep::SetClientUp), !client_up);
            prop_assert_eq!(steps.contains(&VethStep::SetBackendUp), !backend_up);
            // The new conditional-emit predicate: present pair (recreated
            // == false) ⇒ disable emitted IFF offload still on.
            prop_assert_eq!(steps.contains(&VethStep::DisableClientTxOffload), client_tx_on);
            prop_assert_eq!(steps.contains(&VethStep::DisableBackendTxOffload), backend_tx_on);
            prop_assert_eq!(steps.last(), Some(&VethStep::AddRoute));
        }

        /// Property (Hebert ch.3 invariant + generalized-example): for
        /// any /24../30 VIP range,
        ///   (a) the derived client gateway is the first usable host of
        ///       the range — i.e. equals `network() + 1` AND is
        ///       contained in the range;
        ///   (b) the backend gateway, when present, equals `network() +
        ///       2` and is also in the range;
        ///   (c) the route CIDR is exactly the input range;
        ///   (d) the iface names flow through verbatim.
        #[test]
        fn gateway_is_first_usable_host_and_route_is_input_range(
            o1 in 0u8..=255,
            o2 in 0u8..=255,
            o3 in 0u8..=255,
            prefix in 24u8..=30,
            client in "[a-z][a-z0-9]{0,12}",
            backend in "[a-z][a-z0-9]{0,12}",
        ) {
            // Build a canonical network address for the chosen prefix by
            // truncating host bits, so the literal is a valid network()
            // for the Ipv4Net.
            let raw = u32::from(Ipv4Addr::new(o1, o2, o3, 0));
            let mask = u32::MAX << (32 - prefix);
            let network = Ipv4Addr::from(raw & mask);
            let range = Ipv4Net::new(network, prefix).expect("valid prefix 24..=30");

            let plan = derive_veth_plan(&client, &backend, range);

            let first_usable = range.hosts().next().expect("/24..=/30 has >=1 host");
            prop_assert_eq!(plan.client_gateway, first_usable);
            prop_assert_eq!(plan.client_gateway, range.network().saturating_add(1));
            prop_assert!(range.contains(&plan.client_gateway));

            if let Some(bk) = plan.backend_gateway {
                prop_assert_eq!(bk, range.network().saturating_add(2));
                prop_assert!(range.contains(&bk));
            }

            prop_assert_eq!(&plan.route_cidr, &range);
            prop_assert_eq!(&plan.client_iface, &client);
            prop_assert_eq!(&plan.backend_iface, &backend);
        }

        /// Determinism: the same inputs yield byte-identical plans across
        /// repeated invocations (pure function, no hidden state).
        #[test]
        fn derivation_is_deterministic(
            o1 in 0u8..=255, o2 in 0u8..=255, o3 in 0u8..=255,
            prefix in 24u8..=30,
        ) {
            let raw = u32::from(Ipv4Addr::new(o1, o2, o3, 0));
            let mask = u32::MAX << (32 - prefix);
            let network = Ipv4Addr::from(raw & mask);
            let range = Ipv4Net::new(network, prefix).expect("valid prefix");

            let a = derive_veth_plan("ovd-veth-cli", "ovd-veth-bk", range);
            let b = derive_veth_plan("ovd-veth-cli", "ovd-veth-bk", range);
            prop_assert_eq!(a, b);
        }
    }

    // -------------------------------------------------------------------------
    // Per-allocation netns+veth derivation + converge (step 02-01)
    // -------------------------------------------------------------------------

    fn responder() -> Ipv4Addr {
        // The node-local DNS responder address (D-TME-9 / Q5a); carried as a
        // plan INPUT, not derived state.
        Ipv4Addr::new(169, 254, 0, 53)
    }

    fn slot(n: u16) -> NetSlot {
        NetSlot::new(n).expect("valid slot")
    }

    fn workload_plan() -> WorkloadNetnsPlan {
        derive_workload_netns_plan(slot(0), responder())
    }

    /// A complete (all-present, in-netns end moved, both addressed, both ends
    /// up, netns loopback up, default route present, offload OFF, host
    /// prereqs satisfied) observation — the converged baseline the partial
    /// tests mutate one field at a time. "Complete" means fully CONVERGED, so
    /// TX-offload is already OFF on both ends and both rp_filter facts are
    /// relaxed.
    fn complete_workload_observed() -> ObservedWorkloadVeth {
        ObservedWorkloadVeth {
            netns_present: true,
            host_veth_present: true,
            workload_veth_present: true,
            workload_veth_in_netns: true,
            host_addr_present: true,
            workload_addr_present: true,
            host_veth_up: true,
            workload_veth_up: true,
            lo_up: true,
            default_route_present: true,
            host_tx_offload_on: false,
            workload_tx_offload_on: false,
            ip_forward_enabled: true,
            rp_filter_global_relaxed: true,
            host_veth_rp_filter_relaxed: true,
        }
    }

    /// The complete ordered convergence shape from a wholly-absent start.
    /// Ordering is load-bearing: netns and pair must exist before the
    /// in-netns end is moved; the move must precede in-netns addressing, the
    /// in-netns end up, the netns loopback up, and the default route; the
    /// host prereqs (ip_forward, rp_filter splits, tx off) round out the
    /// converged shape. `SetWorkloadVethUp` and `SetLoopbackUp` are ordered
    /// AFTER `MoveWorkloadEndIntoNetns` (B2 — a netns provisioned from the
    /// plan must be able to carry a packet, so both the in-netns veth end and
    /// the netns `lo` must come up).
    fn full_ordered_steps() -> Vec<WorkloadVethStep> {
        vec![
            WorkloadVethStep::CreateNetns,
            WorkloadVethStep::CreateVethPair,
            WorkloadVethStep::MoveWorkloadEndIntoNetns,
            WorkloadVethStep::AddHostAddr,
            WorkloadVethStep::AddWorkloadAddr,
            WorkloadVethStep::SetHostVethUp,
            WorkloadVethStep::SetWorkloadVethUp,
            WorkloadVethStep::SetLoopbackUp,
            WorkloadVethStep::AddDefaultRoute,
            WorkloadVethStep::EnableIpForward,
            WorkloadVethStep::RelaxGlobalRpFilter,
            WorkloadVethStep::RelaxHostVethRpFilter,
            WorkloadVethStep::DisableHostTxOffload,
            WorkloadVethStep::DisableWorkloadTxOffload,
        ]
    }

    /// Derivation golden anchor (D-TME-12): from `slot` + `responder`, the plan
    /// carries the SLOT-DERIVED netns name (`ovd-ns-<4hex-slot>` — 11 chars,
    /// bounded ≤ NAME_MAX and ≤ IFNAMSIZ BY CONSTRUCTION, identical shape to the
    /// veths; B3), the SLOT-DERIVED veth names (`ovd-wl-<4hex-slot>` in-netns
    /// end, `ovd-hv-<4hex-slot>` host-side end — 11 chars each, IFNAMSIZ-safe BY
    /// CONSTRUCTION), the slot-derived /30 subnet (carved from
    /// `WORKLOAD_SUBNET_BASE` at `base + slot*4`), the host-side address (first
    /// usable = net+1), the in-netns address (second usable = net+2), the
    /// in-netns default-route gateway (= host-side address), and the responder
    /// address verbatim (an input, not derived state). Neither the subnet nor
    /// the alloc id is a caller parameter — the derivation owns slot→names/
    /// subnet (S1, B3); the alloc↔slot binding lives in the 02-04 allocator map.
    ///
    /// Slot 0 → netns `ovd-ns-0000`, subnet `10.99.0.0/30`, host-side
    /// `10.99.0.1`, in-netns `10.99.0.2`, veth names `ovd-hv-0000` /
    /// `ovd-wl-0000`. The previous alloc-keyed netns `ovd-ns-payments-0` (which
    /// would overflow NAME_MAX at 260 chars for a 253-char alloc id) is REMOVED.
    #[test]
    fn derives_per_alloc_netns_veth_names_and_addresses() {
        let plan = derive_workload_netns_plan(slot(0), responder());

        // Netns name is SLOT-derived (4-hex), same shape as the veths (B3).
        assert_eq!(plan.netns, "ovd-ns-0000");
        // Veth names are SLOT-derived (4-hex), IFNAMSIZ-safe — 11 chars.
        assert_eq!(plan.host_veth, "ovd-hv-0000");
        assert_eq!(plan.workload_veth, "ovd-wl-0000");
        // The /30 is slot-derived from WORKLOAD_SUBNET_BASE.
        assert_eq!(plan.subnet, "10.99.0.0/30".parse::<Ipv4Net>().expect("valid /30"));
        assert_eq!(plan.host_addr, Ipv4Addr::new(10, 99, 0, 1));
        assert_eq!(plan.workload_addr, Ipv4Addr::new(10, 99, 0, 2));
        // The in-netns default route points at the host-side end.
        assert_eq!(plan.gateway, Ipv4Addr::new(10, 99, 0, 1));
        // Responder address flows through verbatim (carried for the later
        // resolv.conf-injection step, D-TME-9).
        assert_eq!(plan.responder_addr, responder());
    }

    /// A non-zero slot derives a distinct /30 four addresses up per slot and
    /// the matching hex names: slot 1 → `10.99.0.4/30`, host `10.99.0.5`,
    /// in-netns `10.99.0.6`, names `ovd-ns-0001` / `ovd-hv-0001` /
    /// `ovd-wl-0001`. Pins the `slot*4` subnet arithmetic and the `{:04x}` name
    /// formatting (including the slot-keyed netns; B3) against a concrete second
    /// point.
    #[test]
    fn derives_distinct_subnet_and_name_for_nonzero_slot() {
        let plan = derive_workload_netns_plan(slot(1), responder());

        assert_eq!(plan.subnet, "10.99.0.4/30".parse::<Ipv4Net>().expect("valid /30"));
        assert_eq!(plan.host_addr, Ipv4Addr::new(10, 99, 0, 5));
        assert_eq!(plan.workload_addr, Ipv4Addr::new(10, 99, 0, 6));
        assert_eq!(plan.gateway, Ipv4Addr::new(10, 99, 0, 5));
        assert_eq!(plan.netns, "ovd-ns-0001");
        assert_eq!(plan.host_veth, "ovd-hv-0001");
        assert_eq!(plan.workload_veth, "ovd-wl-0001");
    }

    /// Determinism: same inputs → byte-identical plan (pure function).
    #[test]
    fn workload_derivation_is_deterministic() {
        let a = derive_workload_netns_plan(slot(42), responder());
        let b = derive_workload_netns_plan(slot(42), responder());
        assert_eq!(a, b);
    }

    // -------------------------------------------------------------------------
    // NetSlot newtype — completeness + IFNAMSIZ ceiling (D-TME-12)
    // -------------------------------------------------------------------------

    /// `to_hex4` is a zero-padded 4-char lowercase hex of the slot value
    /// (the IFNAMSIZ-bounded name segment). Input variations of one
    /// formatting behaviour (Mandate 5) — one parametrised assertion.
    #[test]
    fn net_slot_to_hex4_is_zero_padded_lowercase() {
        let cases: &[(u16, &str)] =
            &[(0, "0000"), (1, "0001"), (255, "00ff"), (4095, "0fff"), (4094, "0ffe")];
        for (n, expected) in cases {
            assert_eq!(slot(*n).to_hex4(), *expected, "to_hex4({n}) should be {expected}");
        }
    }

    /// `NetSlot::new` rejects any value beyond `NET_SLOT_MAX` and accepts the
    /// whole `0..=NET_SLOT_MAX` range. The boundary (`NET_SLOT_MAX` ok,
    /// `NET_SLOT_MAX + 1` rejected) is the killable predicate.
    #[test]
    fn net_slot_new_validates_bound() {
        assert!(NetSlot::new(0).is_ok(), "0 is in range");
        assert!(NetSlot::new(NET_SLOT_MAX).is_ok(), "NET_SLOT_MAX is in range");
        assert!(NetSlot::new(NET_SLOT_MAX + 1).is_err(), "NET_SLOT_MAX + 1 is rejected");
        assert!(NetSlot::new(u16::MAX).is_err(), "u16::MAX is rejected");
    }

    proptest! {
        /// NetSlot completeness roundtrip (development.md § Newtype
        /// completeness): for every in-range slot,
        ///   (a) Display is the canonical DECIMAL form and FromStr round-trips
        ///       it back bit-for-bit;
        ///   (b) serde (to_string + from_str of the JSON) round-trips and
        ///       matches Display/FromStr;
        ///   (c) any value beyond NET_SLOT_MAX is rejected by both `new` and
        ///       `FromStr`.
        #[test]
        fn net_slot_roundtrips_and_rejects_out_of_range(n in 0u16..=NET_SLOT_MAX) {
            let s = NetSlot::new(n).expect("in-range");

            // (a) Display = decimal; FromStr round-trips.
            prop_assert_eq!(s.to_string(), n.to_string());
            prop_assert_eq!(NetSlot::from_str(&s.to_string()).expect("parse"), s);

            // (b) serde matches Display/FromStr.
            let json = serde_json::to_string(&s).expect("serialize");
            prop_assert_eq!(&json, &format!("\"{n}\""));
            let back: NetSlot = serde_json::from_str(&json).expect("deserialize");
            prop_assert_eq!(back, s);
        }

        /// Out-of-range rejection: every value strictly above NET_SLOT_MAX is
        /// rejected by `new` AND `FromStr` (the bound is enforced on both
        /// construction paths).
        #[test]
        fn net_slot_rejects_above_max(n in (u32::from(NET_SLOT_MAX) + 1)..=u32::from(u16::MAX)) {
            let n = u16::try_from(n).expect("range is bounded by u16::MAX");
            prop_assert!(NetSlot::new(n).is_err());
            prop_assert!(NetSlot::from_str(&n.to_string()).is_err());
        }

        /// IFNAMSIZ + slot-space containment over the FULL `0..=NET_SLOT_MAX`
        /// slot space (D-TME-12 / B1 / B3 / S6):
        ///   (a) every slot's netns, host_veth AND workload_veth name is
        ///       <= 15 chars (IFNAMSIZ — the tightest of the two ceilings; the
        ///       slot-keyed netns is bounded the same as the veths, B3); and
        ///   (b) the derived /30 subnet lies WITHIN WORKLOAD_SUBNET_BASE
        ///       (containment, NOT an arithmetic recompute — S6: assert the
        ///       /30's network AND broadcast both fall inside the base, so a
        ///       future NET_SLOT_MAX raise or #239 tunable base that carved a
        ///       /30 OUTSIDE the base fails this property), prefix 30, with the
        ///       host-side address = its-network+1 and the in-netns address =
        ///       its-network+2 (a /30 ALWAYS has two usable hosts, so no Option
        ///       / network() fallback — S2).
        #[test]
        fn every_slot_name_fits_ifnamsiz_and_tiles_the_base(n in 0u16..=NET_SLOT_MAX) {
            let plan = derive_workload_netns_plan(slot(n), responder());

            // (a) IFNAMSIZ — all three names fit by construction for EVERY slot
            // (the netns is slot-keyed and bounded the same as the veths, B3).
            prop_assert!(plan.netns.len() <= 15, "netns {} > 15", plan.netns);
            prop_assert!(plan.host_veth.len() <= 15, "host_veth {} > 15", plan.host_veth);
            prop_assert!(plan.workload_veth.len() <= 15, "workload_veth {} > 15", plan.workload_veth);

            // (b) The /30 is CONTAINED in the base — assert containment, NOT the
            // `base + slot*4` arithmetic the production code already uses (S6).
            // Both bounding addresses of the /30 (its network AND its broadcast
            // at network+3) must fall inside WORKLOAD_SUBNET_BASE's closed
            // address interval `[base_net, base_net + base_span - 1]`; a slot
            // whose /30 escaped the base would fail here even though the
            // recompute-and-equality form would still pass. (`ipnet::Contains`
            // is a `pub` trait in a private module, not re-exported from the
            // crate root, so containment is expressed as the `u32` range check
            // it denotes.)
            let base_net = u32::from(WORKLOAD_SUBNET_BASE.network());
            let base_span = 1u32 << (32 - u32::from(WORKLOAD_SUBNET_BASE.prefix_len()));
            let base_last = base_net + base_span - 1;
            let subnet_net = plan.subnet.network();
            let subnet_net_u32 = u32::from(subnet_net);
            let subnet_broadcast_u32 = subnet_net_u32 + 3;
            prop_assert!(
                (base_net..=base_last).contains(&subnet_net_u32),
                "/30 network {subnet_net} escaped base {WORKLOAD_SUBNET_BASE}"
            );
            prop_assert!(
                (base_net..=base_last).contains(&subnet_broadcast_u32),
                "/30 broadcast {} escaped base {WORKLOAD_SUBNET_BASE}",
                Ipv4Addr::from(subnet_broadcast_u32)
            );
            prop_assert_eq!(plan.subnet.prefix_len(), 30);
            // host = the /30's own network+1, workload = its network+2 — anchored
            // to the subnet's network (NOT a re-derived base+slot*4), so this
            // checks the addressing relationship, not the slot arithmetic.
            prop_assert_eq!(plan.host_addr, subnet_net.saturating_add(1));
            prop_assert_eq!(plan.workload_addr, subnet_net.saturating_add(2));
            prop_assert_eq!(plan.gateway, plan.host_addr);
            // A /30 always yields two usable hosts — derivation is total.
            prop_assert_ne!(plan.host_addr, plan.workload_addr);
        }

        /// Collision-freedom BY CONSTRUCTION (D-TME-12 / B1 / B3), NOT by hash:
        /// for any two DISTINCT slots, all THREE derived names (netns + both
        /// veths) AND the derived /30 subnets are distinct. This is the property
        /// the previous `ovd-hv-<alloc>` / `ovd-ns-<alloc>` schemes violated
        /// (truncating two long alloc ids onto one 15-char iface name, or
        /// overflowing NAME_MAX on the netns) — a bounded `NetSlot` keying every
        /// name makes the collision unrepresentable.
        #[test]
        fn distinct_slots_yield_distinct_names_and_subnets(
            a in 0u16..=NET_SLOT_MAX,
            b in 0u16..=NET_SLOT_MAX,
        ) {
            prop_assume!(a != b);
            let pa = derive_workload_netns_plan(slot(a), responder());
            let pb = derive_workload_netns_plan(slot(b), responder());

            prop_assert_ne!(&pa.netns, &pb.netns, "distinct slots → distinct netns");
            prop_assert_ne!(&pa.host_veth, &pb.host_veth, "distinct slots → distinct host_veth");
            prop_assert_ne!(
                &pa.workload_veth,
                &pb.workload_veth,
                "distinct slots → distinct workload_veth"
            );
            prop_assert_ne!(pa.subnet, pb.subnet, "distinct slots → distinct /30");
        }
    }

    /// Wholly-absent (first provision of a fresh alloc) → the full ordered
    /// step set, CreateNetns FIRST.
    #[test]
    fn workload_converge_creates_everything_when_wholly_absent() {
        let plan = workload_plan();
        let observed = ObservedWorkloadVeth {
            netns_present: false,
            host_veth_present: false,
            workload_veth_present: false,
            workload_veth_in_netns: false,
            host_addr_present: false,
            workload_addr_present: false,
            host_veth_up: false,
            workload_veth_up: false,
            lo_up: false,
            default_route_present: false,
            host_tx_offload_on: false,
            workload_tx_offload_on: false,
            ip_forward_enabled: false,
            rp_filter_global_relaxed: false,
            host_veth_rp_filter_relaxed: false,
        };

        let steps = workload_converge_steps(&plan, &observed);

        assert_eq!(
            steps,
            full_ordered_steps(),
            "wholly-absent alloc must create netns first, then converge every resource in order, got {steps:?}"
        );
        assert_eq!(
            steps.first(),
            Some(&WorkloadVethStep::CreateNetns),
            "CreateNetns must be the FIRST step"
        );
    }

    /// Complete (fully-converged) → all-noop (empty step set). This is the
    /// converge-on-boot idempotency guarantee: a second provision over a
    /// good alloc does nothing.
    #[test]
    fn workload_converge_complete_is_noop() {
        let plan = workload_plan();
        let steps = workload_converge_steps(&plan, &complete_workload_observed());
        assert!(
            steps.is_empty(),
            "fully-converged alloc must converge to an empty step set, got {steps:?}"
        );
    }

    /// Half-provisioned (netns + pair present, in-netns end moved, but the
    /// in-netns address missing — a boot crashed mid-converge) → completed
    /// in place: emits exactly AddWorkloadAddr, never re-creating the netns
    /// or pair.
    #[test]
    fn workload_converge_completes_half_provisioned_missing_workload_addr() {
        let plan = workload_plan();
        let observed =
            ObservedWorkloadVeth { workload_addr_present: false, ..complete_workload_observed() };

        let steps = workload_converge_steps(&plan, &observed);

        assert_eq!(
            steps,
            vec![WorkloadVethStep::AddWorkloadAddr],
            "half-provisioned (workload addr absent) must complete in place with exactly AddWorkloadAddr, got {steps:?}"
        );
        assert!(
            !steps.contains(&WorkloadVethStep::CreateNetns),
            "must not recreate a present netns: {steps:?}"
        );
        assert!(
            !steps.contains(&WorkloadVethStep::CreateVethPair),
            "must not recreate a present pair: {steps:?}"
        );
    }

    /// Corrupted (netns present, veth pair absent) → recreate the pair from
    /// scratch, then re-converge every veth-dependent downstream resource
    /// (move, addresses, up, route, tx off). The netns survives (it is
    /// usable); only the absent pair is rebuilt — never tear down a usable
    /// netns.
    #[test]
    fn workload_converge_recreates_veth_when_pair_absent_but_netns_present() {
        let plan = workload_plan();
        let observed = ObservedWorkloadVeth {
            netns_present: true,
            host_veth_present: false,
            workload_veth_present: false,
            workload_veth_in_netns: false,
            host_addr_present: false,
            workload_addr_present: false,
            host_veth_up: false,
            workload_veth_up: false,
            // The netns lo survives the veth pair (it is per-netns, not
            // per-pair); host-global ip_forward + the GLOBAL rp_filter
            // relaxation also survive. Only the per-host-veth rp_filter
            // relaxation is lost — a freshly (re)built veth defaults STRICT
            // (S3), so the rebuild must re-emit RelaxHostVethRpFilter.
            lo_up: true,
            default_route_present: false,
            host_tx_offload_on: false,
            workload_tx_offload_on: false,
            ip_forward_enabled: true,
            rp_filter_global_relaxed: true,
            host_veth_rp_filter_relaxed: true,
        };

        let steps = workload_converge_steps(&plan, &observed);

        // Must NOT recreate the usable netns.
        assert!(
            !steps.contains(&WorkloadVethStep::CreateNetns),
            "must NOT recreate a present, usable netns: {steps:?}"
        );
        // Must rebuild the pair and re-converge every veth-dependent step.
        // SetLoopbackUp is omitted (lo survives the netns); but the freshly
        // rebuilt host-side veth defaults strict rp_filter, so
        // RelaxHostVethRpFilter is re-emitted (S3), while the GLOBAL relax and
        // ip_forward are NOT (they survived).
        assert_eq!(
            steps,
            vec![
                WorkloadVethStep::CreateVethPair,
                WorkloadVethStep::MoveWorkloadEndIntoNetns,
                WorkloadVethStep::AddHostAddr,
                WorkloadVethStep::AddWorkloadAddr,
                WorkloadVethStep::SetHostVethUp,
                WorkloadVethStep::SetWorkloadVethUp,
                WorkloadVethStep::AddDefaultRoute,
                WorkloadVethStep::RelaxHostVethRpFilter,
                WorkloadVethStep::DisableHostTxOffload,
                WorkloadVethStep::DisableWorkloadTxOffload,
            ],
            "corrupted (netns present, pair absent) must rebuild the pair then re-converge every veth-dependent resource (incl. the per-host-veth rp_filter relax on the fresh veth), got {steps:?}"
        );
    }

    /// Single-end veth corruption keys `pair_rebuilt` on EACH end's presence
    /// independently — a present netns with EITHER the host end OR the
    /// in-netns end (but not both) missing must rebuild the pair. This pins
    /// the three-way disjunction `(!netns || !workload_veth || !host_veth)`:
    /// with exactly ONE operand differing, the `||`→`&&` mutant would compute
    /// the wrong `pair_rebuilt` and SUPPRESS `CreateVethPair`. A test that
    /// sets BOTH ends absent cannot distinguish `||` from `&&` (both yield
    /// rebuild), so each single-absent edge is asserted on its own.
    #[test]
    fn workload_converge_rebuilds_pair_when_either_single_end_absent() {
        let plan = workload_plan();

        // netns present, host end present, WORKLOAD end absent → rebuild.
        let workload_end_gone = ObservedWorkloadVeth {
            netns_present: true,
            host_veth_present: true,
            workload_veth_present: false,
            ..complete_workload_observed()
        };
        assert!(
            workload_converge_steps(&plan, &workload_end_gone)
                .contains(&WorkloadVethStep::CreateVethPair),
            "workload end absent (netns + host end present) must rebuild the pair"
        );

        // netns present, workload end present, HOST end absent → rebuild.
        let host_end_gone = ObservedWorkloadVeth {
            netns_present: true,
            host_veth_present: false,
            workload_veth_present: true,
            ..complete_workload_observed()
        };
        assert!(
            workload_converge_steps(&plan, &host_end_gone)
                .contains(&WorkloadVethStep::CreateVethPair),
            "host end absent (netns + workload end present) must rebuild the pair"
        );

        // netns ABSENT, both ends present → rebuild (the netns-absent operand
        // alone forces the rebuild even with both ends present). Pins the
        // first `||` operand against `||`→`&&` (which, with both ends present,
        // would compute `false` and SUPPRESS the rebuild a fresh netns needs).
        let netns_gone = ObservedWorkloadVeth {
            netns_present: false,
            host_veth_present: true,
            workload_veth_present: true,
            ..complete_workload_observed()
        };
        let steps = workload_converge_steps(&plan, &netns_gone);
        assert!(
            steps.contains(&WorkloadVethStep::CreateNetns),
            "absent netns must CreateNetns: {steps:?}"
        );
        assert!(
            steps.contains(&WorkloadVethStep::CreateVethPair),
            "absent netns forces a pair rebuild even with both ends present (stale ends in a \
             vanished netns are unusable): {steps:?}"
        );
    }

    /// The spike-proven host prereqs are emitted only when not already
    /// satisfied: ip_forward off → EnableIpForward; GLOBAL rp_filter not
    /// relaxed → RelaxGlobalRpFilter; per-host-veth rp_filter not relaxed →
    /// RelaxHostVethRpFilter; per-end tx offload on → DisableHostTxOffload /
    /// DisableWorkloadTxOffload. Each keyed on its OWN observed fact (guards a
    /// collapse where one prereq's presence suppresses another's step). The
    /// rp_filter split (S3) is the key new property: the two relaxations are
    /// independent observed facts, not one lossy bool.
    #[test]
    fn workload_converge_emits_host_prereqs_only_when_unsatisfied() {
        let plan = workload_plan();

        // ip_forward off on an otherwise-complete alloc → exactly EnableIpForward.
        let no_forward =
            ObservedWorkloadVeth { ip_forward_enabled: false, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &no_forward),
            vec![WorkloadVethStep::EnableIpForward],
            "ip_forward off → exactly EnableIpForward"
        );

        // GLOBAL rp_filter not relaxed → exactly RelaxGlobalRpFilter (the
        // host-veth relax is unaffected — independent fact).
        let no_global_rp = ObservedWorkloadVeth {
            rp_filter_global_relaxed: false,
            ..complete_workload_observed()
        };
        assert_eq!(
            workload_converge_steps(&plan, &no_global_rp),
            vec![WorkloadVethStep::RelaxGlobalRpFilter],
            "global rp_filter not relaxed → exactly RelaxGlobalRpFilter"
        );

        // Per-host-veth rp_filter not relaxed (on a present, non-rebuilt pair)
        // → exactly RelaxHostVethRpFilter (the global relax is unaffected).
        let no_host_veth_rp = ObservedWorkloadVeth {
            host_veth_rp_filter_relaxed: false,
            ..complete_workload_observed()
        };
        assert_eq!(
            workload_converge_steps(&plan, &no_host_veth_rp),
            vec![WorkloadVethStep::RelaxHostVethRpFilter],
            "host-veth rp_filter not relaxed → exactly RelaxHostVethRpFilter"
        );

        // tx offload still on (host end only) → exactly DisableHostTxOffload.
        let host_tx =
            ObservedWorkloadVeth { host_tx_offload_on: true, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &host_tx),
            vec![WorkloadVethStep::DisableHostTxOffload],
            "host tx on → exactly DisableHostTxOffload"
        );

        // tx offload still on (workload end only) → exactly DisableWorkloadTxOffload.
        let wl_tx =
            ObservedWorkloadVeth { workload_tx_offload_on: true, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &wl_tx),
            vec![WorkloadVethStep::DisableWorkloadTxOffload],
            "workload tx on → exactly DisableWorkloadTxOffload"
        );
    }

    /// B2 up-state regression: the in-netns veth end (`SetWorkloadVethUp`) and
    /// the netns loopback (`SetLoopbackUp`) are each emitted ONLY when down,
    /// and ONLY when their fact is unsatisfied on an otherwise-complete alloc.
    /// Without these, a netns provisioned from the plan cannot carry a packet
    /// (the in-netns end and `lo` stay DOWN). Each keyed on its own fact.
    #[test]
    fn workload_converge_brings_in_netns_end_and_loopback_up_when_down() {
        let plan = workload_plan();

        // In-netns veth end down (on a present, non-rebuilt pair) → exactly
        // SetWorkloadVethUp.
        let wl_down =
            ObservedWorkloadVeth { workload_veth_up: false, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &wl_down),
            vec![WorkloadVethStep::SetWorkloadVethUp],
            "in-netns end down → exactly SetWorkloadVethUp"
        );

        // Netns loopback down → exactly SetLoopbackUp.
        let lo_down = ObservedWorkloadVeth { lo_up: false, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &lo_down),
            vec![WorkloadVethStep::SetLoopbackUp],
            "netns lo down → exactly SetLoopbackUp"
        );
    }

    proptest! {
        /// The named scenario. Property: over the full present-netns,
        /// present-pair partial-state space (each converge-relevant fact
        /// independently satisfied/unsatisfied), `workload_converge_steps`:
        ///   (a) observed==desired (complete) ⇒ EMPTY step set;
        ///   (b) never re-creates the netns or pair (both present);
        ///   (c) emits each completion / prereq step IFF its observed fact
        ///       is unsatisfied — `MoveWorkloadEndIntoNetns` iff not moved,
        ///       `AddHostAddr` / `AddWorkloadAddr` iff that addr absent,
        ///       `SetHostVethUp` iff host end down, `SetWorkloadVethUp` iff
        ///       in-netns end down (B2), `SetLoopbackUp` iff netns lo down
        ///       (B2), `AddDefaultRoute` iff absent, `EnableIpForward` iff
        ///       disabled, `RelaxGlobalRpFilter` iff the global relax is
        ///       missing, `RelaxHostVethRpFilter` iff the per-host-veth relax
        ///       is missing (S3 — two independent rp_filter facts),
        ///       `DisableHostTxOffload` / `DisableWorkloadTxOffload` iff that
        ///       end's offload still on;
        ///   (d) re-applying the produced steps (i.e. converging from the
        ///       resulting satisfied state) is a no-op (idempotence).
        /// This is the exhaustive desired-vs-actual + idempotency invariant
        /// for the per-alloc completion path (ADR-0061 § 3.1 Bar-1), extended
        /// for the B2 up-state facts and the S3 rp_filter split.
        #[test]
        fn workload_netns_converge_steps_are_minimal_and_idempotent(
            moved in any::<bool>(),
            host_addr in any::<bool>(),
            workload_addr in any::<bool>(),
            host_up in any::<bool>(),
            workload_up in any::<bool>(),
            lo_up in any::<bool>(),
            route in any::<bool>(),
            ip_forward in any::<bool>(),
            global_rp in any::<bool>(),
            host_veth_rp in any::<bool>(),
            host_tx_on in any::<bool>(),
            workload_tx_on in any::<bool>(),
        ) {
            let plan = workload_plan();
            let observed = ObservedWorkloadVeth {
                netns_present: true,
                host_veth_present: true,
                workload_veth_present: true,
                workload_veth_in_netns: moved,
                host_addr_present: host_addr,
                workload_addr_present: workload_addr,
                host_veth_up: host_up,
                workload_veth_up: workload_up,
                lo_up,
                default_route_present: route,
                host_tx_offload_on: host_tx_on,
                workload_tx_offload_on: workload_tx_on,
                ip_forward_enabled: ip_forward,
                rp_filter_global_relaxed: global_rp,
                host_veth_rp_filter_relaxed: host_veth_rp,
            };

            let steps = workload_converge_steps(&plan, &observed);

            // (b) present netns + pair ⇒ no (re)create.
            prop_assert!(!steps.contains(&WorkloadVethStep::CreateNetns));
            prop_assert!(!steps.contains(&WorkloadVethStep::CreateVethPair));

            // (c) each step emitted IFF its fact is unsatisfied.
            prop_assert_eq!(steps.contains(&WorkloadVethStep::MoveWorkloadEndIntoNetns), !moved);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::AddHostAddr), !host_addr);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::AddWorkloadAddr), !workload_addr);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::SetHostVethUp), !host_up);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::SetWorkloadVethUp), !workload_up);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::SetLoopbackUp), !lo_up);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::AddDefaultRoute), !route);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::EnableIpForward), !ip_forward);
            prop_assert_eq!(steps.contains(&WorkloadVethStep::RelaxGlobalRpFilter), !global_rp);
            prop_assert_eq!(
                steps.contains(&WorkloadVethStep::RelaxHostVethRpFilter),
                !host_veth_rp
            );
            prop_assert_eq!(steps.contains(&WorkloadVethStep::DisableHostTxOffload), host_tx_on);
            prop_assert_eq!(
                steps.contains(&WorkloadVethStep::DisableWorkloadTxOffload),
                workload_tx_on
            );

            // (a) complete ⇒ empty.
            let all_satisfied = moved && host_addr && workload_addr && host_up && workload_up
                && lo_up && route && ip_forward && global_rp && host_veth_rp
                && !host_tx_on && !workload_tx_on;
            if all_satisfied {
                prop_assert!(
                    steps.is_empty(),
                    "all facts satisfied must converge to an empty step set, got {:?}",
                    steps
                );
            }

            // (d) idempotence: applying the produced steps yields a satisfied
            // state from which converge is a no-op. Model step application as
            // flipping the corresponding observed fact to its satisfied value.
            let mut after = observed;
            for step in &steps {
                match step {
                    WorkloadVethStep::MoveWorkloadEndIntoNetns => after.workload_veth_in_netns = true,
                    WorkloadVethStep::AddHostAddr => after.host_addr_present = true,
                    WorkloadVethStep::AddWorkloadAddr => after.workload_addr_present = true,
                    WorkloadVethStep::SetHostVethUp => after.host_veth_up = true,
                    WorkloadVethStep::SetWorkloadVethUp => after.workload_veth_up = true,
                    WorkloadVethStep::SetLoopbackUp => after.lo_up = true,
                    WorkloadVethStep::AddDefaultRoute => after.default_route_present = true,
                    WorkloadVethStep::EnableIpForward => after.ip_forward_enabled = true,
                    WorkloadVethStep::RelaxGlobalRpFilter => after.rp_filter_global_relaxed = true,
                    WorkloadVethStep::RelaxHostVethRpFilter => {
                        after.host_veth_rp_filter_relaxed = true;
                    }
                    WorkloadVethStep::DisableHostTxOffload => after.host_tx_offload_on = false,
                    WorkloadVethStep::DisableWorkloadTxOffload => after.workload_tx_offload_on = false,
                    WorkloadVethStep::CreateNetns
                    | WorkloadVethStep::CreateVethPair => {
                        prop_assert!(false, "unexpected (re)create over a present netns+pair: {:?}", steps);
                    }
                }
            }
            let reapplied = workload_converge_steps(&plan, &after);
            prop_assert!(
                reapplied.is_empty(),
                "re-applying the converge step set must be a no-op, got {:?}",
                reapplied
            );
        }
    }
}
