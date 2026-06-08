//! Acceptance scaffolds for workload-identity-manager Slice 02 and the DST
//! running-set invariant.
//!
//! Layer 2: sim adapter equivalence. Pending DISTILL scaffolds; DELIVER
//! replaces the RED bodies once `SimIdentityRead` and the invariant exist.

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

/// `@in-memory` `@dst_invariant` `@property` `@S-WIM-11` -- the DST invariant
/// eventually holds: every Running allocation has a valid held SVID and no
/// stopped allocation has one. Broken hold/drop mutations fail it.
#[test]
#[should_panic(expected = "RED scaffold")]
fn running_set_identity_invariant_fails_on_broken_hold_or_drop() {
    red_scaffold("S-WIM-11 running-set identity invariant has teeth");
}
