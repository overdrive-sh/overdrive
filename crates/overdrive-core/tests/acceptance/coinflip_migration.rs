//! Acceptance scenarios for `workload-kind-discriminator` Slice 01,
//! §7 — `examples/coinflip.toml` migration.
//!
//! Driving port: `WorkloadSpecInput::from_toml_str` against the
//! migrated file content.
//!
//! Scenarios from
//! `docs/feature/workload-kind-discriminator/distill/test-scenarios.md` §7.

#![allow(clippy::expect_used)]
#![allow(clippy::expect_fun_call)]

use overdrive_core::aggregate::{WorkloadKind, WorkloadSpecInput};

/// Path to the migrated example, relative to this crate's manifest dir.
/// `CARGO_MANIFEST_DIR` is `crates/overdrive-core/`; the example lives
/// at the workspace root.
fn migrated_coinflip_path() -> std::path::PathBuf {
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root resolves above crates/overdrive-core/")
        .join("examples")
        .join("coinflip.toml")
}

// ---------------------------------------------------------------------------
// S-07-01 — Migrated coinflip.toml parses as Job kind
// ---------------------------------------------------------------------------

#[test]
fn s_07_01_migrated_coinflip_parses_as_job_kind() {
    let path = migrated_coinflip_path();
    let src = std::fs::read_to_string(&path)
        .expect(&format!("examples/coinflip.toml must exist at {}", path.display()));

    let parsed = WorkloadSpecInput::from_toml_str(&src)
        .expect(&format!("migrated coinflip.toml must parse; content was:\n{src}"));

    assert_eq!(parsed.kind(), WorkloadKind::Job, "coinflip.toml must be a Job kind");
    assert_eq!(parsed.id_as_str(), "coinflip");
    assert_eq!(parsed.exec_command(), "/bin/bash");
}
