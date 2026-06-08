//! Acceptance scaffolds for workload-identity-manager Slice 01.
//!
//! Layer 1/2: action-shim executor contract. Pending DISTILL scaffolds; the
//! real bodies will drive the action shim through its public dispatch surface.

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

/// `@in-memory` `@S-WIM-02` -- `Action::IssueSvid` calls
/// `ca_issuance::issue_and_audit`, observes the `issued_certificates` audit row,
/// then holds the returned `SvidMaterial` in `IdentityMgr`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn issue_svid_executor_audits_before_hold() {
    red_scaffold("S-WIM-02 IssueSvid audits before hold");
}

/// `@in-memory` `@error` `@S-WIM-07` -- if the audit write fails, issuance is
/// refused and no unaudited SVID is placed in the held map.
#[test]
#[should_panic(expected = "RED scaffold")]
fn audit_write_failure_refuses_hold() {
    red_scaffold("S-WIM-07 audit-write failure refuses hold");
}
