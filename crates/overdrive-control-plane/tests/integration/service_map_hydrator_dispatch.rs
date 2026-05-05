//! S-2.2-28 — Action shim writes `service_hydration_results` row on
//! dispatch.
//!
//! Tags: `@US-08` `@K8` `@slice-08` `@in-memory` `@pending`.
//!
//! Spec (Gherkin, NOT executed):
//!
//! ```gherkin
//! Given the hydrator emits `Action::DataplaneUpdateService { service_id, vip, backends, correlation }`
//! And `SimDataplane::update_service` returns `Ok(())`
//! When the action shim dispatches the action
//! Then the shim writes a `service_hydration_results` row with `status: Completed { fingerprint, applied_at: tick.now }`
//! And the row is keyed on `service_id` matching the emitted action
//! And the next reconcile tick reads the row via `actual` and observes convergence
//! ```
//!
//! See `docs/feature/phase-2-xdp-service-map/distill/test-scenarios.md`
//! for the full scenario specification.

#[tokio::test]
#[ignore = "RED scaffold S-2.2-28 — DELIVER fills the body per Slice 08"]
async fn dispatch_writes_completed_row_on_dataplane_ok() {
    panic!(
        "Not yet implemented -- RED scaffold: S-2.2-28 — \
         action shim writes service_hydration_results row on \
         Dataplane::update_service Ok(())"
    );
}
