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
//! `by_addr` arm-3 fall-through, unchanged). EQUIV-01 proves the production
//! `classify` trajectory matches an INDEPENDENT reference oracle (the spec
//! re-stated as a model fold — NOT a struct compared against itself) over an
//! arbitrary ordered step sequence, plus a determinism clause (same seed →
//! bit-identical trajectory).

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
// S-DBN-EQUIV-01 — reference-oracle equivalence + determinism: the production
// `BackendIndex` re-keyed-classify trajectory over an arbitrary ordered step
// sequence matches an INDEPENDENT hand-written reference oracle that re-derives
// the expected verdict at every classify-probe WITHOUT calling `BackendIndex`.
// ---------------------------------------------------------------------------

/// One step in the replayed sequence: either a row apply, a frontend binding,
/// or a classify probe. BOTH the production `BackendIndex` and the independent
/// reference oracle fold the SAME sequence; they must agree on every probe.
#[derive(Clone, Debug)]
enum Step {
    ApplyRow { service: u64, addrs: Vec<(u8, bool)> },
    BindFrontend { f_lo: u8, port: u16, proto_udp: bool, service: u64 },
    Classify { hi: u8, lo: u8, port: u16, proto_udp: bool, frontend: bool },
}

// Small, CORRELATED domains so frontend `bind` keys and frontend `classify`
// probes COLLIDE often — otherwise the arm-1 (`by_frontend` HIT) path that runs
// the first-by-`Ord` tie-break is essentially never reached by an uncorrelated
// random sequence, and the `min`/`max` mutant would survive EQUIV-01. `f_lo` /
// `port` are drawn from the SAME tiny pools the classify probe uses, and a
// service's backend set draws ≥2 candidate host octets so a frontend-bound
// service routinely has multiple healthy backends (exercising the tie-break).
const POOL_FRONTEND_LO: std::ops::RangeInclusive<u8> = 1..=3;
const POOL_PORT: std::ops::RangeInclusive<u16> = 1..=3;
const POOL_SERVICE: std::ops::RangeInclusive<u64> = 1..=2;

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        // ApplyRow: 0..=4 backends from a small host-octet pool (1..=6) so a
        // service routinely holds ≥2 healthy backends → the first-by-Ord
        // tie-break is genuinely exercised when that service is frontend-bound.
        (POOL_SERVICE, prop::collection::vec((1u8..=6, any::<bool>()), 0..=4))
            .prop_map(|(service, addrs)| Step::ApplyRow { service, addrs }),
        (POOL_FRONTEND_LO, POOL_PORT, any::<bool>(), POOL_SERVICE).prop_map(
            |(f_lo, port, proto_udp, service)| Step::BindFrontend {
                f_lo,
                port,
                proto_udp,
                service,
            }
        ),
        // Classify: `hi`/`port` from the SAME frontend pools (so a frontend probe
        // can HIT a bound key); `lo` from the backend host pool (so a
        // workload-subnet probe can HIT a by_addr entry).
        (POOL_FRONTEND_LO, 1u8..=6, POOL_PORT, any::<bool>(), any::<bool>()).prop_map(
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

/// The proto for a step's `proto_udp` flag.
const fn proto_of(proto_udp: bool) -> Proto {
    if proto_udp { Proto::Udp } else { Proto::Tcp }
}

/// The destination a `Classify` step probes — frontend-subnet addr OR
/// workload-subnet addr per the flag, so the sequence exercises all three arms.
/// SHARED by `drive` (which applies it to `BackendIndex`) and the oracle (which
/// re-derives the verdict for it), so both observe the byte-identical input.
fn classify_dst(hi: u8, lo: u8, port: u16, is_frontend: bool) -> SocketAddrV4 {
    if is_frontend { frontend(3, hi, port) } else { backend_addr(0, lo.max(1), port) }
}

/// Apply one step to the production `BackendIndex`, returning the classify
/// verdict when the step is a probe (so the oracle can be compared against it).
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
            index.bind_frontend(
                FrontendKey::new(frontend(3, *f_lo, *port), proto_of(*proto_udp)),
                svc(*service),
            );
            None
        }
        Step::Classify { hi, lo, port, proto_udp, frontend: is_frontend } => {
            Some(index.classify(classify_dst(*hi, *lo, *port, *is_frontend), proto_of(*proto_udp)))
        }
    }
}

/// The INDEPENDENT reference oracle for the three-way re-keyed classify — the
/// SPEC re-stated as a model, NOT a second copy of production. It folds the
/// SAME `Step` sequence into its own model state and re-derives every
/// classify-probe verdict WITHOUT calling `BackendIndex`/`FrontendKey`/`classify`.
///
/// The model carries exactly the inputs the spec consumes:
/// - `by_addr: addr → { service → healthy }` — per-contributing-service
///   readiness at each addr (the F-A ownership-aware shape; an addr is
///   resolvable while ANY service has a healthy backend there);
/// - `addrs_by_service: service → contributed addrs` — so a full-row apply
///   REPLACES exactly that service's prior addrs (full-row-write contract);
/// - `by_frontend: (F, port, proto) → service`.
///
/// `classify` is the three-way spec verbatim: (1) `by_frontend` HIT →
/// first-by-`Ord` healthy backend of that service → `Mesh` else
/// `MeshUnreachable`; (2) miss but `ip ∈ 10.98.0.0/16` → `MeshUnreachable`;
/// (3) miss outside → `by_addr` fall-through (any-healthy-at-addr → `Mesh`,
/// claimed-but-none-healthy → `MeshUnreachable`, unclaimed → `NonMesh`).
///
/// A production logic bug DIVERGES the trajectories and flips the assertion:
/// `min → max` in the first-by-`Ord` pick, flattening any arm, or dropping the
/// proto axis (so `tcp/P` and `udp/P` collide) all make `BackendIndex` disagree
/// with this oracle — REAL independent kill power, unlike a struct compared
/// against itself.
#[derive(Default)]
struct ClassifyOracle {
    by_addr: std::collections::BTreeMap<SocketAddrV4, std::collections::BTreeMap<u64, bool>>,
    addrs_by_service: std::collections::BTreeMap<u64, Vec<SocketAddrV4>>,
    by_frontend: std::collections::BTreeMap<(SocketAddrV4, Proto), u64>,
}

impl ClassifyOracle {
    /// Full-row apply: drop ONLY this service's prior addrs, then insert its
    /// current ones (the full-row-write + per-service-eviction spec).
    fn apply_row(&mut self, service: u64, addrs: &[(u8, bool)]) {
        if let Some(stale) = self.addrs_by_service.remove(&service) {
            for addr in stale {
                if let Some(by_service) = self.by_addr.get_mut(&addr) {
                    by_service.remove(&service);
                    if by_service.is_empty() {
                        self.by_addr.remove(&addr);
                    }
                }
            }
        }
        let mut contributed = Vec::new();
        for &(h, healthy) in addrs {
            let addr = backend_addr(0, h, 9000);
            self.by_addr.entry(addr).or_default().insert(service, healthy);
            contributed.push(addr);
        }
        self.addrs_by_service.insert(service, contributed);
    }

    fn bind_frontend(&mut self, f: SocketAddrV4, proto: Proto, service: u64) {
        self.by_frontend.insert((f, proto), service);
    }

    /// The smallest-by-`Ord` addr at which `service` has a healthy backend
    /// (the first-by-`Ord` tie-break), re-derived from the model — NOT via
    /// production.
    fn first_healthy_for(&self, service: u64) -> Option<SocketAddrV4> {
        self.addrs_by_service
            .get(&service)?
            .iter()
            .filter(|addr| {
                self.by_addr
                    .get(addr)
                    .and_then(|by_service| by_service.get(&service))
                    .copied()
                    .unwrap_or(false)
            })
            .copied()
            .min()
    }

    /// The three-way classify spec, re-derived independently.
    fn classify(&self, dst: SocketAddrV4, proto: Proto) -> MtlsResolution {
        // Arm 1 — by_frontend HIT.
        if let Some(&service) = self.by_frontend.get(&(dst, proto)) {
            return self
                .first_healthy_for(service)
                .map_or(MtlsResolution::MeshUnreachable, |addr| {
                    MtlsResolution::Mesh(ResolvedBackend { addr, expected_svid: None })
                });
        }
        // Arm 2 — by_frontend MISS inside the frontend subnet → fail-closed.
        // `10.98.0.0/16` membership, re-derived from the octets (NOT the prod
        // const) so a const-drift mutant in production is still caught.
        if dst.ip().octets()[0] == 10 && dst.ip().octets()[1] == 98 {
            return MtlsResolution::MeshUnreachable;
        }
        // Arm 3 — outside the subnet: by_addr fall-through.
        match self.by_addr.get(&dst) {
            Some(by_service) if by_service.values().any(|&healthy| healthy) => {
                MtlsResolution::Mesh(ResolvedBackend { addr: dst, expected_svid: None })
            }
            Some(_) => MtlsResolution::MeshUnreachable,
            None => MtlsResolution::NonMesh,
        }
    }

    /// Fold one step into the oracle, returning the expected verdict on a probe.
    fn drive(&mut self, step: &Step) -> Option<MtlsResolution> {
        match step {
            Step::ApplyRow { service, addrs } => {
                self.apply_row(*service, addrs);
                None
            }
            Step::BindFrontend { f_lo, port, proto_udp, service } => {
                self.bind_frontend(frontend(3, *f_lo, *port), proto_of(*proto_udp), *service);
                None
            }
            Step::Classify { hi, lo, port, proto_udp, frontend: is_frontend } => Some(
                self.classify(classify_dst(*hi, *lo, *port, *is_frontend), proto_of(*proto_udp)),
            ),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// S-DBN-EQUIV-01 (reference-oracle equivalence; the trait-docstring contract
    /// guard, development.md § "Trait definitions specify behavior"): the
    /// production `BackendIndex` re-keyed-classify trajectory over an arbitrary
    /// ordered sequence of (row-apply | frontend-binding | classify-probe) steps
    /// matches an INDEPENDENT reference oracle that re-derives the expected
    /// verdict at EVERY probe WITHOUT calling `BackendIndex`. The oracle is the
    /// SPEC (a model fold), not a second copy of production — so a logic bug in
    /// `classify`/`first_healthy_backend_for` (swap `min` for `max`, flatten an
    /// arm, drop the proto axis) DIVERGES the two trajectories and flips this
    /// assertion. This is real independent kill power.
    #[test]
    fn equiv_01_classify_matches_independent_reference_oracle(
        steps in prop::collection::vec(arb_step(), 1..=24),
    ) {
        let mut index = BackendIndex::default();
        let mut oracle = ClassifyOracle::default();

        // Deterministic prelude — STRUCTURALLY guarantee the arm-1 (`by_frontend`
        // HIT) first-by-`Ord` tie-break is exercised EVERY run (an uncorrelated
        // random sequence reaches it too rarely for the `min`/`max` mutant to be
        // reliably killed). Bind a frontend key to a service that holds THREE
        // healthy backends at distinct addrs, then probe that exact key: the
        // expected verdict is `Mesh(smallest-by-Ord healthy addr)`. A `min → max`
        // mutant picks the LARGEST instead → the oracle and production diverge on
        // this probe. Both fold the prelude identically, so the comparison stays
        // valid; the random suffix exercises the rest of the trajectory.
        let prelude = [
            Step::ApplyRow { service: 1, addrs: vec![(2, true), (5, true), (3, true)] },
            Step::BindFrontend { f_lo: 1, port: 1, proto_udp: false, service: 1 },
            Step::Classify { hi: 1, lo: 0, port: 1, proto_udp: false, frontend: true },
        ];

        for step in prelude.iter().chain(steps.iter()) {
            let got = drive(&mut index, step);
            let want = oracle.drive(step);
            prop_assert_eq!(
                got,
                want,
                "production BackendIndex must match the independent reference oracle at every \
                 classify-probe (the three-way spec); a classify logic bug diverges them",
            );
        }
    }

    /// S-DBN-EQUIV-01 DETERMINISM clause: the same step sequence yields a
    /// bit-identical verdict trajectory across two fresh `BackendIndex`
    /// constructions. This isolates the determinism half of the criterion — a
    /// mutant introducing iteration nondeterminism (e.g. a `HashMap` swap that
    /// reorders the first-by-`Ord` scan) would flip the byte-identity of the two
    /// trajectories. (The CORRECTNESS half — that the trajectory matches the spec
    /// — is the oracle property above; determinism alone is not correctness.)
    #[test]
    fn equiv_01_verdict_trajectory_is_deterministic(
        steps in prop::collection::vec(arb_step(), 1..=24),
    ) {
        let mut index_a = BackendIndex::default();
        let mut index_b = BackendIndex::default();

        let trajectory_a: Vec<Option<MtlsResolution>> =
            steps.iter().map(|step| drive(&mut index_a, step)).collect();
        let trajectory_b: Vec<Option<MtlsResolution>> =
            steps.iter().map(|step| drive(&mut index_b, step)).collect();

        prop_assert_eq!(
            trajectory_a,
            trajectory_b,
            "the same seed yields a bit-identical verdict trajectory (deterministic replay)",
        );
    }
}
