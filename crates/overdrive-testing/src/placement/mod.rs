//! The placement surface a test declares itself against.
//!
//! The closed registry of guest groups and profiles, the per-test and
//! shared-guest entry points, the shared-guest owner's pure core and the
//! Metal-run predicate.
//!
//! Every item here is the contract pinned in the feature's design
//! (`docs/feature/metal-test-placement-attribute/feature-delta.md`, "Guest kit
//! public surface" and "Owner core").

// Stays: the module re-exports `RunInGuestError` and `inside_guest` from
// `guest_suite`, which carries its own `#![cfg(target_os = "linux")]` like every
// guest-kit module of this test-support crate; without it the crate would not
// compile off Linux.
#![cfg(target_os = "linux")]
#![allow(
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    reason = "the placement surface keeps the design's signatures"
)]

use std::path::{Path, PathBuf};

pub mod groups;
/// The owner process's shell. Internal plumbing of the
/// `overdrive-shared-guest-owner` binary, which runs it; not part of the
/// placement contract.
#[doc(hidden)]
pub mod owner;
pub mod owner_core;
mod shared;

pub use crate::guest_suite::{RunInGuestError, inside_guest};
pub use groups::{GuestGroup, GuestPlacement, LanePlacement, OwnerHostScript, Sharing};

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
/// placement boots one fresh guest of the placement's group for this test; a
/// `Sharing::Module` placement attaches to the shared guest of its scope,
/// electing the scope's owner first when none serves.
///
/// # Errors
///
/// [`RunInGuestError::LaneNotPrepared`] when the guest kit (or the shared-guest
/// owner binary) is not prepared, [`RunInGuestError::GuestExecution`] when the
/// guest cannot run the test to completion (the detail names the stage), and
/// [`RunInGuestError::BodyFailed`] when the test exits unsuccessfully inside
/// the guest.
pub fn run_in_guest(
    placement: &GuestPlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    crate::guest_suite::retain_console_on_termination(placement.group, test_name)?;
    match placement.sharing {
        Sharing::Isolated => crate::guest_suite::run_isolated_guest(
            placement.group,
            placement.profile(),
            exe,
            test_name,
        ),
        Sharing::Module => {
            let endpoint = shared::endpoint_for(placement).map_err(|detail| {
                RunInGuestError::GuestExecution {
                    group: placement.group.to_owned(),
                    test: test_name.to_owned(),
                    detail: format!("stage owner: {detail}"),
                }
            })?;
            attach_shared(&endpoint, placement, test_name)
        }
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
///
/// # Errors
///
/// [`RunInGuestError::LaneNotPrepared`] naming `endpoint.owner_binary` when it
/// is absent (returned before the election lock is taken);
/// [`RunInGuestError::GuestExecution`] at stage `owner` when the connection
/// ends twice before the owner acknowledges the request or ends after it, and
/// at stage `boot`, `guest_lost` or `exec_bound` for the owner's verdicts of
/// those names; [`RunInGuestError::BodyFailed`] when the test exits
/// unsuccessfully inside the guest.
pub fn attach_shared(
    endpoint: &SharedEndpoint,
    placement: &GuestPlacement,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    shared::attach(endpoint, placement, test_name)
}

/// Runs `test_name` of the test binary `exe` in a lane guest of `placement`.
///
/// The lane guest runs the in-guest body (the test binary itself) as the
/// guest's operator command. For an owner-side placement the host also plays
/// the placement's host script against the cell.
///
/// Preconditions: the caller has found [`in_metal_run`] true and
/// [`inside_guest`] false, and the guest kit and the in-guest lane binary are
/// prepared.
///
/// # Errors
///
/// [`RunInGuestError::LaneNotPrepared`] when the test binary is not the lane's
/// binary (the detail names the fixture and the test, and nothing is started)
/// or when the lane's artifacts are not prepared;
/// [`RunInGuestError::GuestExecution`] when the guest cannot run to completion
/// or its host script fails; [`RunInGuestError::BodyFailed`] when the body
/// exits unsuccessfully inside the guest.
pub fn run_lane(
    placement: &LanePlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    crate::guest_suite::run_lane_guest(placement, exe, test_name)
}

#[cfg(test)]
mod tests;
