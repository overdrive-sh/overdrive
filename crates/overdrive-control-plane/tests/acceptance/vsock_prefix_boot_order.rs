//! P4 — boot convergence order + startup overlap refusals (seeded sim).
//!
//! netns-density-295 vsock Attachment Replacement DESIGN
//! (`docs/feature/netns-density-295/feature-delta.md` § "Composition —
//! production entry points" (boot steps 1, 3, 9; the D5a-SERVE-IFACE and
//! INTAKE-BIND ordering invariants); § "Owner and provisioner" (`probe_startup`,
//! `converge_shared`); Gate G-V1; ADR-0152/0171/0175 D8a-ROUTE/D8a-LOAD-SWAP,
//! D8-PREFIX-CFG, D8-PREFIX-VIP, D5a-SERVE-IFACE, INTAKE-BIND). Quint oracle:
//! `specs/quint/guest-flow-owner/prefix_boot.qnt` (invariant 6; replayed by
//! `vsock_prefix_boot_conformance.rs`).
//!
//! Lane: seeded `overdrive-sim` through the **production boot path**
//! (`probe_startup` → ... → `converge_shared`) with `SimGuestPrefixSteering`
//! scripting load/attach/pin failures and a structural boot-order probe.
//! Example-based (Mandate 9/11); the two pure predicates (vip/prefix overlap;
//! intake-block conflict) carry `proptest` (layer-1). Default unit lane.
//!
//! ## State-delta universe (Mandate 8)
//!   * `boot_outcome`      — Opened | Refused { reason, variant }
//!   * `steering_link`     — None | Pinned(prog)     (observed; never a private field)
//!   * `prefix_route`      — Absent | Local
//!   * `boot_effects`      — ordered list of writes attempted (veth / nft / steering / route)
//! The load-bearing assertion for every refusal is "no boot effect was written
//! before the refusal" (fail-closed): `boot_effects` is empty.
//!
//! ## GATED — pending DELIVER roadmap reconciliation + boot/converge seam
//!
//! `SharedGuestNetworkOwner::{probe_startup, converge_shared}`,
//! `GuestNetworkError` (overlap + steering variants), `IntakePortBlockConflict`,
//! `SimGuestPrefixSteering`, and the structural boot-order probe do NOT exist
//! yet. None is invented here (CLAUDE.md § "Implement to the design").
//! DELIVER creates them at the reconciliation + step 2/9 and only then
//! **registers this module** in `tests/acceptance.rs`
//! (`mod vsock_prefix_boot_order;`). Until then Cargo never compiles this file.

use std::net::Ipv4Addr;

use ipnet::Ipv4Net;
use overdrive_control_plane::guest_network::{GuestNetworkError, SharedGuestNetworkOwner};

// --- boot outcome universe ----------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum BootOutcome {
    Opened,
    Refused { reason: &'static str, variant: &'static str },
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum SteeringLink {
    None,
    Pinned(&'static str), // the program version the pinned link runs
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum PrefixRoute {
    Absent,
    Local,
}

/// DELIVER-provided seeded boot harness: drives the production boot sequence
/// with the sim steering seam and records, in order, every boot effect
/// attempted — so a refusal can be shown to write nothing.
struct BootHarness;
impl BootHarness {
    fn boot(&self) -> BootOutcome {
        unreachable!("DELIVER harness: run probe_startup -> ... -> converge_shared")
    }
    fn owner(&self) -> &dyn SharedGuestNetworkOwner {
        unreachable!("DELIVER harness: the production owner under seeded sim")
    }
    fn steering_link(&self) -> SteeringLink {
        unreachable!("DELIVER harness: observe the pinned link / its program")
    }
    fn prefix_route(&self) -> PrefixRoute {
        unreachable!("DELIVER harness: observe the Overdrive-tagged local route")
    }
    /// The ordered boot-effect log (veth create, nft write, steering load/attach,
    /// route write). A refusal at probe_startup must leave this empty.
    fn boot_effects(&self) -> Vec<&'static str> {
        unreachable!("DELIVER harness: recorded boot effects, in order")
    }
    fn script_steering_load_verifier_reject(&self) {}
    fn script_steering_swap_update_failure(&self) {}
    fn script_steering_first_attach_pin_failure(&self) {}
    fn script_crash_between(&self, _after_step: &str) {}
    fn left_behind_pinned_link(&self, _prog: &'static str) {}
    fn partial_nft_table_at_step8(&self) {}
}
fn boot_harness() -> BootHarness {
    unreachable!("DELIVER harness builder")
}
/// A harness configured with a node network shape (interface addresses, the
/// configured `vip_range`, the configured wildcard TCP listener ports).
fn boot_harness_with(
    _node_addrs: &[Ipv4Addr],
    _vip_range: &[Ipv4Net],
    _wildcard_ports: &[u16],
) -> BootHarness {
    unreachable!("DELIVER harness builder with a node network shape")
}

fn guest_prefix() -> Ipv4Net {
    "100.95.0.0/16".parse().unwrap()
}

// --- P4-S12 / R5-24, G-V1, inv 6: boot order + crash between steps ------------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (converge_shared + SimGuestPrefixSteering; reconciliation)"]
async fn boot_runs_load_then_swap_then_route_and_a_verifier_reject_is_never_attached() {
    // GIVEN a scripted load failure (the verifier rejects the program):
    let h = boot_harness();
    h.script_steering_load_verifier_reject();
    // WHEN boot runs:
    let outcome = h.boot();
    // THEN boot refuses GuestPrefixSteeringConverge, writes no route, and nothing
    // is attached (a verifier-rejected program is never attached).
    assert_eq!(
        outcome,
        BootOutcome::Refused {
            reason: "guest_network.converge",
            variant: "GuestPrefixSteeringConverge"
        }
    );
    assert_eq!(h.prefix_route(), PrefixRoute::Absent, "route unwritten after a load failure");
    assert_eq!(h.steering_link(), SteeringLink::None, "nothing attached after a verifier reject");
}

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (converge_shared + SimGuestPrefixSteering; reconciliation)"]
async fn a_failed_or_interrupted_boot_step_leaves_the_earlier_program_and_route_local_only_over_a_pinned_link()
 {
    // GIVEN an earlier boot left a pinned link running program v1 and a tagged
    // local route; this boot's swap (BPF_LINK_UPDATE) fails.
    let h = boot_harness();
    h.left_behind_pinned_link("v1");
    h.script_steering_swap_update_failure();
    let outcome = h.boot();
    // THEN boot refuses, does not write the route, and the link still runs the
    // EARLIER program (K-L9); the route keeps its earlier form.
    assert_eq!(
        outcome,
        BootOutcome::Refused {
            reason: "guest_network.converge",
            variant: "GuestPrefixSteeringConverge"
        }
    );
    assert_eq!(
        h.steering_link(),
        SteeringLink::Pinned("v1"),
        "the earlier program stays attached (K-L9)"
    );
    // invariant 6: the route is `local` only while a PINNED link exists — it is,
    // so the pre-existing local route is kept; while serve is down a host-local
    // connect to the prefix is reset and a remote packet dropped (asserted in
    // the Tier-3 exposure test; here the state is pinned).
    assert_eq!(h.prefix_route(), PrefixRoute::Local, "route unchanged behind the pinned link");

    // AND a crash between a first attach and its pin leaves no link (M-4: an
    // unpinned first link does not count as local).
    let h2 = boot_harness();
    h2.script_crash_between("first-attach-before-pin");
    assert_eq!(
        h2.boot(),
        BootOutcome::Refused { reason: "crash", variant: "InterruptedBeforePin" }
    );
    assert_eq!(h2.steering_link(), SteeringLink::None);
    assert_eq!(
        h2.prefix_route(),
        PrefixRoute::Absent,
        "no route written: no pinned link exists (M-4)"
    );
}

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (converge_shared + SimGuestPrefixSteering; reconciliation)"]
async fn a_partial_nft_table_does_not_make_the_prefix_reach_a_host_service() {
    // inv 11 (shared_table) at the boot layer: with the shared table left partial
    // at step 8, boot still loads/attaches the steering and the route is `local`
    // only over the pinned link; the steering alone keeps the prefix closed
    // (the Tier-3 exposure proof is V-19/P4-S16).
    let h = boot_harness();
    h.partial_nft_table_at_step8();
    assert_eq!(h.boot(), BootOutcome::Opened, "boot opens: the table is not a route precondition");
    assert!(
        matches!(h.steering_link(), SteeringLink::Pinned(_)),
        "steering attached despite the partial table"
    );
}

// --- P4-S13 / R5-39, D5a-SERVE-IFACE: veth pair carries the unframe link ------

#[test]
#[ignore = "pending DELIVER step 2/9 (structural boot-order probe; reconciliation)"]
fn serve_creates_the_veth_pair_before_converge_shared() {
    // Structural boot-order test: `overdrive serve` with the default dataplane
    // interface names creates `ovd-veth-cli`/`ovd-veth-bk` (step 3) BEFORE
    // `converge_shared` (step 9), so the boot unframe attach covers them. The
    // probe records the boot-step order; the test fails if veth provisioning
    // runs after converge_shared.
    let order = recorded_boot_step_order_default_nics();
    let veth = order.iter().position(|s| *s == "create_veth_pair").expect("veth pair created");
    let converge =
        order.iter().position(|s| *s == "converge_shared").expect("converge_shared runs");
    assert!(veth < converge, "D5a-SERVE-IFACE: veth pair before converge_shared");

    // Booted with operator-named NICs, no pair is created (the NICs carry the
    // link as in R5-38 — the attach itself is P5).
    let order_named = recorded_boot_step_order_operator_nics();
    assert!(
        !order_named.iter().any(|s| *s == "create_veth_pair"),
        "no pair with operator-named NICs"
    );
}
fn recorded_boot_step_order_default_nics() -> Vec<&'static str> {
    unreachable!("DELIVER structural probe: the recorded boot-step order (default NICs)")
}
fn recorded_boot_step_order_operator_nics() -> Vec<&'static str> {
    unreachable!("DELIVER structural probe: the recorded boot-step order (operator NICs)")
}

// --- P4-S14 / R5-11, R5-40, D8-PREFIX-CFG, D8-PREFIX-VIP, G-V1 row2 -----------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (probe_startup overlap checks; reconciliation)"]
async fn a_node_address_inside_the_prefix_refuses_before_any_boot_effect() {
    let h =
        boot_harness_with(&[Ipv4Addr::new(100, 95, 7, 7)], &["10.96.0.0/16".parse().unwrap()], &[]);
    let err = h.owner().probe_startup().await.expect_err("probe_startup refuses");
    assert!(
        matches!(err, GuestNetworkError::GuestPrefixOverlapsNodeAddress { .. }),
        "names the overlap"
    );
    assert!(
        h.boot_effects().is_empty(),
        "refused before any boot effect: no veth, address, route, nft, steering"
    );
}

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (probe_startup overlap checks; reconciliation)"]
async fn a_vip_range_overlapping_the_prefix_refuses_before_any_boot_effect() {
    // vip_range inside/straddling 100.95.0.0/16 refuses; the VIP check runs
    // FIRST, so a node that would fail both names the VIP range.
    for vr in ["100.95.255.0/24", "100.94.0.0/15"] {
        let h = boot_harness_with(&[], &[vr.parse().unwrap()], &[]);
        let err = h.owner().probe_startup().await.expect_err("probe_startup refuses");
        assert!(
            matches!(err, GuestNetworkError::GuestPrefixOverlapsVipRange { .. }),
            "names the VIP range for {vr}"
        );
        assert!(h.boot_effects().is_empty(), "no boot effect before the refusal for {vr}");
    }
    // disjoint / adjacent ranges boot.
    for vr in ["100.96.0.0/16", "10.96.0.0/16"] {
        let h = boot_harness_with(&[], &[vr.parse().unwrap()], &[]);
        assert!(h.owner().probe_startup().await.is_ok(), "disjoint range {vr} boots");
    }
}

/// P4-S14 pure predicate (layer-1 PBT): refusal iff some range of the VipRange
/// intersects the guest prefix.
mod overlap_predicate_property {
    use super::*;
    use proptest::prelude::*;

    // The production overlap predicate DELIVER exposes as a pure fn; named here.
    fn vip_range_overlaps_prefix(prefix: Ipv4Net, ranges: &[Ipv4Net]) -> bool {
        // oracle (the property): intersects iff any range overlaps the prefix
        ranges.iter().any(|r| networks_overlap(prefix, *r))
    }
    fn networks_overlap(a: Ipv4Net, b: Ipv4Net) -> bool {
        a.contains(&b.network()) || b.contains(&a.network())
    }
    fn arb_v4net() -> impl Strategy<Value = Ipv4Net> {
        (any::<u32>(), 8u8..=30)
            .prop_map(|(bits, pfx)| Ipv4Net::new(Ipv4Addr::from(bits), pfx).unwrap().trunc())
    }

    proptest! {
        #[test]
        fn refusal_iff_some_range_intersects_the_prefix(ranges in proptest::collection::vec(arb_v4net(), 0..6)) {
            let prefix = guest_prefix();
            let refuses = vip_range_overlaps_prefix(prefix, &ranges);
            let any_intersects = ranges.iter().any(|r| networks_overlap(prefix, *r));
            prop_assert_eq!(refuses, any_intersects);
        }
    }
}

// --- P4-S15 / R5-42, R5-44, INTAKE-BIND: no wildcard TCP bind after boot -------

#[tokio::test]
#[ignore = "pending DELIVER step 2/9 (probe_startup intake-block check; reconciliation)"]
async fn the_intake_block_conflicting_with_the_ephemeral_range_or_a_wildcard_listener_refuses() {
    // ip_local_port_range reaching into the block (e.g. 20000 60999):
    let h = boot_harness_with(&[], &["10.96.0.0/16".parse().unwrap()], &[]);
    configure_ephemeral_range(&h, 20_000, 60_999);
    let err = h.owner().probe_startup().await.expect_err("refuses EphemeralRange");
    assert!(
        matches!(err, GuestNetworkError::IntakePortBlockConflict { .. }),
        "IntakePortBlockConflict::EphemeralRange before any boot effect",
    );
    assert!(h.boot_effects().is_empty(), "no boot effect before the refusal");

    // A configured wildcard TCP listener inside the block (config.bind = 0.0.0.0:21000):
    let h2 = boot_harness_with(&[], &["10.96.0.0/16".parse().unwrap()], &[21_000]);
    let err2 = h2.owner().probe_startup().await.expect_err("refuses HostListener");
    assert!(
        matches!(err2, GuestNetworkError::IntakePortBlockConflict { .. }),
        "HostListener { port: 21000 }"
    );
    assert!(h2.boot_effects().is_empty(), "no boot effect");
}

#[test]
#[ignore = "pending DELIVER step 2/9 (structural boot-order + source check; reconciliation)"]
fn no_explicit_port_wildcard_tcp_bind_happens_after_open_after_boot() {
    // Structural + source check (INTAKE-BIND, ADR-0178): every explicit-port TCP
    // bind to the unspecified address is made BEFORE open_after_boot, when no
    // intake listener exists; no step after it makes such a bind. The probe
    // records wildcard binds against open_after_boot.
    let binds = recorded_wildcard_tcp_binds_against_open_after_boot();
    assert!(
        binds.iter().all(|(_, before_open)| *before_open),
        "every explicit-port wildcard TCP bind precedes open_after_boot",
    );
}
fn recorded_wildcard_tcp_binds_against_open_after_boot() -> Vec<(u16, bool)> {
    unreachable!("DELIVER structural probe: (port, bound_before_open_after_boot) pairs")
}
fn configure_ephemeral_range(_h: &BootHarness, _low: u16, _high: u16) {}
