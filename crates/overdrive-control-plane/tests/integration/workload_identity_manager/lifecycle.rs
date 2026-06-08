//! Integration scaffolds for workload-identity-manager.
//!
//! Layer 3: real stores plus real CA/openssl verification, gated behind the
//! `integration-tests` feature by the crate entrypoint. Pending DISTILL
//! scaffolds; DELIVER replaces the RED bodies with real end-to-end assertions.

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

/// `@walking_skeleton` `@real-io` `@adapter-integration` `@S-WIM-WS` -- an alloc
/// reaches Running, `IssueSvid` mints via the built-in CA, the SVID is held in
/// `IdentityMgr`, an audit row is observable, `openssl verify` accepts the
/// chain, and Stop drops the held entry.
#[test]
#[should_panic(expected = "RED scaffold")]
fn walking_skeleton_running_alloc_issues_holds_audits_and_verifies_svid() {
    red_scaffold("S-WIM-WS issue/hold/audit/verify/drop walking skeleton");
}

/// `@real-io` `@error` `@S-WIM-12` -- after a control-plane restart the held set
/// starts empty, every still-Running allocation is re-issued once during
/// recovery, and each re-issue leaves an `issued_certificates` audit row.
#[test]
#[should_panic(expected = "RED scaffold")]
fn restart_reissues_each_still_running_alloc_with_audit_row() {
    red_scaffold("S-WIM-12 bounded audited restart re-issue");
}
