//! `cargo xtask placement-lint` — the eager check of the placement records a
//! test declares on itself.
//!
//! The lint lists the workspace's tests with `cargo nextest list
//! --message-format json`, reads the `overdrive_test_placement` section of
//! every listed test binary and reports, one line each on stderr:
//!
//! - `placement-lint: missing-fixture <binary-id> <test> <fixture>` for a
//!   `lane_operator` / `lane_owner` test whose fixture is not a listed test of
//!   the package that provides it.
//!
//! It reads Linux test binaries (ELF) and writes nothing. A binary it cannot
//! read is an error, never a skipped file.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::Value;

/// The ELF section the placement records are written to.
const SECTION: &[u8] = b"overdrive_test_placement";

/// Runs the lint from the workspace root; `Err` when it reported anything.
///
/// # Errors
///
/// The test listing fails, a listed binary cannot be read, or a finding was
/// reported.
pub fn run() -> Result<()> {
    let listing = list_tests()?;
    let findings = findings(&listing)?;
    for finding in &findings {
        eprintln!("placement-lint: {finding}");
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(eyre!("placement-lint found {} problem(s)", findings.len()))
    }
}

fn list_tests() -> Result<Value> {
    let output = Command::new("cargo")
        .args(["nextest", "list", "--message-format", "json", "--workspace"])
        .args(["--features", "integration-tests"])
        .output()
        .wrap_err("running `cargo nextest list`")?;
    if !output.status.success() {
        return Err(eyre!(
            "`cargo nextest list` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).wrap_err("`cargo nextest list` printed no JSON")
}

/// One listed test binary.
struct Suite<'a> {
    id: &'a str,
    path: &'a str,
    tests: BTreeSet<&'a str>,
}

impl Suite<'_> {
    /// The package a binary id (`<package>::<binary>`) belongs to.
    fn package(&self) -> &str {
        self.id.split("::").next().unwrap_or(self.id)
    }
}

fn suites(listing: &Value) -> Result<Vec<Suite<'_>>> {
    let listed = listing
        .get("rust-suites")
        .and_then(Value::as_object)
        .ok_or_else(|| eyre!("the test listing has no `rust-suites`"))?;
    listed
        .values()
        .map(|suite| {
            let text = |key: &str| {
                suite
                    .get(key)
                    .and_then(Value::as_str)
                    .ok_or_else(|| eyre!("a listed suite has no `{key}`"))
            };
            let tests = suite
                .get("testcases")
                .and_then(Value::as_object)
                .map(|cases| cases.keys().map(String::as_str).collect())
                .unwrap_or_default();
            Ok(Suite { id: text("binary-id")?, path: text("binary-path")?, tests })
        })
        .collect()
}

/// A `lane_operator` / `lane_owner` record: the test (crate segment dropped) and
/// its fixture.
struct LaneRecord {
    test: String,
    fixture: String,
}

fn findings(listing: &Value) -> Result<Vec<String>> {
    let suites = suites(listing)?;
    let mut findings = Vec::new();
    for suite in &suites {
        for lane in lane_records(suite.path)? {
            let provided = suites
                .iter()
                .filter(|other| other.package() == suite.package())
                .any(|other| other.tests.contains(lane.fixture.as_str()));
            if !provided {
                findings
                    .push(format!("missing-fixture {} {} {}", suite.id, lane.test, lane.fixture));
            }
        }
    }
    Ok(findings)
}

/// The lane records of the binary at `path`.
fn lane_records(path: &str) -> Result<Vec<LaneRecord>> {
    let Some(section) = placement_section(Path::new(path))? else {
        return Ok(Vec::new());
    };
    let mut fields: Vec<&[u8]> = section.split(|byte| *byte == 0).collect();
    if fields.pop() != Some(&[]) {
        return Err(eyre!("{path}: the placement section is malformed"));
    }
    let utf8 = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| eyre!("{path}: the placement section is not UTF-8"))
    };
    let mut records = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let [kind, test, argc] = fields[index..]
            .first_chunk::<3>()
            .copied()
            .ok_or_else(|| eyre!("{path}: the placement section ends inside a record"))?;
        let argc: usize = utf8(argc)?
            .parse()
            .wrap_err_with(|| format!("{path}: a placement record has no argument count"))?;
        let arguments = fields
            .get(index + 3..index + 3 + argc)
            .ok_or_else(|| eyre!("{path}: the placement section ends inside a record"))?;
        if matches!(kind, b"lane_operator" | b"lane_owner") {
            let [fixture] = arguments else {
                return Err(eyre!("{path}: a lane record carries one fixture"));
            };
            let test = utf8(test)?;
            let test = test.split_once("::").map_or(test.as_str(), |(_, rest)| rest).to_owned();
            records.push(LaneRecord { test, fixture: utf8(fixture)? });
        }
        index += 3 + argc;
    }
    Ok(records)
}

/// The bytes of the placement section of the 64-bit little-endian ELF at
/// `path`, or `None` when it has no such section.
fn placement_section(path: &Path) -> Result<Option<Vec<u8>>> {
    let bytes = std::fs::read(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    let not_elf = || {
        eyre!(
            "{}: not a 64-bit little-endian ELF (the lint reads the Linux test binaries)",
            path.display()
        )
    };
    let header = bytes.get(..64).ok_or_else(not_elf)?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return Err(not_elf());
    }
    let word = |offset: usize, width: usize| -> Result<usize> {
        let field = header.get(offset..offset + width).ok_or_else(not_elf)?;
        let mut value = 0_u64;
        for (shift, byte) in field.iter().enumerate() {
            value |= u64::from(*byte) << (8 * shift);
        }
        usize::try_from(value).map_err(|_| not_elf())
    };
    let (table, entry_size, entry_count, names_index) =
        (word(40, 8)?, word(58, 2)?, word(60, 2)?, word(62, 2)?);
    let entry = |index: usize| -> Result<(usize, usize, usize)> {
        let at = table + index * entry_size;
        let raw = bytes.get(at..at + 64).ok_or_else(not_elf)?;
        let field = |offset: usize, width: usize| {
            usize::try_from(
                raw[offset..offset + width]
                    .iter()
                    .enumerate()
                    .fold(0_u64, |value, (shift, byte)| value | u64::from(*byte) << (8 * shift)),
            )
            .map_err(|_| not_elf())
        };
        Ok((field(0, 4)?, field(24, 8)?, field(32, 8)?))
    };
    if table == 0 || entry_size < 64 || names_index >= entry_count {
        return Ok(None);
    }
    let (_, names_offset, names_size) = entry(names_index)?;
    let names = bytes.get(names_offset..names_offset + names_size).ok_or_else(not_elf)?;
    for index in 0..entry_count {
        let (name, offset, size) = entry(index)?;
        let name = names.get(name..).ok_or_else(not_elf)?;
        let end = name.iter().position(|byte| *byte == 0).ok_or_else(not_elf)?;
        if &name[..end] == SECTION {
            return Ok(Some(bytes.get(offset..offset + size).ok_or_else(not_elf)?.to_vec()));
        }
    }
    Ok(None)
}
