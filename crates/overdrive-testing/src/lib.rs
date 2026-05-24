//! Shared real-infra test fixtures consumed by multiple workspace
//! crates' integration tests.
//!
//! Per `.claude/rules/development.md` § "Shared real-infra test
//! fixtures — overdrive-testing":
//!
//! - **What lives here**: real-OS test fixtures (network namespace
//!   manipulation, veth pair setup, ip-route plumbing, sysctl tweaks)
//!   used by ≥ 2 crates' integration tests OR non-trivial enough that
//!   duplication would drift across consumers.
//! - **What does NOT live here**: pure in-process test doubles (those
//!   are Sim\* adapters in `overdrive-sim`); production code (this is
//!   a `dev-dependencies`-only crate); per-crate fixtures with only
//!   one consumer (keep those local in `tests/`).
//! - **Crate class**: `adapter-host` per ADR-0003. Real OS I/O is
//!   expected; the dst-lint gate skips this crate.
//! - **Dependency placement**: consumers add
//!   `overdrive-testing.workspace = true` under `[dev-dependencies]`
//!   only — never `[dependencies]`.
//!
//! Linux-only items are gated `#[cfg(target_os = "linux")]` at the
//! item level; consumers gate their tests the same way (or, more
//! commonly, behind their own `integration-tests` feature whose
//! body is already Linux-only).

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub mod netns;
