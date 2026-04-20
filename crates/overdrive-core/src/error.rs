//! Crate-wide error types.
//!
//! Library crates in Overdrive return typed errors via [`thiserror`]. `eyre`
//! and `color-eyre` live only at binary boundaries (CLI, daemon entry points),
//! where the rich report formatting is valuable and the loss of variant
//! matching is acceptable.

use thiserror::Error;

use crate::id::IdParseError;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Top-level error for `overdrive-core`.
///
/// Higher layers embed this via `#[from]` on their own error enums rather
/// than flattening fields, preserving the full error chain for audit and
/// investigation-agent tooling.
#[derive(Debug, Error)]
pub enum Error {
    /// A domain identifier failed to parse or validate.
    #[error(transparent)]
    Id(#[from] IdParseError),
}
