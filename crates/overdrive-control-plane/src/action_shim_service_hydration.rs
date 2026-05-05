//! Action shim for `Action::DataplaneUpdateService` per
//! architecture.md § 7 / § 9 + ADR-0042.
//!
//! **Module location note** — DESIGN locks the canonical path as
//! `crates/overdrive-control-plane/src/action_shim/service_hydration.rs`,
//! which requires converting `action_shim.rs` into a directory
//! module. That conversion is a structural refactor (DELIVER's
//! responsibility per the orchestrator brief — DISTILL ships
//! scaffolds, not Cargo or module-tree restructures). DELIVER's
//! first GREEN commit on Slice 08 moves this file's contents
//! into the canonical path.
//!
//! Until then, this scaffold lives at the sibling path so the
//! file exists and other modules can reference it; the
//! `mod action_shim_service_hydration;` declaration in `lib.rs`
//! is the one DELIVER renames.
//!
//! Responsibilities per architecture.md § 7 *Failure surface*:
//!
//! - On `Ok(())` — write `service_hydration_results` row with
//!   `status: Completed { fingerprint, applied_at: tick.now }`.
//! - On `Err(DataplaneError::*)` — write a row with
//!   `status: Failed { reason: Display::to_string(&err),
//!   failed_at: tick.now }`.
//!
//! Failure surface is **observation, NOT `TerminalCondition`**
//! per architecture.md § 7 — service hydration cannot terminate
//! an allocation; mixing the channels would erode ADR-0037's
//! "every terminal claim has a single typed source" invariant.
//!
//! **RED scaffold** — every body panics via `todo!()` until
//! DELIVER fills it per Slice 08.

#![allow(dead_code)]

use thiserror::Error;

use overdrive_core::traits::dataplane::DataplaneError;
use overdrive_core::traits::observation_store::ObservationStoreError;

/// Dispatch error for the service-hydration shim. Pass-through
/// embedding via `#[from]` per `.claude/rules/development.md`
/// § Errors / pass-through embedding.
#[derive(Debug, Error)]
pub enum ServiceHydrationDispatchError {
    /// `Dataplane::update_service` returned an error.
    #[error("dataplane update_service failed: {source}")]
    Dataplane {
        #[from]
        source: DataplaneError,
    },
    /// Writing the `service_hydration_results` row failed.
    #[error("observation store write failed: {source}")]
    ObservationWrite {
        #[from]
        source: ObservationStoreError,
    },
}

/// Marker for DELIVER — scaffold body not yet present. The
/// canonical signature lands per Slice 08:
/// `pub async fn dispatch(action: DataplaneUpdateService, dataplane:
///   &Arc<dyn Dataplane>, observation: &Arc<dyn ObservationStore>,
///   tick: &TickContext) -> Result<(), ServiceHydrationDispatchError>`.
pub const SCAFFOLD: bool = true;
