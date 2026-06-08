//! Acceptance scaffolds for workload-identity-manager Slice 02 and the DST
//! running-set North-Star invariant (Slice 01 CAPSTONE).
//!
//! Layer 2: sim adapter equivalence + the held-SVID convergence invariant.
//! S-WIM-06 (`sim_identity_read_matches_identity_mgr_contract`) is left as a
//! RED scaffold — it is 02-02's. S-WIM-11
//! (`running_set_identity_invariant_fails_on_broken_hold_or_drop`) is
//! ACTIVATED here: it drives the North-Star held-SVID convergence invariant
//! (ADR-0067 D9, O1 / K1) through the DST harness driving port, proves its
//! TEETH (a deliberately-broken executor fails the held-vs-running relation),
//! and proves twin-run determinism (K5 — bit-identical verdict from a seed).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::str::FromStr;

use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;
use overdrive_sim::invariants::svid_running_set::{
    ExecutorDefect, drive_churn_with_executor_defect,
};

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

/// `@in-memory` `@property` `@S-WIM-06` -- `IdentityMgr` and
/// `SimIdentityRead` return equivalent observable values through the same
/// `IdentityRead` calls.
#[test]
#[should_panic(expected = "RED scaffold")]
fn sim_identity_read_matches_identity_mgr_contract() {
    red_scaffold("S-WIM-06 SimIdentityRead matches IdentityMgr contract");
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
