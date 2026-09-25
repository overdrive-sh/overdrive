//! `cargo xtask cloexec-lint` — the creation-time close-on-exec source gate
//! (OBL-295-CLOEXEC; D-295-R3, ADR-0129).
//!
//! Every raw descriptor that first-party code in the `overdrive serve` process
//! creates is created close-on-exec. This gate scans the source of every
//! first-party crate linked into `overdrive serve` and rejects calls under the
//! `libc::`, `nix::`, and `rustix::` paths that create an inheritable
//! descriptor.
//!
//! Shaped like [`crate::dst_lint`]: pure functions over already-read source,
//! with file and metadata access only in [`scan_workspace`] and [`run`]. The
//! scanner is purely syntactic and imports no `overdrive-*` crate.
//!
//! Unlike `dst_lint::scan_workspace`, [`scan_workspace`] fails closed: a file
//! it cannot read or parse is an error, never a skipped file.

use std::path::{Path, PathBuf};

use color_eyre::eyre::Result;

/// Why a matched call is rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloexecRule {
    /// The flag or type argument is resolved and lacks the call's
    /// close-on-exec flag.
    MissingFlag,
    /// The call as written can never set close-on-exec (`accept`, `pipe`,
    /// `dup`, `dup2`, `inotify_init`, `epoll_create`, `fcntl` with `F_DUPFD`,
    /// or a wrapper with no flags argument).
    AlwaysInheritable,
    /// The flag argument is not a literal or constant expression.
    UnresolvedFlag,
}

/// One rejected call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloexecViolation {
    /// The `file` label passed to `scan_source`.
    pub file: PathBuf,
    /// 1-based line of the call.
    pub line: usize,
    /// 1-based column of the call.
    pub column: usize,
    /// The matched call as `<root crate>::<final segment>`, after `use`
    /// renames are resolved (for example `libc::socket` for
    /// `use libc::socket as s; s(..)`).
    pub call: String,
    /// The rule the call violates.
    pub rule: CloexecRule,
}

/// Scan one already-read source file.
///
/// Parses `source` with `syn`; a parse failure is `Err`, distinct from a clean
/// file. Applies the call-family table and the `cloexec-lint: ok` marker
/// (on the line of the call or the line immediately above, suppressing only
/// that line), skips `#[cfg(test)]` items, and returns violations in source
/// order.
#[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-04")]
pub fn scan_source(source: &str, file: impl AsRef<Path>) -> Result<Vec<CloexecViolation>> {
    let _ = (source, file.as_ref());
    todo!("RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04")
}

/// Scan every source file linked into `overdrive serve`.
///
/// Through `cargo_metadata`'s resolved dependency graph, scans the
/// `overdrive-cli` package and every workspace member in its normal
/// (non-dev, non-build) dependency closure: every `src/**/*.rs` file except
/// `src/bin/**`. Fails closed: a file it cannot read, or that
/// [`scan_source`] cannot parse, is `Err` naming that file; metadata without
/// an `overdrive-cli` package is `Err`.
#[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-04")]
pub fn scan_workspace(manifest_path: &Path) -> Result<Vec<CloexecViolation>> {
    let _ = manifest_path;
    todo!("RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_workspace — DELIVER step 05-04")
}

/// Render one violation as the block [`run`] writes to stderr.
#[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-04")]
pub fn render_violation(v: &CloexecViolation) -> String {
    let _ = v;
    todo!("RED scaffold: OBL-295-CLOEXEC cloexec_lint::render_violation — DELIVER step 05-04")
}

/// Entry point for `cargo xtask cloexec-lint`: writes each
/// [`render_violation`] block to stderr and returns `Err` when any violation
/// exists.
#[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-04")]
pub fn run(manifest_path: &Path) -> Result<()> {
    let _ = manifest_path;
    todo!("RED scaffold: OBL-295-CLOEXEC cloexec_lint::run — DELIVER step 05-04")
}
