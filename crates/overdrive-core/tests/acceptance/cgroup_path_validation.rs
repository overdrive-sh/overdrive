//! US-02 Scenario 2.10 — `CgroupPath` rejects path-traversal characters.
//!
//! @in-memory — table-driven over a curated list of malicious shapes.
//! PORT-TO-PORT: enters via `CgroupPath::from_str`, asserts on the
//! returned `Result::Err` variant.
//!
//! Relocated from `overdrive-worker/tests/acceptance/` (review remediation,
//! microvm-driver-cloud-hypervisor step 01-01 F6): `CgroupPath` itself now
//! lives in `overdrive_core::cgroup` (ADR-0082 §D2, Amendment 2026-08-12,
//! gap 1, GH #42) — testing it only through the `overdrive_worker`
//! re-export left a scoped `-p overdrive-core` mutation run blind to these
//! `FromStr` rejection killers. Tests live beside the code they defend.

use std::str::FromStr;

use overdrive_core::cgroup::CgroupPath;

#[test]
fn cgroup_path_rejects_traversal_characters() {
    let invalid: &[&str] = &[
        "",                                            // empty
        "/overdrive.slice/workloads.slice/x.scope",    // leading slash
        "overdrive.slice//workloads.slice/x.scope",    // double slash
        "overdrive.slice/../workloads.slice/x.scope",  // dotdot segment
        "..",                                          // standalone dotdot
        "overdrive.slice/workloads.slice/x.scope/..",  // trailing dotdot
        "overdrive.slice/workloads.slice/\0bad.scope", // NUL byte
    ];

    for raw in invalid {
        let result = CgroupPath::from_str(raw);
        assert!(result.is_err(), "expected CgroupPath::from_str({raw:?}) to fail, got {result:?}");
    }
}
