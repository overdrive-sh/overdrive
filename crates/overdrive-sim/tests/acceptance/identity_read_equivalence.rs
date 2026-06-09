//! Acceptance scaffolds for workload-identity-manager Slice 02 and the DST
//! running-set North-Star invariant (Slice 01 CAPSTONE).
//!
//! Layer 2: sim adapter equivalence + the held-SVID convergence invariant.
//! S-WIM-06 (`sim_identity_read_matches_identity_mgr_contract`) is ACTIVATED
//! here (step 02-02): it is the STRUCTURAL GUARD for the `IdentityRead` trait
//! contract (`.claude/rules/development.md` § "The DST equivalence test is the
//! structural guard"; ADR-0067 D7/D9), mirroring ADR-0063's `ca_equivalence`.
//! S-WIM-11 (`running_set_identity_invariant_fails_on_broken_hold_or_drop`) is
//! ACTIVATED in 01-07: it drives the North-Star held-SVID convergence invariant
//! (ADR-0067 D9, O1 / K1) through the DST harness driving port, proves its
//! TEETH (a deliberately-broken executor fails the held-vs-running relation),
//! and proves twin-run determinism (K5 — bit-identical verdict from a seed).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_control_plane::identity_mgr::IdentityMgr;
use overdrive_core::traits::ca::{CaCertDer, CaCertPem, CaKeyPem, SvidMaterial, TrustBundle};
use overdrive_core::traits::identity_read::IdentityRead;
use overdrive_core::wall_clock::UnixInstant;
use overdrive_core::{AllocationId, CertSerial, SpiffeId};
use overdrive_sim::adapters::SimIdentityRead;
use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;
use overdrive_sim::invariants::svid_running_set::{
    ExecutorDefect, drive_churn_with_executor_defect,
};

/// A contract-proving test consumer that takes `Arc<dyn IdentityRead>` as a
/// **MANDATORY constructor parameter** — no default, no `with_*` builder
/// (`.claude/rules/development.md` § "Port-trait dependencies": "optional means
/// tests can forget"). This DEMONSTRATES the port-trait dependency discipline as
/// a contract (ADR-0067 D7): the production consumers (sockops #26 / gateway /
/// telemetry) are deferred to those features, so this fixture is the only
/// Slice-02 proof that a consumer is wired the required-param way. It is a thin
/// pass-through to the injected port — its only job is to make "the dependency
/// is mandatory in `new()`" a compile-time fact.
struct IdentityConsumer {
    identity: Arc<dyn IdentityRead>,
}

impl IdentityConsumer {
    /// Construct over a REQUIRED `IdentityRead` port. The dependency is
    /// mandatory at construction — a consumer that forgets to inject it fails to
    /// compile, which is the whole point of the discipline.
    fn new(identity: Arc<dyn IdentityRead>) -> Self {
        Self { identity }
    }

    fn svid_for(&self, alloc: &AllocationId) -> Option<SvidMaterial> {
        self.identity.svid_for(alloc)
    }

    fn current_bundle(&self) -> Option<TrustBundle> {
        self.identity.current_bundle()
    }
}

/// Build a `SvidMaterial` for `(spiffe, not_after_secs)` with fixture cert/key
/// bytes — the equivalence asserts on the trait-observable `SvidMaterial`
/// (`PartialEq`/`Eq` over all observable parts), so both adapters MUST be
/// preloaded from the SAME fixture material per alloc. Mirrors the
/// `IdentityMgr` unit-test fixture shape; `overdrive-sim` owns these fixtures.
fn svid(spiffe: &str, not_after_secs: u64) -> SvidMaterial {
    SvidMaterial::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into()),
        CaCertDer::new(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        CertSerial::new("0badc0de").expect("serial parses"),
        SpiffeId::new(spiffe).expect("valid SpiffeId"),
        CaKeyPem::new("-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into()),
        UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    )
}

fn alloc(id: &str) -> AllocationId {
    AllocationId::new(id).expect("valid AllocationId")
}

fn bundle() -> TrustBundle {
    TrustBundle::new(
        CaCertPem::new("-----BEGIN CERTIFICATE-----\nROOT\n-----END CERTIFICATE-----\n".into()),
        Some(CaCertPem::new(
            "-----BEGIN CERTIFICATE-----\nINTERMEDIATE\n-----END CERTIFICATE-----\n".into(),
        )),
    )
}

/// `@in-memory` `@property` `@S-WIM-06` -- `IdentityMgr` (the host holder) and
/// `SimIdentityRead` (the sim double) return equivalent observable values
/// through the same `IdentityRead` calls.
///
/// THE STRUCTURAL GUARD (`.claude/rules/development.md` § "The DST equivalence
/// test is the structural guard"; ADR-0067 D7/D9), the `IdentityRead` mirror of
/// ADR-0063's `ca_equivalence`. It drives the REAL `IdentityMgr` read path AND
/// `SimIdentityRead` through the SAME call sequence — both preloaded with the
/// SAME fixture SVID(s) + bundle (`IdentityMgr` via `hold` + `new(Some(bundle))`,
/// `SimIdentityRead` via its preloaded constructor) — and asserts IDENTICAL
/// observable reads. When this fails, exactly one of {contract, `IdentityMgr`,
/// `SimIdentityRead`} is wrong, and the test isolates which.
///
/// # Port-to-port
///
/// Both adapters are exercised through the `IdentityRead` driving port (and via
/// the required-param `IdentityConsumer`, proving the consumer discipline);
/// observable outcomes are asserted on the port's own return values
/// (`SvidMaterial` / `TrustBundle` / `None`) — trait-observable values ONLY,
/// never internal fields. The five observable clauses (D7) are covered:
/// held → equivalent `Some`; absent → `None` in both; bundle → equivalent;
/// post-drop → `None` in both (clause 5).
///
/// # `@property` — generative over the same fixture call sequence
///
/// Property-based over an arbitrary set of `(alloc, spiffe, not_after)` held
/// entries plus a disjoint set of absent allocs: for EVERY held alloc both
/// adapters return the equivalent `Some(svid)`; for EVERY absent alloc both
/// return `None`; the bundle is equivalent across both; and after dropping one
/// held alloc, both return `None` for it while the survivors stay equivalent.
#[test]
fn sim_identity_read_matches_identity_mgr_contract() {
    use proptest::prelude::*;
    use proptest::test_runner::{Config, TestRunner};

    // A held entry: a unique alloc index, a spiffe path component, and a
    // not_after second. The strategy keeps the alloc indices distinct so the
    // BTreeMap keys never collide (the held map is keyed by AllocationId).
    let held_entries = proptest::collection::vec(
        (0u32..32, 0u32..1_000_000, 1_600_000_000u64..1_900_000_000),
        0..6,
    );
    // Disjoint absent indices (offset past the held index range so they are
    // never held).
    let absent_indices = proptest::collection::vec(100u32..132, 0..4);

    let mut runner = TestRunner::new(Config { cases: 64, ..Config::default() });
    runner
        .run(&(held_entries, absent_indices), |(raw_held, raw_absent)| {
            // Deduplicate held by alloc index — last write wins, mirroring the
            // BTreeMap insert both adapters use.
            let mut held: BTreeMap<AllocationId, SvidMaterial> = BTreeMap::new();
            for (idx, spiffe_seed, not_after) in raw_held {
                let a = alloc(&format!("alloc-held{idx:03}-0"));
                let spiffe = format!("spiffe://overdrive.local/job/j{spiffe_seed}/alloc/held{idx}");
                held.insert(a, svid(&spiffe, not_after));
            }
            let absent: Vec<AllocationId> = raw_absent
                .into_iter()
                .map(|idx| alloc(&format!("alloc-absent{idx:03}-0")))
                .collect();

            let trust = bundle();

            // GIVEN both adapters preloaded with the SAME fixture material,
            // each behind the required-param `IdentityConsumer` (proving the
            // port-trait discipline — the dependency is mandatory in `new()`).
            // IdentityMgr (host holder): construct with the bundle, hold each
            // fixture SVID via the production mutator, THEN inject as the port.
            let mgr = IdentityMgr::new(Some(trust.clone()));
            for (a, material) in &held {
                mgr.hold(a.clone(), material.clone());
            }
            let mgr_consumer = IdentityConsumer::new(Arc::new(mgr));
            // SimIdentityRead (sim double): preload the same map + bundle, then
            // inject as the port.
            let sim_consumer = IdentityConsumer::new(Arc::new(SimIdentityRead::new(
                held.clone(),
                Some(trust.clone()),
            )));

            // CLAUSE 1 — every held alloc returns equivalent SvidMaterial from
            // both adapters, read THROUGH the injected `IdentityRead` port.
            for a in held.keys() {
                let from_mgr = mgr_consumer.svid_for(a);
                let from_sim = sim_consumer.svid_for(a);
                prop_assert_eq!(
                    &from_mgr,
                    &from_sim,
                    "held alloc {} must read equivalently from IdentityMgr and SimIdentityRead",
                    a.as_str()
                );
                prop_assert!(from_mgr.is_some(), "a held alloc reads Some from both adapters");
            }

            // CLAUSE 2 — absent allocs return None (explicit absence) in both.
            for a in &absent {
                prop_assert_eq!(
                    mgr_consumer.svid_for(a),
                    None,
                    "absent alloc {} reads None from IdentityMgr (explicit absence)",
                    a.as_str()
                );
                prop_assert_eq!(
                    sim_consumer.svid_for(a),
                    None,
                    "absent alloc {} reads None from SimIdentityRead (explicit absence)",
                    a.as_str()
                );
            }

            // CLAUSE 3/4 — current_bundle returns equivalent trust-bundle
            // material from both, read THROUGH the same injected
            // `Arc<dyn IdentityRead>` port.
            prop_assert_eq!(
                mgr_consumer.current_bundle(),
                sim_consumer.current_bundle(),
                "current_bundle must read equivalently through the IdentityRead port"
            );

            // CLAUSE 5 — after a drop, svid_for(alloc) == None in BOTH; the
            // survivors stay equivalent (drop-on-stop is observable through the
            // read surface, O2/K2). The host arm exercises the REAL
            // `IdentityMgr::drop_svid` mutator (the `DropSvid` executor's effect);
            // the sim double has no in-place drop (it is a preloaded read
            // double) — its post-drop snapshot is the same preloaded map MINUS
            // the dropped entry, per the dispatch.
            if let Some(dropped) = held.keys().next().cloned() {
                let mgr_after = IdentityMgr::new(Some(trust.clone()));
                for (a, material) in &held {
                    mgr_after.hold(a.clone(), material.clone());
                }
                mgr_after.drop_svid(&dropped);

                let mut after = held.clone();
                after.remove(&dropped);
                let sim_after = SimIdentityRead::new(after.clone(), Some(trust));

                prop_assert_eq!(
                    mgr_after.svid_for(&dropped),
                    None,
                    "post-drop: IdentityMgr returns None for the dropped alloc (clause 5)"
                );
                prop_assert_eq!(
                    sim_after.svid_for(&dropped),
                    None,
                    "post-drop: the sim-without-that-entry returns None for the dropped alloc"
                );
                // Survivors still read equivalently across both adapters.
                for a in after.keys() {
                    prop_assert_eq!(
                        &mgr_after.svid_for(a),
                        &sim_after.svid_for(a),
                        "post-drop survivor {} reads equivalently from both adapters",
                        a.as_str()
                    );
                }
            }

            Ok(())
        })
        .expect(
            "IdentityMgr and SimIdentityRead agree on every observable read across the property",
        );
}

/// `@in-memory` `@dst_invariant` `@property` `@S-WIM-11` -- the North-Star DST
/// invariant (ADR-0067 D9, O1 / K1): the held-SVID set eventually converges
/// against the running-allocation set — every Running alloc holds a valid SVID
/// AND no Stopped alloc holds one — AND a deliberately-broken executor (drops
/// the hold, or fails to drop on stop) makes it FAIL.
///
/// This is the Slice 01 CAPSTONE: it PROVES the riskiest assumption ("identity
/// warrants its own convergence target") by driving the REAL svid-lifecycle
/// convergence loop (the pure `SvidLifecycle` reconciler + the `issue_svid` /
/// `drop_svid` action-shim executors over `SimCa` + `SimObservationStore` +
/// `IdentityMgr`) through allocations churning Running↔Stopped.
///
/// # Port-to-port
///
/// The driving port is the DST harness (`Harness::only(...).run(seed)`) — the
/// same surface `cargo dst --only svid-running-set-holds-valid-svid` drives.
/// The observable outcome is asserted at the `RunReport` boundary (the named
/// invariant is present + green) for the healthy path, and at the held
/// `BTreeMap` + `issued_certificates` audit + `alloc_status` driven-port
/// boundaries (via the evaluator's churn driver) for the teeth proof.
///
/// # Why one test, not three
///
/// The three assertions (healthy convergence, teeth, twin-run determinism) are
/// one behavioral unit — "the North-Star invariant is real and has teeth and
/// is deterministic." Splitting them would re-run the same churn fixture three
/// times for no additional behavioral coverage; the single test body asserts
/// all three observable outcomes of the one North-Star behavior.
#[test]
fn running_set_identity_invariant_fails_on_broken_hold_or_drop() {
    const SEED: u64 = 0x5111_d000;

    // (1) HEALTHY North-Star — the held-SVID set converges against the running
    // set, surfaced GREEN through the harness driving port (the same surface
    // `cargo dst --only svid-running-set-holds-valid-svid` drives).
    let report = Harness::new()
        .only(Invariant::SvidRunningSetHoldsValidSvid)
        .run(SEED)
        .expect("harness composes");
    let result = report
        .invariants
        .iter()
        .find(|r| r.name == "svid-running-set-holds-valid-svid")
        .expect("the North-Star invariant ran (it is on the critical path)");
    assert_eq!(
        result.status,
        InvariantStatus::Pass,
        "K1/O1 + K2/O2 — every Running alloc holds a valid SVID and no Stopped alloc holds one: {:?}",
        result.cause
    );

    // (2) TEETH (ADR-0067 D9, load-bearing) — a deliberately-broken executor
    // MUST make the held-vs-running relation FAIL. A scenario test without
    // teeth is a smell (`.claude/rules/testing.md` § Tier 1). We drive the
    // SAME churn through the evaluator's driving port with each defect injected
    // and assert the relation is violated (Err) — falsifiability the green run
    // alone does not prove.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds for the teeth drive");

    // 2a — a broken IssueSvid (mints but never holds): a Running alloc reads
    // `¬held` forever. The invariant MUST catch the unsatisfied
    // `running ∧ ¬held` relation.
    let drops_hold =
        rt.block_on(drive_churn_with_executor_defect(Some(ExecutorDefect::DropsTheHold)));
    assert!(
        drops_hold.is_err(),
        "TEETH: a broken IssueSvid that drops the hold MUST fail the held-vs-running relation \
         (running ∧ ¬held), but the invariant passed — it has no teeth"
    );
    assert!(
        drops_hold.as_ref().unwrap_err().contains("running ∧ ¬held"),
        "the DropsTheHold defect must fail on the running-but-not-held relation, got: {drops_hold:?}"
    );

    // 2b — a broken DropSvid (never drops on stop): a Stopped alloc's held leaf
    // key stays reachable. The invariant MUST catch the leaked
    // `¬running ∧ held` relation (O2/K2 — leak resistance on stop).
    let fails_drop =
        rt.block_on(drive_churn_with_executor_defect(Some(ExecutorDefect::FailsToDropOnStop)));
    assert!(
        fails_drop.is_err(),
        "TEETH: a broken DropSvid that fails to drop on stop MUST fail the held-vs-running \
         relation (¬running ∧ held), but the invariant passed — it has no teeth"
    );
    assert!(
        fails_drop.as_ref().unwrap_err().contains("¬running ∧ held"),
        "the FailsToDropOnStop defect must fail on the held-but-not-running relation, got: \
         {fails_drop:?}"
    );

    // 2c — sanity: the SAME churn with NO defect converges cleanly (the teeth
    // failures above are caused by the defect, not by a flaky harness).
    let healthy = rt.block_on(drive_churn_with_executor_defect(None));
    assert!(
        healthy.is_ok(),
        "the undefective churn must converge — the teeth failures must be caused by the defect, \
         not a broken fixture: {healthy:?}"
    );

    // (3) TWIN-RUN DETERMINISM (K5) — a second harness run at the SAME seed
    // produces a bit-identical verdict for the North-Star invariant. The held
    // `BTreeMap` iterates deterministically, `SimCa` draws serials from the
    // seeded entropy, and the fixture cert/key bytes are `const`. Flaky DST is
    // a sim-layer bug, never a rerun (`.claude/rules/testing.md` § Tier 1).
    let report_again = Harness::new()
        .only(Invariant::SvidRunningSetHoldsValidSvid)
        .run(SEED)
        .expect("harness composes (second run)");
    let result_again = report_again
        .invariants
        .iter()
        .find(|r| r.name == "svid-running-set-holds-valid-svid")
        .expect("the North-Star invariant ran on the second pass");
    assert_eq!(
        result, result_again,
        "the same seed reproduces the North-Star verdict bit-for-bit (seed {SEED:#x})"
    );
}

/// The North-Star invariant is a NAMED catalogue variant on the `cargo dst`
/// critical path (no inline string literal — house convention) and round-trips
/// losslessly through `FromStr → Display`. The whole subsystem exists to
/// satisfy this relation, so it must be a first-class, default-run invariant.
#[test]
fn svid_running_set_invariant_is_a_named_catalogue_variant() {
    let by_name = Invariant::from_str("svid-running-set-holds-valid-svid")
        .expect("the North-Star variant resolves by its canonical kebab name");
    assert_eq!(
        by_name,
        Invariant::SvidRunningSetHoldsValidSvid,
        "the canonical name maps to the named variant — no inline literal"
    );
    assert_eq!(
        by_name.to_string(),
        "svid-running-set-holds-valid-svid",
        "Display round-trips the canonical name"
    );
    assert!(
        Invariant::ALL.contains(&Invariant::SvidRunningSetHoldsValidSvid),
        "the North-Star invariant is on the default `cargo dst` critical path"
    );
}
