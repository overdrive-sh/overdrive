//! C-probe-success — `RealCgroupFs::probe()` succeeds against real
//! `/sys/fs/cgroup` per ADR-0054 § Production probe.
//!
//! Tier 3, real-io, walking-skeleton. Requires Lima sudo (writes to
//! `/sys/fs/cgroup/.overdrive-probe-<uuid>/`).
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-probe-success.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;

async fn snapshot_top_level(root: &Path) -> BTreeSet<OsString> {
    let mut entries = BTreeSet::new();
    let mut rd = tokio::fs::read_dir(root).await.expect("read /sys/fs/cgroup");
    while let Some(entry) = rd.next_entry().await.expect("next_entry") {
        entries.insert(entry.file_name());
    }
    entries
}

#[tokio::test]
async fn probe_succeeds_against_real_cgroupfs() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let before = snapshot_top_level(cgroup_root).await;

    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());

    fs.probe().await.expect("probe must succeed against real /sys/fs/cgroup under Lima sudo");

    let after = snapshot_top_level(cgroup_root).await;

    // Probe self-cleans: no `.overdrive-probe-*` entries remain.
    let leaked: Vec<_> = after
        .iter()
        .filter(|name| name.to_string_lossy().starts_with(".overdrive-probe-"))
        .collect();
    assert!(leaked.is_empty(), "probe leaked scratch dirs: {leaked:?}");

    // `/sys/fs/cgroup` top-level is byte-identical pre/post (modulo
    // unrelated kernel/workload churn — we only assert no
    // `.overdrive-probe-*` survivors above, since the rest of
    // `/sys/fs/cgroup` is not under this test's control).
    let before_probes: BTreeSet<_> =
        before.iter().filter(|n| n.to_string_lossy().starts_with(".overdrive-probe-")).collect();
    assert!(
        before_probes.is_empty(),
        "pre-test environment already had leaked probes: {before_probes:?}"
    );
}
