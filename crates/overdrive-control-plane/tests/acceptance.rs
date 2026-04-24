//! Acceptance-test entrypoint for `overdrive-control-plane`.
//!
//! Per ADR-0005 and `.claude/rules/testing.md`, acceptance tests live
//! under `tests/acceptance/*.rs` and are wired into a single
//! Cargo integration-test binary via this entrypoint. The inline
//! `mod acceptance { ... }` block shifts the module lookup base into
//! the `acceptance/` subdirectory — an integration-test crate root
//! resolves `mod foo;` against `tests/foo.rs`, not
//! `tests/acceptance/foo.rs`, so the wrapping inline module is
//! load-bearing.
//!
//! Acceptance tests in this crate stay in the default unit lane —
//! they exercise only in-process serde round-trips and utoipa schema
//! emission, no real infrastructure.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]
#![allow(clippy::unwrap_used)]

mod acceptance {
    mod api_type_shapes;
    mod error_mapping_exhaustive;
    mod eval_broker_collapse;
}
