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
    //
    // Step 01-08 (cgroup-fs-port migration): Class C kernel-semantics
    // scenarios per ADR-0054 § D3 + DISTILL Class C. Each scenario
    // exercises a kernel-side effect that SimCgroupFs cannot model
    // (cgroup.kill mass-kill, subtree_control EBUSY, controller
    // EINVAL, pseudo-file synthesis at mkdir, rmdir auto-reap, PID
    // movement via cgroup.procs). Proves SimCgroupFs's
    // non-replacement contract is honest.
    mod real_cgroup_fs {
        mod cgroup_kill_terminates_pids;
        mod controller_validation;
        mod probe_success;
        mod probe_with_custom_root;
        mod procs_pid_movement;
        mod pseudo_file_synthesis;
        mod rmdir_auto_reap;
        mod subtree_control_ebusy;
    }

    // Step 01-05 (cgroup-fs-port migration): E1 KEEP-TEMPFILE rows
    // 8 + 10 relocated here per ADR-0054 § D5. These tempfile-backed
    // tests against `RealCgroupFs` defend the substrate boundary —
    // that `CgroupManager` correctly propagates a REAL `io::Error`
    // from a REAL `tokio::fs::*` syscall against the REAL kernel VFS.
    // ENOTDIR-via-regular-file-in-dir-slot is a contrivance to
    // *trigger* the error; the test *boundary* is real.
    //
    // Step 01-08 (cgroup-fs-port migration): Class C
    // `write_to_readonly_cgroup_file` scenario — production-realistic
    // EACCES propagation from a kernel-read-only pseudo-file
    // (cgroup.events) through CgroupManager-wired RealCgroupFs to the
    // caller. Per DISTILL § E1 rows 8 / 10 rationale, this scenario
    // is the candidate replacement for the two KEEP-TEMPFILE rows
    // above; retirement of those is a follow-on DELIVER decision
    // once this scenario is green.
    mod cgroup_manager {
        mod cgroup_kill_propagates_real_io_error;
        mod remove_workload_scope_propagates_real_io_error;
        mod write_to_readonly_cgroup_file;
    }

    // transparent-mtls-host-socket (D-MTLS-14, GH #26; step 06-02) —
    // Tier 3 acceptance test for the worker's intercept-install +
    // leg-acquire role (`overdrive_worker::mtls_intercept`): IP_TRANSPARENT
    // leg-C listener, inbound nft-TPROXY install + RAII teardown, and the
    // outbound/inbound leg-acquire → `InterceptedConnection` build.
    mod mtls_intercept_install;

    // service-health-check-probes — Tier 3 integration tests for
    // the ProbeRunner subsystem per ADR-0054. Slices 01 / 02 / 03.
    // RED scaffolds — production bodies land in DELIVER.
    mod probe_runner {
        mod real_exec_probe_cgroup;
        mod real_http_probe;
        mod real_tcp_probe;
    }

    pub mod exec_driver {
        mod cgroup_procs;
        // Per-alloc RAII cleanup helper used by every real-cgroupfs test
        // below. Phase 02 of `fix-cgroup-subtree-control-delegation`
        // migrated the suite off `tempfile::TempDir` onto real
        // `/sys/fs/cgroup`; this guard reaps any leftover scope on
        // panic / SIGKILL so the next test's mkdir does not hit EEXIST.
        //
        // Re-used cross-sibling by the Class C real_cgroup_fs tests
        // (step 01-08), the cgroup_manager
        // write_to_readonly_cgroup_file test, and the probe_runner
        // Tier-3 suite (`real_exec_probe_cgroup.rs`, step 02-02) —
        // `pub` so siblings under the `integration` mod can import it
        // via `super::super::exec_driver::cleanup::AllocCleanup`. The
        // AllocCleanup shape is the canonical cgroup-leak-hygiene
        // primitive for the crate's real-cgroupfs tests.
        pub mod cleanup;
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
