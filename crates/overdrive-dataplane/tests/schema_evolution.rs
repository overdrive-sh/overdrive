//! Schema-evolution test entrypoint for `overdrive-dataplane`.
//!
//! Per ADR-0048 § 6 and `.claude/rules/testing.md` § "Archive schema-
//! evolution roundtrip", every rkyv-versioned envelope ships a per-
//! version golden-bytes fixture. Each test constructs the canonical
//! `V<N>` payload, hex-decodes the pinned `FIXTURE_V<N>` bytes, rkyv-
//! deserialises into the envelope, calls `into_latest()`, and asserts
//! equality against the canonical `Latest` projection.
//!
//! Pre-existing fixtures are NEVER touched — adding a new variant adds
//! a new fixture and a new assertion in the same commit.
//!
//! Submodules MUST be declared inside the inline `mod schema_evolution
//! { ... }` block — Cargo treats each `tests/*.rs` file as a crate
//! root, so a bare `mod foo;` resolves to `tests/foo.rs`, not
//! `tests/schema_evolution/foo.rs`. The inline wrapper shifts the
//! lookup base into the subdirectory.

#![allow(clippy::expect_used, clippy::expect_fun_call)]

mod schema_evolution {
    mod service_vip_allocator_entry;
}
