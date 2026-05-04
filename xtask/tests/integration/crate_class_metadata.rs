//! Asserts the new Phase-2 crates declare the expected `crate_class`
//! metadata so dst-lint correctly skips them per ADR-0003 / ADR-0038 §8.
//!
//! Step-ID 01-04 verification: dst-lint scans only `core`-class crates;
//! both `overdrive-bpf` and `overdrive-dataplane` are non-`core`, so the
//! scanner skips them automatically. This guards the metadata
//! declarations themselves against drift — if a future refactor
//! accidentally flipped either crate to `core`, the dst-lint scanner
//! would attempt to scan kernel-side `#![no_std]` source with the
//! adapter-host visitor and produce nonsensical violations; if the
//! declaration vanished entirely, the workspace-level guard-rail in
//! `dst_lint::scan_workspace` would bail at PR time.
//!
//! The two assertions below verify the declarations match the
//! decisions in `wave-decisions.md` D2.

use cargo_metadata::MetadataCommand;

#[test]
fn overdrive_bpf_declares_binary_crate_class() {
    let metadata = MetadataCommand::new().no_deps().exec().expect("cargo metadata succeeds");
    let pkg = metadata
        .packages
        .iter()
        .find(|p| p.name == "overdrive-bpf")
        .expect("overdrive-bpf is a workspace member");
    let class = pkg
        .metadata
        .pointer("/overdrive/crate_class")
        .and_then(|v| v.as_str())
        .expect("overdrive-bpf declares package.metadata.overdrive.crate_class");
    assert_eq!(
        class, "binary",
        "overdrive-bpf must be `binary` per ADR-0038 §1 / wave-decisions D2; \
         non-`core` so dst-lint skips it",
    );
}

#[test]
fn overdrive_dataplane_declares_adapter_host_crate_class() {
    let metadata = MetadataCommand::new().no_deps().exec().expect("cargo metadata succeeds");
    let pkg = metadata
        .packages
        .iter()
        .find(|p| p.name == "overdrive-dataplane")
        .expect("overdrive-dataplane is a workspace member");
    let class = pkg
        .metadata
        .pointer("/overdrive/crate_class")
        .and_then(|v| v.as_str())
        .expect("overdrive-dataplane declares package.metadata.overdrive.crate_class");
    assert_eq!(
        class, "adapter-host",
        "overdrive-dataplane must be `adapter-host` per ADR-0038 §1 / \
         wave-decisions D2; non-`core` so dst-lint skips it",
    );
}
