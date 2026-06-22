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

    // --- Per-allocation netns + veth sites (step 02-02) -------------------
    // Distinct variant per `ip netns` / `ip -n <ns>` / `sysctl`
    // shell-out site that the host-netns variants above do not cover, per
    // `.claude/rules/development.md` § Errors (one variant per failing
    // boundary; never an `Internal(String)` catch-all). The shared sites
    // (`ip link add`, `ip addr add`, `ip link set up`, `ip route add`,
    // `ethtool -K`) REUSE the variants above — the executor maps the netns
    // steps onto them so the operator gets a cause-specific message.
    /// `ip netns add <netns>` failed (and not because the netns already
    /// exists — that is swallowed as the idempotent converge success).
    #[error("`ip netns add {netns}` failed (status={status:?}): {stderr}")]
    NetnsAddFailed { netns: String, stderr: String, status: Option<i32> },
    /// `ip netns list` / `ip -n <netns> link show` (the observer's read of
    /// actual netns/veth state) failed for a reason that is neither
    /// "present" nor "absent" (e.g. permission denied).
    #[error("`ip {operation}` failed (status={status:?}): {stderr}")]
    NetnsObserveFailed { operation: String, stderr: String, status: Option<i32> },
    /// `ip link set <workload_veth> netns <netns>` (moving the in-netns end
    /// into the workload netns) failed.
    #[error("`ip link set {iface} netns {netns}` failed (status={status:?}): {stderr}")]
    NetnsMoveFailed { iface: String, netns: String, stderr: String, status: Option<i32> },
    /// `ip netns del <netns>` (teardown) failed for a non-benign reason. An
    /// "absent" failure is benign (already gone) and is swallowed before
    /// this surfaces.
    #[error("`ip netns del {netns}` failed (status={status:?}): {stderr}")]
    NetnsDelFailed { netns: String, stderr: String, status: Option<i32> },
    /// `sysctl -w <key>=<value>` (an `ip_forward` / `rp_filter` host
    /// prerequisite) failed. The knob is load-bearing for egress routing
    /// (`ip_forward`) and asymmetric-ingress survival (`rp_filter`), so a
    /// failure refuses the boot rather than silently shipping a path that
    /// drops the workload's packets.
    #[error("`sysctl -w {key}={value}` failed (status={status:?}): {stderr}")]
    SysctlSetFailed { key: String, value: String, stderr: String, status: Option<i32> },
    /// Writing the per-netns `/etc/netns/<netns>/resolv.conf` (the stock
    /// `ip netns` per-netns convention — bind-mounted over `/etc/resolv.conf`
    /// inside the namespace; the D-TME-9 / Q5a node-local DNS responder
    /// injection) failed — creating the `/etc/netns/<netns>/` directory or
    /// writing the file. Per ADR-0071 § Enforcement the injection is part of
    /// the SAME converge-on-boot pass, so a netns whose resolv.conf cannot be
    /// written refuses the boot rather than silently shipping a workload that
    /// resolves names against the wrong (host) nameserver.
    #[error("writing per-netns resolv.conf `{path}` failed: {source}")]
    ResolvConfWriteFailed { path: String, source: std::io::Error },
    /// Removing the per-netns resolv.conf dir `/etc/netns/<netns>/` during
    /// teardown failed for a reason other than the benign `NotFound` (e.g.
    /// permission denied). The dir is host-side and NOT reaped by `ip netns
    /// del`, so teardown removes it explicitly; a non-benign removal failure is
    /// fatal rather than silently leaving stale DNS config under a slot a later
    /// provision may reuse. Distinct from [`Self::ResolvConfWriteFailed`] so the
    /// Display verb matches the operation (removal, not write).
    #[error("removing per-netns resolv.conf dir `{path}` failed: {source}")]
    ResolvConfRemoveFailed { path: String, source: std::io::Error },
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

/// The node-local DNS-responder / mTLS-interception address for `slot` — the
/// per-netns **gateway** (the host-side veth-end address), i.e. the SAME value
/// [`derive_workload_netns_plan`] computes as `plan.host_addr` / `plan.gateway`
/// (`WORKLOAD_SUBNET_BASE.network() + slot*4 + 1`, the first usable host of the
/// slot's /30). D-TME-12 G1: the responder IS the per-netns gateway — reachable
/// by construction (it is the in-netns default route), collision-free (the
/// slot's own /30 host address), and the Overdrive analogue of Fly's fixed
/// `fdaa::3`.
///
/// Pure — performs no I/O, deterministic, total over `0..=NET_SLOT_MAX`. Exposed
/// so the action-shim C3 site can pass a concrete `responder_addr` into
/// `derive_workload_netns_plan` in ONE line without re-deriving the gateway
/// arithmetic; `debug_assert_eq!(plan.responder_addr, plan.host_addr)` holds at
/// the call site.
#[must_use]
pub fn responder_addr_for_slot(slot: NetSlot) -> Ipv4Addr {
    // Mirror the plan's own gateway math: net = base + slot*4; gateway = net+1.
    WORKLOAD_SUBNET_BASE.network().saturating_add(u32::from(slot.0) * 4).saturating_add(1)
}

/// The desired body of a per-workload netns's `/etc/resolv.conf`: a single
/// `nameserver <responder_addr>` line with a trailing newline (D-TME-9 / Q5a,
/// the Fly.io `fdaa::3` injection model). This is the stock single-`nameserver`
/// shape the `ip netns` per-netns convention bind-mounts over
/// `/etc/resolv.conf` inside the namespace.
///
/// Pure — performs no I/O, deterministic (same `responder_addr` → same body).
/// The responder address is a plan INPUT (`WorkloadNetnsPlan::responder_addr`),
/// never derived state; this function only formats it.
#[must_use]
pub fn resolv_conf_contents(responder_addr: Ipv4Addr) -> String {
    format!("nameserver {responder_addr}\n")
}

// --- Per-host NetSlot allocator (step 02-04, D-TME-12 "Slot-allocator home") ---
//
// The STATEFUL companion to the PURE [`derive_workload_netns_plan`] derivation:
// that function takes a [`NetSlot`] as an input; this allocator is what hands
// the slot out. It is a single-node, per-host free-list — assign the
// smallest-free slot on alloc start, release it on alloc terminal — NOT
// distributed IPAM and NOT the #167 VIP allocator.
//
// The pure assign/release DECISION ([`smallest_free_slot`]) is separated from
// the stateful held-map WRAPPER ([`NetSlotAllocator`]) precisely so the
// assign-smallest-free / release / double-release / exhaustion behaviour is
// default-lane unit + mutation testable without touching the real kernel
// (criterion 1 / 5). The held slot↔alloc map is the allocator's state.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use overdrive_core::AllocationId;
use parking_lot::Mutex;

/// The error returned when every [`NetSlot`] in `0..=NET_SLOT_MAX` is already
/// held, so a NEW allocation cannot be assigned a collision-free slot.
///
/// Exhaustion REFUSES the alloc (criterion 4) — it is NEVER a panic and NEVER a
/// silent reuse of a held slot. A reused slot would collide two live allocs
/// onto one veth/subnet, the exact B1 collision the slot model exists to
/// prevent. The caller (the C3 `on_alloc_running` hook) maps this into the
/// fail-the-alloc path so the workload is refused rather than dropped onto a
/// shared veth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "no free network slot: all {capacity} slots (0..={max}) are held",
    max = capacity - 1
)]
pub struct NetSlotExhausted {
    /// The total slot-space capacity (`NET_SLOT_MAX + 1`) — every one of which
    /// is held when this error is returned.
    pub capacity: u32,
}

/// PURE decision: the smallest [`NetSlot`] in `0..=NET_SLOT_MAX` that is NOT in
/// `held`.
///
/// Total over the bounded slot space, deterministic (same `held` → same slot),
/// performs no I/O. This is the assign-smallest-free contract (criterion 1):
/// the lowest GAP, not the next-monotonic value — so a slot freed by a release
/// is re-used by the next assign.
///
/// # Errors
///
/// Returns [`NetSlotExhausted`] when every slot `0..=NET_SLOT_MAX` is in `held`
/// — never a (reused) slot (criterion 4).
fn smallest_free_slot(held: &BTreeSet<NetSlot>) -> Result<NetSlot, NetSlotExhausted> {
    // Scan ascending for the first slot not in the held set. `held` is ordered,
    // but a linear scan over the bounded `0..=NET_SLOT_MAX` space is trivially
    // fast and obviously correct (single-node Phase 1).
    for candidate in 0..=NET_SLOT_MAX {
        let slot = NetSlot::new(candidate)
            .unwrap_or_else(|_| unreachable!("0..=NET_SLOT_MAX is in range by construction"));
        if !held.contains(&slot) {
            return Ok(slot);
        }
    }
    Err(NetSlotExhausted { capacity: u32::from(NET_SLOT_MAX) + 1 })
}

/// Per-host stateful [`NetSlot`] free-list (D-TME-12 "Slot-allocator home").
///
/// Hands out the host-unique, collision-free-BY-CONSTRUCTION [`NetSlot`] each
/// live allocation's netns/veth/subnet is keyed from (B3). The held
/// `AllocationId → NetSlot` map is the allocator's state — the alloc id's HOME
/// (the netns/veth/subnet are slot-keyed, NOT alloc-keyed; the alloc id lives
/// ONLY here, mirroring Cilium's `cilium endpoint list`).
///
/// Held-state shape mirrors [`crate::identity_mgr::IdentityMgr`]'s
/// `Arc<RwLock<BTreeMap<AllocationId, ...>>>` in-RAM held snapshot: ephemeral
/// runtime state, NEVER persisted, rebuilt on restart by re-assigning for every
/// still-Running alloc (single-node Phase 1; no cross-restart slot persistence
/// — criterion 6). `BTreeMap` (not `HashMap`) for deterministic iteration order
/// (§ "Ordered-collection choice"); `parking_lot::Mutex` (not `tokio::sync`)
/// because the only critical section is a point smallest-free-scan + insert /
/// remove that never crosses an `.await`.
///
/// # Atomicity (criterion 2)
///
/// [`assign`](Self::assign) takes the lock ONCE and performs the smallest-free
/// scan AND the insert in that single critical section — there is no
/// contains-then-insert TOCTOU window (`development.md` § "Check-and-act must be
/// atomic"). A `ClaimSet` does not fit: it claims a KEY with no value, whereas
/// the allocator must scan for the smallest-free slot AND bind it to the alloc
/// id, so the held map IS a `BTreeMap<AllocationId, NetSlot>` whose scan+insert
/// is the one locked op.
#[derive(Clone, Debug, Default)]
pub struct NetSlotAllocator {
    /// `AllocationId → NetSlot` binding for every currently-held allocation.
    /// `Arc<Mutex<…>>` so a clone shares the same held map (the allocator is
    /// composed once at boot and shared across the action-shim dispatch path).
    held: Arc<Mutex<BTreeMap<AllocationId, NetSlot>>>,
}

impl NetSlotAllocator {
    /// Construct an empty allocator. On a fresh process boot nothing is held —
    /// every still-Running alloc is re-assigned on its next `on_alloc_running`
    /// (the restart rebuild, criterion 6).
    #[must_use]
    pub fn new() -> Self {
        Self { held: Arc::new(Mutex::new(BTreeMap::new())) }
    }

    /// Assign the smallest-free [`NetSlot`] to `alloc`, recording the
    /// `alloc → slot` binding, and return it.
    ///
    /// **Idempotent re-entry (criterion 2):** if `alloc` is ALREADY held its
    /// EXISTING slot is returned unchanged and no new slot is consumed — a
    /// re-fire of `on_alloc_running` for the same alloc must not allocate a
    /// second slot. The held check, the smallest-free scan, and the insert are
    /// ONE locked critical section — no contains-then-insert TOCTOU.
    ///
    /// # Errors
    ///
    /// Returns [`NetSlotExhausted`] when `alloc` is NOT already held and every
    /// slot `0..=NET_SLOT_MAX` is taken — refusing the alloc rather than reusing
    /// a held slot (criterion 4). An already-held alloc re-assigns successfully
    /// even at full capacity (re-entry is never starved by exhaustion).
    ///
    /// # Atomicity
    ///
    /// One `self.held.lock()`; the guard is dropped within the call (never
    /// across an `.await`; § "Concurrency & async").
    pub fn assign(&self, alloc: AllocationId) -> Result<NetSlot, NetSlotExhausted> {
        // ONE locked critical section: the held check, the smallest-free scan,
        // and the insert all happen under a single guard — no contains-then-
        // insert TOCTOU window. The guard is scoped to this block so it drops
        // before the function returns (clippy::significant_drop_tightening),
        // while still spanning the whole check-and-act.
        let mut held = self.held.lock();
        // Idempotent re-entry: an already-held alloc returns its existing slot,
        // consuming no new slot. There is no window for a racer between "is
        // alloc held?" and "claim a slot for alloc" — both are under `held`.
        if let Some(existing) = held.get(&alloc) {
            return Ok(*existing);
        }
        // Smallest-free scan over the values currently bound — then bind it to
        // `alloc` in the SAME critical section.
        let taken: BTreeSet<NetSlot> = held.values().copied().collect();
        let slot = smallest_free_slot(&taken)?;
        held.insert(alloc, slot);
        drop(held);
        Ok(slot)
    }

    /// Release `alloc`'s held slot, freeing it for the next assign's
    /// smallest-free scan.
    ///
    /// **Idempotent teardown (criterion 2):** releasing an alloc that is not
    /// held is a benign no-op (`BTreeMap::remove` of an absent key), so a
    /// double-release or a release for an alloc that never reached Running does
    /// not panic and does not disturb the held set. The released slot becomes
    /// the smallest-free candidate again iff it is the lowest free value.
    pub fn release(&self, alloc: &AllocationId) {
        // Lock → remove → drop the guard within the call. `remove` returning
        // `None` (the alloc was not held) is the idempotent no-op — exactly the
        // teardown-of-an-unheld-alloc case.
        self.held.lock().remove(alloc);
    }

    /// Snapshot the currently-held `alloc → slot` bindings.
    ///
    /// A point-in-time clone for read-only observers (e.g. a future restart
    /// rebuild or a status surface), decoupled from the live map. Iteration
    /// order is `Ord` on [`AllocationId`], deterministic across processes and
    /// seeds (§ "Ordered-collection choice"). Mirrors
    /// [`crate::identity_mgr::IdentityMgr::held_snapshot`].
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<AllocationId, NetSlot> {
        self.held.lock().clone()
    }

    /// Claim the SPECIFIC `(alloc, slot)` binding observed surviving a restart
    /// (adopt-on-restart, 04-04) — the inverse of [`assign`](Self::assign)'s
    /// smallest-free pick. Used ONLY by the boot recovery pass
    /// ([`adopt_on_restart_recovery`]) to rebuild the held map from the
    /// recovered slot↔alloc correlation BEFORE any smallest-free `assign` can
    /// run, so a subsequent `assign` cannot hand a surviving slot to a new
    /// alloc (the cross-restart B1 collision).
    ///
    /// **Atomic check-and-act (`development.md` § "Check-and-act must be
    /// atomic"):** ONE locked critical section scans the held map's values for
    /// `slot` and inserts iff free OR already held by THIS alloc (idempotent
    /// re-adopt). The conflict verdict IS the scan's own outcome under the
    /// guard — never a separate contains-then-insert pre-check.
    ///
    /// # Errors
    ///
    /// Returns [`NetSlotAdoptConflict`] when `slot` is already held by a
    /// DIFFERENT alloc — the boot pass treats this as a fatal correlation bug
    /// (two survivors claiming one slot is impossible by construction; distinct
    /// slots ⇒ distinct netns) and refuses to boot rather than silently
    /// overwrite. Re-adopting the SAME `(alloc, slot)` is an idempotent no-op
    /// success.
    pub fn adopt(&self, alloc: AllocationId, slot: NetSlot) -> Result<(), NetSlotAdoptConflict> {
        // ONE locked critical section: scan the held values for `slot` AND
        // insert, with no contains-then-insert TOCTOU window. The scan's own
        // outcome IS the conflict verdict.
        let mut held = self.held.lock();
        // Is `slot` already held by SOMEONE? Find the holder.
        if let Some((holder, _)) = held.iter().find(|&(_, &held_slot)| held_slot == slot) {
            if holder == &alloc {
                // Idempotent re-adopt of the SAME (alloc, slot): the binding is
                // already present and correct — a no-op success.
                drop(held);
                return Ok(());
            }
            // Held by a DIFFERENT alloc: a fatal correlation bug. Refuse.
            let conflict =
                NetSlotAdoptConflict { slot, held_by: holder.clone(), requested_by: alloc };
            drop(held);
            return Err(conflict);
        }
        // The slot is free — bind it to `alloc` in the SAME critical section.
        // (An idempotent re-adopt where THIS alloc already holds a DIFFERENT
        // slot is not expected from the recovery pass — each alloc has exactly
        // one surviving netns/slot — so we record the observed binding as-is.)
        held.insert(alloc, slot);
        drop(held);
        Ok(())
    }
}

/// The error returned when [`NetSlotAllocator::adopt`] is asked to claim a
/// `slot` that is ALREADY held by a DIFFERENT allocation — a fatal
/// boot-recovery correlation bug (two survivors cannot share one slot, since
/// distinct slots ⇒ distinct netns by construction). The boot pass refuses to
/// start (`health.startup.refused`, reason `netns.adopt`) rather than silently
/// overwrite the binding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("net slot {slot} already held by {held_by}, cannot adopt for {requested_by}")]
pub struct NetSlotAdoptConflict {
    /// The contested slot.
    pub slot: NetSlot,
    /// The allocation currently holding `slot`.
    pub held_by: AllocationId,
    /// The allocation the recovery pass tried to adopt `slot` for.
    pub requested_by: AllocationId,
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
    reason = "sixteen independent observed kernel facts (netns presence, host-veth/workload-veth \
              presence, in-netns move, per-end addr, host-end up, in-netns-end up, netns lo up, \
              default route, per-end tx-offload, ip_forward, global rp_filter, host-veth \
              rp_filter, per-netns resolv.conf injected); a flag-per-fact value object is the \
              clearest model of the converge input and mirrors the host-netns ObservedVeth shape \
              ADR-0061 § 3.1 prescribes"
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
    /// The per-netns `/etc/netns/<netns>/resolv.conf` already carries the
    /// desired `nameserver <responder_addr>` line (D-TME-9 / Q5a). Read by the
    /// observer from the host-side per-netns file; when `false` the converge
    /// emits [`WorkloadVethStep::WriteResolvConf`]. The observer reads this as
    /// `netns_present && resolv_conf_has_responder(plan)`, so an absent netns
    /// forces `false` and the write is re-emitted — not because the per-netns
    /// dir vanishes with the netns (it does not: `/etc/netns/<netns>/` is a
    /// host-side `/etc` dir reaped only by the explicit teardown step, see
    /// `teardown_workload_netns`), but because the desired line cannot be
    /// observed present when there is no netns to read it through.
    pub resolv_conf_injected: bool,
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
    /// Write `/etc/netns/<netns>/resolv.conf` (creating `/etc/netns/<netns>/`
    /// first) with `nameserver <responder_addr>` — the stock `ip netns`
    /// per-netns convention, bind-mounted over `/etc/resolv.conf` inside the
    /// namespace (the Fly.io `fdaa::3` node-local DNS injection model, D-TME-9
    /// / Q5a). The write targets a host-side `/etc` path and has no real kernel
    /// dependency on the netns/veth/route (it would succeed even before `ip
    /// netns add`); it is sequenced with the netns steps only because the
    /// bind-mount it backs takes effect when the netns runs the workload. The
    /// responder address is a plan INPUT (`plan.responder_addr`), not derived
    /// state. Per ADR-0071 § Enforcement this write is part of the same
    /// converge-on-boot pass — a netns whose resolv.conf cannot be written
    /// refuses the boot.
    WriteResolvConf,
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
///   (B2), `AddDefaultRoute` when the in-netns default route is absent,
///   `WriteResolvConf` when the per-netns resolv.conf is not yet injected (or
///   the netns is absent — the observer cannot read the desired line present
///   without a netns, so it forces `resolv_conf_injected == false`; the
///   host-side `/etc/netns/<netns>/` dir itself is NOT coupled to the netns
///   lifecycle and is reaped only by the explicit teardown step; D-TME-9 /
///   Q5a). The two up-steps are ordered AFTER `MoveWorkloadEndIntoNetns` — a
///   netns provisioned from the plan must be able to carry a packet.
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
    // Per-netns resolv.conf injection (D-TME-9 / Q5a): write the node-local
    // DNS responder into `/etc/netns/<netns>/resolv.conf`. Keyed on the
    // observed `resolv_conf_injected` fact so the pure diff stays observed-only
    // and a converged netns re-emits nothing. The `!netns_present ||` disjunct
    // is defence-in-depth mirroring SetLoopbackUp (`:780`), NOT a consequence
    // of the dir's lifecycle: `/etc/netns/<netns>/` is a host-side dir under
    // `/etc` with no coupling to the kernel netns object (which is exactly why
    // teardown reaps it explicitly — `ip netns del` does not; see
    // `teardown_workload_netns` / `resolv_conf_dir_remove`, `:1473`). The
    // observer already forces `resolv_conf_injected == false` whenever
    // `!netns_present` (it reads `netns_present && resolv_conf_has_responder`,
    // `:1545`), so the disjunct never changes the outcome for an
    // observer-produced state — it only guards the kernel-impossible
    // `{netns_present:false, resolv_conf_injected:true}` against future drift.
    if !observed.netns_present || !observed.resolv_conf_injected {
        steps.push(WorkloadVethStep::WriteResolvConf);
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

// =============================================================================
// Per-allocation netns + veth real-execution path (step 02-02)
// =============================================================================
//
// The real `ip netns` / `ip -n <ns>` / `sysctl` / `ethtool` execution path
// for the per-allocation surface, mirroring the host-netns `provision` /
// `observe` / `execute_step` shape above but operating across a per-alloc
// network namespace. The pure derivation (`derive_workload_netns_plan`) and
// the pure converge diff (`workload_converge_steps`) are 02-01; this is the
// thin impure observer + executor + driver + teardown that applies them.

/// Provision one allocation's netns + veth pair from `plan`.
///
/// **Idempotent converge-on-boot** (ADR-0061 § 3.1, per-allocation parallel
/// of [`provision`]): OBSERVE the actual kernel netns/veth state
/// ([`observe_workload_netns`]), compute the per-resource diff
/// ([`workload_converge_steps`]), then EXECUTE each step idempotently
/// (swallowing `EEXIST` / `File exists` on netns/link/addr/route add). A
/// complete netns converges to an all-noop; a half-provisioned netns (the
/// netns survives but the veth is absent — the crash-mid-provision case) is
/// COMPLETED in place. The provisioner tolerates being interrupted at any
/// point and re-run from the top (research R7 self-heal — the appliance OS
/// has no operator shell, so the system must self-heal).
///
/// Synchronous (`std::process::Command`) — provisioning is a per-alloc
/// one-shot at lifecycle-start, matching the host-netns `provision` shape
/// and keeping the `ip` shell-out out of an `async fn`.
///
/// # Errors
///
/// Returns a distinct [`VethProvisionError`] variant per failing `ip` /
/// `sysctl` / `ethtool` step so the caller can branch on which step failed.
pub fn provision_workload_netns(plan: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
    let observed = observe_workload_netns(plan)?;
    for step in workload_converge_steps(plan, &observed) {
        execute_workload_step(plan, step)?;
    }
    Ok(())
}

/// Tear down one allocation's netns + veth pair, leaving ZERO residue.
///
/// `ip netns del <netns>` reaps the in-netns veth end (it dies with the
/// netns); a follow-up idempotent `ip link del <host_veth>` reaps the
/// host-side end if it survived (it should die with its peer, but the del is
/// belt-and-suspenders for a corrupted half-pair). The per-netns resolv.conf
/// dir (`/etc/netns/<netns>/`) is NOT reaped by `ip netns del`, so it is
/// removed explicitly — otherwise a re-provision under the same slot would
/// adopt a stale responder line. Idempotent — an absent netns / link /
/// resolv.conf dir is benign (the teardown success case), so a second
/// teardown is a silent no-op.
///
/// # Errors
///
/// Returns [`VethProvisionError::NetnsDelFailed`] / [`VethProvisionError::LinkDelFailed`]
/// only on a NON-benign failure (e.g. permission denied); "absent" is
/// swallowed. A failure removing the per-netns resolv.conf dir surfaces as
/// [`VethProvisionError::ResolvConfRemoveFailed`] (an absent dir is benign).
pub fn teardown_workload_netns(plan: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
    // `ip netns del <netns>` reaps the in-netns veth end with the namespace.
    netns_del(&plan.netns)?;
    // Belt-and-suspenders: reap the host-side end if it survived (it should
    // die with its peer, but a corrupted half-pair may leave it). `link_del`
    // swallows "absent", so this is a no-op in the common case.
    link_del(&plan.host_veth)?;
    // `ip netns del` does NOT remove `/etc/netns/<netns>/`; reap it so a
    // re-provision under the same slot starts clean (zero residue). An absent
    // dir is benign (NotFound swallowed); any other io::Error is fatal so a
    // permission failure does not silently leave stale DNS config behind.
    resolv_conf_dir_remove(&plan.netns)
}

// --- Adopt-on-restart boot recovery (step 04-04, D-TME-12 §1–§4) ---------
//
// On a `serve` restart the in-RAM NetSlotAllocator map is reconstructed EMPTY,
// but workloads SURVIVE (setsid + kill_on_drop(false) + own cgroup scope —
// SPIKE-A, kernel 7.0.0) inside their old `ovd-ns-<slot>` netns. A naive empty
// allocator hands smallest-free slot 0 to the next NEW alloc → collides with a
// survivor still occupying `ovd-ns-0000` (B1 resurrected across restart). Plus
// an orphan-netns leak: a pre-restart `ovd-ns-<slot>` whose workload DIED in
// the restart window is never torn down. SPIKE-B confirmed
// `WorkloadLifecycle::reconcile` does NOT re-drive already-Running survivors,
// so a dedicated boot-time recovery pass is the ONLY trigger that rebuilds the
// slot↔alloc map. B3: the netns name carries NO alloc identity, so the binding
// is RECOVERED via cgroup→PID→`/proc/<pid>/ns/net` inode correlation (SPIKE-C).

/// One surviving netns observed at boot: its slot, and the alloc that owns it
/// (recovered via PID→netns correlation) if any live PID claims it (D-TME-12
/// §1). `Some(alloc)` ⇒ ADOPT the binding; `None` ⇒ ORPHAN (GC candidate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedAdoptNetns {
    /// The slot parsed back from the surviving `ovd-ns-<4hex>` netns name.
    pub slot: NetSlot,
    /// `Some(alloc)` when a live PID inside `<alloc>.scope` resolves
    /// (`/proc/<pid>/ns/net`) to this netns's inode; `None` = orphan (no live
    /// owner → GC candidate).
    pub owner: Option<AllocationId>,
}

/// The PURE adopt-vs-GC decision over the observed surviving netns (D-TME-12
/// §1, the "DECISION LOGIC … must be a SEPARATE PURE function" mandate). Total,
/// deterministic, no I/O — the default-lane unit + mutation surface
/// (criterion 1).
///
/// - every `{ slot, owner: Some(alloc) }` → an ADOPT of `(alloc, slot)`;
/// - every `{ slot, owner: None }` → a GC of `slot`.
///
/// Order is preserved from the input so the boot pass's adopt-then-GC ordering
/// (§3) is the caller's, not buried here.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AdoptPlan {
    /// `(alloc, slot)` bindings to ADOPT into the allocator.
    pub adopt: Vec<(AllocationId, NetSlot)>,
    /// Slots whose netns is an ORPHAN (no live owner) to GC.
    pub gc: Vec<NetSlot>,
}

/// Pure adopt-vs-GC planner (D-TME-12 §1). Owned → adopt; orphan → GC.
#[must_use]
pub fn plan_adopt_actions(observed: &[ObservedAdoptNetns]) -> AdoptPlan {
    let mut plan = AdoptPlan::default();
    for o in observed {
        match &o.owner {
            Some(alloc) => plan.adopt.push((alloc.clone(), o.slot)),
            None => plan.gc.push(o.slot),
        }
    }
    plan
}

/// The error returned when the boot recovery pass cannot reconstruct a
/// consistent slot↔alloc map — a fatal boot condition the node refuses to start
/// on (`health.startup.refused`, reason `netns.adopt`) rather than serve with a
/// half-rebuilt allocator that would collide a fresh alloc onto a survivor.
#[derive(Debug, thiserror::Error)]
pub enum NetnsRecoveryError {
    /// Two surviving netns correlated to the SAME slot for DIFFERENT allocs —
    /// impossible by construction (distinct slots ⇒ distinct netns), so it
    /// signals a correlation bug. Pass-through the typed
    /// [`NetSlotAdoptConflict`] per `.claude/rules/development.md` § "Never
    /// flatten a typed error to `Internal(String)`".
    #[error("adopt-on-restart slot conflict: {source}")]
    AdoptConflict {
        /// The underlying allocator conflict.
        #[from]
        source: NetSlotAdoptConflict,
    },
    /// An `ip netns` / procfs read failed while observing the surviving netns.
    #[error("adopt-on-restart observe failed: {source}")]
    Observe {
        /// The underlying `ip(8)` / observe failure.
        #[from]
        source: VethProvisionError,
    },
    /// A boot-recovery observe read (`cgroup.procs` / a netns handle `stat` /
    /// `/proc/<pid>/ns/net`) failed for a NON-absent reason (EACCES, EIO,
    /// transient). Refuses the boot rather than misclassify a live workload's
    /// netns as an orphan and destructively tear it down — the fail-closed
    /// posture the rest of 04-04 holds. (A genuine `NotFound` is NOT this — it
    /// is the legitimate "no live PID / scope reaped" signal handled inline by
    /// [`io_error_is_benign_absence`].) Distinct from [`Self::Observe`], which
    /// wraps an `ip(8)` shell-out failure, so the Display verb names the actual
    /// operation (a direct cgroup/proc read, not an `ip` invocation).
    #[error("adopt-on-restart observe read failed (non-absent): {source}")]
    ObserveRead {
        /// The underlying non-absent `cgroup.procs` / netns-handle /
        /// `/proc/<pid>/ns/net` read failure.
        #[source]
        source: std::io::Error,
    },
    /// Reading the ObservationStore Running set failed.
    #[error("adopt-on-restart could not read alloc_status rows: {source}")]
    Observation {
        /// The underlying observation-store read failure.
        #[from]
        source: overdrive_core::traits::observation_store::ObservationStoreError,
    },
}

/// True iff `err` is the BENIGN "the thing is genuinely absent" signal
/// (`NotFound`) — distinct from a genuine read failure (EACCES, EIO,
/// transient). The orphan-GC observe path treats a benign absence as
/// "legitimately no live PID / no handle" (orphan-eligible / skip-this-PID)
/// but MUST propagate every other `io::ErrorKind` rather than silently degrade
/// into the destructive `ip netns del` branch. Pure so a unit test pins the
/// classification without a real fs.
fn io_error_is_benign_absence(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::NotFound
}

/// Read the netns inode from a `/proc/<pid>/ns/net` symlink target. The symlink
/// resolves to `net:[<inode>]`; parse out the inode.
///
/// Returns `Ok(None)` for the BENIGN cases: the PID died between the
/// `cgroup.procs` read and now (`NotFound` on the symlink — a common,
/// legitimate race → skip this PID), or a malformed symlink target (a parse
/// guard for the should-never-happen shape, NOT the swallowed-io case). Returns
/// `Err(NetnsRecoveryError::ObserveRead)` for a NON-absent read failure (EACCES,
/// EIO, …) so the recovery pass refuses the boot rather than misclassify a live
/// workload's netns as an orphan and destructively tear it down (the
/// fail-closed posture the rest of 04-04 holds).
///
/// Production copy of the proven in-tree mechanism (the
/// `overdrive-worker/tests/.../netns_entry.rs` precedent, SPIKE-C); the
/// recovery pass MUST NOT depend on a test module.
fn read_proc_netns_inode(pid: u32) -> Result<Option<u64>, NetnsRecoveryError> {
    let link = match std::fs::read_link(format!("/proc/{pid}/ns/net")) {
        Ok(link) => link,
        // The PID died between the cgroup.procs read and now — a common,
        // legitimate race. Skip this PID.
        Err(e) if io_error_is_benign_absence(&e) => return Ok(None),
        Err(e) => return Err(NetnsRecoveryError::ObserveRead { source: e }),
    };
    let s = link.to_string_lossy();
    // `strip_prefix`/`strip_suffix`/`parse` are PARSE guards (a malformed
    // symlink target — should never happen), NOT the swallowed-io case: a
    // genuinely unparseable target is "no resolvable inode here", skip the PID.
    Ok(s.strip_prefix("net:[")
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|n| n.parse::<u64>().ok()))
}

/// Read the inode of a named netns handle at `/var/run/netns/<netns>` (the
/// same inode `/proc/<pid>/ns/net` resolves to when a PID lives in it — the
/// `ip netns identify` mechanism).
///
/// Returns `Ok(None)` when the handle is genuinely absent (`NotFound`).
/// Returns `Err(NetnsRecoveryError::ObserveRead)` for a NON-absent `stat`
/// failure (EACCES, EIO, …): a surviving netns whose inode cannot be read must
/// NOT silently fall out of the `by_inode` map (which would leave it
/// `owner: None` → orphan → destructive `ip netns del` of a live workload's
/// netns). Refuse the boot instead — fail-closed.
fn netns_file_inode(netns: &str) -> Result<Option<u64>, NetnsRecoveryError> {
    use std::os::unix::fs::MetadataExt;
    match std::fs::metadata(format!("/var/run/netns/{netns}")) {
        Ok(m) => Ok(Some(m.ino())),
        Err(e) if io_error_is_benign_absence(&e) => Ok(None),
        Err(e) => Err(NetnsRecoveryError::ObserveRead { source: e }),
    }
}

/// Parse the slot back from an `ovd-ns-<4hex>` netns name (the inverse of
/// [`NetSlot::to_hex4`] + the [`WORKLOAD_NETNS_PREFIX`]). `None` for a name that
/// is not a workload netns or whose hex is out of range.
fn slot_from_netns_name(name: &str) -> Option<NetSlot> {
    let hex = name.strip_prefix(WORKLOAD_NETNS_PREFIX)?;
    let value = u16::from_str_radix(hex, 16).ok()?;
    NetSlot::new(value).ok()
}

/// Enumerate every surviving `ovd-ns-*` workload netns via `ip netns list`,
/// parsing each back to its [`NetSlot`] (reuses the [`netns_exists`] line-parse
/// shape — the first whitespace token is the name).
fn list_workload_netns_slots() -> Result<Vec<NetSlot>, VethProvisionError> {
    let out = std::process::Command::new("ip").args(["netns", "list"]).output()?;
    if !out.status.success() {
        return Err(VethProvisionError::NetnsObserveFailed {
            operation: "netns list".to_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            status: out.status.code(),
        });
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(slot_from_netns_name)
        .collect())
}

/// Read the live PIDs of an allocation from its cgroup scope's `cgroup.procs`
/// (`overdrive.slice/workloads.slice/<alloc>.scope/cgroup.procs`, resolved
/// under `cgroup_root`).
///
/// An ABSENT scope (`NotFound`) yields an EMPTY pid list: a Running row whose
/// cgroup scope was reaped is exactly a survivor that died → legitimately no
/// live PID here → the orphan-GC path then handles it. But a NON-absent read
/// failure (EACCES, EIO, a transient, or a cgroup-path regression) is NOT the
/// same as "empty scope" — swallowing it into `Vec::new()` would make a LIVE,
/// Running workload look like an orphan and drive a destructive `ip netns del`
/// against its netns. Propagate every non-`NotFound` io error as
/// [`NetnsRecoveryError::ObserveRead`] so the boot refuses rather than mass-GC
/// live workloads (`.claude/rules/development.md` § "Distinct failure modes get
/// distinct error variants. Never silently absorb a `Result` into a default").
fn alloc_scope_pids(
    alloc: &AllocationId,
    cgroup_root: &std::path::Path,
) -> Result<Vec<u32>, NetnsRecoveryError> {
    let procs = overdrive_worker::cgroup_manager::CgroupPath::for_alloc(alloc)
        .resolve(cgroup_root)
        .join("cgroup.procs");
    let body = match std::fs::read_to_string(&procs) {
        Ok(body) => body,
        Err(e) if io_error_is_benign_absence(&e) => return Ok(Vec::new()),
        Err(e) => return Err(NetnsRecoveryError::ObserveRead { source: e }),
    };
    Ok(body.lines().filter_map(|l| l.trim().parse::<u32>().ok()).collect())
}

/// Observe the surviving slot↔alloc bindings at boot (D-TME-12 §1) — a thin
/// impure observer (real `ip netns list` + procfs reads, NO decision logic;
/// the adopt-vs-GC decision is the pure [`plan_adopt_actions`]).
///
/// Walk:
/// 1. Enumerate surviving `ovd-ns-<slot>` netns ([`list_workload_netns_slots`])
///    and their inodes ([`netns_file_inode`]).
/// 2. Read the Running alloc set from `obs.alloc_status_rows()` filtered to
///    `state == Running` (B3: the row carries `alloc_id`, NOT the slot).
/// 3. For each Running alloc, read its `cgroup.procs` PIDs and resolve each
///    PID's `/proc/<pid>/ns/net` inode; match against the enumerated netns
///    inodes → the `(slot, owner=alloc)` binding. A surviving netns with no
///    matched live owner is an ORPHAN (`owner: None`).
async fn adopt_observe(
    obs: &dyn overdrive_core::traits::observation_store::ObservationStore,
    cgroup_root: &std::path::Path,
) -> Result<Vec<ObservedAdoptNetns>, NetnsRecoveryError> {
    use overdrive_core::traits::observation_store::AllocState;

    // (1) the surviving workload netns + their inodes.
    let slots = list_workload_netns_slots()?;
    let mut by_inode: BTreeMap<u64, NetSlot> = BTreeMap::new();
    let mut owner: BTreeMap<NetSlot, Option<AllocationId>> = BTreeMap::new();
    for slot in slots {
        let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
        // `?`: a NON-absent inode `stat` failure refuses the boot — a surviving
        // netns that silently fell out of `by_inode` would be misclassified as
        // an orphan and destructively torn down.
        if let Some(ino) = netns_file_inode(&plan.netns)? {
            by_inode.insert(ino, slot);
        }
        owner.entry(slot).or_insert(None);
    }

    // (2) the Running alloc set (B3: alloc id, not slot).
    let running: Vec<AllocationId> = obs
        .alloc_status_rows()
        .await?
        .into_iter()
        .filter(|r| r.state == AllocState::Running)
        .map(|r| r.alloc_id)
        .collect();

    // (3) correlate PID→netns per Running alloc → owner binding.
    //
    // `owner` is keyed by SLOT, so two Running allocs correlating to the SAME
    // slot collapse here to last-write-wins — `plan_adopt_actions` therefore
    // never emits two adopts for one slot, and `NetSlotAllocator::adopt`'s
    // conflict arm (the `?` boot-refusal in `adopt_on_restart_recovery`) is
    // STRUCTURALLY UNREACHABLE from this real observe path. That arm is a
    // defensive guard reachable only by the direct unit tests
    // (`adopt_conflicts_when_slot_held_by_a_different_alloc`); do not mistake
    // criterion-3's "adopt-conflict refuses boot" for a live production path.
    for alloc in running {
        // `?`: a NON-absent `cgroup.procs` read failure refuses the boot — a
        // Running alloc that silently contributed zero PIDs would let its live
        // netns be misclassified as an orphan and destructively torn down.
        for pid in alloc_scope_pids(&alloc, cgroup_root)? {
            // `?`: a NON-absent `/proc/<pid>/ns/net` read failure refuses the
            // boot; a benign `NotFound` (PID died in the read window) skips
            // this PID (`Ok(None) => continue`), which is correct.
            let Some(ino) = read_proc_netns_inode(pid)? else {
                continue;
            };
            if let Some(&slot) = by_inode.get(&ino) {
                owner.insert(slot, Some(alloc.clone()));
                break;
            }
        }
    }

    Ok(owner.into_iter().map(|(slot, owner)| ObservedAdoptNetns { slot, owner }).collect())
}

/// The boot-time adopt-on-restart recovery pass (D-TME-12 §3). Driven by
/// `run_server` after `AppState` construction, BEFORE the convergence loop /
/// exit-observer spawn, gated by the same `mtls_worker.is_some()` composition
/// gate G1 uses (a no-op on a non-mTLS boot where no per-alloc netns exist).
///
/// PINNED order (adopt-BEFORE-GC is load-bearing):
/// 1. [`adopt_observe`] → the surviving slot↔alloc bindings.
/// 2. [`plan_adopt_actions`] → the pure adopt-vs-GC decision.
/// 3. ADOPT every owned binding via [`NetSlotAllocator::adopt`] — rebuilds the
///    held map so the very next smallest-free `assign` cannot collide with a
///    survivor. A [`NetSlotAdoptConflict`] REFUSES the boot.
/// 4. GC every orphan via [`teardown_workload_netns`] (teardown-not-release:
///    an orphan holds no binding) — its slot returns to the free pool.
///
/// This is the netns half (§1–§4) only. The §5 nft-rule sweep is a SEPARATE
/// surviving-resource class whose machinery lives in
/// `overdrive-worker::mtls_intercept` (private predicates + by-handle delete);
/// SPIKE-D (findings-adopt-restart.md § SPIKE-D, kernel 7.0.0) confirmed the
/// rules survive and the sweep is needed, but it cannot be built within this
/// step's file boundary without inventing new public surface in
/// `mtls_intercept.rs` — surfaced as a boundary blocker, not built here.
///
/// # Errors
///
/// [`NetnsRecoveryError`] when observe fails, the obs read fails, or an adopt
/// conflicts (two survivors on one slot — a fatal correlation bug).
pub async fn adopt_on_restart_recovery(
    obs: &dyn overdrive_core::traits::observation_store::ObservationStore,
    allocator: &NetSlotAllocator,
    cgroup_root: &std::path::Path,
) -> Result<(), NetnsRecoveryError> {
    let observed = adopt_observe(obs, cgroup_root).await?;
    let plan = plan_adopt_actions(&observed);

    // (3) ADOPT first — rebuild the held map before any free-slot scan. The `?`
    // boot-refusal on a `NetSlotAdoptConflict` is a defensive guard: the real
    // observe path keys `owner` by slot (last-write-wins; see `adopt_observe`
    // step 3), so it can NEVER emit two adopts for one slot. This arm is
    // exercised only by the direct unit tests, not by production recovery.
    for (alloc, slot) in plan.adopt {
        allocator.adopt(alloc, slot)?;
    }

    // (4) GC orphans second — return their slots to the free pool.
    for slot in plan.gc {
        let orphan_plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
        // Teardown is idempotent (swallows "absent"); a non-benign failure
        // (e.g. permission denied) surfaces so the boot does not silently leave
        // a leaked netns behind.
        teardown_workload_netns(&orphan_plan)?;
    }

    Ok(())
}

/// Read the actual kernel state of one allocation's netns + veth pair into
/// an [`ObservedWorkloadVeth`] — the input to the pure
/// [`workload_converge_steps`] diff (the per-allocation parallel of
/// [`observe`]).
///
/// Each field is one observable fact read via `ip netns list`,
/// `ip [-n <ns>] link show`, `ip -n <ns> addr/route show`, `sysctl -n`, and
/// `ethtool -k` (host) / `ip netns exec <ns> ethtool -k` (in-netns). The
/// observer is a thin impure shim; the KILLABLE decision logic lives in the
/// pure `workload_converge_steps` (02-01-covered).
fn observe_workload_netns(
    plan: &WorkloadNetnsPlan,
) -> Result<ObservedWorkloadVeth, VethProvisionError> {
    let netns_present = netns_exists(&plan.netns)?;

    // Host-side end presence + up-state (always in the host netns).
    let (host_veth_present, host_veth_up) = host_link_state(&plan.host_veth)?;

    // The in-netns end is present iff it is found in EITHER the host netns
    // (not yet moved) or the workload netns (moved). "in netns" is the
    // narrower fact: present specifically inside the workload netns.
    let workload_in_host = host_link_state(&plan.workload_veth)?.0;
    let (workload_in_ns, workload_veth_up) = if netns_present {
        netns_link_state(&plan.netns, &plan.workload_veth)?
    } else {
        (false, false)
    };
    let workload_veth_present = workload_in_host || workload_in_ns;
    let workload_veth_in_netns = workload_in_ns;

    // Host-side address presence (host netns getifaddrs walk).
    let host_addr_present = host_veth_present && iface_has_addr(&plan.host_veth, plan.host_addr);
    // In-netns address presence + default route + lo up — only meaningful
    // once the end is inside the netns.
    let workload_addr_present = netns_present
        && workload_in_ns
        && netns_iface_has_addr(&plan.netns, &plan.workload_veth, plan.workload_addr)?;
    let lo_up = netns_present && netns_link_state(&plan.netns, "lo")?.1;
    let default_route_present =
        netns_present && netns_default_route_present(&plan.netns, plan.gateway)?;

    // TX-offload: only meaningful for a present end. An absent end reads
    // `false`; the converge `pair_rebuilt` path re-emits the disable after a
    // fresh create regardless, so the false never suppresses a needed step.
    // (Same end-state-insensitive impure-shim class as `observe`.)
    // mutants: skip — impure observer, `&&`→`||` is end-state-insensitive
    let host_tx_offload_on = host_veth_present && host_iface_tx_offload_on(&plan.host_veth);
    // mutants: skip — impure observer, `&&`→`||` is end-state-insensitive
    let workload_tx_offload_on = netns_present
        && workload_in_ns
        && netns_iface_tx_offload_on(&plan.netns, &plan.workload_veth);

    // Host prerequisites — global sysctls + the per-host-veth knob. The
    // per-host-veth knob only exists once the host-side end exists.
    let ip_forward_enabled = sysctl_is_one("net.ipv4.ip_forward");
    let rp_filter_global_relaxed = sysctl_rp_filter_relaxed("net.ipv4.conf.all.rp_filter")
        && sysctl_rp_filter_relaxed("net.ipv4.conf.lo.rp_filter");
    let host_veth_rp_filter_relaxed = host_veth_present
        && sysctl_rp_filter_relaxed(&format!("net.ipv4.conf.{}.rp_filter", plan.host_veth));

    // Per-netns resolv.conf injection: the desired `nameserver <responder>`
    // line is already present in `/etc/netns/<netns>/resolv.conf`. Gated on
    // `netns_present` because the line is only observable through the
    // namespace's bind-mount — NOT because the host-side `/etc/netns/<netns>/`
    // dir tracks the netns lifecycle (it does not; teardown reaps it
    // explicitly, see `resolv_conf_dir_remove`). Read directly from the
    // host-side per-netns file — the same path the executor writes, so observer
    // and consumer agree on "injected".
    let resolv_conf_injected = netns_present && resolv_conf_has_responder(plan);

    Ok(ObservedWorkloadVeth {
        netns_present,
        host_veth_present,
        workload_veth_present,
        workload_veth_in_netns,
        host_addr_present,
        workload_addr_present,
        host_veth_up,
        workload_veth_up,
        lo_up,
        default_route_present,
        host_tx_offload_on,
        workload_tx_offload_on,
        ip_forward_enabled,
        rp_filter_global_relaxed,
        host_veth_rp_filter_relaxed,
        resolv_conf_injected,
    })
}

/// Apply a single [`WorkloadVethStep`] via `ip netns` / `ip -n <ns>` /
/// `sysctl` / `ethtool` — each arm maps 1:1 to the command in the variant's
/// rustdoc. Idempotent: `EEXIST` / `File exists` on netns/link/addr/route add
/// is swallowed; `ip link set up` and `sysctl -w` are idempotent at the
/// kernel.
fn execute_workload_step(
    plan: &WorkloadNetnsPlan,
    step: WorkloadVethStep,
) -> Result<(), VethProvisionError> {
    let prefix = plan.subnet.prefix_len();
    match step {
        WorkloadVethStep::CreateNetns => netns_add(&plan.netns),
        WorkloadVethStep::CreateVethPair => {
            // `ip link add <workload_veth> type veth peer name <host_veth>`.
            // A fresh pair may collide with a surviving end from a corrupted
            // half-pair; del both ends first (idempotent — absent is benign)
            // so the create cannot hit "File exists".
            link_del(&plan.host_veth)?;
            // The in-netns end may have been moved into the netns; del it
            // there too. Absent (or no netns) is benign.
            if netns_exists(&plan.netns)? {
                netns_link_del(&plan.netns, &plan.workload_veth)?;
            }
            link_del(&plan.workload_veth)?;
            workload_link_add(plan)
        }
        WorkloadVethStep::MoveWorkloadEndIntoNetns => netns_move(&plan.workload_veth, &plan.netns),
        WorkloadVethStep::AddHostAddr => {
            let cidr = format!("{}/{}", plan.host_addr, prefix);
            addr_add(&plan.host_veth, &cidr)
        }
        WorkloadVethStep::AddWorkloadAddr => {
            let cidr = format!("{}/{}", plan.workload_addr, prefix);
            netns_addr_add(&plan.netns, &plan.workload_veth, &cidr)
        }
        WorkloadVethStep::SetHostVethUp => link_up(&plan.host_veth),
        WorkloadVethStep::SetWorkloadVethUp => netns_link_up(&plan.netns, &plan.workload_veth),
        WorkloadVethStep::SetLoopbackUp => netns_link_up(&plan.netns, "lo"),
        WorkloadVethStep::AddDefaultRoute => netns_default_route_add(&plan.netns, plan.gateway),
        WorkloadVethStep::WriteResolvConf => resolv_conf_write(plan),
        WorkloadVethStep::EnableIpForward => sysctl_set("net.ipv4.ip_forward", "1"),
        WorkloadVethStep::RelaxGlobalRpFilter => {
            sysctl_set("net.ipv4.conf.all.rp_filter", "0")?;
            sysctl_set("net.ipv4.conf.lo.rp_filter", "0")
        }
        WorkloadVethStep::RelaxHostVethRpFilter => {
            sysctl_set(&format!("net.ipv4.conf.{}.rp_filter", plan.host_veth), "0")
        }
        WorkloadVethStep::DisableHostTxOffload => tx_offload_off(&plan.host_veth),
        WorkloadVethStep::DisableWorkloadTxOffload => {
            netns_tx_offload_off(&plan.netns, &plan.workload_veth)
        }
    }
}

// ---- per-allocation `ip` / `sysctl` / `ethtool` helpers ----

/// `ip netns add <netns>` — idempotent ("File exists" / "already exists" is
/// the converge success case, swallowed).
fn netns_add(netns: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip").args(["netns", "add", netns]).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("File exists") || stderr.contains("already exists") {
        return Ok(());
    }
    Err(VethProvisionError::NetnsAddFailed {
        netns: netns.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip netns del <netns>` — idempotent (an absent netns is benign).
fn netns_del(netns: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip").args(["netns", "del", netns]).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if netns_absent(&stderr) {
        return Ok(());
    }
    Err(VethProvisionError::NetnsDelFailed {
        netns: netns.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// True iff `<netns>` is listed by `ip netns list`. A non-zero `ip netns
/// list` exit (e.g. permission denied) surfaces as
/// [`VethProvisionError::NetnsObserveFailed`].
fn netns_exists(netns: &str) -> Result<bool, VethProvisionError> {
    let out = std::process::Command::new("ip").args(["netns", "list"]).output()?;
    if !out.status.success() {
        return Err(VethProvisionError::NetnsObserveFailed {
            operation: "netns list".to_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            status: out.status.code(),
        });
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Each line is e.g. `ovd-ns-0fff (id: 0)` — match the first token.
    Ok(stdout.lines().any(|line| line.split_whitespace().next() == Some(netns)))
}

/// `ip link add <workload_veth> type veth peer name <host_veth>` — the pair
/// is created with the in-netns end named `workload_veth` and the host end
/// named `host_veth` (both born in the host netns; the move follows).
fn workload_link_add(plan: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip")
        .args(["link", "add", &plan.workload_veth, "type", "veth", "peer", "name", &plan.host_veth])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::LinkAddFailed {
        // The host-netns variant names client/backend; for the per-alloc pair
        // the "client" slot carries the in-netns end and the "backend" slot
        // the host end — the message still names the exact `ip link add` args.
        client_iface: plan.workload_veth.clone(),
        backend_iface: plan.host_veth.clone(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip link set <iface> netns <netns>` — move `iface` from the host netns
/// into the workload netns. Idempotent at the kernel only in the sense that
/// a second move of an already-moved iface fails ("Cannot find device" in
/// the host netns) — but converge only emits this when the end is NOT yet in
/// the netns, so the move always has the iface present in the host netns.
fn netns_move(iface: &str, netns: &str) -> Result<(), VethProvisionError> {
    let out =
        std::process::Command::new("ip").args(["link", "set", iface, "netns", netns]).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::NetnsMoveFailed {
        iface: iface.to_owned(),
        netns: netns.to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip -n <netns> addr add <cidr> dev <iface>`. Idempotent — swallows
/// `EEXIST` / `File exists` (already-assigned is the converge success case).
fn netns_addr_add(netns: &str, iface: &str, cidr: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip")
        .args(["-n", netns, "addr", "add", cidr, "dev", iface])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("File exists") {
        return Ok(());
    }
    Err(VethProvisionError::AddrAddFailed {
        iface: iface.to_owned(),
        cidr: cidr.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip -n <netns> link set <iface> up`. Idempotent at the kernel.
fn netns_link_up(netns: &str, iface: &str) -> Result<(), VethProvisionError> {
    let out = std::process::Command::new("ip")
        .args(["-n", netns, "link", "set", iface, "up"])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::LinkUpFailed {
        iface: iface.to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip -n <netns> link del <iface>` — del an in-netns end. "Absent" is
/// benign (swallowed); used by the rebuild path to clear a moved in-netns
/// end before the pair is recreated.
fn netns_link_del(netns: &str, iface: &str) -> Result<(), VethProvisionError> {
    let out =
        std::process::Command::new("ip").args(["-n", netns, "link", "del", iface]).output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if link_absent(&stderr) {
        return Ok(());
    }
    Err(VethProvisionError::LinkDelFailed {
        iface: iface.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip -n <netns> route add default via <gateway>`. Idempotent — swallows
/// `File exists` (the route already present is the converge success case).
fn netns_default_route_add(netns: &str, gateway: Ipv4Addr) -> Result<(), VethProvisionError> {
    let gw = gateway.to_string();
    let out = std::process::Command::new("ip")
        .args(["-n", netns, "route", "add", "default", "via", &gw])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("File exists") {
        return Ok(());
    }
    Err(VethProvisionError::RouteAddFailed {
        cidr: "default".to_owned(),
        iface: format!("via {gw} (netns {netns})"),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip netns exec <netns> ethtool -K <iface> tx off` — disable
/// TX-checksum-offload on the in-netns end (same incremental-L4-csum
/// invariant as the host-side [`tx_offload_off`]; a "fixed / not supported"
/// non-zero exit is benign, EPERM is fatal).
fn netns_tx_offload_off(netns: &str, iface: &str) -> Result<(), VethProvisionError> {
    let out = match std::process::Command::new("ip")
        .args(["netns", "exec", netns, "ethtool", "-K", iface, "tx", "off"])
        .output()
    {
        Ok(out) => out,
        Err(err) => {
            return Err(VethProvisionError::TxOffloadDisableFailed {
                iface: iface.to_owned(),
                stderr: format!("spawning `ip netns exec … ethtool` failed: {err}"),
                status: None,
            });
        }
    };
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if tx_offload_benign(&stderr) {
        return Ok(());
    }
    Err(VethProvisionError::TxOffloadDisableFailed {
        iface: iface.to_owned(),
        stderr: stderr.trim().to_owned(),
        status: out.status.code(),
    })
}

/// The host-side directory the `ip netns` per-netns convention bind-mounts
/// `resolv.conf` from: `/etc/netns/<netns>/`.
fn resolv_conf_dir(netns: &str) -> String {
    format!("/etc/netns/{netns}")
}

/// The host-side per-netns resolv.conf path: `/etc/netns/<netns>/resolv.conf`.
/// `ip netns exec <netns> …` bind-mounts THIS file over `/etc/resolv.conf`
/// inside the namespace iff it exists.
fn resolv_conf_path(netns: &str) -> String {
    format!("{}/resolv.conf", resolv_conf_dir(netns))
}

/// Write the per-netns `/etc/netns/<netns>/resolv.conf` with the node-local
/// DNS responder (`nameserver <responder_addr>`), creating `/etc/netns/<netns>/`
/// first (idempotent, `mkdir -p`-shaped). Overwrite-to-desired-content, so a
/// re-apply over an already-injected file is safe (the converge gate only
/// emits this step when not yet injected, so a converged netns re-emits
/// nothing). A write failure refuses the boot per ADR-0071 § Enforcement (a
/// netns that resolves names against the wrong nameserver is a silent
/// correctness landmine), surfacing as
/// [`VethProvisionError::ResolvConfWriteFailed`] with the offending path and
/// the originating `io::Error` (distinct failure mode per `.claude/rules/
/// development.md` § Errors — never an `unwrap_or_default()` on a fallible
/// write). Sync `std::fs` is correct here: the provisioner is a synchronous
/// boot-time one-shot, NOT an `async fn` (the no-blocking-fs-in-async rule
/// does not apply).
fn resolv_conf_write(plan: &WorkloadNetnsPlan) -> Result<(), VethProvisionError> {
    let dir = resolv_conf_dir(&plan.netns);
    std::fs::create_dir_all(&dir).map_err(|source| VethProvisionError::ResolvConfWriteFailed {
        path: dir.clone(),
        source,
    })?;
    let path = resolv_conf_path(&plan.netns);
    std::fs::write(&path, resolv_conf_contents(plan.responder_addr))
        .map_err(|source| VethProvisionError::ResolvConfWriteFailed { path, source })
}

/// True iff `/etc/netns/<netns>/resolv.conf` already carries the desired
/// `nameserver <responder_addr>` line. Conservative on a read failure
/// (missing file, unreadable) → `false`, so the converge re-emits the write
/// (the safe default). An unreadable-for-another-reason file (e.g. permission
/// denied) also reads `false` here; the subsequent write then surfaces the
/// real `io::Error` via [`resolv_conf_write`], so the failure is not silently
/// swallowed — it is deferred to the write that refuses the boot.
fn resolv_conf_has_responder(plan: &WorkloadNetnsPlan) -> bool {
    let want = resolv_conf_contents(plan.responder_addr);
    matches!(std::fs::read_to_string(resolv_conf_path(&plan.netns)), Ok(body) if body == want)
}

/// Remove the per-netns resolv.conf dir `/etc/netns/<netns>/` (teardown). An
/// absent dir is benign (`NotFound` swallowed — the teardown success case);
/// any other `io::Error` (e.g. permission denied) is fatal so a teardown does
/// not silently leave stale DNS config behind under a slot a later provision
/// may reuse.
fn resolv_conf_dir_remove(netns: &str) -> Result<(), VethProvisionError> {
    let dir = resolv_conf_dir(netns);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(VethProvisionError::ResolvConfRemoveFailed { path: dir, source }),
    }
}

/// `sysctl -w <key>=<value>`. The `ip_forward` / `rp_filter` knobs are
/// load-bearing for egress routing + asymmetric-ingress survival, so a
/// failure is fatal (refuse the boot rather than ship a path that drops the
/// workload's packets).
fn sysctl_set(key: &str, value: &str) -> Result<(), VethProvisionError> {
    let out =
        std::process::Command::new("sysctl").args(["-w", &format!("{key}={value}")]).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::SysctlSetFailed {
        key: key.to_owned(),
        value: value.to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

/// `ip link show <iface>` in the HOST netns → `(present, up)`. Absent →
/// `(false, false)`; any other non-zero exit →
/// [`VethProvisionError::NetnsObserveFailed`].
fn host_link_state(iface: &str) -> Result<(bool, bool), VethProvisionError> {
    link_state(iface)
}

/// `ip -n <netns> link show <iface>` → `(present, up)`. Absent →
/// `(false, false)`; any other non-zero exit →
/// [`VethProvisionError::NetnsObserveFailed`].
fn netns_link_state(netns: &str, iface: &str) -> Result<(bool, bool), VethProvisionError> {
    let show =
        std::process::Command::new("ip").args(["-n", netns, "link", "show", iface]).output()?;
    if show.status.success() {
        let stdout = String::from_utf8_lossy(&show.stdout);
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
    Err(VethProvisionError::NetnsObserveFailed {
        operation: format!("-n {netns} link show {iface}"),
        stderr: stderr.trim().to_owned(),
        status: show.status.code(),
    })
}

/// True when `ip -n <netns> addr show dev <iface>` reports `want` bound.
/// A non-zero `ip addr show` (e.g. absent iface) → `Ok(false)`; a spawn
/// failure propagates.
fn netns_iface_has_addr(
    netns: &str,
    iface: &str,
    want: Ipv4Addr,
) -> Result<bool, VethProvisionError> {
    let out = std::process::Command::new("ip")
        .args(["-n", netns, "addr", "show", "dev", iface])
        .output()?;
    if !out.status.success() {
        return Ok(false);
    }
    let needle = format!("inet {want}/");
    Ok(String::from_utf8_lossy(&out.stdout).contains(&needle))
}

/// True when `ip -n <netns> route show default` carries `default via
/// <gateway>`.
fn netns_default_route_present(netns: &str, gateway: Ipv4Addr) -> Result<bool, VethProvisionError> {
    let out = std::process::Command::new("ip")
        .args(["-n", netns, "route", "show", "default"])
        .output()?;
    if !out.status.success() {
        return Ok(false);
    }
    let needle = format!("default via {gateway}");
    Ok(String::from_utf8_lossy(&out.stdout).contains(&needle))
}

/// True when `ethtool -k <iface>` (host netns) reports `tx-checksumming:
/// on`. Conservative on failure (returns `false`) — same untestable impure
/// shim class as the host-side [`iface_tx_offload_on`].
// mutants: skip — impure I/O shim; body mutants are end-state-insensitive
// (the downstream disable is idempotent), same class as `iface_tx_offload_on`.
fn host_iface_tx_offload_on(iface: &str) -> bool {
    iface_tx_offload_on(iface)
}

/// True when `ip netns exec <netns> ethtool -k <iface>` reports
/// `tx-checksumming: on`. Conservative on failure (returns `false`).
// mutants: skip — impure I/O shim; body mutants are end-state-insensitive
// (the downstream disable is idempotent), same class as `iface_tx_offload_on`.
fn netns_iface_tx_offload_on(netns: &str, iface: &str) -> bool {
    let Ok(out) = std::process::Command::new("ip")
        .args(["netns", "exec", netns, "ethtool", "-k", iface])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    tx_checksumming_on(&String::from_utf8_lossy(&out.stdout))
}

/// Read a `sysctl` integer knob, returning `None` when it cannot be read
/// (missing per-iface knob, spawn failure, non-integer output). A `NotFound`
/// per-iface knob legitimately reads `None` — the converge then treats it as
/// "not relaxed" / "not enabled" (the strict default).
fn sysctl_read(key: &str) -> Option<i64> {
    let out = std::process::Command::new("sysctl").args(["-n", key]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// True iff the integer sysctl `key` reads exactly `1`.
fn sysctl_is_one(key: &str) -> bool {
    sysctl_read(key) == Some(1)
}

/// True iff the `rp_filter` knob `key` is RELAXED — i.e. NOT strict. Strict
/// reverse-path filtering is `1`; `0` (off) and `2` (loose) both count as
/// relaxed. A knob that cannot be read (`None`) is treated as NOT relaxed
/// (so the converge re-emits the relax — the safe default).
fn sysctl_rp_filter_relaxed(key: &str) -> bool {
    matches!(sysctl_read(key), Some(v) if v != 1)
}

/// True when `ip netns del/list` stderr indicates the netns is simply
/// ABSENT, as opposed to a genuine failure (permission denied, etc.).
/// iproute2 phrasing varies: `Cannot remove namespace file "...": No such
/// file or directory` / `No such file or directory`.
fn netns_absent(stderr: &str) -> bool {
    stderr.contains("No such file or directory") || stderr.contains("does not exist")
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use super::{
        AdoptPlan, NET_SLOT_MAX, NetSlot, NetSlotAdoptConflict, NetSlotAllocator, NetSlotExhausted,
        ObservedAdoptNetns, ObservedVeth, ObservedWorkloadVeth, VethProvisionPlan, VethStep,
        WORKLOAD_SUBNET_BASE, WorkloadNetnsPlan, WorkloadVethStep, converge_steps,
        derive_veth_plan, derive_workload_netns_plan, io_error_is_benign_absence, link_absent,
        plan_adopt_actions, resolv_conf_contents, smallest_free_slot, tx_checksumming_on,
        tx_offload_benign, workload_converge_steps,
    };
    use ipnet::{IpAdd, Ipv4Net};
    use overdrive_core::AllocationId;
    use proptest::prelude::*;
    use std::collections::BTreeSet;
    use std::net::Ipv4Addr;
    use std::str::FromStr;

    /// Build an [`AllocationId`] for the allocator tests.
    fn alloc(id: &str) -> AllocationId {
        AllocationId::new(id).expect("valid AllocationId")
    }

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
            resolv_conf_injected: true,
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
            WorkloadVethStep::WriteResolvConf,
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
            resolv_conf_injected: false,
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
            // The per-netns resolv.conf is per-NETNS, not per-pair — it
            // survives a veth-pair rebuild (the netns is not torn down), so the
            // injected line is still present and WriteResolvConf is NOT
            // re-emitted (mirrors lo_up surviving the rebuild).
            resolv_conf_injected: true,
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

    /// Default-lane unit (criterion 5): the PURE "what should resolv.conf
    /// contain" derivation. Given the node-local DNS responder INPUT, the
    /// per-netns resolv.conf body is exactly `nameserver <addr>\n` — the stock
    /// single-`nameserver` shape `ip netns` bind-mounts into the namespace
    /// (the Fly.io `fdaa::3` injection model, D-TME-9 / Q5a). Input variations
    /// of one derivation behaviour (Mandate 5) — one parametrised assertion.
    #[test]
    fn resolv_conf_contents_is_a_single_nameserver_line() {
        let cases = [
            Ipv4Addr::new(10, 99, 255, 1),
            Ipv4Addr::new(169, 254, 0, 53),
            Ipv4Addr::new(127, 0, 0, 53),
        ];
        for addr in cases {
            assert_eq!(
                resolv_conf_contents(addr),
                format!("nameserver {addr}\n"),
                "resolv.conf body for responder {addr} must be exactly one trailing-newline \
                 `nameserver <addr>` line",
            );
        }
    }

    /// Converge rule (criterion 1, 5): `WriteResolvConf` is emitted IFF the
    /// per-netns resolv.conf is not yet injected — present (injected) ⇒ no
    /// step (idempotent no-op), absent (not injected) ⇒ exactly
    /// `WriteResolvConf`. Keyed on its own observed fact, on a present,
    /// otherwise-complete netns+pair (so no other step fires).
    #[test]
    fn workload_converge_writes_resolv_conf_only_when_not_injected() {
        let plan = workload_plan();

        // Already injected (the complete baseline) → no WriteResolvConf.
        let injected = complete_workload_observed();
        assert!(
            !workload_converge_steps(&plan, &injected).contains(&WorkloadVethStep::WriteResolvConf),
            "an already-injected netns must NOT re-emit WriteResolvConf (idempotent no-op)",
        );

        // Not yet injected (on a present, complete pair) → exactly
        // WriteResolvConf and nothing else.
        let not_injected =
            ObservedWorkloadVeth { resolv_conf_injected: false, ..complete_workload_observed() };
        assert_eq!(
            workload_converge_steps(&plan, &not_injected),
            vec![WorkloadVethStep::WriteResolvConf],
            "a netns whose resolv.conf is not injected must emit exactly WriteResolvConf",
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
            resolv_injected in any::<bool>(),
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
                resolv_conf_injected: resolv_injected,
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
            prop_assert_eq!(
                steps.contains(&WorkloadVethStep::WriteResolvConf),
                !resolv_injected
            );

            // (a) complete ⇒ empty.
            let all_satisfied = moved && host_addr && workload_addr && host_up && workload_up
                && lo_up && route && ip_forward && global_rp && host_veth_rp
                && !host_tx_on && !workload_tx_on && resolv_injected;
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
                    WorkloadVethStep::WriteResolvConf => after.resolv_conf_injected = true,
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

    // --- NetSlot allocator (step 02-04, D-TME-12 "Slot-allocator home") -------
    //
    // The STATEFUL companion to 02-01's PURE slot-keyed derivation: 02-01 takes
    // `slot` as an input; the allocator is what hands out the slot. These tests
    // exercise the PURE assign/release decision logic (`smallest_free_slot`) and
    // the held-map wrapper (`NetSlotAllocator`) — default-lane, no kernel. The
    // C3 wiring's real `provision_workload_netns` call is Tier-3-covered by
    // 02-02; it is NOT re-proved here (criterion 5).
    //
    // Every assertion is load-bearing (mutation-killable): the
    // assign-smallest-free, idempotent-re-assign, release-frees,
    // double-release-noop, and exhaustion predicates each FAIL if the decision
    // logic is mutated (the 02-02-vacuous-assertion lesson).

    /// PURE decision — `smallest_free_slot` over an empty held set yields slot
    /// 0 (the smallest of `0..=NET_SLOT_MAX`). The canonical assign-from-empty
    /// case; kills a "return NET_SLOT_MAX" / "return held.len()" mutant.
    #[test]
    fn smallest_free_slot_of_empty_held_set_is_zero() {
        let held: BTreeSet<NetSlot> = BTreeSet::new();
        assert_eq!(
            smallest_free_slot(&held).expect("an empty held set always has a free slot"),
            NetSlot::new(0).expect("0 is in range"),
            "the smallest free slot of an empty held set is 0"
        );
    }

    /// PURE decision — `smallest_free_slot` returns the smallest GAP, not the
    /// next-monotonic value. With {0, 1, 3} held it returns 2 (the hole), NOT 4
    /// (max+1). This is the assign-smallest-free contract (criterion 1): after a
    /// release frees a low slot, the next assign re-uses it. Kills a
    /// "return max_held + 1" monotonic mutant.
    #[test]
    fn smallest_free_slot_returns_the_lowest_gap_not_the_next_monotonic() {
        let held: BTreeSet<NetSlot> =
            [0u16, 1, 3].into_iter().map(|s| NetSlot::new(s).expect("in range")).collect();
        assert_eq!(
            smallest_free_slot(&held).expect("a sparse held set has a free slot"),
            NetSlot::new(2).expect("2 is in range"),
            "smallest_free_slot fills the lowest gap (2), not the next-monotonic slot (4)"
        );
    }

    /// PURE decision — a fully-held space (`0..=NET_SLOT_MAX` all held) yields
    /// the typed [`NetSlotExhausted`] error, NEVER a slot. Exhaustion is the
    /// criterion-4 refuse-the-alloc path: typed error, no panic, no reuse.
    #[test]
    fn smallest_free_slot_of_full_space_is_exhausted_error() {
        let held: BTreeSet<NetSlot> =
            (0..=NET_SLOT_MAX).map(|s| NetSlot::new(s).expect("in range")).collect();
        assert_eq!(
            smallest_free_slot(&held),
            Err(NetSlotExhausted { capacity: u32::from(NET_SLOT_MAX) + 1 }),
            "a fully-held slot space returns the typed exhaustion error, never a (reused) slot"
        );
    }

    // PROPERTY — for ANY held set with at least one free slot, the returned
    // slot is (a) NOT already held and (b) the minimum free value (no smaller
    // free slot exists). Generalises the gap example over the whole input
    // space; kills off-by-one and "return any free" mutants.
    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn smallest_free_slot_returns_an_unheld_minimum(
            // A small held set drawn from a small universe so a free slot
            // (and a low gap) is reliably present. `0..=20` keeps the search
            // cheap while still exercising gap-filling.
            held_raw in prop::collection::btree_set(0u16..=20, 0..=18)
        ) {
            let held: BTreeSet<NetSlot> =
                held_raw.iter().map(|&s| NetSlot::new(s).expect("in range")).collect();

            let chosen = smallest_free_slot(&held).expect("0..=20 minus <=18 entries has a free slot");

            prop_assert!(!held.contains(&chosen), "the chosen slot must not already be held: {chosen}");
            // No smaller slot is free: every value strictly below the chosen
            // one must be held (else the chosen one was not the minimum).
            // Compare via NetSlot's Ord rather than the inner u16 (no public
            // accessor is added for the inner value).
            for lower in 0u16..=20 {
                let lower_slot = NetSlot::new(lower).expect("in range");
                if lower_slot < chosen {
                    prop_assert!(
                        held.contains(&lower_slot),
                        "every slot below the chosen one must be held; {lower_slot} was free yet {chosen} was chosen"
                    );
                }
            }
        }
    }

    /// WRAPPER — a fresh allocator assigns slot 0 to the first alloc and slot 1
    /// to a second distinct alloc (smallest-free, ascending). The canonical
    /// assign-two-distinct case; kills a "always return 0" mutant on the second
    /// assign.
    #[test]
    fn allocator_assigns_ascending_smallest_free_to_distinct_allocs() {
        let allocator = NetSlotAllocator::new();
        let first = allocator.assign(alloc("alloc-aaa-0")).expect("first assign succeeds");
        let second = allocator.assign(alloc("alloc-bbb-0")).expect("second assign succeeds");

        assert_eq!(first, NetSlot::new(0).expect("0 in range"), "first alloc gets slot 0");
        assert_eq!(
            second,
            NetSlot::new(1).expect("1 in range"),
            "second distinct alloc gets slot 1"
        );
        assert_ne!(
            first, second,
            "distinct live allocs never share a slot (the B1 collision the model prevents)"
        );
    }

    /// WRAPPER — re-assigning an already-held alloc returns its EXISTING slot
    /// (idempotent re-entry of `on_alloc_running`, criterion 2), and does NOT
    /// consume a second slot — a subsequent distinct alloc still gets slot 1.
    /// Kills a "re-assign allocates a fresh slot" mutant.
    #[test]
    fn allocator_re_assign_of_held_alloc_returns_existing_slot_idempotently() {
        let allocator = NetSlotAllocator::new();
        let a = alloc("alloc-aaa-0");
        let first = allocator.assign(a.clone()).expect("first assign");
        let again = allocator.assign(a).expect("re-assign of held alloc");

        assert_eq!(
            first, again,
            "re-assigning a held alloc returns its existing slot (idempotent)"
        );
        // The re-assign must NOT have consumed slot 1 — a fresh distinct alloc
        // claims slot 1, proving the re-assign was a no-op claim.
        let other = allocator.assign(alloc("alloc-bbb-0")).expect("distinct assign");
        assert_eq!(
            other,
            NetSlot::new(1).expect("1 in range"),
            "idempotent re-assign consumed no slot — the next distinct alloc still gets slot 1"
        );
    }

    /// WRAPPER — releasing a held alloc frees its slot for the smallest-free
    /// scan: after releasing the slot-0 holder, a NEW alloc re-uses slot 0
    /// (smallest-free, not next-monotonic). Kills a "release is a no-op" mutant
    /// (which would push the new alloc to slot 2).
    #[test]
    fn allocator_release_frees_the_slot_for_reuse() {
        let allocator = NetSlotAllocator::new();
        let a = alloc("alloc-aaa-0");
        let b = alloc("alloc-bbb-0");
        let zero = allocator.assign(a.clone()).expect("a gets 0");
        let one = allocator.assign(b).expect("b gets 1");
        assert_eq!(zero, NetSlot::new(0).expect("0"), "a holds slot 0");
        assert_eq!(one, NetSlot::new(1).expect("1"), "b holds slot 1");

        allocator.release(&a);

        let reused = allocator.assign(alloc("alloc-ccc-0")).expect("c assign after release");
        assert_eq!(
            reused,
            NetSlot::new(0).expect("0 in range"),
            "after releasing slot 0, a new alloc re-uses the freed slot 0 (smallest-free)"
        );
    }

    /// WRAPPER — release of an UNHELD alloc is a no-op (idempotent teardown,
    /// criterion 2): it does not disturb the held set. Double-release (release
    /// the same alloc twice) is likewise benign. Kills a "release of unheld
    /// panics / clears the map" mutant.
    #[test]
    fn allocator_release_of_unheld_and_double_release_are_noops() {
        let allocator = NetSlotAllocator::new();
        let a = alloc("alloc-aaa-0");
        let held_slot = allocator.assign(a.clone()).expect("a gets a slot");

        // Releasing an alloc that was never held must not disturb the held set:
        // `a` still holds its slot, so re-assigning `a` returns the SAME slot.
        allocator.release(&alloc("alloc-never-held-0"));
        assert_eq!(
            allocator.assign(a.clone()).expect("a still held"),
            held_slot,
            "release of an unheld alloc is a no-op — the held alloc keeps its slot"
        );

        // Double-release of `a`: the first frees it, the second is a benign
        // no-op (not a panic). After both, slot 0 is free again.
        allocator.release(&a);
        allocator.release(&a);
        assert_eq!(
            allocator.assign(alloc("alloc-bbb-0")).expect("assign after double-release"),
            NetSlot::new(0).expect("0 in range"),
            "double-release is benign; the slot is freed exactly once and re-usable"
        );
    }

    /// WRAPPER — when every slot `0..=NET_SLOT_MAX` is held, `assign` for a NEW
    /// alloc returns the typed exhaustion error (criterion 4: refuse the alloc,
    /// no panic, no silent reuse of a held slot). Modelled by pre-filling the
    /// allocator's whole capacity, then asserting one more distinct alloc is
    /// refused — AND that an already-held alloc still re-assigns idempotently
    /// even at capacity (re-entry must not be starved by exhaustion).
    #[test]
    fn allocator_assign_at_capacity_refuses_new_alloc_with_typed_error() {
        let allocator = NetSlotAllocator::new();
        // Fill the entire slot space with distinct allocs.
        for slot in 0..=NET_SLOT_MAX {
            let assigned = allocator
                .assign(alloc(&format!("alloc-fill-{slot}")))
                .expect("each fill assign within capacity succeeds");
            assert_eq!(
                assigned,
                NetSlot::new(slot).expect("in range"),
                "the free-list assigns ascending slots while filling"
            );
        }

        // A NEW distinct alloc at capacity is refused with the typed error —
        // NOT a panic, NOT a reused (collision) slot.
        assert_eq!(
            allocator.assign(alloc("alloc-overflow-0")),
            Err(NetSlotExhausted { capacity: u32::from(NET_SLOT_MAX) + 1 }),
            "a new alloc at full capacity is refused with NetSlotExhausted, never a reused slot"
        );

        // An ALREADY-HELD alloc still re-assigns at capacity (idempotent
        // re-entry returns its existing slot — exhaustion gates only NEW claims).
        assert_eq!(
            allocator.assign(alloc("alloc-fill-0")).expect("held alloc re-assigns at capacity"),
            NetSlot::new(0).expect("0 in range"),
            "an already-held alloc re-assigns idempotently even when the space is full"
        );
    }

    /// WRAPPER — `snapshot` reflects the live `alloc → slot` bindings (the
    /// restart-rebuild / status read surface, criterion 6). After two assigns
    /// and one release, the snapshot contains exactly the still-held alloc with
    /// its slot. Kills a "snapshot -> BTreeMap::new()" mutant (which would make
    /// the restart rebuild see nothing held).
    #[test]
    fn snapshot_reflects_the_live_alloc_to_slot_bindings() {
        let allocator = NetSlotAllocator::new();
        let a = alloc("alloc-aaa-0");
        let b = alloc("alloc-bbb-0");
        let a_slot = allocator.assign(a.clone()).expect("a assign");
        let _b_slot = allocator.assign(b.clone()).expect("b assign");
        allocator.release(&b);

        let snap = allocator.snapshot();
        assert_eq!(
            snap.get(&a).copied(),
            Some(a_slot),
            "snapshot reflects the still-held alloc's slot binding (not an empty map)"
        );
        assert!(!snap.contains_key(&b), "a released alloc is absent from the snapshot");
        assert_eq!(snap.len(), 1, "snapshot contains exactly the one still-held alloc");
    }

    // --- adopt-on-restart §2 (NetSlotAllocator::adopt) -------------------
    //
    // Test budget: 4 distinct behaviors × 2 = 8. Behaviors:
    //   B1 adopt a free slot → bound;  B2 idempotent re-adopt (alloc,slot) → Ok;
    //   B3 conflict on a slot held by a DIFFERENT alloc → typed error;
    //   B4 pure planner: owner Some→adopt / None→GC.
    // Parametrized PBT covers the input variations of each (Mandate 5).

    /// B1: adopting a FREE slot records the `(alloc, slot)` binding, and a
    /// subsequent `assign` for a NEW alloc cannot be handed that slot (the
    /// whole point of adopt — close the cross-restart B1 collision).
    #[test]
    fn adopt_claims_a_free_slot_so_a_later_assign_cannot_collide() {
        let allocator = NetSlotAllocator::new();
        let survivor = alloc("alloc-survivor-0");
        allocator.adopt(survivor.clone(), slot(0)).expect("adopt free slot 0");
        assert_eq!(
            allocator.snapshot().get(&survivor).copied(),
            Some(slot(0)),
            "adopt must record the (survivor, slot 0) binding"
        );
        // The very next smallest-free assign must skip slot 0 (now held).
        let fresh = allocator.assign(alloc("alloc-fresh-0")).expect("fresh assign");
        assert_ne!(fresh, slot(0), "a fresh assign must not collide with the adopted slot 0");
    }

    /// B2: re-adopting the SAME `(alloc, slot)` is an idempotent no-op success
    /// (a re-run of the recovery pass must not error or double-bind).
    #[test]
    fn adopt_is_idempotent_for_the_same_alloc_and_slot() {
        let allocator = NetSlotAllocator::new();
        let a = alloc("alloc-aaa-0");
        allocator.adopt(a.clone(), slot(5)).expect("first adopt");
        allocator.adopt(a.clone(), slot(5)).expect("idempotent re-adopt is Ok");
        let snap = allocator.snapshot();
        assert_eq!(snap.get(&a).copied(), Some(slot(5)), "binding unchanged after re-adopt");
        assert_eq!(snap.len(), 1, "idempotent re-adopt must not double-bind");
    }

    /// B3: adopting a slot ALREADY held by a DIFFERENT alloc returns the typed
    /// `NetSlotAdoptConflict { slot, held_by, requested_by }` and does NOT
    /// overwrite the existing binding (fatal correlation bug → refuse).
    #[test]
    fn adopt_conflicts_when_slot_held_by_a_different_alloc() {
        let allocator = NetSlotAllocator::new();
        let first = alloc("alloc-first-0");
        let second = alloc("alloc-second-0");
        allocator.adopt(first.clone(), slot(3)).expect("first adopt");
        let err = allocator.adopt(second.clone(), slot(3)).expect_err("conflict expected");
        assert_eq!(
            err,
            NetSlotAdoptConflict {
                slot: slot(3),
                held_by: first.clone(),
                requested_by: second.clone(),
            },
            "the conflict must name the contested slot, the holder, and the requester"
        );
        // The existing binding is untouched; the conflicting alloc holds nothing.
        let snap = allocator.snapshot();
        assert_eq!(snap.get(&first).copied(), Some(slot(3)), "holder's binding preserved");
        assert!(!snap.contains_key(&second), "conflicting alloc must not be bound");
    }

    // B3 (input variation, parametrized via proptest): for ANY two distinct
    // allocs and ANY in-range slot, the second adopt of the same slot
    // conflicts and the holder's binding survives.
    proptest! {
        #[test]
        fn adopt_conflict_holds_for_any_distinct_allocs_and_slot(raw in 0u16..=NET_SLOT_MAX) {
            let allocator = NetSlotAllocator::new();
            let s = NetSlot::new(raw).expect("in range");
            let holder = alloc("alloc-holder-0");
            let other = alloc("alloc-other-0");
            allocator.adopt(holder.clone(), s).expect("holder adopt");
            let err = allocator.adopt(other.clone(), s).expect_err("must conflict");
            prop_assert_eq!(err.slot, s);
            prop_assert_eq!(&err.held_by, &holder);
            prop_assert_eq!(err.requested_by, other);
            prop_assert_eq!(allocator.snapshot().get(&holder).copied(), Some(s));
        }
    }

    /// B4: the PURE adopt-vs-GC planner maps each owned observation to an
    /// ADOPT and each orphan (owner None) to a GC, preserving input order.
    #[test]
    fn plan_adopt_actions_splits_owned_from_orphan() {
        let owned = ObservedAdoptNetns { slot: slot(2), owner: Some(alloc("alloc-owned-0")) };
        let orphan = ObservedAdoptNetns { slot: slot(4), owner: None };
        let owned2 = ObservedAdoptNetns { slot: slot(6), owner: Some(alloc("alloc-owned2-0")) };
        let plan = plan_adopt_actions(&[owned, orphan, owned2]);
        assert_eq!(
            plan,
            AdoptPlan {
                adopt: vec![(alloc("alloc-owned-0"), slot(2)), (alloc("alloc-owned2-0"), slot(6)),],
                gc: vec![slot(4)],
            },
            "owned → adopt (in order), orphan → gc"
        );
    }

    /// B4 (boundary): an EMPTY observation yields an empty plan (a fresh boot
    /// with no surviving netns is a no-op, never a spurious adopt or GC).
    #[test]
    fn plan_adopt_actions_on_empty_is_empty() {
        let plan = plan_adopt_actions(&[]);
        assert_eq!(plan, AdoptPlan::default(), "no survivors → no adopt, no gc");
    }

    /// The orphan-GC observe path classifies a `NotFound` io error as the
    /// BENIGN "the thing is genuinely absent" signal (legitimately no live
    /// PID / scope reaped → orphan-eligible), and EVERY other io error
    /// (EACCES, EIO, transient, …) as a genuine read failure that MUST
    /// propagate (refuse the boot, fail-closed) rather than silently degrade
    /// into the destructive `ip netns del` branch. This pins the
    /// classification the three swallow-site fixes rely on — without it a
    /// swallowed read could tear down a LIVE workload's netns (the
    /// `development.md` "never absorb a Result into a default" rule applied to
    /// a DESTRUCTIVE action). Pure → no real fs needed.
    #[test]
    fn io_error_is_benign_absence_only_classifies_not_found_as_benign() {
        use std::io::{Error, ErrorKind};

        // The ONE benign kind: the resource is genuinely gone.
        assert!(
            io_error_is_benign_absence(&Error::from(ErrorKind::NotFound)),
            "NotFound is the benign absence signal → orphan-eligible / skip-this-PID"
        );

        // Every genuine read failure MUST be classified non-benign so the
        // observe path propagates it and refuses the boot.
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
            ErrorKind::InvalidData,
            ErrorKind::ConnectionRefused,
            ErrorKind::WouldBlock,
            ErrorKind::TimedOut,
            ErrorKind::Interrupted,
        ] {
            assert!(
                !io_error_is_benign_absence(&Error::from(kind)),
                "{kind:?} is a genuine read failure → must propagate (fail-closed), not benign"
            );
        }

        // An OS-code-shaped EIO (raw_os_error path, not a from-kind synthetic)
        // is also non-benign — the read failed for a real reason.
        assert!(
            !io_error_is_benign_absence(&Error::from_raw_os_error(libc::EIO)),
            "EIO (a genuine I/O failure) is non-benign → must propagate"
        );
    }
}
