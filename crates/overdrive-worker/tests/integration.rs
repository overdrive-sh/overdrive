//! Integration-test entrypoint for `overdrive-worker`.
//!
//! Linux-only. Gated `--features integration-tests` per
//! `.claude/rules/testing.md` § Integration vs unit gating.
//!
//! These tests spawn real `/bin/sleep` processes and write to
//! `/sys/fs/cgroup/...` directly; they require:
//!  - A Linux host (cgroup v2)
//!  - cgroup v2 delegated to the running UID
//!  - `/bin/sleep` on PATH

#![cfg(all(feature = "integration-tests", target_os = "linux"))]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod integration {
    // Step 01-07 (cgroup-fs-port migration): D1 Real/Sim equivalence
    // proptest. Validates the byte-store contract holds between
    // `RealCgroupFs` (rooted at a `tempfile::TempDir`) and
    // `SimCgroupFs`. Kernel-side effects (`cgroup.kill` mass-kill,
    // `cgroup.subtree_control` EBUSY, pseudo-file synthesis,
    // `EINVAL`) are EXPLICITLY OUT OF SCOPE per ADR-0054 § D3 and
    // covered by Class C scenarios in step 01-08.
    mod cgroup_fs_equivalence;

    // Step 01-02 (cgroup-fs-port migration): Tier 3 acceptance tests
    // for `overdrive_host::RealCgroupFs::probe()`.
    mod real_cgroup_fs {
        mod probe_success;
        mod probe_with_custom_root;
    }

    // Step 01-05 (cgroup-fs-port migration): E1 KEEP-TEMPFILE rows
    // 8 + 10 relocated here per ADR-0054 § D5. These tempfile-backed
    // tests against `RealCgroupFs` defend the substrate boundary —
    // that `CgroupManager` correctly propagates a REAL `io::Error`
    // from a REAL `tokio::fs::*` syscall against the REAL kernel VFS.
    // ENOTDIR-via-regular-file-in-dir-slot is a contrivance to
    // *trigger* the error; the test *boundary* is real. Candidates
    // for retirement once Class C scenario
    // `write_to_readonly_cgroup_file` lands (per
    // `docs/feature/cgroup-fs-port/distill/test-scenarios.md` § E1
    // rows 8 / 10 rationale).
    mod cgroup_manager {
        mod cgroup_kill_propagates_real_io_error;
        mod remove_workload_scope_propagates_real_io_error;
    }

    mod exec_driver {
        mod cgroup_procs;
        // Per-alloc RAII cleanup helper used by every real-cgroupfs test
        // below. Phase 02 of `fix-cgroup-subtree-control-delegation`
        // migrated the suite off `tempfile::TempDir` onto real
        // `/sys/fs/cgroup`; this guard reaps any leftover scope on
        // panic / SIGKILL so the next test's mkdir does not hit EEXIST.
        mod cleanup;
        mod limit_write_failure_warns;
        mod live_map_bounded;
        mod missing_binary;
        mod netns_entry;
        mod resize_updates_limits;
        mod resource_enforcement;
        mod start_and_running;
        mod stop_escalates_to_sigkill;
        mod stop_pid_none_handle_delivers_sigterm;
        mod stop_with_grace;
    }
}
