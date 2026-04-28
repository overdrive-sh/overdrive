//! US-02 Scenario 2.10 — `CgroupPath` rejects path-traversal characters.
//!
//! @in-memory — table-driven over a curated list of malicious shapes.
//! PORT-TO-PORT: enters via `CgroupPath::from_str`, asserts on the
//! returned `Result::Err` variant.

use std::str::FromStr;

use overdrive_worker::CgroupPath;

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
