//! Trybuild compile-fail fixtures for ADR-0048 § 2 Layer 1.
//!
//! Layer 1 of the envelope write-enforcement story per ADR-0048 § 2
//! is **non-re-export of the codec-internal envelope enum from
//! `overdrive-core::lib.rs`**. The envelope (`AllocStatusRowEnvelope`,
//! `JobEnvelope`, …) is declared `pub` at its defining module so the
//! persistence-boundary code in `overdrive-store-local` can reach it
//! via the verbose `overdrive_core::traits::observation_store::…` path
//! — but it is NOT re-exported from `overdrive_core::lib.rs`. Public
//! callers thus see only the payload alias (`AllocStatusRow = AllocStatusRowV1`)
//! at the short path under the UI-02 alias-to-payload public API.
//!
//! The fixtures here prove the structural property: cross-crate code
//! reaching for the envelope at the short re-exported path fails to
//! compile with rustc E0432 (`unresolved import`).
//!
//! See `.claude/rules/testing.md` § "Compile-fail testing (trybuild)"
//! for the discipline — `trybuild` is pinned exactly at the workspace
//! root; the per-fixture `.stderr` files are committed verbatim and
//! regenerated only when the diagnostic shifts deliberately.

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
