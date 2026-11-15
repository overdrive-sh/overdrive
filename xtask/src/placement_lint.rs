//! `cargo xtask placement-lint` — the eager check of the placement records a
//! test declares on itself.
//!
//! The lint lists the workspace's tests with `cargo nextest list
//! --message-format json` and hands the listing to
//! `infra/metal/test-wrapper.sh --check-placement`, the one reader of the
//! `overdrive_test_placement` records, so the lint and a Metal run cannot read
//! the format differently. It reports, one line each on stderr:
//!
//! - `placement-lint: unplaced <binary-id> <test>` for a test no record of its
//!   own, of its module or of its binary places;
//! - `placement-lint: conflict <binary-id>` for a binary with a default beside
//!   a record that is not host-pure;
//! - `placement-lint: missing-fixture <binary-id> <test> <fixture>` for a
//!   `lane_operator` / `lane_owner` test whose fixture is not a listed test of
//!   the package that provides it.
//!
//! It reads Linux test binaries (ELF) through the wrapper and writes nothing.
//! A binary the wrapper cannot read is an error, never a skipped file.

use std::io::Write;
use std::process::{Command, Stdio};

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::Value;

/// The feature a member declares to put its real-guest tests behind.
const KVM_FEATURE: &str = "kvm-tests";

/// The wrapper that reads the placement records, relative to the workspace root.
const WRAPPER: &str = "infra/metal/test-wrapper.sh";

/// The wrapper's exit status for findings (`unplaced`, `conflict`,
/// `missing-fixture`); any other non-zero status is a failure to check.
const FINDINGS_STATUS: i32 = 1;

/// Runs the lint from the workspace root; `Err` when it reported anything.
///
/// # Errors
///
/// The test listing fails, a listed binary cannot be read, or a finding was
/// reported.
pub fn run() -> Result<()> {
    let listing = list_tests()?;
    let report = check_placement(&listing)?;
    let findings: Vec<String> =
        String::from_utf8_lossy(&report.stdout).lines().map(str::to_owned).collect();
    for finding in &findings {
        eprintln!("placement-lint: {finding}");
    }
    let diagnostics = String::from_utf8_lossy(&report.stderr);
    match report.status.code() {
        Some(0) => Ok(()),
        Some(FINDINGS_STATUS) => Err(eyre!("placement-lint found {} problem(s)", findings.len())),
        _ => Err(eyre!(
            "`{WRAPPER} --check-placement` could not read the listed test binaries ({}): {}",
            report.status,
            diagnostics.trim()
        )),
    }
}

/// The wrapper's report on a `cargo nextest list` JSON listing.
fn check_placement(listing: &[u8]) -> Result<std::process::Output> {
    let mut child = Command::new("bash")
        .args([WRAPPER, "--check-placement"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .wrap_err_with(|| format!("running `{WRAPPER} --check-placement`"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| eyre!("the wrapper has no stdin"))?;
    // The wrapper may end before it has read the whole listing; its exit status
    // and report are what the lint acts on, so a broken pipe is not an error.
    let writer = std::thread::spawn({
        let listing = listing.to_vec();
        move || {
            let _ = stdin.write_all(&listing);
        }
    });
    let output = child.wait_with_output().wrap_err("waiting for the wrapper")?;
    writer.join().map_err(|_| eyre!("the listing writer panicked"))?;
    Ok(output)
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

fn list_tests() -> Result<Vec<u8>> {
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
    serde_json::from_slice::<Value>(&output.stdout)
        .wrap_err("`cargo nextest list` printed no JSON")?;
    Ok(output.stdout)
}
