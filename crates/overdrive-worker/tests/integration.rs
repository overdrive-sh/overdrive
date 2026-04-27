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

#![cfg(feature = "integration-tests")]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod integration {
    mod process_driver {
        mod start_and_running;
        mod cgroup_procs;
        mod resource_enforcement;
        mod missing_binary;
        mod stop_with_grace;
        mod stop_escalates_to_sigkill;
        mod limit_write_failure_warns;
    }
}
