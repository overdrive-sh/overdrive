//! C-pseudo-file-synthesis — the kernel synthesises `cgroup.events` at
//! mkdir time. The file exists and its body matches the
//! `^(populated|frozen) [01]$` (multi-line) shape documented in the
//! cgroup v2 docs.
//!
//! Tier 3, real-io. Requires Lima sudo.
//!
//! Exercises ADR-0054 § D3 row 4 — the kernel synthesises pseudo-files
//! at `mkdir(2)` time. `SimCgroupFs` creates ONLY the directory; the
//! pseudo-files are absent from the in-memory store. This scenario is
//! the structural defense for any future code path (e.g. an
//! `EventsObserver` reconciler reading `cgroup.events` for
//! populated-status notifications) that relies on this synthesis.
//!
//! Read uses `tokio::fs::read` directly because the `CgroupFs` trait
//! does not expose a read method (only `probe()` internally) — the
//! scenario asserts on the substrate behaviour the probe relies on,
//! not on the trait surface itself.
//!
//! Scenario reference: `docs/feature/cgroup-fs-port/distill/test-scenarios.md`
//! § C-pseudo-file-synthesis.

use std::path::Path;
use std::sync::Arc;

use overdrive_core::id::AllocationId;
use overdrive_core::traits::CgroupFs;
use overdrive_host::RealCgroupFs;
use overdrive_worker::cgroup_manager::CgroupManager;
use serial_test::serial;

use super::super::exec_driver::cleanup::AllocCleanup;

#[tokio::test]
#[serial(cgroup)]
async fn cgroup_events_appears_at_mkdir_time() {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let fs: Arc<dyn CgroupFs> = Arc::new(RealCgroupFs::new());
    CgroupManager::new(cgroup_root.to_path_buf(), fs.clone())
        .create_workloads_slice_with_controllers()
        .await
        .expect("workloads.slice bootstrap succeeds");

    let alloc = AllocationId::new("alloc-pseudoC-0").expect("valid alloc id");
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let scope_dir = cgroup_root.join(format!("overdrive.slice/workloads.slice/{alloc}.scope"));
    fs.create_dir(&scope_dir).await.expect("create alloc scope");

    // The kernel synthesises cgroup.events at mkdir time — no write
    // by us is required.
    let body = tokio::fs::read_to_string(scope_dir.join("cgroup.events"))
        .await
        .expect("cgroup.events readable post-mkdir");

    // cgroup v2 cgroup.events body shape (cgroup-v2.rst):
    //   populated <0|1>
    //   frozen <0|1>
    //
    // We assert each non-empty line matches `<key> <value>` where
    // key is `populated` or `frozen` and value is `0` or `1`.
    // Equivalent to the regex `^(populated|frozen) [01]$` without
    // pulling in the `regex` crate as a dep.
    let mut saw_populated = false;
    let mut saw_frozen = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split(' ');
        let key = parts.next().expect("non-empty line has a key");
        let value = parts.next().expect("key-value line has a value");
        assert!(parts.next().is_none(), "unexpected extra tokens on line: {trimmed:?}");
        assert!(
            value == "0" || value == "1",
            "cgroup.events value must be 0 or 1; line={trimmed:?}"
        );
        match key {
            "populated" => saw_populated = true,
            "frozen" => saw_frozen = true,
            other => panic!("unknown cgroup.events key: {other:?} (full line: {trimmed:?})"),
        }
    }
    assert!(saw_populated, "cgroup.events missing `populated <0|1>`; body={body:?}");
    assert!(saw_frozen, "cgroup.events missing `frozen <0|1>`; body={body:?}");
}
