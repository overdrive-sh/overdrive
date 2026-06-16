//! `SimMtlsResolve` — in-memory [`MtlsResolve`] double for DST
//! (transparent-mtls-enrollment, ADR-0071; GH #178 anti-corruption boundary).
//!
//! The sim counterpart to the v1 host `ServiceBackendsResolve`
//! (`overdrive-control-plane`, step 01-03). It models the per-connection
//! enrollment-RESOLVE **OUTCOME** the [`MtlsResolve`] contract defines —
//! classifying each captured connection's `orig_dst` into one of the three
//! [`MtlsResolution`] arms — driven entirely by a preloaded, scripted
//! `orig_dst → MtlsResolution` table the consumer injects. It performs NO real
//! store I/O — no [`ObservationStore`](overdrive_core::traits::ObservationStore),
//! no `service_backends` subscription — the scripted table IS the observable
//! contract.
//!
//! # What it models, and what it does not
//!
//! The `mtls_resolve_equivalence` DST harness (DELIVER) drives BOTH this double
//! and the host adapter through the same call sequence and asserts the SAME
//! observable outcome per arm (`.claude/rules/development.md` § "The DST
//! equivalence test is the structural guard"). The observable surface the
//! contract pins and this double mirrors:
//!
//! - **`resolve` outcome** = a pure function of the scripted table:
//!   - a `orig_dst` present in the scripted table ⇒ its scripted
//!     [`MtlsResolution`] arm (`Mesh` / `NonMesh` / `MeshUnreachable`);
//!   - a `orig_dst` ABSENT from the table ⇒ the configurable default arm
//!     (default [`NonMesh`](MtlsResolution::NonMesh) — the
//!     no-`running`-mesh-backend classification);
//!   - a scripted store-fault ⇒ an `Err` of
//!     [`MtlsResolveError::StoreUnreadable`]
//!     — a store-layer fault that is NOT a per-connection classification (the
//!     contract's asymmetry: a should-be-mesh-but-can't outcome classifies INTO
//!     the `Ok(MeshUnreachable)` arm; only a poisoned-handle / corrupt-table
//!     fault returns `Err`). The real `ObservationStore` read the host adapter
//!     performs is NOT modelled (no store I/O in the sim); the outcome IS the
//!     observable contract.
//! - **`probe`** — scriptable: `Ok(())` by default (the in-process sim substrate
//!   honours its Earned-Trust contract by construction — there is no real store
//!   to be unreadable), OR a scripted `Err` of [`MtlsResolveError::Probe`]
//!   so consumers (04-* / 05-*) can exercise the wire → probe → use composition
//!   invariant's startup-refusal (`health.startup.refused`) path.
//!
//! # Determinism
//!
//! The scripted table is a [`BTreeMap`] keyed by [`SocketAddrV4`] for
//! deterministic iteration across seeds (§ "Ordered-collection choice"; the
//! table is observed under DST). The double carries no clock / entropy / store —
//! the outcome is a deterministic function of the scripted state.
//!
//! # Dependency discipline
//!
//! `SimMtlsResolve::new` takes the scripted table + the default arm as
//! **required constructor parameters** — no builder, no production-binding
//! default of the port's behavior (`.claude/rules/development.md` §
//! "Port-trait dependencies" / "Production code is not shaped by simulation").
//! The probe/resolve-fault scripting is set explicitly via the inherent helpers
//! below.

use std::collections::BTreeMap;
use std::net::SocketAddrV4;

use async_trait::async_trait;
use overdrive_core::traits::mtls_resolve::{MtlsResolution, MtlsResolve, MtlsResolveError, Result};
use parking_lot::Mutex;

/// In-memory [`MtlsResolve`] double for DST.
///
/// Classifies each `orig_dst` against a scripted `BTreeMap<SocketAddrV4,
/// MtlsResolution>` table (a known addr ⇒ its scripted arm; an unknown addr ⇒
/// the configurable default arm). The probe outcome and a one-shot resolve
/// store-fault are scriptable through the inherent helpers so consumers can
/// exercise the fail-closed and probe-refusal paths. `Send + Sync + 'static`
/// (held as `Arc<dyn MtlsResolve>`), matching the host adapter.
pub struct SimMtlsResolve {
    /// The scripted `orig_dst → MtlsResolution` table. [`BTreeMap`], NOT
    /// `HashMap` — the table is observed under DST and its iteration order must
    /// be deterministic across seeds (§ "Ordered-collection choice").
    scripted: BTreeMap<SocketAddrV4, MtlsResolution>,
    /// The classification arm returned for any `orig_dst` ABSENT from
    /// [`scripted`](Self::scripted) — the configurable default (default
    /// [`NonMesh`](MtlsResolution::NonMesh), the no-`running`-mesh-backend
    /// classification).
    default_arm: MtlsResolution,
    /// When set, the NEXT `resolve` (for ANY `orig_dst`) returns
    /// `Err(StoreUnreadable { reason })`, consumed on use — the harness arms the
    /// store-layer fault the contract distinguishes from the per-connection
    /// `MeshUnreachable` classification. Wrapped in [`Mutex`] for interior
    /// mutability behind the `&self` trait method.
    resolve_fault: Mutex<Option<String>>,
    /// When `Some(reason)`, `probe` returns `Err(Probe { reason })` (the
    /// `health.startup.refused`-shaped startup-refusal); when `None`, `probe`
    /// returns `Ok(())`. Set at construction-adjacent scripting time, read-only
    /// thereafter — no interior mutability needed.
    probe_failure: Option<String>,
}

impl SimMtlsResolve {
    /// Construct the double from its REQUIRED inputs — the scripted
    /// `orig_dst → MtlsResolution` table and the default arm returned for any
    /// unscripted `orig_dst`. Both mandatory: no defaulting, no builder (a
    /// consumer that forgets the table fails to construct, the discipline the
    /// trait surface exists to enforce). The double starts with `probe`
    /// succeeding and no resolve fault armed; use
    /// [`with_probe_failure`](Self::with_probe_failure) /
    /// [`script_resolve_fault`](Self::script_resolve_fault) to script those.
    #[must_use]
    pub const fn new(
        scripted: BTreeMap<SocketAddrV4, MtlsResolution>,
        default_arm: MtlsResolution,
    ) -> Self {
        Self { scripted, default_arm, resolve_fault: Mutex::new(None), probe_failure: None }
    }

    /// Script `probe` to FAIL with [`MtlsResolveError::Probe`] carrying `reason`
    /// (the `health.startup.refused`-shaped startup refusal), so consumers
    /// (04-* / 05-*) can exercise the wire → probe → use composition invariant's
    /// refusal path. Consumes and returns `self` for construction-time chaining.
    #[must_use]
    pub fn with_probe_failure(mut self, reason: impl Into<String>) -> Self {
        self.probe_failure = Some(reason.into());
        self
    }

    /// DST scripting: arm a store-layer fault for the NEXT `resolve` (for ANY
    /// `orig_dst`), consumed on use. The sim has no real store to corrupt, so
    /// the harness scripts the OUTCOME the contract pins — the
    /// [`StoreUnreadable`](MtlsResolveError::StoreUnreadable) `Err` the host
    /// adapter surfaces from a poisoned handle / corrupt table, distinct from
    /// the per-connection [`MeshUnreachable`](MtlsResolution::MeshUnreachable)
    /// classification.
    pub fn script_resolve_fault(&self, reason: impl Into<String>) {
        *self.resolve_fault.lock() = Some(reason.into());
    }
}

#[async_trait]
impl MtlsResolve for SimMtlsResolve {
    async fn probe(&self) -> Result<()> {
        // Scriptable: the in-process sim substrate honours its Earned-Trust
        // contract by construction (no real store to be unreadable), unless the
        // harness armed a refusal to exercise the `health.startup.refused` path.
        self.probe_failure
            .as_ref()
            .map_or(Ok(()), |reason| Err(MtlsResolveError::Probe { reason: reason.clone() }))
    }

    async fn resolve(&self, orig_dst: SocketAddrV4) -> Result<MtlsResolution> {
        // A scripted store-layer fault (consumed on use) is the ONLY `Err` arm —
        // the contract's asymmetry: a per-connection should-be-mesh-but-can't
        // outcome classifies INTO `Ok(MeshUnreachable)`, never into `Err`. Take
        // the armed fault into a local before the `if let` so the lock guard is
        // dropped at the end of this statement, not held across the branch.
        let armed_fault = self.resolve_fault.lock().take();
        if let Some(reason) = armed_fault {
            return Err(MtlsResolveError::StoreUnreadable { reason });
        }

        // A known `orig_dst` resolves to its scripted arm; an unknown one to the
        // configurable default (default `NonMesh` — the
        // no-`running`-mesh-backend classification). Read-only and idempotent in
        // observable state: two consecutive calls for the same `orig_dst` against
        // an unchanged table (and no armed fault) produce the same arm.
        Ok(self.scripted.get(&orig_dst).cloned().unwrap_or_else(|| self.default_arm.clone()))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::net::Ipv4Addr;

    use overdrive_core::traits::mtls_resolve::ResolvedBackend;
    use proptest::prelude::*;

    use super::*;

    /// Build a [`SocketAddrV4`] from a deterministic octet-and-port quad — the
    /// generator surface the property below explores.
    fn addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
    }

    /// Drive an async future to completion on a fresh current-thread runtime —
    /// the sync `proptest!` block cannot `.await`, so each property iteration
    /// blocks on the async `resolve` (mirrors `journal.rs`'s runtime-per-case
    /// shape).
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("current-thread runtime builds")
            .block_on(fut)
    }

    /// Scenario — `sim_mtls_resolve_returns_scripted_arm_per_orig_dst`.
    ///
    /// Construct `SimMtlsResolve` with a scripted table pinning a known
    /// `orig_dst` to `Mesh(ResolvedBackend { addr, expected_svid: None })`, and
    /// assert `resolve` returns exactly that scripted arm. This is the
    /// per-`orig_dst` controllability the worker (04-02) and the walking
    /// skeleton (05-*) depend on to pin classification deterministically.
    #[tokio::test]
    async fn sim_mtls_resolve_returns_scripted_arm_per_orig_dst() {
        let orig_dst = addr(10, 0, 0, 1, 8080);
        let backend = ResolvedBackend { addr: orig_dst, expected_svid: None };

        let mut scripted = BTreeMap::new();
        scripted.insert(orig_dst, MtlsResolution::Mesh(backend.clone()));
        let sut = SimMtlsResolve::new(scripted, MtlsResolution::NonMesh);

        let resolution = sut.resolve(orig_dst).await.expect("scripted resolve is Ok");
        assert_eq!(resolution, MtlsResolution::Mesh(backend));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// PBT (criterion 1, 4, 5): for an arbitrary scripted table keyed by
        /// `orig_dst`, `resolve` returns the scripted arm for a KNOWN addr and
        /// the configurable default for an UNKNOWN addr. The property holds over
        /// every (table, queried-addr) pair the strategy generates — the
        /// per-`orig_dst` controllability is total over the address space.
        ///
        /// Universe (observable): the [`MtlsResolution`] arm returned by
        /// `resolve` for the queried `orig_dst`. Invariant: present-in-table ⇒
        /// the table's arm; absent-from-table ⇒ the default arm.
        #[test]
        fn resolve_returns_scripted_arm_for_known_and_default_for_unknown(
            // Up to 8 distinct addrs each scripted to one of the three arms.
            entries in prop::collection::vec(
                (
                    (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>(), any::<u16>()),
                    0u8..3,
                ),
                0..8,
            ),
            // The addr the property queries `resolve` with.
            query in (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>(), any::<u16>()),
        ) {
            let mut scripted = BTreeMap::new();
            for ((a, b, c, d, port), selector) in &entries {
                let arm = match selector {
                    0 => MtlsResolution::Mesh(ResolvedBackend {
                        addr: addr(*a, *b, *c, *d, *port),
                        expected_svid: None,
                    }),
                    1 => MtlsResolution::NonMesh,
                    _ => MtlsResolution::MeshUnreachable,
                };
                scripted.insert(addr(*a, *b, *c, *d, *port), arm);
            }

            let default_arm = MtlsResolution::NonMesh;
            let expected = scripted
                .get(&addr(query.0, query.1, query.2, query.3, query.4))
                .cloned()
                .unwrap_or_else(|| default_arm.clone());

            let sut = SimMtlsResolve::new(scripted, default_arm);
            let got = block_on(sut.resolve(addr(query.0, query.1, query.2, query.3, query.4)))
                .expect("resolve with no armed fault is Ok");

            prop_assert_eq!(got, expected);
        }
    }

    /// Criterion 5 — unknown-addr ⇒ the configurable `NonMesh` default (the
    /// no-`running`-mesh-backend classification arm).
    #[tokio::test]
    async fn unknown_addr_resolves_to_configured_default_nonmesh() {
        // Empty table: every addr is unknown.
        let sut = SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh);
        let got = sut.resolve(addr(203, 0, 113, 7, 443)).await.expect("unknown resolve is Ok");
        assert_eq!(got, MtlsResolution::NonMesh);
    }

    /// Criterion 2 / 5 — a scripted `MeshUnreachable` is an `Ok` arm (the
    /// per-connection should-be-mesh-but-can't fail-closed outcome the worker
    /// acts on), NOT an `Err`.
    #[tokio::test]
    async fn scripted_mesh_unreachable_is_an_ok_fail_closed_arm() {
        let orig_dst = addr(10, 0, 0, 9, 9000);
        let mut scripted = BTreeMap::new();
        scripted.insert(orig_dst, MtlsResolution::MeshUnreachable);
        let sut = SimMtlsResolve::new(scripted, MtlsResolution::NonMesh);

        let got = sut.resolve(orig_dst).await.expect("scripted MeshUnreachable is Ok");
        assert_eq!(got, MtlsResolution::MeshUnreachable);
    }

    /// Criterion 2 / 5 — a scripted store-layer fault returns
    /// `Err(StoreUnreadable)`, distinct from the per-connection
    /// `Ok(MeshUnreachable)` classification (the contract's asymmetry).
    #[tokio::test]
    async fn scripted_resolve_fault_returns_store_unreadable_err() {
        let sut = SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh);
        sut.script_resolve_fault("poisoned service_backends handle");

        let err = sut.resolve(addr(10, 0, 0, 1, 8080)).await.expect_err("armed fault returns Err");
        assert!(
            matches!(&err, MtlsResolveError::StoreUnreadable { reason } if reason == "poisoned service_backends handle"),
            "expected StoreUnreadable with the scripted reason, got {err:?}",
        );
    }

    /// Criterion 3 / 5 — `probe` succeeds (`Ok(())`) by default (the in-process
    /// substrate honours its Earned-Trust contract by construction).
    #[tokio::test]
    async fn probe_succeeds_by_default() {
        let sut = SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh);
        sut.probe().await.expect("default probe is Ok");
    }

    /// Criterion 3 / 5 — a scripted probe failure returns
    /// `Err(Probe)` (the `health.startup.refused`-shaped startup refusal the
    /// composition root acts on).
    #[tokio::test]
    async fn scripted_probe_failure_returns_probe_err() {
        let sut = SimMtlsResolve::new(BTreeMap::new(), MtlsResolution::NonMesh)
            .with_probe_failure("service_backends surface unreadable at startup");

        let err = sut.probe().await.expect_err("scripted probe failure returns Err");
        assert!(
            matches!(&err, MtlsResolveError::Probe { reason } if reason == "service_backends surface unreadable at startup"),
            "expected Probe with the scripted reason, got {err:?}",
        );
    }
}
