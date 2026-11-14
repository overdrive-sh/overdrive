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

// Stays: the module re-exports `RunInGuestError` and `inside_guest` from
// `guest_suite`, which carries its own `#![cfg(target_os = "linux")]` like every
// guest-kit module of this test-support crate; without it the crate would not
// compile off Linux.
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

use std::path::{Path, PathBuf};

pub mod groups;
pub mod owner_core;

pub use crate::guest_suite::{RunInGuestError, inside_guest};
pub use groups::{GuestGroup, GuestPlacement, LanePlacement, Sharing};

/// True when any of `OVERDRIVE_METAL_RUN_ID`, `OVERDRIVE_METAL_EXECUTION_CLASS`,
/// `OVERDRIVE_METAL_SOURCE_ROOT` is set and non-empty: the wrapper's own
/// predicate for "a Metal run context is present".
pub fn in_metal_run() -> bool {
    METAL_RUN_VARIABLES
        .iter()
        .any(|variable| std::env::var_os(variable).is_some_and(|value| !value.is_empty()))
}

/// The environment variables the Metal wrapper sets for a run it places.
const METAL_RUN_VARIABLES: [&str; 3] =
    ["OVERDRIVE_METAL_RUN_ID", "OVERDRIVE_METAL_EXECUTION_CLASS", "OVERDRIVE_METAL_SOURCE_ROOT"];

/// Runs `test_name` of the test binary `exe` in a guest of `placement`.
///
/// Preconditions: the caller has found [`in_metal_run`] true and
/// [`inside_guest`] false, and the guest kit is prepared. An isolated
/// placement boots one fresh guest of the placement's group for this test.
///
/// # Panics
///
/// A `Sharing::Module` placement reaches the shared-guest hand-off, which the
/// slice `module-tests-share-one-guest` builds.
// SCAFFOLD: true for the `Sharing::Module` arm only.
pub fn run_in_guest(
    placement: &GuestPlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    match placement.sharing {
        Sharing::Isolated => {
            crate::guest_suite::run_isolated_guest(placement.group, exe, test_name)
        }
        Sharing::Module => todo!("run_in_guest: shared-guest hand-off"),
    }
}

/// The rendezvous and owner binary of one shared scope.
///
/// On a `Sharing::Module` placement `run_in_guest` computes it (scope key, the run's runtime directory,
/// the kit's owner-binary contract path) and passes it to `attach_shared`; that
/// is its only production producer.
pub struct SharedEndpoint {
    /// The owner's unix socket.
    pub socket: PathBuf,
    /// The election lock file.
    pub lock: PathBuf,
    /// The owner binary the electing client starts.
    pub owner_binary: PathBuf,
}

/// The client half of the shared-guest protocol: attaches to the owner at
/// `endpoint` (electing one when none serves) and runs `test_name` in the shared
/// guest.
// SCAFFOLD: true
pub fn attach_shared(
    endpoint: &SharedEndpoint,
    placement: &GuestPlacement,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let _ = (endpoint, placement, test_name);
    todo!("attach_shared: owner binary check, election, attach, acknowledgement, verdict mapping")
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
