//! C-probe-with-custom-root — `with_probe_root` builder override
//! genuinely scopes the probe directory away from `/sys/fs/cgroup`.
//!
//! Tier 3, real-io. Validates the test-fixture knob does what it
//! says: the probe runs against the override path, NOT against
//! `/sys/fs/cgroup`. Under the amended ADR-0054 probe spec
//! (2026-05-24) the probe round-trips on a kernel-managed pseudo-file
//! (`cgroup.subtree_control`) that only exists under real cgroupfs;
//! routing the probe to a tempdir (regular filesystem) therefore
//! SHOULD fail at the write step with `ENOENT` —
//! `ProbeError::Substrate { source: <ENOENT io::Error> }`.
//!
//! The test's purpose is NOT to prove the probe succeeds on
//! arbitrary paths; it is to prove the override genuinely scopes
//! AWAY from `/sys/fs/cgroup`. The two assertions:
//!  1. probe errors with `ProbeError::Substrate(_)` (the tempdir is
//!     not cgroupfs, so step 2 cannot succeed),
//!  2. `/sys/fs/cgroup` top-level is byte-identical pre/post (the
//!     override genuinely scoped — no probe directory landed under
//!     the real cgroupfs root).
//!
//! Partial-leftover note: the amended probe contract does not
//! guarantee teardown on a step-2 failure (the step-1 `mkdir` of
//! `.overdrive-probe-<uuid>` is leaked when step-2 errors). The test
//! does NOT assert tempdir is empty post-probe; tempdir cleanup is
//! handled by `TempDir`'s `Drop`.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-probe-with-custom-root.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use overdrive_core::traits::{CgroupFs, ProbeError};
use overdrive_host::RealCgroupFs;
use serial_test::serial;

async fn snapshot_top_level(root: &Path) -> BTreeSet<OsString> {
    let mut entries = BTreeSet::new();
    let mut rd = tokio::fs::read_dir(root).await.expect("read root");
    while let Some(entry) = rd.next_entry().await.expect("next_entry") {
        entries.insert(entry.file_name());
    }
    entries
}

#[tokio::test]
#[serial(cgroup)]
async fn with_probe_root_scopes_away_from_real_cgroupfs() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let sys_fs_cgroup = Path::new("/sys/fs/cgroup");

    let cgroup_before = snapshot_top_level(sys_fs_cgroup).await;

    let fs: Arc<dyn CgroupFs> =
        Arc::new(RealCgroupFs::new().with_probe_root(tmp.path().to_path_buf()));

    let result = fs.probe().await;

    // Tempdir is NOT cgroupfs; step 2 (write to
    // `<probe_dir>/cgroup.subtree_control`) must fail with ENOENT —
    // the kernel-managed pseudo-file does not exist under a regular
    // tempdir directory.
    match result {
        Err(ProbeError::Substrate { source }) => {
            // ENOENT is the expected ErrorKind, but we don't assert
            // on it specifically — any io::Error surfacing as
            // Substrate proves the probe ran against the override
            // path (the only paths the probe touches when
            // probe_root = tmp.path() are under tmp).
            let _ = source;
        }
        Err(other) => panic!(
            "expected ProbeError::Substrate from non-cgroupfs tempdir routing, got: {other:?}"
        ),
        Ok(()) => panic!(
            "probe must NOT succeed against a non-cgroupfs tempdir — \
             the amended ADR-0054 spec round-trips on `cgroup.subtree_control`, \
             which a regular tempdir does not synthesise after mkdir"
        ),
    }

    // `/sys/fs/cgroup` byte-identical pre/post — the override
    // genuinely scoped away from real cgroupfs. No probe directory
    // landed under the real cgroupfs root.
    let cgroup_after = snapshot_top_level(sys_fs_cgroup).await;
    assert_eq!(
        cgroup_before, cgroup_after,
        "with_probe_root override did not scope: /sys/fs/cgroup changed"
    );
}
