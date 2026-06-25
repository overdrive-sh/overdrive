//! S-DBN-REKEY-01..04 + S-DBN-FAILCLOSED-01 + S-DBN-EQUIV-01 — the re-keyed
//! `MtlsResolve`/`BackendIndex` proptests (Tier 1, default unit lane,
//! in-process; ADR-0072 REV-2 "stable-frontend", GH #243; roadmap 02-00 /
//! Finding-1 / Finding-3 / K-DBN-4).
//!
//! These properties exercise the `BackendIndex`/`classify` data structure
//! in-process at Tier 1 (NO real socket — the socket is irreducibly Tier 3,
//! DDN-4). The re-keyed `classify(orig_dst, proto)` IS the data-structure
//! driving port the criteria name; `BackendIndex` (and `FrontendKey`,
//! `apply_row`, `bind_frontend`) are the public 02-00 EXTEND surface. The
//! `ServiceId` keyed into `by_frontend` is the row's existing content-addressed
//! `service_id`, NOT a re-derivation, and the `F` it keys on is the SAME stable
//! frontend the `FrontendAddrAllocator` binds (DDN-2 single-owner invariant).
//!
//! REKEY-01/02/03 + FAILCLOSED-01 + EQUIV-01 drive the genuinely-new arm-1/arm-2
//! `classify` behaviour; REKEY-04 is GREEN-by-preservation (the additive-EXTEND
//! `by_addr` arm-3 fall-through, unchanged).

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use overdrive_control_plane::mtls_resolve_adapter::{BackendIndex, FrontendKey};
use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::{ServiceId, SpiffeId};
use overdrive_core::traits::dataplane::Backend;
use overdrive_core::traits::mtls_resolve::{MtlsResolution, ResolvedBackend};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies + fixtures.
// ---------------------------------------------------------------------------

/// A V4 socket addr from four octets + a port.
const fn v4(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// A `Backend` at the given V4 addr with the given readiness. The `alloc`
/// SVID is derived from the addr so distinct backends carry distinct
/// identities (the value is irrelevant to `classify`, which keys on `addr` +
/// `healthy`).
fn backend(addr: SocketAddrV4, healthy: bool) -> Backend {
    Backend {
        alloc: SpiffeId::new(&format!("spiffe://overdrive.local/job/svc/alloc/{}", addr.port()))
            .expect("valid spiffe id"),
        addr: SocketAddr::V4(addr),
        weight: 1,
        healthy,
    }
}

fn svc(id: u64) -> ServiceId {
    ServiceId::new(id).expect("valid service id")
}

/// A stable frontend `F` inside `10.98.0.0/16` (the dial-by-name frontend
/// block) at the given host octets + port. `(F, port, proto)` is a frontend
/// key the `FrontendAddrAllocator` would bind.
const fn frontend(hi: u8, lo: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 98, hi, lo), port)
}

/// A per-instance backend addr inside `10.99.0.0/16` (the workload subnet,
/// distinct from the frontend block) — a direct backend-addr dial target.
const fn backend_addr(hi: u8, lo: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(10, 99, hi, lo), port)
}

// ---------------------------------------------------------------------------
// S-DBN-REKEY-01 — frontend → first-by-Ord live-backend translation.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-DBN-REKEY-01 (K-DBN-4): for a service with ≥1 running-AND-healthy
    /// backend, a frontend key `K = (F, port, Tcp)` bound `by_frontend[K] == Svc`
    /// (and arbitrary `by_addr` entries) makes `classify((F, port), Tcp)` return
    /// `Mesh(first-by-Ord healthy backend)` — NEVER `NonMesh` (a frontend HIT is
    /// always mesh), NEVER an unhealthy backend.
    ///
    /// The selected backend is the SMALLEST-by-`Ord` running-AND-healthy backend
    /// addr (BLOCKER-2: the deterministic tie-break). The property generates a
    /// multi-backend service (some unhealthy) and asserts the selected addr is
    /// exactly `min` over the healthy set — killing a last-by-Ord or
    /// pick-unhealthy mutant.
    #[test]
    fn rekey_01_frontend_hit_selects_first_by_ord_healthy_backend(
        f_lo in 1u8..=254,
        port in 1u16..=u16::MAX,
        // Three candidate backend host octets, each with a readiness bit.
        h0 in 1u8..=254, healthy0 in any::<bool>(),
        h1 in 1u8..=254, healthy1 in any::<bool>(),
        h2 in 1u8..=254, healthy2 in any::<bool>(),
    ) {
        // Distinct backend addrs (workload subnet) so `min` is well-defined.
        prop_assume!(h0 != h1 && h1 != h2 && h0 != h2);
        // At least one healthy backend (REKEY-01's precondition).
        prop_assume!(healthy0 || healthy1 || healthy2);

        let f = frontend(0, f_lo, port);
        let backends = vec![
            backend(backend_addr(0, h0, 9000), healthy0),
            backend(backend_addr(0, h1, 9000), healthy1),
            backend(backend_addr(0, h2, 9000), healthy2),
        ];
        // The expected pick: the smallest-by-Ord HEALTHY backend addr.
        let expected_addr = backends
            .iter()
            .filter(|b| b.healthy)
            .map(|b| match b.addr {
                SocketAddr::V4(v) => v,
                SocketAddr::V6(_) => unreachable!("v4 fixture"),
            })
            .min()
            .expect("at least one healthy backend by precondition");

        let mut index = BackendIndex::default();
        index.apply_row(svc(1), &backends);
        index.bind_frontend(FrontendKey::new(f, Proto::Tcp), svc(1));

        let got = index.classify(f, Proto::Tcp);
        prop_assert_eq!(
            got,
            MtlsResolution::Mesh(ResolvedBackend { addr: expected_addr, expected_svid: None }),
            "a frontend HIT resolves to the first-by-Ord running-AND-healthy backend",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-REKEY-02 — frontend hit but zero-healthy → MeshUnreachable.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-DBN-REKEY-02: a frontend key `K` bound `by_frontend[K] == Svc` whose
    /// backend set is EMPTY or all-unhealthy makes `classify(K)` return
    /// `MeshUnreachable` (the service is KNOWN — the key matched — but has no
    /// healthy backend right now: fail-closed, refuse, NO cleartext). NEVER
    /// `Mesh`, NEVER `NonMesh`.
    #[test]
    fn rekey_02_frontend_hit_zero_healthy_is_mesh_unreachable(
        f_lo in 1u8..=254,
        port in 1u16..=u16::MAX,
        // 0..=3 backends, ALL unhealthy (empty when n == 0).
        n in 0usize..=3,
        h0 in 1u8..=254, h1 in 1u8..=254, h2 in 1u8..=254,
    ) {
        let f = frontend(1, f_lo, port);
        let all_unhealthy: Vec<Backend> = [h0, h1, h2]
            .into_iter()
            .take(n)
            .map(|h| backend(backend_addr(1, h, 9000), false))
            .collect();

        let mut index = BackendIndex::default();
        index.apply_row(svc(2), &all_unhealthy);
        index.bind_frontend(FrontendKey::new(f, Proto::Tcp), svc(2));

        prop_assert_eq!(
            index.classify(f, Proto::Tcp),
            MtlsResolution::MeshUnreachable,
            "a frontend HIT with an empty/all-unhealthy backend set is MeshUnreachable (fail-closed)",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-REKEY-03 — the proto axis is a key field (Finding-1).
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// S-DBN-REKEY-03 (Finding-1): the frontend key discriminates proto. For ONE
    /// frontend IP `F`, ONE port `P`, and two DISTINCT services whose listeners
    /// are `(F,P,Tcp)` and `(F,P,Udp)`, `by_frontend` holding both yields
    /// `Svc_tcp` for `(F,P,Tcp)` and `Svc_udp` for `(F,P,Udp)`. A bare
    /// `SocketAddrV4` `(F,P)` WITHOUT the proto axis CANNOT distinguish them —
    /// the collision Finding-1 names. A mutant dropping the `Proto` field
    /// collapses the two services onto one key (then ONE of the two assertions
    /// flips).
    #[test]
    fn rekey_03_frontend_key_discriminates_proto(
        f_lo in 1u8..=254,
        port in 1u16..=u16::MAX,
        // Distinct healthy backend host octets for the two services.
        tcp_h in 1u8..=127, udp_h in 128u8..=254,
    ) {
        let f = frontend(2, f_lo, port);
        let tcp_backend = backend_addr(2, tcp_h, 7000);
        let udp_backend = backend_addr(2, udp_h, 7000);

        let mut index = BackendIndex::default();
        index.apply_row(svc(10), &[backend(tcp_backend, true)]);
        index.apply_row(svc(20), &[backend(udp_backend, true)]);
        // SAME (F, P), DIFFERENT proto → two DISTINCT keys → two services.
        index.bind_frontend(FrontendKey::new(f, Proto::Tcp), svc(10));
        index.bind_frontend(FrontendKey::new(f, Proto::Udp), svc(20));

        // The TCP key resolves Svc_tcp's backend.
        prop_assert_eq!(
            index.classify(f, Proto::Tcp),
            MtlsResolution::Mesh(ResolvedBackend { addr: tcp_backend, expected_svid: None }),
            "the (F,P,Tcp) key resolves the TCP service's backend",
        );
        // The UDP key resolves Svc_udp's DISTINCT backend — the proto axis kept
        // them apart (a proto-blind key would collide them onto one entry).
        prop_assert_eq!(
            index.classify(f, Proto::Udp),
            MtlsResolution::Mesh(ResolvedBackend { addr: udp_backend, expected_svid: None }),
            "the (F,P,Udp) key resolves the UDP service's DISTINCT backend — proto discriminates",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-REKEY-04 — backward-compatible additive EXTEND (by_addr preserved).
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-DBN-REKEY-04: a direct backend-addr dial still resolves via `by_addr`.
    /// For a running-and-healthy backend addr `B` in `10.99.0.0/16` indexed in
    /// `by_addr` (and NOT in `by_frontend`), `classify(B, proto)` returns the
    /// SAME verdict the pre-REV-2 `by_addr` path returns (Mesh for healthy `B`;
    /// MeshUnreachable for unhealthy `B`); a true non-mesh dst (not in
    /// `by_frontend`, not in `by_addr`, not in `10.98.0.0/16`) still returns
    /// `NonMesh`. A mutant routing a `by_addr` hit through the frontend path, or
    /// breaking the non-mesh fall-through, flips this.
    ///
    /// GREEN-by-preservation (NOT `#[should_panic]`): REKEY-04 is the
    /// backward-compatibility guard — its inputs reach ONLY arm 3 (the `by_addr`
    /// fall-through), which the additive EXTEND keeps real even in RED. It is
    /// GREEN from the start (the additive EXTEND must not regress the
    /// security-critical existing path); the genuinely-new arm-1/arm-2 behaviour
    /// is RED-scaffolded in REKEY-01/02/03 + FAILCLOSED-01.
    #[test]
    fn rekey_04_by_addr_path_preserved_and_nonmesh_fall_through(
        b_hi in 0u8..=254, b_lo in 1u8..=254, port in 1u16..=u16::MAX,
        healthy in any::<bool>(),
        // A non-mesh dst OUTSIDE all three /16s (e.g. 203.0.113.x — TEST-NET-3).
        nm_lo in 1u8..=254, nm_port in 1u16..=u16::MAX,
    ) {
        let b = backend_addr(b_hi, b_lo, port);
        let non_mesh = v4(203, 0, 113, nm_lo, nm_port);

        let mut index = BackendIndex::default();
        // `by_addr` ONLY — NO frontend binding for `b`.
        index.apply_row(svc(3), &[backend(b, healthy)]);

        let expected_b = if healthy {
            MtlsResolution::Mesh(ResolvedBackend { addr: b, expected_svid: None })
        } else {
            MtlsResolution::MeshUnreachable
        };
        prop_assert_eq!(
            index.classify(b, Proto::Tcp),
            expected_b,
            "a direct backend-addr dial resolves via by_addr unchanged (additive EXTEND)",
        );
        // A true non-mesh dst (outside every map and the frontend subnet) →
        // NonMesh (legitimate non-mesh egress, unchanged).
        prop_assert_eq!(
            index.classify(non_mesh, Proto::Tcp),
            MtlsResolution::NonMesh,
            "a true non-mesh dst outside the frontend subnet is NonMesh (cleartext, by design)",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-FAILCLOSED-01 — the three-way fail-closed-on-frontend-subnet-miss arm.
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-DBN-FAILCLOSED-01 (Finding-3; K-DBN-4) — the structural defense, two
    /// load-bearing properties:
    ///
    /// PROPERTY 1: for every `orig_dst` whose ip ∈ `10.98.0.0/16` that MISSES
    /// `by_frontend` AND MISSES `by_addr`, `classify` returns `MeshUnreachable`
    /// (refuse, NO cleartext — a mesh dial that arrived before the index was
    /// ready (race) OR to a withdrawn `<job>`) and is NEVER `NonMesh` (the
    /// fail-OPEN regression the subnet-scoped arm exists to prevent).
    ///
    /// PROPERTY 2: for every `orig_dst` whose ip is NOT ∈ `10.98.0.0/16` that
    /// misses both maps, `classify` returns `NonMesh` (legitimate non-mesh
    /// egress → cleartext, by design).
    ///
    /// A mutant treating the subnet miss as `NonMesh` flips Property 1 (the
    /// fail-open footgun); a mutant treating EVERY miss as `MeshUnreachable`
    /// flips Property 2 (breaks legitimate non-mesh egress).
    #[test]
    fn failclosed_01_subnet_miss_fails_closed_general_miss_nonmesh(
        in_hi in 0u8..=255, in_lo in 0u8..=255, in_port in 1u16..=u16::MAX,
        // An out-of-subnet octet pair that is NOT 10.98.x (and avoid the
        // workload subnet collision is irrelevant — by_addr is empty here).
        out_a in 0u8..=255, out_b in 0u8..=255, out_c in 0u8..=255, out_d in 0u8..=255,
        out_port in 1u16..=u16::MAX,
    ) {
        // The index is EMPTY — every lookup MISSES both `by_frontend` and
        // `by_addr`. The ONLY discriminator is subnet membership.
        let index = BackendIndex::default();

        // PROPERTY 1 — a miss INSIDE 10.98.0.0/16 is MeshUnreachable (fail-closed).
        let inside = v4(10, 98, in_hi, in_lo, in_port);
        prop_assert_eq!(
            index.classify(inside, Proto::Tcp),
            MtlsResolution::MeshUnreachable,
            "a frontend-subnet MISS fails closed (MeshUnreachable, NO cleartext) — never NonMesh",
        );

        // PROPERTY 2 — a miss OUTSIDE 10.98.0.0/16 is NonMesh (cleartext egress).
        prop_assume!(!(out_a == 10 && out_b == 98));
        let outside = v4(out_a, out_b, out_c, out_d, out_port);
        prop_assert_eq!(
            index.classify(outside, Proto::Tcp),
            MtlsResolution::NonMesh,
            "a general (non-frontend-subnet) MISS stays NonMesh (legitimate non-mesh egress)",
        );
    }
}

// ---------------------------------------------------------------------------
// S-DBN-EQUIV-01 — DST equivalence: two BackendIndex constructions agree on the
// re-keyed classify over the SAME ordered call sequence.
// ---------------------------------------------------------------------------

/// One step in the replayed sequence: either a row apply, a frontend binding,
/// or a classify probe. The two index constructions are driven through the
/// SAME sequence and must agree on every probe verdict.
#[derive(Clone, Debug)]
enum Step {
    ApplyRow { service: u64, addrs: Vec<(u8, bool)> },
    BindFrontend { f_lo: u8, port: u16, proto_udp: bool, service: u64 },
    Classify { hi: u8, lo: u8, port: u16, proto_udp: bool, frontend: bool },
}

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (1u64..=4, prop::collection::vec((1u8..=254, any::<bool>()), 0..=3))
            .prop_map(|(service, addrs)| Step::ApplyRow { service, addrs }),
        (1u8..=254, 1u16..=u16::MAX, any::<bool>(), 1u64..=4).prop_map(
            |(f_lo, port, proto_udp, service)| Step::BindFrontend {
                f_lo,
                port,
                proto_udp,
                service,
            }
        ),
        (0u8..=255, 0u8..=255, 1u16..=u16::MAX, any::<bool>(), any::<bool>()).prop_map(
            |(hi, lo, port, proto_udp, frontend)| Step::Classify {
                hi,
                lo,
                port,
                proto_udp,
                frontend,
            }
        ),
    ]
}

/// Apply one step to an index, returning the classify verdict when the step is
/// a probe (so two constructions can be compared verdict-for-verdict).
fn drive(index: &mut BackendIndex, step: &Step) -> Option<MtlsResolution> {
    match step {
        Step::ApplyRow { service, addrs } => {
            let backends: Vec<Backend> = addrs
                .iter()
                .map(|&(h, healthy)| backend(backend_addr(0, h, 9000), healthy))
                .collect();
            index.apply_row(svc(*service), &backends);
            None
        }
        Step::BindFrontend { f_lo, port, proto_udp, service } => {
            let proto = if *proto_udp { Proto::Udp } else { Proto::Tcp };
            index.bind_frontend(FrontendKey::new(frontend(3, *f_lo, *port), proto), svc(*service));
            None
        }
        Step::Classify { hi, lo, port, proto_udp, frontend: is_frontend } => {
            let proto = if *proto_udp { Proto::Udp } else { Proto::Tcp };
            // Probe a frontend-subnet addr OR a workload-subnet addr depending
            // on the flag, so the sequence exercises all three classify arms.
            let dst = if *is_frontend {
                frontend(3, *hi, *port)
            } else {
                backend_addr(0, (*lo).max(1), *port)
            };
            Some(index.classify(dst, proto))
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// S-DBN-EQUIV-01 (the trait-docstring contract guard, development.md
    /// § "Trait definitions specify behavior"): two `BackendIndex` instances
    /// driven through the SAME ordered sequence of row applies + frontend
    /// bindings return the byte-identical verdict at EVERY classify step, and
    /// the verdict trajectory is deterministic (the first-by-`Ord` selection is
    /// what makes this hold across builds). The three-way arm + the first-by-Ord
    /// selection is the contract; this equivalence test is the enforcement.
    #[test]
    fn equiv_01_two_constructions_agree_on_every_classify(
        steps in prop::collection::vec(arb_step(), 1..=24),
    ) {
        let mut index_a = BackendIndex::default();
        let mut index_b = BackendIndex::default();

        for step in &steps {
            let verdict_a = drive(&mut index_a, step);
            let verdict_b = drive(&mut index_b, step);
            prop_assert_eq!(
                verdict_a,
                verdict_b,
                "two constructions driven through the SAME sequence agree on every classify verdict",
            );
        }
    }
}
