//! HTTP/JSON wire-shape types — the third corner of the Rust type-family
//! universe per ADR-0051 (Accepted 2026-05-15).
//!
//! Three distinct type families exist for "a workload":
//!
//! | Layer | Type | Encoding | Module |
//! |---|---|---|---|
//! | TOML parser | `WorkloadSpec` / `WorkloadSpecInput` | TOML | [`crate::aggregate::workload_spec`] (ADR-0047) |
//! | HTTP wire | [`SubmitSpecInput`](submit::SubmitSpecInput) | JSON | this module (ADR-0051) |
//! | Persisted | [`WorkloadIntent`](crate::aggregate::WorkloadIntent) | rkyv envelope | [`crate::aggregate`] (ADR-0050) |
//!
//! Each layer evolves on its own cadence. The wire layer is JSON-only;
//! ADR-0048's rkyv envelope discipline does NOT apply here. Validation
//! happens at the wire → intent boundary inside per-kind validating
//! constructors on the intent payloads — see
//! [`crate::aggregate::JobV1::from_submit`],
//! [`crate::aggregate::ServiceV1::from_submit`], and
//! [`crate::aggregate::ScheduleV1::from_submit`].

pub mod submit;

pub use submit::{ListenerInput, ScheduleSpecInput, ServiceSpecInput, SubmitSpecInput};
