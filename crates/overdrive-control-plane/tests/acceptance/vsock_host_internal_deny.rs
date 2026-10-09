//! P5 — host-internal deny (D26) policy order, mesh/non-mesh/VIP resolution
//! (D24a), owner-egress mark disjointness, and the seeded-sim owner refusal.
//!
//! DISTILL (netns-density-295 vsock replacement), piece P5. Scenarios P5-S5,
//! P5-S7; obligations R5-6 (policy-order half), R5-8 (resolve half); gate G-V6.
//! Decisions D26, D24, D24a, D26-PINS (mark half). ADR-0167, ADR-0164,
//! ADR-0153. Spec:
//! `docs/feature/netns-density-295/distill/test-scenarios-p5-egress-unframe-hostinternal.md`.
//!
//! ## Lane
//! `tests/acceptance/` is this crate's DEFAULT (unit) lane — in-process only,
//! no real infrastructure. The D26 policy order, the D24a resolve branches, and
//! the mark-disjointness check are pure/layer-1; the owner refusal runs under
//! seeded `overdrive-sim`. Correct placement (G-4).
//!
//! ## GATED — authored RED, NOT YET REGISTERED
//! The reshaped owner + D26 policy + the D24a VIP branch + `OWNER_EGRESS_MARK`
//! do not exist yet (confirmed 2026-10-09). Not added to `tests/acceptance.rs`
//! `mod acceptance { … }` until the DELIVER scaffold lands (shared-infra §6).
//!
//! ## Design-pinned symbols (DELIVER step ~9 scaffold)
//! `overdrive_control_plane::guest_network::{FlowRefusal, OWNER_EGRESS_MARK}`;
//! `overdrive_worker::mtls_intercept::MTLS_LEG_S_DIAL_MARK` + the TPROXY mark;
//! `overdrive_worker::{MtlsResolve, MtlsResolution}`,
//! `ServiceBackendsResolve::new(vip_ranges)` (D24a branch);
//! `overdrive_dataplane::allocators::vip_range::VipRange` (existing);
//! `overdrive_sim::adapters::{SimMtlsResolve, guest_network::SimSharedGuestNetworkOwner}`.
//! CRAFTER-EXTRACTION (G-1): the per-flow policy classifier (pure, 5-step order)
//! and the D24a resolve classification arm (internal structure).

use std::net::{Ipv4Addr, SocketAddrV4};

// ---------------------------------------------------------------------------
// P5-S5 — per-flow policy order (D26). LAYER-1 pure proptest + pinned examples.
// Order (first match decides): 1 static Policy; 2 static HostInternal
// (127/8, 169.254/16); 3 mesh resolution (TCP); 4 local-delivery HostInternal
// (route type local/broadcast/multicast/anycast, except gateway:53); 5 datagram
// kernel-peer check. @contract-shape:pure-function.
// ---------------------------------------------------------------------------

/// The request kind the owner classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    TcpConnect,
    Datagram,
}

/// The host route type for a destination (what the kernel FIB would return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteType {
    Unicast,
    Local,
    Broadcast,
    Multicast,
    Anycast,
}

/// The owner's per-flow disposition (the pinned observable the classifier
/// produces before any connect). CRAFTER-EXTRACTION: DELIVER names the real
/// enum; the test asserts the classification semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    Policy,
    HostInternal,
    Resolve, // hand to mesh resolution (TCP) — P5-S7 covers the branch outcomes
    DirectConnect,
    DnsResponder, // gateway:53
}

fn gateway() -> Ipv4Addr {
    Ipv4Addr::new(100, 95, 0, 1)
}

// CRAFTER-EXTRACTION recommended signature for the pure policy classifier.
fn classify(kind: RequestKind, dst: SocketAddrV4, route: RouteType) -> Disposition {
    overdrive_control_plane::guest_network::classify_flow_destination(kind, dst, route, gateway())
}

#[test]
#[ignore = "pending DELIVER: classify_flow_destination (D26, ~step 9)"]
fn host_internal_destinations_are_refused_and_gateway_dns_resolves() {
    // Pinned R5-6 literals.
    let cases: &[(RequestKind, SocketAddrV4, RouteType, Disposition)] = &[
        // 127.0.0.1:p — static HostInternal (step 2).
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080),
            RouteType::Local,
            Disposition::HostInternal,
        ),
        (
            RequestKind::Datagram,
            SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 9999),
            RouteType::Local,
            Disposition::HostInternal,
        ),
        // 169.254.169.254:80 (instance metadata) — static HostInternal.
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(Ipv4Addr::new(169, 254, 169, 254), 80),
            RouteType::Local,
            Disposition::HostInternal,
        ),
        // the host's own address (routed local) — local-delivery HostInternal (step 4).
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 22),
            RouteType::Local,
            Disposition::HostInternal,
        ),
        // gateway:22 — local-delivery HostInternal (not port 53).
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(gateway(), 22),
            RouteType::Local,
            Disposition::HostInternal,
        ),
        // gateway:53 — resolves to the DnsResponder (exception).
        (
            RequestKind::Datagram,
            SocketAddrV4::new(gateway(), 53),
            RouteType::Local,
            Disposition::DnsResponder,
        ),
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(gateway(), 53),
            RouteType::Local,
            Disposition::DnsResponder,
        ),
        // an ordinary off-host unicast TCP dst hands to mesh resolution (step 3).
        (
            RequestKind::TcpConnect,
            SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 443),
            RouteType::Unicast,
            Disposition::Resolve,
        ),
    ];
    for &(kind, dst, route, expect) in cases {
        assert_eq!(classify(kind, dst, route), expect, "dst={dst} route={route:?}");
    }
}

#[cfg(test)]
mod policy_order_property {
    use super::*;
    use proptest::prelude::*;

    fn octet() -> impl Strategy<Value = u8> {
        any::<u8>()
    }
    fn route() -> impl Strategy<Value = RouteType> {
        prop_oneof![
            Just(RouteType::Unicast),
            Just(RouteType::Local),
            Just(RouteType::Broadcast),
            Just(RouteType::Multicast),
            Just(RouteType::Anycast),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[ignore = "pending DELIVER: classify_flow_destination (D26, ~step 9)"]
        fn loopback_and_link_local_are_always_host_internal(
            b in octet(), c in octet(), d in octet(), port in 1u16..=u16::MAX, r in route(),
        ) {
            let lo = SocketAddrV4::new(Ipv4Addr::new(127, b, c, d), port);
            prop_assert_eq!(super::classify(RequestKind::TcpConnect, lo, r), Disposition::HostInternal);
            let ll = SocketAddrV4::new(Ipv4Addr::new(169, 254, c, d), port);
            prop_assert_eq!(super::classify(RequestKind::Datagram, ll, r), Disposition::HostInternal);
        }

        #[test]
        #[ignore = "pending DELIVER: classify_flow_destination (D26, ~step 9)"]
        fn non_unicast_locally_delivered_is_host_internal_except_dns(
            a in 11u8..=223, b in octet(), c in octet(), d in octet(), port in 1u16..=u16::MAX,
        ) {
            prop_assume!(a != 127 && !(a == 169 && b == 254)); // not covered by step 2
            let dst = SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port);
            // Locally delivered non-unicast (and local) => HostInternal (step 4),
            // unless it is gateway:53.
            for r in [RouteType::Local, RouteType::Broadcast, RouteType::Multicast, RouteType::Anycast] {
                let expect = if dst.ip() == &super::gateway() && port == 53 {
                    Disposition::DnsResponder
                } else {
                    Disposition::HostInternal
                };
                prop_assert_eq!(super::classify(RequestKind::TcpConnect, dst, r), expect);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P5-S7 — mesh/non-mesh/VIP resolution (D24a). The `mtls_resolve_rekey`
// equivalence test extended with the VIP branch, per-arm mutation coverage.
// @contract-shape:pure-function.
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: ServiceBackendsResolve VIP branch (D24a, ~step 9)"]
async fn tcp_vip_is_mesh_resolved_unknown_vip_is_mesh_unreachable() {
    use overdrive_worker::mtls_intercept_port::{MtlsResolution, MtlsResolve};
    use overdrive_worker::mtls_intercept_worker::ServiceBackendsResolve;

    // The configured VIP ranges — one SSOT also handed to the VIP allocator.
    let vip_ranges = overdrive_dataplane::allocators::vip_range::VipRange::parse("10.96.0.0/16")
        .expect("vip range");
    let backends = test_service_backends_rows(); // one service VIP 10.96.0.7:443 -> backend
    let resolve = ServiceBackendsResolve::new(backends, vip_ranges.clone());

    // Arm 1: a TCP destination equal to a service's (VIP, port) is a frontend
    // hit — Mesh(first healthy backend).
    let vip_hit = SocketAddrV4::new(Ipv4Addr::new(10, 96, 0, 7), 443);
    assert!(
        matches!(resolve.resolve(vip_hit), MtlsResolution::Mesh(_)),
        "service VIP resolves to the mesh backend"
    );
    // Arm 2: a TCP destination in a VIP range matching no service is
    // MeshUnreachable (fail closed, never NonMesh).
    let vip_miss = SocketAddrV4::new(Ipv4Addr::new(10, 96, 200, 1), 443);
    assert!(
        matches!(resolve.resolve(vip_miss), MtlsResolution::MeshUnreachable),
        "unknown VIP in range is MeshUnreachable, never NonMesh"
    );
    // Arm 3: a destination outside the VIP ranges keeps its non-VIP
    // classification (NonMesh or mesh-by-frontend as before).
    let off_mesh = SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 443);
    assert!(
        !matches!(resolve.resolve(off_mesh), MtlsResolution::MeshUnreachable),
        "an address outside the VIP ranges is not fail-closed as MeshUnreachable"
    );
    // Per-arm mutation coverage is proven by the mutation gate; each arm above
    // is killed by a distinct mutant (frontend hit, in-range miss, out-of-range).
    // The datagram-VIP connect4 rewrite (D24) is a Tier-3 cell (P5-S7 Tier-3 / P6).
}

fn test_service_backends_rows() -> overdrive_worker::mtls_intercept_worker::ServiceBackendsRows {
    // DELIVER supplies one service whose (vip, listener-port, Tcp) is
    // 10.96.0.7:443 with one healthy backend. Shape pinned by ADR-0071/0072.
    overdrive_worker::mtls_intercept_worker::ServiceBackendsRows::single_for_test(
        SocketAddrV4::new(Ipv4Addr::new(10, 96, 0, 7), 443),
        SocketAddrV4::new(Ipv4Addr::new(10, 0, 3, 9), 8443),
    )
}

// ---------------------------------------------------------------------------
// D26-PINS F-6 (mark half) — the Overdrive egress marks are pairwise-disjoint
// single bits. LAYER-1 unit; default lane. Full F-1..F-5 rule construction is
// `vsock_host_internal_enforce.rs` (Tier-3).
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending DELIVER: OWNER_EGRESS_MARK (D26-PINS, ~step 9)"]
fn overdrive_egress_marks_are_pairwise_disjoint_single_bits() {
    let owner = overdrive_control_plane::guest_network::OWNER_EGRESS_MARK;
    let leg_s = overdrive_worker::mtls_intercept::MTLS_LEG_S_DIAL_MARK;
    let tproxy = overdrive_worker::mtls_intercept::MTLS_TPROXY_MARK;
    for (name, m) in [("owner", owner), ("leg_s", leg_s), ("tproxy", tproxy)] {
        assert_eq!(m.count_ones(), 1, "{name} mark is a single bit");
    }
    assert_eq!(owner & leg_s, 0, "owner vs leg_s disjoint");
    assert_eq!(owner & tproxy, 0, "owner vs tproxy disjoint");
    assert_eq!(leg_s & tproxy, 0, "leg_s vs tproxy disjoint");
}

// ---------------------------------------------------------------------------
// P5-S5 (seeded sim) — the owner refuses a host-internal flow and counts it,
// through the production owner path with SimMtlsResolve. Layer-2; universe-bound
// (Rust realization: explicit observable before/after + unchanged, §State-delta).
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "pending DELIVER: owner D26 flow refusal (D26, ~step 9)"]
async fn owner_refuses_host_internal_flow_without_touching_allocation_state() {
    use overdrive_control_plane::guest_network::FlowRefusal;
    use overdrive_sim::adapters::SimMtlsResolve;
    use overdrive_sim::adapters::clock::SimClock;
    use overdrive_sim::adapters::guest_network::SimSharedGuestNetworkOwner;

    let seed = P5_S5_u64_seed();
    let clock = SimClock::new();
    let resolve = SimMtlsResolve::new(); // no mesh match for the host-internal dst
    let mut harness = SimSharedGuestNetworkOwner::builder()
        .clock(clock)
        .resolve(resolve)
        .seed(seed)
        .build()
        .await;
    let alloc = harness.provision_and_activate_one().await.expect("active alloc");

    // Universe: owner-observable surface only (port-exposed), captured before.
    let before = OwnerUniverse {
        alloc_running: harness.allocation_running(&alloc),
        flow_refusal: None,
        host_internal_count: harness.counter_host_internal(),
    };

    // WHEN the guest opens a TCP flow to 127.0.0.1:8080 (host-internal).
    let outcome = harness
        .open_guest_tcp_flow(&alloc, SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 8080))
        .await;

    let after = OwnerUniverse {
        alloc_running: harness.allocation_running(&alloc),
        flow_refusal: outcome.refusal(),
        host_internal_count: harness.counter_host_internal(),
    };

    // THEN: delta on the named entries; everything else unchanged (fail-closed).
    assert_eq!(
        after.flow_refusal,
        Some(FlowRefusal::HostInternal),
        "seed={seed}: refused HostInternal"
    );
    assert_eq!(after.host_internal_count, before.host_internal_count + 1, "seed={seed}: counted");
    assert_eq!(
        after.alloc_running, before.alloc_running,
        "seed={seed}: allocation Running unchanged"
    );
    assert!(after.alloc_running, "seed={seed}: allocation stays Running (flow-local refusal)");
}

#[derive(Debug, PartialEq, Eq)]
struct OwnerUniverse {
    alloc_running: bool,
    flow_refusal: Option<overdrive_control_plane::guest_network::FlowRefusal>,
    host_internal_count: u64,
}

// Placeholder seed literal — DELIVER replaces with a printed-on-failure seed.
#[allow(non_snake_case)]
fn _P5_S5_u64_seed() -> u64 {
    0x5055_0005
}
#[allow(non_snake_case)]
fn P5_S5_u64_seed() -> u64 {
    _P5_S5_u64_seed()
}
