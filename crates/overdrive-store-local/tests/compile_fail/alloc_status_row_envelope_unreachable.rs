//! S-EV-02a — cross-crate attempt to import the codec-internal
//! envelope enum `AllocStatusRowEnvelope` from the `overdrive-core`
//! crate root must fail to compile.
//!
//! Per ADR-0048 § 2 Layer 1: `AllocStatusRowEnvelope` is declared
//! `pub` at `overdrive_core::traits::observation_store` so the
//! persistence-boundary code in `overdrive-store-local` can reach it
//! via the verbose path. It is intentionally **NOT re-exported** from
//! `overdrive_core::lib.rs`. Cross-crate callers reaching for the
//! envelope at the short `overdrive_core::AllocStatusRowEnvelope`
//! path will see rustc E0432 — `unresolved import`.
//!
//! This is the load-bearing non-re-export target under the UI-02
//! alias-to-payload public API. The public re-exported
//! `AllocStatusRow = AllocStatusRowV1` payload alias remains the
//! unimpeded callers' interface for struct-literal construction.

use overdrive_core::AllocStatusRowEnvelope;

fn main() {
    let _ = std::any::TypeId::of::<AllocStatusRowEnvelope>();
}
