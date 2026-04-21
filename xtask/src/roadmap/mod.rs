//! `cargo xtask roadmap …` — bulk-create GitHub issues from
//! `.context/roadmap-issues.md`.
//!
//! Pragmatic one-off utility. Shells out to `gh` rather than wiring up an
//! API client. Writes a state file at `.context/roadmap-sync-state.json`
//! so a rate-limited or network-interrupted run can resume via `--resume`.

// Keep this module's lint posture pragmatic — it is a one-off utility that
// lives alongside production code and does not need the pedantic ceremony.
#![allow(
    clippy::items_after_statements,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines
)]

pub mod gh;
pub mod init;
pub mod parse;
pub mod state;
pub mod sync;
