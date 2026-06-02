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
//! - [`provision`] — the real `ip(8)` shell-out (`#[cfg(target_os =
//!   "linux")]` production). Idempotent detect-and-reuse per ADR-0061
//!   § 3.1: `ip link show <cli>` FIRST; if the pair pre-exists (created
//!   by a prior serve boot, an OS image, or a Lima/Yocto provisioner)
//!   ADOPT it untouched. Only create the pair + assign addresses + bring
//!   both up + add the route when the pair is ABSENT. Never tear down
//!   (DQ-4 leave-and-reuse).
//!
//! Single-node runs entirely in the host netns — there is no netns
//! machinery here (no `ip netns add`, no `ip link set <if> netns <ns>`).
//! `CAP_NET_ADMIN` is already a precondition of serve boot (XDP attach +
//! cgroup delegation), so provisioning adds no new privilege.

use ipnet::Ipv4Net;
use std::net::Ipv4Addr;

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

/// Provision the single-node veth pair in the host netns from `plan`.
///
/// **Idempotent detect-and-reuse** (ADR-0061 § 3.1): runs
/// `ip link show <client_iface>` FIRST. If the pair already exists
/// (created by a prior serve boot, an OS image, or a Lima/Yocto
/// provisioner) it is ADOPTED untouched — no recreate, no failure, a
/// pre-existing route is left in place. Only when the pair is ABSENT
/// does it create the pair, assign the gateway address(es), bring both
/// ends up, and add the on-link route. Never tears down (DQ-4
/// leave-and-reuse).
///
/// Synchronous (`std::process::Command`) — provisioning is a boot-time
/// one-shot, so the sync shape (matching `ThreeIfaceTopology::create`)
/// is simplest and avoids dragging the `ip` shell-out into an `async fn`
/// (which the dst-lint async-fs gate would otherwise scrutinise).
///
/// # Errors
///
/// Returns a distinct [`VethProvisionError`] variant per failing `ip(8)`
/// step (link-show, link-add, addr-add, link-up, route-add) so the
/// caller can branch on which boot step failed.
#[cfg(target_os = "linux")]
pub fn provision(plan: &VethProvisionPlan) -> Result<(), VethProvisionError> {
    use std::process::Command;

    // Detect: does the client-side veth already exist? `ip link show`
    // exits 0 when present, non-zero ("does not exist") when absent.
    let show = Command::new("ip").args(["link", "show", &plan.client_iface]).output()?;
    if show.status.success() {
        // Adopt the pre-existing pair untouched — no recreate, no
        // re-address, leave any pre-existing route in place.
        return Ok(());
    }
    // `ip link show` returns exit 1 with "does not exist" on stderr when
    // the iface is absent — the normal create path. Any other failure
    // (e.g. EPERM) is a genuine error.
    let show_stderr = String::from_utf8_lossy(&show.stderr);
    if !show_stderr.contains("does not exist") {
        return Err(VethProvisionError::LinkShowFailed {
            iface: plan.client_iface.clone(),
            stderr: show_stderr.trim().to_owned(),
            status: show.status.code(),
        });
    }

    // Absent — create the pair in the host netns (no `netns` flags).
    let add = Command::new("ip")
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
    if !add.status.success() {
        return Err(VethProvisionError::LinkAddFailed {
            client_iface: plan.client_iface.clone(),
            backend_iface: plan.backend_iface.clone(),
            stderr: String::from_utf8_lossy(&add.stderr).trim().to_owned(),
            status: add.status.code(),
        });
    }

    // Assign the client-side gateway address + bring both ends up.
    let client_cidr = format!("{}/{}", plan.client_gateway, plan.route_cidr.prefix_len());
    addr_add(&plan.client_iface, &client_cidr)?;
    if let Some(backend_gw) = plan.backend_gateway {
        let backend_cidr = format!("{}/{}", backend_gw, plan.route_cidr.prefix_len());
        addr_add(&plan.backend_iface, &backend_cidr)?;
    }
    link_up(&plan.client_iface)?;
    link_up(&plan.backend_iface)?;

    // On-link route: <vip_range> dev <client_iface>. Idempotent — a
    // pre-existing route for this CIDR is left in place (ADR-0061
    // § 3.1). Assigning the client/backend gateway address(es) above
    // also auto-creates a kernel `proto kernel scope link` connected
    // route for the same /N, so `ip route add` here can legitimately
    // collide with `File exists`; that is the "already reachable" case,
    // not a failure.
    let route_cidr = plan.route_cidr.to_string();
    let route = Command::new("ip")
        .args(["route", "add", &route_cidr, "dev", &plan.client_iface])
        .output()?;
    if !route.status.success() {
        let stderr = String::from_utf8_lossy(&route.stderr);
        if !stderr.contains("File exists") {
            return Err(VethProvisionError::RouteAddFailed {
                cidr: route_cidr,
                iface: plan.client_iface.clone(),
                stderr: stderr.trim().to_owned(),
                status: route.status.code(),
            });
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn addr_add(iface: &str, cidr: &str) -> Result<(), VethProvisionError> {
    let out =
        std::process::Command::new("ip").args(["addr", "add", cidr, "dev", iface]).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(VethProvisionError::AddrAddFailed {
        iface: iface.to_owned(),
        cidr: cidr.to_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        status: out.status.code(),
    })
}

#[cfg(target_os = "linux")]
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "test code: expect is the canonical assertion pattern")]
mod tests {
    use super::{VethProvisionPlan, derive_veth_plan};
    use ipnet::{IpAdd, Ipv4Net};
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

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
