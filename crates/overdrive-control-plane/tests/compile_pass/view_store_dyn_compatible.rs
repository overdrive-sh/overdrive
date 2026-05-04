//! Compile-pass fixture — `ViewStore` is dyn-compatible.
//!
//! Per ADR-0035 §7: the `ViewStore` trait must be dyn-compatible so the
//! `ReconcilerRuntime` (step 01-06) can hold `Arc<dyn ViewStore>` as
//! its constructor-required port-trait dependency
//! (`.claude/rules/development.md` § "Port-trait dependencies").
//!
//! If a future change adds a generic-by-method or `-> impl Trait` method
//! to the `ViewStore` trait surface (which would silently break dyn
//! compatibility), this fixture stops compiling and the trybuild
//! harness in `tests/compile_pass.rs` fails the build.

use std::sync::Arc;

use overdrive_control_plane::view_store::ViewStore;
use overdrive_sim::adapters::view_store::SimViewStore;

fn _it_compiles() {
    let _: Arc<dyn ViewStore> = Arc::new(SimViewStore::new());
}

fn main() {}
