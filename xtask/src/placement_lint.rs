//! `cargo xtask placement-lint` — the eager check of the placement records a
//! test declares on itself.
//!
//! The lint lists the workspace's tests with `cargo nextest list
//! --message-format json`, reads the `overdrive_test_placement` section of
//! every listed test binary and reports, one line each on stderr:
//!
//! - `placement-lint: unplaced <binary-id> <test>` for a test no record of its
//!   own, of its module or of its binary places;
//! - `placement-lint: conflict <binary-id>` for a binary with a default beside
//!   a record that is not host-pure;
//! - `placement-lint: missing-fixture <binary-id> <test> <fixture>` for a
//!   `lane_operator` / `lane_owner` test whose fixture is not a listed test of
//!   the package that provides it.
//!
//! It reads Linux test binaries (ELF) and writes nothing. A binary it cannot
//! read is an error, never a skipped file.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::Value;

/// The ELF section the placement records are written to.
const SECTION: &[u8] = b"overdrive_test_placement";

/// The kinds of record that place one test.
const PER_TEST: [&str; 5] =
    ["host_pure", "native_launcher", "guest", "lane_operator", "lane_owner"];

/// The kinds a binary default cannot sit beside.
const NOT_HOST_PURE: [&str; 4] = ["guest", "native_launcher", "lane_operator", "lane_owner"];

/// The feature a member declares to put its real-guest tests behind.
const KVM_FEATURE: &str = "kvm-tests";

/// Runs the lint from the workspace root; `Err` when it reported anything.
///
/// # Errors
///
/// The test listing fails, a listed binary cannot be read, or a finding was
/// reported.
pub fn run() -> Result<()> {
    let listing = list_tests()?;
    let suites = suites(&listing)?;
    let findings = findings(&suites);
    for finding in &findings {
        eprintln!("placement-lint: {finding}");
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(eyre!("placement-lint found {} problem(s)", findings.len()))
    }
}

/// The features that put every test behind a feature in the listing: the
/// shared `integration-tests` and each member's own `kvm-tests`.
fn listing_features() -> Result<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .wrap_err("running `cargo metadata`")?;
    if !output.status.success() {
        return Err(eyre!(
            "`cargo metadata` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).wrap_err("`cargo metadata` printed no JSON")?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| eyre!("`cargo metadata` listed no packages"))?;
    let mut features = vec!["integration-tests".to_owned()];
    for package in packages {
        let declares = package
            .get("features")
            .and_then(Value::as_object)
            .is_some_and(|declared| declared.contains_key(KVM_FEATURE));
        if let (true, Some(name)) = (declares, package.get("name").and_then(Value::as_str)) {
            features.push(format!("{name}/{KVM_FEATURE}"));
        }
    }
    Ok(features.join(","))
}

fn list_tests() -> Result<Value> {
    let output = Command::new("cargo")
        .args(["nextest", "list", "--message-format", "json", "--workspace"])
        .args(["--features", &listing_features()?])
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

/// One record of a binary's placement section.
struct Record {
    kind: String,
    test: String,
    arguments: Vec<String>,
}

/// One listed test binary and the records it carries.
struct Suite {
    id: String,
    /// The listed tests; an ignored test never reaches the runner.
    tests: BTreeMap<String, bool>,
    records: Vec<Record>,
}

impl Suite {
    /// The package a binary id (`<package>::<binary>`) belongs to.
    fn package(&self) -> &str {
        self.id.split("::").next().unwrap_or(&self.id)
    }

    /// A default beside a record that is not host-pure.
    fn conflicts(&self) -> bool {
        let has = |kinds: &[&str]| self.records.iter().any(|r| kinds.contains(&r.kind.as_str()));
        has(&["default_host_pure"]) && has(&NOT_HOST_PURE)
    }

    /// The record that places `test`: its own, else its module's, else the
    /// binary's default.
    fn record_for(&self, test: &str) -> Option<(&str, &[String])> {
        let module = test.rsplit_once("::").map_or("", |(module, _)| module);
        let own = self
            .records
            .iter()
            .find(|r| PER_TEST.contains(&r.kind.as_str()) && drop_crate(&r.test) == test);
        let of_module = || {
            self.records
                .iter()
                .find(|r| r.kind == "module_host_pure" && drop_crate(&r.test) == module)
        };
        let default = || self.records.iter().find(|r| r.kind == "default_host_pure");
        own.or_else(of_module)
            .or_else(default)
            .map(|record| (record.kind.as_str(), record.arguments.as_slice()))
    }
}

/// A test path without its leading crate segment, as nextest names the test.
fn drop_crate(path: &str) -> &str {
    path.split_once("::").map_or("", |(_, rest)| rest)
}

fn suites(listing: &Value) -> Result<Vec<Suite>> {
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
                .map(|cases| {
                    cases
                        .iter()
                        .map(|(name, case)| {
                            let ignored = case.get("ignored").and_then(Value::as_bool);
                            (name.clone(), ignored.unwrap_or(false))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Suite {
                id: text("binary-id")?.to_owned(),
                tests,
                records: records(Path::new(text("binary-path")?))?,
            })
        })
        .collect()
}

fn findings(suites: &[Suite]) -> Vec<String> {
    let mut findings = Vec::new();
    for suite in suites {
        if suite.conflicts() {
            findings.push(format!("conflict {}", suite.id));
            continue;
        }
        for (test, ignored) in &suite.tests {
            if !ignored && suite.record_for(test).is_none() {
                findings.push(format!("unplaced {} {test}", suite.id));
            }
        }
        findings.extend(missing_fixtures(suite, suites));
    }
    findings
}

/// The lane records of `suite` whose fixture no binary of the package lists.
fn missing_fixtures(suite: &Suite, suites: &[Suite]) -> Vec<String> {
    let mut missing = Vec::new();
    for record in &suite.records {
        if !matches!(record.kind.as_str(), "lane_operator" | "lane_owner") {
            continue;
        }
        let fixture = record.arguments.first().map_or("", String::as_str);
        let provided = suites
            .iter()
            .filter(|other| other.package() == suite.package())
            .any(|other| other.tests.contains_key(fixture));
        if !provided {
            missing.push(format!(
                "missing-fixture {} {} {fixture}",
                suite.id,
                drop_crate(&record.test)
            ));
        }
    }
    missing
}

/// The records of the binary at `path`.
fn records(path: &Path) -> Result<Vec<Record>> {
    let path_text = path.display();
    let Some(section) = placement_section(path)? else {
        return Ok(Vec::new());
    };
    let mut fields: Vec<&[u8]> = section.split(|byte| *byte == 0).collect();
    if fields.pop() != Some(&[]) {
        return Err(eyre!("{path_text}: the placement section is malformed"));
    }
    let utf8 = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| eyre!("{path_text}: the placement section is not UTF-8"))
    };
    let mut records = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let [kind, test, argc] = fields[index..]
            .first_chunk::<3>()
            .copied()
            .ok_or_else(|| eyre!("{path_text}: the placement section ends inside a record"))?;
        let argc: usize = utf8(argc)?
            .parse()
            .wrap_err_with(|| format!("{path_text}: a placement record has no argument count"))?;
        let arguments = fields
            .get(index + 3..index + 3 + argc)
            .ok_or_else(|| eyre!("{path_text}: the placement section ends inside a record"))?
            .iter()
            .map(|argument| utf8(argument))
            .collect::<Result<Vec<_>>>()?;
        records.push(Record { kind: utf8(kind)?, test: utf8(test)?, arguments });
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
