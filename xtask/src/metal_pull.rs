//! Validation of the repo-relative paths named to `cargo xtask metal pull`.
//!
//! The paths are handed to `infra/metal/bootstrap.sh --pull`, which re-checks
//! them before any ssh or rsync. Validating here first gives the operator the
//! refusal before the metal plumbing starts.

use thiserror::Error;

/// Why a named pull path was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PullPathError {
    /// No path was named, or the path is empty.
    #[error("pull path is empty")]
    Empty,
    /// The path starts with `/`; only repo-relative paths are pulled.
    #[error("pull path `{0}` is absolute; name a repo-relative path")]
    Absolute(String),
    /// A component is empty, `.`, `..` or `.git`.
    #[error("pull path `{0}` has an empty, `.`, `..` or `.git` component")]
    ForbiddenComponent(String),
    /// The path is under the repo-root `target/` build output, which is never pulled.
    #[error("pull path `{0}` is under target/, which is never transferred")]
    UnderTarget(String),
    /// The path has a character outside `[A-Za-z0-9._/-]`.
    #[error("pull path `{0}` has a character outside [A-Za-z0-9._/-]")]
    Charset(String),
}

/// Validate one repo-relative path and return it without a trailing `/`.
///
/// # Errors
///
/// Returns the [`PullPathError`] naming the first rule the path breaks.
pub fn validate_pull_path(raw: &str) -> Result<String, PullPathError> {
    let path = raw.strip_suffix('/').unwrap_or(raw);
    if path.is_empty() {
        return Err(if raw.is_empty() { PullPathError::Empty } else { absolute(raw) });
    }
    if path.starts_with('/') {
        return Err(absolute(raw));
    }
    if !path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
    {
        return Err(PullPathError::Charset(raw.to_owned()));
    }
    if path.split('/').any(|component| matches!(component, "" | "." | ".." | ".git")) {
        return Err(PullPathError::ForbiddenComponent(raw.to_owned()));
    }
    if path == "target" || path.starts_with("target/") {
        return Err(PullPathError::UnderTarget(raw.to_owned()));
    }
    Ok(path.to_owned())
}

fn absolute(raw: &str) -> PullPathError {
    PullPathError::Absolute(raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_relative_paths_are_accepted_without_a_trailing_slash() {
        assert_eq!(
            validate_pull_path("specs/quint/x/evidence").as_deref(),
            Ok("specs/quint/x/evidence")
        );
        assert_eq!(
            validate_pull_path("specs/quint/x/evidence/").as_deref(),
            Ok("specs/quint/x/evidence")
        );
        assert_eq!(
            validate_pull_path("a/.hidden-file_1.txt").as_deref(),
            Ok("a/.hidden-file_1.txt")
        );
    }

    #[test]
    fn absolute_paths_are_refused() {
        assert!(matches!(validate_pull_path("/etc"), Err(PullPathError::Absolute(_))));
        assert!(matches!(validate_pull_path("/"), Err(PullPathError::Absolute(_))));
    }

    #[test]
    fn parent_current_git_and_empty_components_are_refused() {
        for bad in ["../x", "a/../b", "..", ".", "a/./b", "a//b", ".git", "a/.git/config"] {
            assert!(
                matches!(validate_pull_path(bad), Err(PullPathError::ForbiddenComponent(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn target_and_empty_and_odd_characters_are_refused() {
        assert!(matches!(validate_pull_path("target"), Err(PullPathError::UnderTarget(_))));
        assert!(matches!(validate_pull_path("target/debug"), Err(PullPathError::UnderTarget(_))));
        assert_eq!(validate_pull_path(""), Err(PullPathError::Empty));
        assert!(matches!(validate_pull_path("a b"), Err(PullPathError::Charset(_))));
        assert!(matches!(validate_pull_path("a;b"), Err(PullPathError::Charset(_))));
    }

    #[test]
    fn a_name_that_merely_starts_with_target_is_accepted() {
        assert_eq!(validate_pull_path("targets/x").as_deref(), Ok("targets/x"));
    }
}
