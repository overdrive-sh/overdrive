//! P5 — egress unframe boot-attach, errno map, stacked-interface refusal, USO.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5. Scenarios
//! P5-S1, P5-S2, P5-S3; obligations R5-38, R5-46, R5-39 (dataplane half),
//! gate G-V1 (unframe-attach half) / G-V6 (USO). Decisions D5a, D5a-IFACE,
//! D5a-IFACE-BOOT, IFACE-NO-STACK, USO-NO-SEGMENT (ADR-0165, ADR-0181).
//! Spec: `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//!
//! ## GATED — authored RED, NOT YET REGISTERED
//! The production surface this file calls DOES NOT EXIST YET (confirmed
//! 2026-10-09) and is built by the DELIVER roadmap reconciliation (the first
//! DELIVER activity, feature-delta L16554-16558). Per CLAUDE.md §"Implement to
//! the design — never invent API surface" and `.claude/rules/testing.md`
//! §"RED acceptance scaffolds and activation", this body is authored against
//! the design-pinned contract and is **not** added to `tests/integration.rs`
//! `mod integration { … }` until the co-sequenced scaffold lands, so it does
//! not break the workspace compile (shared-infra §6). DELIVER adds the
//! `mod vsock_egress_unframe;` line, the `host-kernel-shared` nextest-group
//! entry, and relocates the `// LAYER-1` proptests to the crate default lane.
//!
//! ## Design-pinned symbols this file depends on (DELIVER step ~4 scaffold)
//! `overdrive_dataplane::guest_flow::{GuestFlowForwarder, UnframeConvergence,
//! UnframeAttachError, GuestFlowInventory, GuestFlowCounters}`;
//! `overdrive_core::guest_network::InterfaceIndex`;
//! `overdrive_control_plane::guest_network::{SharedGuestNetworkOwner}` +
//! `GuestNetworkError::{StackedInterface, UnframeAttach}` with
//! `InterfaceStacking` / `StackedLinkKind`. CRAFTER-EXTRACTION fns (internal
//! structure, Mandate 4 / G-1): the attach-errno classifier and the
//! stacked-interface predicate.

use std::collections::BTreeSet;

use overdrive_core::guest_network::InterfaceIndex;
use overdrive_dataplane::guest_flow::{GuestFlowForwarder, GuestFlowInventory, UnframeAttachError};

use super::helpers; // RAII veth-pair + capability gating (existing fixture)

/// A guest datagram shape for the byte-exactness oracle. The egress unframe
/// program strips the 8-byte frame carrier so an empty datagram arrives as 0
/// bytes and a frame-shaped application payload arrives unchanged (D5a, P-31).
#[derive(Debug, Clone)]
struct DatagramCase {
    label: &'static str,
    /// Application payload the guest sends (what the peer must receive).
    payload: Vec<u8>,
}

fn datagram_cases() -> Vec<DatagramCase> {
    vec![
        DatagramCase { label: "empty", payload: vec![] },
        DatagramCase { label: "one-byte", payload: vec![0xAB] },
        // A payload that itself parses as a valid frame (the verdict kept the
        // frame; the egress program strips it on the registered tuple): the
        // application payload must still arrive byte-exact (P-31, 8 B / 13 B).
        DatagramCase { label: "frame-shaped-8", payload: b"OVDF\x00\x00\x00\x00".to_vec() },
        DatagramCase { label: "frame-shaped-13", payload: b"OVDF\x00\x00\x00\x05hello".to_vec() },
        DatagramCase { label: "mtu-minus-1", payload: vec![0x5A; 1430] },
        DatagramCase { label: "max-59000", payload: vec![0x33; 59_000] },
    ]
}

// ---------------------------------------------------------------------------
// P5-S1 — every interface present at boot carries the egress unframe program.
// Tier-3 (host-kernel-shared). Drives the real `GuestFlowForwarder` over the
// veth fixture; NOT `boot_vsock_node` (would cycle on `tests/conformance`).
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: egress-unframe forwarder (D5a-IFACE, ~step 4)"]
async fn every_boot_interface_carries_unframe_and_datagrams_arrive_byte_exact() {
    // GIVEN: the forwarder loaded (attaches `lo`) and several independent
    // interfaces present at boot under policy routing.
    let caps = helpers::require_net_admin(); // skips with a clear reason if absent
    let topo = helpers::PolicyRoutedInterfaces::new(&caps, /* count */ 3)
        .expect("independent policy-routed interfaces");
    let owner_cgroup = helpers::non_root_owner_cgroup(&caps).expect("owner cgroup");

    let forwarder =
        GuestFlowForwarder::load(owner_cgroup.as_fd()).expect("forwarder loads, lo attached");
    forwarder.probe().expect("earned-trust probe");

    // WHEN: boot converges the unframe program onto every other interface.
    let requested: BTreeSet<InterfaceIndex> = topo.interface_indices();
    let convergence =
        forwarder.converge_unframe_interfaces(&requested).expect("converge runs (program loaded)");

    // THEN: every requested interface landed in `attached`, none in `failed`.
    assert_eq!(convergence.attached, requested, "every present interface attached");
    assert!(convergence.failed.is_empty(), "no attach failed: {:?}", convergence.failed);
    let inv: GuestFlowInventory = forwarder.inventory().expect("inventory");
    assert!(inv.links_attached.unframe_lo, "lo carries the program");
    assert_eq!(inv.unframe_interfaces, requested, "inventory lists every converged interface");

    // AND: an empty and a frame-shaped datagram leaving through ANY boot-present
    // interface reach their peer byte-exact (empty -> 0 B; payload unchanged).
    for iface in &topo.interfaces {
        for case in datagram_cases() {
            let received = topo
                .send_registered_guest_datagram(&forwarder, iface, &case.payload)
                .await
                .unwrap_or_else(|e| panic!("{} through {}: {e}", case.label, iface.name));
            assert_eq!(received, case.payload, "byte-exact {} through {}", case.label, iface.name);
        }
    }
    // AND: a non-owner (unregistered) frame-shaped datagram is passed unchanged,
    // never stripped (D5a: TC_ACT_OK on a miss, never a drop for bad magic).
    let unregistered = topo
        .send_unregistered_datagram(&topo.interfaces[0], b"OVDF\x00\x00\x00\x00plain")
        .await
        .expect("unregistered send");
    assert_eq!(unregistered, b"OVDF\x00\x00\x00\x00plain", "unregistered passes untouched");
}

/// P5-S1 sad path — a kernel attach refusal at boot refuses startup,
/// `UnframeAttach` naming the interface, before `open_after_boot`. One cell per
/// observed errno class (Mandate 11: enumerated, not PBT-generated at Tier-3).
#[tokio::test]
#[ignore = "pending DELIVER: boot unframe refusal (D5a-IFACE-BOOT, ~step 4)"]
async fn boot_refuses_when_an_unframe_attach_fails_naming_the_interface() {
    let caps = helpers::require_net_admin();
    let topo = helpers::PolicyRoutedInterfaces::new(&caps, 2).expect("interfaces");
    let owner_cgroup = helpers::non_root_owner_cgroup(&caps).expect("owner cgroup");
    let forwarder = GuestFlowForwarder::load(owner_cgroup.as_fd()).expect("load");

    // Sad: EEXIST — attach the same program twice onto one interface; the second
    // lands in `failed`, and the owner's `converge_shared` turns ANY non-empty
    // `failed` into a startup refusal (D5a-IFACE-BOOT).
    let one: BTreeSet<InterfaceIndex> = [topo.interfaces[0].index].into_iter().collect();
    forwarder.converge_unframe_interfaces(&one).expect("first attach ok");
    let second = forwarder.converge_unframe_interfaces(&one).expect("convergence runs");
    assert!(
        second.failed.contains_key(&topo.interfaces[0].index),
        "second attach of the same program fails (EEXIST)"
    );
    let err = second.failed.get(&topo.interfaces[0].index).unwrap();
    assert_eq!(err.ifindex(), topo.interfaces[0].index, "error names the interface");

    // The boot path: `converge_shared` promotes the first `failed` entry to
    // `GuestNetworkError::UnframeAttach { source }` and refuses startup with the
    // route unwritten. Driven through the owner's convergence at Tier-3 here;
    // the full `run_server` `health.startup.refused` projection is a P6 cell.
    helpers::assert_converge_shared_refuses_unframe(&topo, &one);
}

// LAYER-1 (relocate to crate default unit lane; G-4) — the attach-errno map.
// CRAFTER-EXTRACTION (G-1): the design pins the map by behaviour (ENOMEM ->
// KernelMemory; every other errno -> Attach) but not a public pure fn; DELIVER
// extracts `classify_unframe_attach_errno(ifindex, errno) -> UnframeAttachError`
// (internal structure). This proptest pins the PROPERTY over the errno domain.
mod errno_classifier {
    use super::*;
    use proptest::prelude::*;

    /// Recommended extracted signature (CRAFTER-EXTRACTION). The test asserts
    /// the classification, not the name; DELIVER may name it differently.
    fn classify(ifindex: InterfaceIndex, errno: i32) -> UnframeAttachError {
        overdrive_dataplane::guest_flow::classify_unframe_attach_errno(ifindex, errno)
    }

    fn some_ifindex() -> InterfaceIndex {
        InterfaceIndex::new(42).expect("nonzero ifindex")
    }

    #[test]
    #[ignore = "pending DELIVER: classify_unframe_attach_errno (D5a-IFACE-BOOT, ~step 4)"]
    fn enomem_classifies_kernel_memory() {
        assert!(matches!(
            classify(some_ifindex(), libc::ENOMEM),
            UnframeAttachError::KernelMemory { .. }
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[ignore = "pending DELIVER: classify_unframe_attach_errno (D5a-IFACE-BOOT, ~step 4)"]
        fn every_non_enomem_errno_classifies_attach(errno in 1_i32..4096) {
            prop_assume!(errno != libc::ENOMEM);
            // Pinned observed classes (EEXIST/ENODEV/EMFILE/ERANGE) and any
            // other errno all map to Attach; ENOMEM alone is KernelMemory.
            prop_assert!(matches!(
                classify(some_ifindex(), errno),
                UnframeAttachError::Attach { .. }
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// P5-S2 — serve refuses to boot on a stacked root-namespace interface
// (IFACE-NO-STACK, ADR-0181). Refusal before any boot effect.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: probe_startup stacked check (IFACE-NO-STACK, ~step 4)"]
async fn serve_refuses_to_boot_on_a_stacked_interface_with_no_boot_effect() {
    use overdrive_control_plane::guest_network::GuestNetworkError;

    let caps = helpers::require_net_admin();

    // GIVEN each stacked topology (VLAN-over, macvlan-over, bridge-with-slave):
    for stacked in helpers::stacked_topologies(&caps) {
        let owner = helpers::new_owner_for_probe(&stacked.host).await;

        // WHEN probe_startup runs (before any boot effect).
        let before = helpers::snapshot_kernel_objects(&stacked.host);
        let result = owner.probe_startup().await;

        // THEN it refuses StackedInterface naming the interface + how it is
        // stacked, and NOTHING on the system changed (no attach, no route, no
        // nft table). @contract-shape:unbounded-preservation.
        match result {
            Err(GuestNetworkError::StackedInterface { interface, stacking }) => {
                assert_eq!(interface, stacked.offending_index, "{}", stacked.label);
                assert_eq!(stacking, stacked.expected_stacking, "{}", stacked.label);
            }
            other => panic!("{}: expected StackedInterface, got {other:?}", stacked.label),
        }
        let after = helpers::snapshot_kernel_objects(&stacked.host);
        assert_eq!(before, after, "{}: no boot effect before refusal", stacked.label);
    }

    // AND a host whose interfaces are independent (veth pairs included) boots.
    let independent = helpers::independent_interfaces(&caps);
    let owner = helpers::new_owner_for_probe(&independent).await;
    owner.probe_startup().await.expect("independent interfaces: boot proceeds");
}

// LAYER-1 (relocate; G-4) — the stacked predicate. CRAFTER-EXTRACTION (G-1):
// refusal iff some interface has a master OR a stacking link kind; a veth peer,
// vxlan/geneve/gre/ipip/xfrm/macsec/wireguard are NOT stacked.
mod stacked_predicate {
    use proptest::prelude::*;

    use overdrive_control_plane::guest_network::{InterfaceStacking, StackedLinkKind};

    // A generated interface inventory row (the fields the predicate reads).
    #[derive(Debug, Clone)]
    struct IfaceRow {
        has_master: bool,
        link_kind: Option<&'static str>,
    }

    fn link_kind() -> impl Strategy<Value = Option<&'static str>> {
        prop_oneof![
            Just(None), // plain / veth
            Just(Some("veth")),
            Just(Some("vlan")),
            Just(Some("macvlan")),
            Just(Some("macvtap")),
            Just(Some("ipvlan")),
            Just(Some("ipvtap")),
            Just(Some("vxlan")),     // tunnel: NOT stacked
            Just(Some("wireguard")), // encrypting: NOT stacked
            Just(Some("gre")),
        ]
    }

    fn is_stacking_kind(kind: &str) -> bool {
        matches!(kind, "vlan" | "macvlan" | "macvtap" | "ipvlan" | "ipvtap")
    }

    // Recommended extracted signature (CRAFTER-EXTRACTION): classify one row.
    fn classify(row: &IfaceRow) -> Option<InterfaceStacking> {
        overdrive_control_plane::guest_network::classify_interface_stacking(
            row.has_master,
            row.link_kind,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[ignore = "pending DELIVER: classify_interface_stacking (IFACE-NO-STACK, ~step 4)"]
        fn refusal_iff_master_or_stacking_link_kind(
            has_master in any::<bool>(),
            link_kind in link_kind(),
        ) {
            let row = IfaceRow { has_master, link_kind };
            let classified = classify(&row);
            let should_refuse =
                row.has_master || row.link_kind.is_some_and(is_stacking_kind);
            prop_assert_eq!(classified.is_some(), should_refuse);
            if row.has_master {
                prop_assert!(matches!(classified, Some(InterfaceStacking::Enslaved { .. })));
            } else if let Some(k) = row.link_kind.filter(|k| is_stacking_kind(k)) {
                let expect_kind = match k {
                    "vlan" => StackedLinkKind::Vlan,
                    "macvlan" => StackedLinkKind::Macvlan,
                    "macvtap" => StackedLinkKind::Macvtap,
                    "ipvlan" => StackedLinkKind::Ipvlan,
                    "ipvtap" => StackedLinkKind::Ipvtap,
                    _ => unreachable!(),
                };
                prop_assert!(matches!(
                    classified,
                    Some(InterfaceStacking::Upper { kind, .. }) if kind == expect_kind
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P5-S3 — guest datagrams survive a USO interface byte-exact (USO-NO-SEGMENT).
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: owner UDP sockets never set UDP_SEGMENT (USO-NO-SEGMENT, ~step 4)"]
async fn guest_datagrams_survive_a_uso_interface_byte_exact() {
    let caps = helpers::require_net_admin();
    let iface = helpers::uso_enabled_interface(&caps).expect("tx-udp-segmentation on");
    let owner_cgroup = helpers::non_root_owner_cgroup(&caps).expect("owner cgroup");
    let forwarder = GuestFlowForwarder::load(owner_cgroup.as_fd()).expect("load");
    forwarder.converge_unframe_interfaces(&[iface.index].into_iter().collect()).unwrap();

    for case in datagram_cases() {
        let (received, udp_sock) = iface
            .send_registered_guest_datagram_capturing_socket(&forwarder, &case.payload)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", case.label));
        assert_eq!(received, case.payload, "byte-exact {} on USO iface", case.label);
        // No owner UDP socket has UDP_SEGMENT set (read back).
        assert_eq!(
            helpers::getsockopt_udp_segment(&udp_sock),
            0,
            "{}: UDP_SEGMENT must be unset on owner socket",
            case.label
        );
    }
}

/// P5-S3 source check (architecture test): neither the owner nor the forwarder
/// crate references the `UDP_SEGMENT` option. LAYER-1 — default lane.
#[test]
#[ignore = "pending DELIVER: USO-NO-SEGMENT source check"]
fn owner_and_forwarder_crates_never_reference_udp_segment() {
    // Scans `overdrive-control-plane` + `overdrive-dataplane` sources for the
    // `UDP_SEGMENT` constant (an architecture test over the crate source tree;
    // not a kernel test). DELIVER points it at the two crate `src/` roots.
    let hits = helpers::grep_crate_sources_for(
        &["overdrive-control-plane", "overdrive-dataplane"],
        "UDP_SEGMENT",
    );
    assert!(hits.is_empty(), "UDP_SEGMENT referenced in owner/forwarder source: {hits:?}");
}
