//! Acceptance scaffolds for workload-identity-manager Slice 02.
//!
//! Layer 1/2: in-process `IdentityRead` contract. Pending DISTILL scaffolds;
//! DELIVER replaces the RED bodies once `IdentityMgr` and `IdentityRead` exist.

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

/// `@in-memory` `@S-WIM-04` -- `IdentityRead::svid_for` returns the held SVID
/// for an allocation and `current_bundle` returns the hydrated trust bundle
/// without issuing a new cert on the read path.
#[test]
#[should_panic(expected = "RED scaffold")]
fn identity_read_returns_svid_and_trust_bundle_without_reissue() {
    red_scaffold("S-WIM-04 IdentityRead returns held SVID and trust bundle");
}

/// `@in-memory` `@error` `@S-WIM-05` -- after `DropSvid`, the same read port
/// returns absence explicitly, so consumers fail closed instead of presenting
/// stale identity.
#[test]
#[should_panic(expected = "RED scaffold")]
fn identity_read_returns_none_after_drop() {
    red_scaffold("S-WIM-05 IdentityRead returns None after DropSvid");
}
