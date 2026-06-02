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
    reason = "six independent observed kernel facts (presence/addr/up × client/peer); \
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
}

/// A single idempotent convergence action the executor applies via
/// `ip(8)`. The ordered `Vec<VethStep>` from [`converge_steps`] is the
/// minimal set of steps that brings an [`ObservedVeth`] to the desired
/// complete shape. Ordering is load-bearing: the pair must exist before
/// addresses can be assigned, and (re)creating the pair subsumes every
/// downstream step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VethStep {
    /// Delete the orphaned client end and recreate the pair from scratch
    /// — the § 3.2 corrupted edge (client present, peer absent). Deleting
    /// one end reaps both; the recreate restores the atomic pair.
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
        (false, _) => {
            steps.push(VethStep::CreatePair);
            true
        }
        (true, false) => {
            // § 3.2: client present, declared peer absent — recreate.
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

    Ok(ObservedVeth {
        client_present,
        peer_present,
        client_addr_present,
        backend_addr_present,
        client_up,
        backend_up,
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

/// Apply a single [`VethStep`] via `ip(8)`. Idempotent: `EEXIST` /
/// `File exists` on address and route add is swallowed; `ip link set up`
/// is idempotent at the kernel.
fn execute_step(plan: &VethProvisionPlan, step: VethStep) -> Result<(), VethProvisionError> {
    match step {
        VethStep::RecreatePair => {
            link_del(&plan.client_iface)?;
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

/// `ip link del <iface>` — deletes the orphaned client end (which reaps
/// both ends of a veth pair). Used only by [`VethStep::RecreatePair`]
/// (§ 3.2). A "does not exist" failure is benign (already gone) and
/// swallowed; any other failure surfaces as
/// [`VethProvisionError::LinkDelFailed`].
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
    };
    use ipnet::{IpAdd, Ipv4Net};
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

    /// A complete (all-present, both-up) observation — the baseline the
    /// converge tests mutate one field at a time.
    fn complete_observed() -> ObservedVeth {
        ObservedVeth {
            client_present: true,
            peer_present: true,
            client_addr_present: true,
            backend_addr_present: true,
            client_up: true,
            backend_up: true,
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
                VethStep::AddRoute,
            ],
            "peer-absent corrupted pair must recreate then converge every downstream resource"
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
        /// (each of the four converge-relevant facts independently
        /// present/absent), `converge_steps`
        ///   (a) never emits Create/Recreate for a present pair with a
        ///       present peer;
        ///   (b) emits `AddClientAddr` iff the client addr is absent;
        ///   (c) emits `AddBackendAddr` iff the backend addr is absent
        ///       (the plan_24 backend gateway is always Some);
        ///   (d) emits `SetClientUp` / `SetBackendUp` iff the respective
        ///       end is down;
        ///   (e) always ends with `AddRoute`.
        /// This is the exhaustive desired-vs-actual invariant for the
        /// completion path — the regression class the old adopt-untouched
        /// branch violated for every absent sub-resource.
        #[test]
        fn converge_present_pair_emits_exactly_the_missing_resources(
            client_addr in any::<bool>(),
            backend_addr in any::<bool>(),
            client_up in any::<bool>(),
            backend_up in any::<bool>(),
        ) {
            let plan = plan_24();
            let observed = ObservedVeth {
                client_present: true,
                peer_present: true,
                client_addr_present: client_addr,
                backend_addr_present: backend_addr,
                client_up,
                backend_up,
            };
            let steps = converge_steps(&plan, &observed);

            prop_assert!(!steps.contains(&VethStep::CreatePair));
            prop_assert!(!steps.contains(&VethStep::RecreatePair));
            prop_assert_eq!(steps.contains(&VethStep::AddClientAddr), !client_addr);
            prop_assert_eq!(steps.contains(&VethStep::AddBackendAddr), !backend_addr);
            prop_assert_eq!(steps.contains(&VethStep::SetClientUp), !client_up);
            prop_assert_eq!(steps.contains(&VethStep::SetBackendUp), !backend_up);
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
