//! The checkout marker `$OVERDRIVE_METAL_SOURCE_ROOT/.overdrive-metal-source` (DDD-6).
//!
//! `bootstrap.sh` writes `commit=<40 hex>`, `workspace=<path>` and `source_digest=<sha256>`; the
//! digest is the SHA-256 of empty input for a clean tree. A missing, unreadable or malformed
//! marker is `Error::Unreadable` for the setting `.overdrive-metal-source`: the run has no
//! provenance, so it refuses before any record exists.

use std::io;
use std::path::Path;

use crate::{Error, Result};

/// The `setting` of a refusal caused by the marker.
pub const MARKER_SETTING: &str = ".overdrive-metal-source";
const MARKER_FILE: &str = ".overdrive-metal-source";
/// SHA-256 of empty input: the `source_digest` of a tree with no change against its commit.
const CLEAN_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const COMMIT_LENGTH: usize = 40;

/// The provenance of the checkout a run measures.
#[derive(Debug, PartialEq, Eq)]
pub struct Marker {
    pub(crate) commit: String,
    pub(crate) dirty: bool,
}

/// Read the marker of the checkout rooted at `source_root`.
pub fn read(source_root: &Path) -> Result<Marker> {
    let text = std::fs::read_to_string(source_root.join(MARKER_FILE)).map_err(unreadable)?;
    parse(&text).map_err(|reason| unreadable(io::Error::new(io::ErrorKind::InvalidData, reason)))
}

fn unreadable(source: io::Error) -> Error {
    Error::Unreadable { setting: MARKER_SETTING.to_owned(), source }
}

fn parse(text: &str) -> std::result::Result<Marker, String> {
    let field = |key: &str| {
        let prefix = format!("{key}=");
        text.lines().find_map(|line| line.strip_prefix(prefix.as_str())).map(str::trim)
    };
    let commit = field("commit").ok_or("the marker has no `commit=` line")?;
    let well_formed = commit.len() == COMMIT_LENGTH
        && commit.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !well_formed {
        return Err(format!("`commit=` is not 40 lowercase hex digits: `{commit}`"));
    }
    let digest = field("source_digest").ok_or("the marker has no `source_digest=` line")?;
    Ok(Marker { commit: commit.to_owned(), dirty: digest != CLEAN_DIGEST })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert their own preconditions and compare exact stored floats"
)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn marker(digest: &str) -> String {
        format!("commit={COMMIT}\nworkspace=/w\nsource_digest={digest}\n")
    }

    #[test]
    fn a_clean_digest_is_not_dirty_and_any_other_is() {
        assert_eq!(
            parse(&marker(CLEAN_DIGEST)),
            Ok(Marker { commit: COMMIT.to_owned(), dirty: false })
        );
        assert_eq!(parse(&marker("ab")), Ok(Marker { commit: COMMIT.to_owned(), dirty: true }));
    }

    #[test]
    fn a_marker_missing_a_field_or_with_a_bad_commit_is_malformed() {
        assert!(parse("workspace=/w\n").unwrap_err().contains("`commit=`"));
        assert!(parse(&format!("commit={COMMIT}\n")).unwrap_err().contains("`source_digest=`"));
        assert!(parse("commit=XYZ\nsource_digest=a\n").unwrap_err().contains("40 lowercase hex"));
    }

    #[test]
    fn read_names_the_marker_as_the_setting_for_every_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        for error in [read(dir.path()).unwrap_err(), {
            std::fs::write(dir.path().join(MARKER_FILE), "nonsense").expect("write");
            read(dir.path()).unwrap_err()
        }] {
            assert!(
                matches!(&error, Error::Unreadable { setting, .. } if setting == MARKER_SETTING),
                "{error:?}"
            );
        }
        std::fs::write(dir.path().join(MARKER_FILE), marker(CLEAN_DIGEST)).expect("write");
        assert_eq!(read(dir.path()).expect("reads").commit, COMMIT);
    }
}
