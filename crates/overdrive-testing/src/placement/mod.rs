//! The placement surface a test declares itself against.
//!
//! The closed registry of guest groups and profiles, the per-test and
//! shared-guest entry points, the shared-guest owner's pure core and the
//! Metal-run predicate.
//!
//! Every item here is the contract pinned in the feature's design
//! (`docs/feature/metal-test-placement-attribute/feature-delta.md`, "Guest kit
//! public surface" and "Owner core"). Bodies marked `SCAFFOLD: true` are
//! scaffolds: the exact signature is authorised by the design and the body is
//! built by the slice named beside it.

#![cfg(target_os = "linux")]
#![allow(
    clippy::todo,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    dead_code,
    reason = "scaffold bodies are `todo!()` until their delivery slice builds them"
)]

use std::path::Path;

pub mod groups;
pub mod owner_core;

pub use crate::guest_suite::{RunInGuestError, inside_guest};
pub use groups::{GuestGroup, GuestPlacement, LanePlacement, Sharing};

/// True when any of `OVERDRIVE_METAL_RUN_ID`, `OVERDRIVE_METAL_EXECUTION_CLASS`,
/// `OVERDRIVE_METAL_SOURCE_ROOT` is set and non-empty: the wrapper's own
/// predicate for "a Metal run context is present".
// SCAFFOLD: true
pub fn in_metal_run() -> bool {
    todo!("in_metal_run: read the three Metal variables through metal_run_context_present")
}

/// The pure core of [`in_metal_run`]: each argument is the variable's value
/// when it is set.
// SCAFFOLD: true
pub(crate) fn metal_run_context_present(
    run_id: Option<&str>,
    execution_class: Option<&str>,
    source_root: Option<&str>,
) -> bool {
    let _ = (run_id, execution_class, source_root);
    todo!("metal_run_context_present: true iff any argument is Some and non-empty")
}

/// Runs `test_name` of the test binary `exe` in a guest of `placement`.
// SCAFFOLD: true
pub fn run_in_guest(
    placement: &GuestPlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let _ = (placement, exe, test_name);
    todo!("run_in_guest: typed placement, isolated or shared")
}

/// Runs the lane placement of `test_name` in a lane guest.
// SCAFFOLD: true
pub fn run_lane(
    placement: &LanePlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let _ = (placement, exe, test_name);
    todo!("run_lane: lane guest, in-guest body plus owner scripts")
}

#[cfg(test)]
mod tests;
