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
//! Single-node runs entirely in the host netns — there is no netns
//! machinery here (no `ip netns add`, no `ip link set <if> netns <ns>`).
//! `CAP_NET_ADMIN` is already a precondition of serve boot (XDP attach +
//! cgroup delegation), so provisioning adds no new privilege.

use ipnet::Ipv4Net;
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
        ObservedVeth, VethProvisionPlan, VethStep, converge_steps, derive_veth_plan, link_absent,
        tx_checksumming_on, tx_offload_benign,
    };
    use ipnet::{IpAdd, Ipv4Net};
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

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
}
