//! D-295-DISTILL-9 real-kernel bridge-guard lifecycle.

use std::collections::BTreeSet;

use overdrive_netlink::nft::bridge::{
    BridgeGuardDeleteOutcome, BridgeGuardMutationOutcome, BridgeGuardObservation, BridgeGuardSpec,
    converge_chain, converge_rules, converge_set, converge_table, delete_chain, delete_member,
    delete_owned_guard, delete_rules, delete_set, delete_table, insert_member, observe,
};

struct Cleanup {
    spec: BridgeGuardSpec,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let mut cleanup = || {
            let _ = delete_owned_guard(&self.spec, &BTreeSet::new());
            let _ = delete_rules(&self.spec);
            let _ = delete_set(&self.spec);
            let _ = delete_chain(&self.spec);
            let _ = delete_table(&self.spec);
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut cleanup));
    }
}

fn spec() -> BridgeGuardSpec {
    BridgeGuardSpec::new(
        format!("ovd-nd295-{}", std::process::id()),
        "prerouting".to_owned(),
        "managed_taps".to_owned(),
        -300,
        0x295a,
        0x295b,
    )
    .expect("valid isolated bridge-guard identity")
}

/// CONTRACT_SHAPE: bounded-change.
#[test]
#[ignore = "pending DELIVER step for D-295-DISTILL-9 real bridge-family codec"]
fn staged_converge_membership_idempotence_reverse_cleanup_and_exact_delete_are_real() {
    // SAFETY: `geteuid` has no memory-safety preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("SKIP bridge_guard_lifecycle: root required");
        return;
    }
    let spec = spec();
    let _cleanup = Cleanup { spec: spec.clone() };
    let empty = BTreeSet::new();
    assert!(matches!(
        observe(&spec, &empty).expect("initial read-only observation"),
        BridgeGuardObservation::Absent { .. }
    ));
    for outcome in [
        converge_table(&spec).expect("converge bridge table"),
        converge_chain(&spec).expect("converge exact base chain"),
        converge_set(&spec).expect("converge ifname set"),
        converge_rules(&spec).expect("converge ordered three-rule program"),
    ] {
        assert!(matches!(outcome, BridgeGuardMutationOutcome::Converged { .. }));
    }
    assert!(matches!(
        converge_rules(&spec).expect("exact rules are idempotent"),
        BridgeGuardMutationOutcome::Converged { .. }
    ));

    let members = BTreeSet::from(["nd295tap0".to_owned(), "nd295tap1".to_owned()]);
    for member in &members {
        assert!(matches!(
            insert_member(&spec, member).expect("insert exact member"),
            BridgeGuardMutationOutcome::Converged { .. }
        ));
        assert!(matches!(
            insert_member(&spec, member).expect("repeated insert is idempotent"),
            BridgeGuardMutationOutcome::Converged { .. }
        ));
    }
    assert!(matches!(
        observe(&spec, &members).expect("complete exact observation"),
        BridgeGuardObservation::Exact { .. }
    ));

    assert!(matches!(
        delete_owned_guard(&spec, &members).expect("delete one exact exclusive guard"),
        BridgeGuardDeleteOutcome::Deleted { .. }
    ));
    assert!(matches!(
        delete_owned_guard(&spec, &empty).expect("absent aggregate delete is idempotent"),
        BridgeGuardDeleteOutcome::Absent { .. }
    ));

    for outcome in [
        converge_table(&spec).expect("re-converge table for granular inverse"),
        converge_chain(&spec).expect("re-converge chain for granular inverse"),
        converge_set(&spec).expect("re-converge set for granular inverse"),
        converge_rules(&spec).expect("re-converge rules for granular inverse"),
    ] {
        assert!(matches!(outcome, BridgeGuardMutationOutcome::Converged { .. }));
    }
    assert!(matches!(
        insert_member(&spec, "nd295tap0").expect("insert member for granular inverse"),
        BridgeGuardMutationOutcome::Converged { .. }
    ));
    assert!(matches!(
        delete_member(&spec, "nd295tap0").expect("delete exact member"),
        BridgeGuardMutationOutcome::Converged { .. }
    ));
    assert!(matches!(
        delete_member(&spec, "nd295tap0").expect("absent member delete is idempotent"),
        BridgeGuardMutationOutcome::Converged { .. }
    ));
    for outcome in [
        delete_rules(&spec).expect("delete owned rules"),
        delete_set(&spec).expect("delete empty owned set"),
        delete_chain(&spec).expect("delete empty owned chain"),
        delete_table(&spec).expect("delete empty owned table"),
    ] {
        assert!(matches!(outcome, BridgeGuardDeleteOutcome::Deleted { .. }));
    }
    assert!(matches!(
        observe(&spec, &empty).expect("final empty observation"),
        BridgeGuardObservation::Absent { .. }
    ));
}
