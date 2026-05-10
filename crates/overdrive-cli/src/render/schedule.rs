//! Schedule kind submit/alloc-status render functions + the deferral
//! tracking-URL SSOT constant.
//!
//! Slice 05 of `workload-kind-discriminator` per ADR-0047 §1. Lands
//! the Schedule-kind operator-facing surface: the submit echo names
//! "Schedule registered." plus a NOTE block referencing the GH #166
//! tracking issue, and the alloc-status render emits a Schedule-kind
//! summary that points operators at the same tracking URL.
//!
//! # KPI K5 — byte-equality of the deferral URL across surfaces
//!
//! Both [`schedule_submit_echo`] and [`schedule_alloc_status_block`]
//! read the deferral URL from the single
//! [`SCHEDULE_EXECUTION_TRACKING_URL`] constant. The byte-equality
//! property is structural — no second string literal of the same URL
//! exists anywhere in the CLI source path, so drift across surfaces
//! is impossible by construction.
//!
//! # Cron string preservation
//!
//! The alloc-status render echoes the operator-supplied cron
//! expression VERBATIM (no canonicalisation, no whitespace
//! collapse). Cron firing semantics, `ConcurrencyPolicy`, and history
//! retention are out of scope for this slice and tracked at GH #166;
//! syntactic operator input is preserved so the operator-facing
//! diagnostic is honest about what was submitted.

use overdrive_core::aggregate::ScheduleSpec;

/// Single SSOT for the Schedule-execution-deferred tracking URL.
///
/// Per slice 05 spec line 80–82 the constant value is byte-equal to
/// `https://github.com/overdrive-sh/overdrive/issues/166`. KPI K5
/// asserts that both the submit echo and the alloc status render
/// emit the same URL by reading from this constant — drift across
/// surfaces is structurally impossible.
///
/// The value is a `&'static str` so every reader sees the same
/// statically-allocated bytes; there is no constructor or
/// canonicalisation step that could re-derive a slightly different
/// form between call sites.
pub const SCHEDULE_EXECUTION_TRACKING_URL: &str =
    "https://github.com/overdrive-sh/overdrive/issues/166";

/// Render the Schedule-kind submit echo.
///
/// Per slice 05 spec the operator-facing block is:
///
/// ```text
/// Submitting schedule '<id>' (kind=Schedule)
/// Spec digest: sha256:<hex>
/// Endpoint: <url>
/// Schedule registered.
///
/// NOTE: schedule execution is not yet implemented in this Phase 1 slice.
///       The spec has been validated and persisted as intent; no Job runs
///       will be spawned automatically.
///       Tracking: <SCHEDULE_EXECUTION_TRACKING_URL>
/// ```
///
/// `spec` is the parsed-and-validated `ScheduleSpec`. `spec_digest`
/// is the 64-char lowercase-hex SHA-256 of the canonical
/// rkyv-archived `WorkloadSpec::Schedule` bytes (ADR-0002).
/// `endpoint` is the control-plane URL the trust triple names.
///
/// Taking the typed `ScheduleSpec` directly (rather than the parent
/// `WorkloadSpecInput` enum) eliminates the wrong-kind panic path —
/// the caller's dispatcher must already have matched the Schedule
/// arm before reaching this renderer, so the type system enforces
/// correctness at compile time.
#[must_use]
pub fn schedule_submit_echo(spec: &ScheduleSpec, spec_digest: &str, endpoint: &str) -> String {
    use std::fmt::Write as _;

    let cron = spec.cron_expr.as_str();
    let id = spec.job_inner.id.as_str();

    let mut s = String::new();
    let _ = writeln!(s, "Submitting schedule '{id}' (kind=Schedule)");
    let _ = writeln!(s, "Spec digest: sha256:{spec_digest}");
    let _ = writeln!(s, "Endpoint: {endpoint}");
    let _ = writeln!(s, "Cron: {cron}");
    let _ = writeln!(s, "Schedule registered.");
    let _ = writeln!(s);
    let _ = writeln!(s, "NOTE: schedule execution is not yet implemented in this Phase 1 slice.");
    let _ = writeln!(s, "      The spec has been validated and persisted as intent; no Job runs");
    let _ = writeln!(s, "      will be spawned automatically.");
    let _ = writeln!(s, "      Tracking: {SCHEDULE_EXECUTION_TRACKING_URL}");
    s
}

/// Render the Schedule-kind alloc-status block.
///
/// Per slice 05 spec the operator-facing block is:
///
/// ```text
/// Job: <id>    (kind: Schedule)
/// Cron: <cron-expr>
///
/// No allocations have been spawned yet.
///
/// Reason: Schedule execution is not yet implemented (issue #166).
///         Tracking: <SCHEDULE_EXECUTION_TRACKING_URL>
/// ```
///
/// `cron_expr` is echoed VERBATIM — the parser preserves the
/// operator's input form (slice 05 explicitly does NOT canonicalise
/// cron strings).
///
/// The `Reason:` line carries the deferral URL byte-equal to the
/// submit-echo render, sourced from the single SSOT
/// [`SCHEDULE_EXECUTION_TRACKING_URL`] constant. This is the KPI K5
/// byte-equality property.
#[must_use]
pub fn schedule_alloc_status_block(workload_id: &str, cron_expr: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "Job: {workload_id}    (kind: Schedule)");
    let _ = writeln!(s, "Cron: {cron_expr}");
    let _ = writeln!(s);
    let _ = writeln!(s, "No allocations have been spawned yet.");
    let _ = writeln!(s);
    let _ = writeln!(s, "Reason: Schedule execution is not yet implemented (issue #166).");
    let _ = writeln!(s, "        Tracking: {SCHEDULE_EXECUTION_TRACKING_URL}");
    s
}
