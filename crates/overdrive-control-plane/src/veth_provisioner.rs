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
//! - [`provision`] — the real typed rtnetlink / ethtool kernel adapter.
//!   **Idempotent converge-on-boot** per ADR-0061
//!   § 3.1: OBSERVE actual kernel state, compute [`converge_steps`], then
//!   EXECUTE each step idempotently (swallowing `EEXIST` / `File exists`
//!   on address/route add). A complete pair converges to all-noop; a
//!   half-provisioned pair (the crash-mid-provision case) is COMPLETED in
//!   place; a corrupted pair (client present, peer absent — § 3.2) is
//!   RECREATED. Never tears down a usable pair (DQ-4 leave-and-reuse).
//!
//! `CAP_NET_ADMIN` is already a precondition of serve boot (XDP attach +
//! cgroup delegation), so this path adds no new privilege.

use ipnet::Ipv4Net;
use overdrive_netlink::error::{NEG_EEXIST, NEG_ENODEV};
use overdrive_netlink::{Client, NetlinkError, ethtool};
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

/// Distinct failure modes of [`provision`]. One variant per provisioning
/// step per `.claude/rules/development.md` § Errors — never collapse to a
/// single `String` variant, so a caller can branch on which step failed
/// (and the operator gets a cause-specific message).
///
/// Every per-site variant embeds the errno-carrying [`NetlinkError`] via
/// `#[source]` and carries the typed `errno` (ADR-0085 D3), so each operation
/// applies its own postcondition-specific rule (`-EEXIST` only after an exact
/// add postcondition is observed; `-ENODEV` only for delete), not a broad
/// classifier or stderr substring. There is no blanket `#[from] io::Error`.
#[derive(Debug, thiserror::Error)]
pub enum VethProvisionError {
    /// The `RTM_GETLINK` presence/up-state read failed for a reason that is
    /// neither "present" nor absent (`-ENODEV`) — e.g. `EPERM`.
    #[error("link show `{iface}` failed (errno={errno:?}): {source}")]
    LinkShowFailed {
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// `link add <cli> type veth peer name <bk>` failed.
    #[error(
        "link add `{client_iface}` type veth peer name `{backend_iface}` failed (errno={errno:?}): {source}"
    )]
    LinkAddFailed {
        client_iface: String,
        backend_iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// `addr add <cidr> dev <iface>` failed.
    #[error("addr add `{cidr}` dev `{iface}` failed (errno={errno:?}): {source}")]
    AddrAddFailed {
        iface: String,
        cidr: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// `link del <iface>` failed (the § 3.2 RecreatePair teardown of a
    /// corrupted, Overdrive-owned half-pair). An "absent" failure is
    /// benign (already gone) and is swallowed before this surfaces.
    #[error("link del `{iface}` failed (errno={errno:?}): {source}")]
    LinkDelFailed {
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// `link set <iface> up` failed.
    #[error("link set `{iface}` up failed (errno={errno:?}): {source}")]
    LinkUpFailed {
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// `route add <cidr> dev <iface>` failed.
    #[error("route add `{cidr}` dev `{iface}` failed (errno={errno:?}): {source}")]
    RouteAddFailed {
        cidr: String,
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// TX-checksum-offload disable failed for a non-benign reason. A veth
    /// with no changeable `tx-checksum-*` feature already delivers a FULL
    /// checksum, so the encoder skips it (no error); a genuine failure
    /// (`EPERM`, a genl-family/socket failure) is fatal — booting with TX
    /// offload still ON would corrupt every NAT'd packet (commit 62fa6be2),
    /// so refuse to boot rather than silently ship the landmine.
    #[error("tx-offload disable on `{iface}` failed (errno={errno:?}): {source}")]
    TxOffloadDisableFailed {
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// The ethtool `FEATURES_GET` READ of a PRESENT iface's TX-checksum-offload
    /// state failed for a non-benign reason (a genl socket / family-resolution
    /// failure, `EPERM`, or a kernel `NLMSG_ERROR`). The observer cannot tell
    /// whether offload is ON, and swallowing the failure to `false` would omit
    /// the needed `Disable*TxOffload` step and boot with offload still ON —
    /// corrupting every NAT'd packet (commit 62fa6be2). A read failure on a
    /// present iface is therefore FATAL (refuse the boot), exactly as
    /// [`Self::TxOffloadDisableFailed`] is — never absorbed into a default, per
    /// `.claude/rules/development.md` § Errors. An ABSENT iface is not read at
    /// all (the caller gates on presence; the recreate path re-disables on a
    /// fresh pair), so this surfaces only a genuine read-of-a-present-iface
    /// failure — never a benign absence.
    #[error("tx-offload read on `{iface}` failed (errno={errno:?}): {source}")]
    TxOffloadReadFailed {
        iface: String,
        errno: Option<i32>,
        #[source]
        source: NetlinkError,
    },
    /// Opening the netlink socket / spawning the rtnetlink connection failed
    /// (the discrete replacement for the obsolete blanket `Spawn(#[from]
    /// io::Error)` — ADR-0085 D3).
    #[error("opening the netlink connection failed: {source}")]
    NetlinkConnect {
        #[source]
        source: NetlinkError,
    },
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

/// Per-host base block of the mesh workload address space. The mesh-membership
/// gate (`addr ∈ WORKLOAD_SUBNET_BASE`) in the service-map hydrator and the
/// bridge keys on it.
///
/// Fixed for Phase-1 single-node; making it operator-configurable is tracked
/// in <https://github.com/overdrive-sh/overdrive/issues/239> (do NOT make it
/// tunable here). `Ipv4Net::new_assert` is `const` in `ipnet` 2.x, so the
/// base is a compile-time constant; the `/16` prefix is statically valid.
pub const WORKLOAD_SUBNET_BASE: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(10, 99, 0, 0), 16);

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

/// A single idempotent convergence action the executor applies through
/// rtnetlink / ethtool. The ordered `Vec<VethStep>` from [`converge_steps`] is the
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
/// `async` (ADR-0085 D5) — the executor/observer drive netlink over
/// [`overdrive_netlink::Client`] and the hand-rolled ethtool genl encoder,
/// not `ip`/`ethtool` subprocesses. The sole call site
/// (`run_server_with_obs_and_driver`) is already `async`, so it `.await`s
/// here with no `spawn_blocking`.
///
/// # Errors
///
/// [`VethProvisionError::NetlinkConnect`] when the netlink socket cannot be
/// opened, otherwise a distinct per-step variant (link-show / link-add /
/// link-del / addr-add / link-up / route-add / tx-offload) carrying the
/// typed netlink `errno`. `-EEXIST` is accepted for address/route add only
/// after observing the exact desired postcondition; `-ENODEV` is accepted only
/// for delete and is fatal for create/add/up/move — never a stderr substring.
pub async fn provision(plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    let client = Client::new().map_err(|source| VethProvisionError::NetlinkConnect { source })?;
    let observed = observe(&client, plan).await?;
    for step in converge_steps(plan, &observed) {
        execute_step(&client, plan, step).await?;
    }
    Ok(())
}

/// Read the actual kernel state of the pair into an [`ObservedVeth`].
///
/// Presence + up-state come from a structured `RTM_GETLINK` (present +
/// `LinkFlags::Up`, or absent via `-ENODEV`); address presence comes from
/// [`crate::iface::resolve_iface_ipv4`] matching the desired gateway —
/// the same `getifaddrs(3)` walk the downstream boot path uses, so the
/// observer and the consumer agree on what "address present" means;
/// TX-offload state comes from the ethtool `FEATURES_GET` bitset.
async fn observe(
    client: &Client,
    plan: &VethProvisionPlan,
) -> Result<ObservedVeth, VethProvisionError> {
    let (client_present, client_up) = observe_link(client, &plan.client_iface).await?;
    let (peer_present, backend_up) = observe_link(client, &plan.backend_iface).await?;

    let client_addr_present =
        client_present && iface_has_addr(&plan.client_iface, plan.client_gateway);
    // No backend gateway derived (e.g. /31) → the address is "present" by
    // vacuous truth so converge never emits AddBackendAddr.
    let backend_addr_present = plan
        .backend_gateway
        .is_none_or(|gw| peer_present && iface_has_addr(&plan.backend_iface, gw));

    // TX-offload is only meaningful for a PRESENT iface; an absent iface is
    // not read at all (when absent the pair is (re)created, and the `recreated`
    // path in converge_steps re-emits the disable on the fresh pair regardless).
    // For a present iface a `FEATURES_GET` failure PROPAGATES (fatal) — it is
    // NOT swallowed to `false`: a `false` guess on an offload-ON iface would
    // omit the needed disable and boot with the packet-corrupting state live
    // (commit 62fa6be2). Refusing the boot is the same discipline the disable
    // step follows (`.claude/rules/development.md` § Errors: never absorb a
    // boundary read into a default).
    let client_tx_offload_on =
        if client_present { observe_tx_offload_on(&plan.client_iface).await? } else { false };
    let backend_tx_offload_on =
        if peer_present { observe_tx_offload_on(&plan.backend_iface).await? } else { false };

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

/// Structured `RTM_GETLINK` presence + up-state read for the host-netns
/// path: `(present, up)`. Absent (`-ENODEV`) → `(false, false)`; any other
/// failure (e.g. `EPERM`) → [`VethProvisionError::LinkShowFailed`] carrying
/// the typed errno.
async fn observe_link(client: &Client, iface: &str) -> Result<(bool, bool), VethProvisionError> {
    match client.observe_link(iface).await {
        Ok(Some(up)) => Ok((true, up)),
        Ok(None) => Ok((false, false)),
        Err(source) => Err(VethProvisionError::LinkShowFailed {
            iface: iface.to_owned(),
            errno: source.errno(),
            source,
        }),
    }
}

/// Whether `iface` still has TX-checksum-offload ON, read from the ethtool
/// `FEATURES_GET` bitset (any changeable `tx-checksum-*` bit active).
///
/// A genl read FAILURE propagates as [`VethProvisionError::TxOffloadReadFailed`]
/// — it is NOT swallowed to `false`. Guessing `false` on a read failure would
/// omit the needed `DisableTxOffload` step for an offload-ON iface and boot the
/// packet-corrupting state live (commit 62fa6be2); a read failure on a present
/// iface is therefore fatal, exactly as the disable step is. The caller invokes
/// this only for a PRESENT iface — an absent one is (re)created, which
/// re-disables regardless — so the propagated error is never a benign absence.
// mutants: skip — impure genl I/O shim; the `Ok(true)`/`Ok(false)` body mutants
// are killable only by the Tier-3 real-kernel drift-repair path
// (`provision_repairs_tx_offload_drifted_back_on`), not the unit lane. The
// pure, unit-KILLABLE derivation lives in
// `overdrive_netlink::ethtool::{changeable_tx_checksum_targets,
// any_tx_checksum_active}` (unit-tested there).
async fn observe_tx_offload_on(iface: &str) -> Result<bool, VethProvisionError> {
    ethtool::tx_offload_on(iface).await.map_err(|source| VethProvisionError::TxOffloadReadFailed {
        iface: iface.to_owned(),
        errno: source.errno(),
        source,
    })
}

/// True when `iface` carries `want` as a bound IPv4 address. Reuses the
/// production `getifaddrs(3)` walk so observer and consumer agree.
fn iface_has_addr(iface: &str, want: Ipv4Addr) -> bool {
    crate::iface::resolve_iface_ipv4(iface).is_ok_and(|got| got == want)
}

/// Operation-specific idempotency for create/add: only "already exists" is
/// success. A vanished dependency (`-ENODEV`) must fail the convergence pass
/// so the caller never reports `Ok` with the desired resource absent.
const fn errno_is_already_exists(errno: Option<i32>) -> bool {
    matches!(errno, Some(code) if code == NEG_EEXIST)
}

/// Operation-specific idempotency for delete: only "already absent" is
/// success. Kept separate from create/add so the two opposite postconditions
/// cannot be collapsed into one broad errno classifier.
const fn errno_is_absent(errno: Option<i32>) -> bool {
    matches!(errno, Some(code) if code == NEG_ENODEV)
}

/// Apply a single [`VethStep`] via netlink (ADR-0085 D1). Delete alone accepts
/// typed `-ENODEV`; add accepts `-EEXIST` only after observing the exact
/// desired address/route; link-up errors are fatal.
async fn execute_step(
    client: &Client,
    plan: &VethProvisionPlan,
    step: VethStep,
) -> Result<(), VethProvisionError> {
    match step {
        VethStep::RecreatePair => {
            nl_del_link(client, &plan.client_iface).await?;
            // Also reap the backend end. For the forward corrupted edge
            // (client present, peer absent) deleting the client reaps both,
            // so this is a no-op. For the inverse edge (client absent, peer
            // present) the client del is the no-op and THIS reaps the
            // surviving/colliding peer — without it, the veth-pair add would
            // hit `-EEXIST` on the peer name.
            nl_del_link(client, &plan.backend_iface).await?;
            nl_add_pair(client, plan).await
        }
        VethStep::CreatePair => nl_add_pair(client, plan).await,
        VethStep::AddClientAddr => {
            nl_add_addr(
                client,
                &plan.client_iface,
                plan.client_gateway,
                plan.route_cidr.prefix_len(),
            )
            .await
        }
        VethStep::AddBackendAddr => {
            // Only emitted when backend_gateway is Some — unreachable
            // otherwise per converge_steps.
            let gateway = plan.backend_gateway.unwrap_or_else(|| {
                unreachable!("AddBackendAddr emitted only when backend_gateway is Some")
            });
            nl_add_addr(client, &plan.backend_iface, gateway, plan.route_cidr.prefix_len()).await
        }
        VethStep::SetClientUp => nl_set_up(client, &plan.client_iface).await,
        VethStep::SetBackendUp => nl_set_up(client, &plan.backend_iface).await,
        VethStep::DisableClientTxOffload => nl_tx_off(&plan.client_iface).await,
        VethStep::DisableBackendTxOffload => nl_tx_off(&plan.backend_iface).await,
        VethStep::AddRoute => nl_add_route(client, plan).await,
    }
}

/// `link add <client> type veth peer name <backend>` (atomic pair creation)
/// over netlink.
async fn nl_add_pair(client: &Client, plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    client.add_veth_pair(&plan.client_iface, &plan.backend_iface).await.map_err(|source| {
        VethProvisionError::LinkAddFailed {
            client_iface: plan.client_iface.clone(),
            backend_iface: plan.backend_iface.clone(),
            errno: source.errno(),
            source,
        }
    })
}

/// `link del <iface>` over netlink. An absent link is a silent no-op inside
/// the client; a racing `-ENODEV` is swallowed via the typed errno.
async fn nl_del_link(client: &Client, iface: &str) -> Result<(), VethProvisionError> {
    match client.del_link(iface).await {
        Ok(()) => Ok(()),
        Err(err) if errno_is_absent(err.errno()) => Ok(()),
        Err(source) => Err(VethProvisionError::LinkDelFailed {
            iface: iface.to_owned(),
            errno: source.errno(),
            source,
        }),
    }
}

/// Converge `<addr>/<prefix> dev <iface>` over netlink. Wrong prefixes are
/// replaced; `-EEXIST` is accepted only after the exact address is observed;
/// `-ENODEV` is fatal.
async fn nl_add_addr(
    client: &Client,
    iface: &str,
    addr: Ipv4Addr,
    prefix: u8,
) -> Result<(), VethProvisionError> {
    match client.converge_addr(iface, addr, prefix).await {
        Ok(()) => Ok(()),
        Err(err)
            if errno_is_already_exists(err.errno())
                && matches!(client.observe_addr(iface, addr, prefix).await, Ok(true)) =>
        {
            Ok(())
        }
        Err(source) => Err(VethProvisionError::AddrAddFailed {
            iface: iface.to_owned(),
            cidr: format!("{addr}/{prefix}"),
            errno: source.errno(),
            source,
        }),
    }
}

/// `link set <iface> up` over netlink. Setting an existing link up is
/// kernel-idempotent; every returned error, including `-ENODEV`, is surfaced
/// fail-closed.
async fn nl_set_up(client: &Client, iface: &str) -> Result<(), VethProvisionError> {
    match client.set_link_up(iface).await {
        Ok(()) => Ok(()),
        Err(source) => Err(VethProvisionError::LinkUpFailed {
            iface: iface.to_owned(),
            errno: source.errno(),
            source,
        }),
    }
}

/// On-link route `<route_cidr> dev <client_iface>` over netlink. Assigning
/// the gateway address auto-creates a kernel connected route for the same
/// /N, so this legitimately collides with `-EEXIST` — the "already
/// reachable" case, swallowed via the typed errno (ADR-0061 § 3.1).
async fn nl_add_route(client: &Client, plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    match client
        .add_onlink_route(
            plan.route_cidr.network(),
            plan.route_cidr.prefix_len(),
            &plan.client_iface,
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(err)
            if errno_is_already_exists(err.errno())
                && matches!(
                    client
                        .observe_onlink_route(
                            plan.route_cidr.network(),
                            plan.route_cidr.prefix_len(),
                            &plan.client_iface,
                        )
                        .await,
                    Ok(true)
                ) =>
        {
            Ok(())
        }
        Err(source) => Err(VethProvisionError::RouteAddFailed {
            cidr: plan.route_cidr.to_string(),
            iface: plan.client_iface.clone(),
            errno: source.errno(),
            source,
        }),
    }
}

/// Disable TX-checksum-offload on `iface` via the hand-rolled ethtool
/// `FEATURES_SET` encoder. A veth with no changeable `tx-checksum-*` feature
/// already delivers a FULL checksum, so the encoder skips it (no error); a
/// genuine failure (`EPERM`, socket / genl-family failure) is FATAL and
/// refuses the boot — booting with offload still ON corrupts every NAT'd
/// packet (commit 62fa6be2).
async fn nl_tx_off(iface: &str) -> Result<(), VethProvisionError> {
    ethtool::disable_tx_offload(iface).await.map_err(|source| {
        VethProvisionError::TxOffloadDisableFailed {
            iface: iface.to_owned(),
            errno: source.errno(),
            source,
        }
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use super::{
        ObservedVeth, VethProvisionPlan, VethStep, converge_steps, derive_veth_plan,
        errno_is_absent, errno_is_already_exists,
    };
    use ipnet::{IpAdd, Ipv4Net};
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

    /// Create/add and delete have opposite successful postconditions, so their
    /// typed errno classifiers must never share `ENODEV`/`EEXIST` handling.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn operation_specific_errno_rules_never_report_disappeared_create_as_success() {
        assert!(errno_is_already_exists(Some(-libc::EEXIST)));
        assert!(!errno_is_already_exists(Some(-libc::ENODEV)));
        assert!(!errno_is_already_exists(None));

        assert!(errno_is_absent(Some(-libc::ENODEV)));
        assert!(!errno_is_absent(Some(-libc::EEXIST)));
        assert!(!errno_is_absent(None));
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
}

/// Regression guard for the "offload read failure suppresses repair" defect
/// (PR #274 review, greptile P1): a `FEATURES_GET` read failure MUST propagate
/// as [`VethProvisionError::TxOffloadReadFailed`] — it must NOT be swallowed to
/// `Ok(false)`. A `false` guess omits the needed `Disable*TxOffload` step in
/// [`converge_steps`] (which the pure tests above prove is emitted iff the
/// observed bit is `true`), booting a present, offload-ON pair with the
/// packet-corrupting checksum state live (commit 62fa6be2).
///
/// Real genl (`NETLINK_GENERIC`) I/O → gated behind `integration-tests` and run
/// under Lima. The read failure is forced with a NONEXISTENT iface — the
/// cheapest, unprivileged, deterministic way to make `FEATURES_GET` return an
/// `NLMSG_ERROR` (`-ENODEV`; empirically confirmed). Production's own trigger is
/// a PRESENT iface whose read fails for a different reason (`EPERM`, a genl
/// socket failure), but the shim treats every read failure identically
/// (propagate), so the ENODEV-forced failure exercises the same `.map_err`
/// branch. The shims gate on presence in production, so an absent iface never
/// reaches them there; this test calls them directly to reach the read-failure
/// branch.
#[cfg(all(test, feature = "integration-tests"))]
mod offload_read_failure_propagation {
    use super::{VethProvisionError, observe_tx_offload_on};

    /// A name that cannot resolve to a real iface, so `FEATURES_GET` fails
    /// (`-ENODEV`) rather than returning a feature bitset.
    const BOGUS_IFACE: &str = "ovd-no-such-if0";

    #[tokio::test]
    async fn host_netns_offload_read_failure_propagates_not_reported_off() {
        let result = observe_tx_offload_on(BOGUS_IFACE).await;
        assert!(
            matches!(&result, Err(VethProvisionError::TxOffloadReadFailed { .. })),
            "a FEATURES_GET read failure must propagate as TxOffloadReadFailed, never be \
             swallowed to Ok(false) (which would suppress the disable); got {result:?}",
        );
    }
}
