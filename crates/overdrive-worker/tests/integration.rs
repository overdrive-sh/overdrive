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
        mod resize_updates_limits;
        mod resource_enforcement;
        mod start_and_running;
        mod stop_escalates_to_sigkill;
        mod stop_pid_none_handle_delivers_sigterm;
        mod stop_with_grace;
    }
}
