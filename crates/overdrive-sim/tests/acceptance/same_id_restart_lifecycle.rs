//! DISTILL RED scaffold for ADR-0089 §7's same-ID restart invariant.
//!
//! DELIVER replaces this single panic only after the exact
//! `MtlsInterceptLifecycle` production port and socket-free
//! `SimMtlsInterceptLifecycle` adapter exist. The activated test registers
//! `same-id-restart-removes-prior-protection-before-replacement-provision`
//! and drives a successful VM `StartAllocation` before a same-ID
//! `RestartAllocation`; it must not preload lifecycle ownership.

#![allow(clippy::doc_markdown)]

const SEED: u64 = 424_242;
const NAME: &str = "same-id-restart-removes-prior-protection-before-replacement-provision";

/// CONTRACT_SHAPE: bounded-change.
///
/// The activated seeded invariant must prove:
///
/// - prior lifecycle ownership is established by real dispatch as exactly
///   `alloc_id -> Live`;
/// - prior driver stop completes before lifecycle stop, structural teardown
///   and slot release, replacement provision, identity-at-driver-start,
///   driver start, and replacement lifecycle start;
/// - clean, transient lifecycle-stop, and transient structural-teardown
///   partitions are safe and converge within one additional dispatch;
/// - neither failure cut admits a later replacement event or stale/partial
///   ownership, and success records exactly one replacement driver start plus
///   lifecycle `StartCompleted` for the same allocation ID;
/// - replacement provision, identity, and driver-start failure cuts leave
///   lifecycle ownership absent and admit no later replacement event, without
///   duplicating their focused error/cleanup assertions;
/// - the report prints `SEED`, and a deletion-sensitive negative control fails
///   when stop completion is removed/reordered or `TeardownPending` is treated
///   as absence.
#[test]
#[should_panic(expected = "RED scaffold")]
fn same_id_restart_removes_prior_protection_before_replacement_provision() {
    let reproduction = format!("cargo dst --seed {SEED} --only {NAME}");
    panic!(
        "RED scaffold: ADR-0089 §7 lifecycle port/Sim adapter and registered evaluator are missing; reproduce after activation with `{reproduction}`"
    );
}
