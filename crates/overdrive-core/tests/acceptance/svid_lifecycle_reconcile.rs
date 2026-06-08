//! Acceptance scaffolds for workload-identity-manager Slice 01 / Slice 03.
//!
//! Layer 1: pure reconciler and typed-contract scenarios for ADR-0067.
//! These are pending DISTILL scaffolds; DELIVER replaces the RED bodies with
//! real `SvidLifecycle` assertions once the type surface exists.

fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

/// `@in-memory` `@property` `@S-WIM-01` -- a Running allocation without held
/// SVID emits exactly one `Action::IssueSvid` and no CA I/O happens inside
/// `reconcile()`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn running_alloc_without_held_svid_emits_issue_svid() {
    red_scaffold("S-WIM-01 running alloc without held SVID emits IssueSvid");
}

/// `@in-memory` `@S-WIM-03` -- a stopped allocation that still has a held
/// SVID emits `Action::DropSvid` so the leaf key becomes unreachable.
#[test]
#[should_panic(expected = "RED scaffold")]
fn stopped_alloc_with_held_svid_emits_drop_svid() {
    red_scaffold("S-WIM-03 stopped alloc with held SVID emits DropSvid");
}

/// `@in-memory` `@property` `@S-WIM-08` -- the View is retry memory only:
/// `IssueRetry { attempts, last_failure_seen_at }`, with no serial,
/// `issued_at`, `spiffe_id`, `expires_at`, or `next_renewal_at` success fact.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_lifecycle_view_is_retry_memory_only() {
    red_scaffold("S-WIM-08 View is retry memory only");
}

/// `@in-memory` `@error` `@S-WIM-09` -- the #40 near-expiry branch is
/// structurally present but emit-gated until `cert_rotation` is registered,
/// so #35 never emits `UnknownWorkflow` every tick.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered() {
    red_scaffold("S-WIM-09 rotation seam is emit-gated");
}

/// `@in-memory` `@S-WIM-10` -- `WorkloadLifecycle` and the exit observer enqueue
/// `SvidLifecycle` evaluation on alloc Running/Stopped transitions. Without this,
/// the pure reconciler can be correct but unreachable.
#[test]
#[should_panic(expected = "RED scaffold")]
fn workload_lifecycle_transitions_enqueue_svid_lifecycle() {
    red_scaffold("S-WIM-10 lifecycle transitions enqueue SvidLifecycle");
}
