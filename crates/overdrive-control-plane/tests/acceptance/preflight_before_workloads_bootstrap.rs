//! Regression test for the ADR-0028 ordering invariant: the cgroup v2
//! delegation preflight MUST execute before the workloads-slice
//! bootstrap in `run_server`. Without this ordering, a misconfigured
//! host sees `WorkloadsBootstrap(WriteFailed: PermissionDenied)`
//! instead of the actionable `CgroupBootstrap(DelegationMissing)`
//! message — the remediation hint never fires.
//!
//! The test reads `lib.rs` source and asserts that `run_preflight()`
//! appears before `create_workloads_slice_with_controllers()` within
//! the `run_server` function body. This is a structural guard: future
//! refactors that reorder the boot path will break this test before
//! operators see the wrong error.

#[test]
fn preflight_precedes_workloads_bootstrap_in_run_server() {
    let lib_src = include_str!("../../src/lib.rs");

    // Locate the `run_server` function body (not `run_server_with_obs_and_driver`).
    let fn_start =
        lib_src.find("pub async fn run_server(").expect("run_server function must exist in lib.rs");

    // Find the end of `run_server` by locating the call to
    // `run_server_with_obs_and_driver` which is its last statement.
    let fn_body = &lib_src[fn_start..];
    let fn_end_marker = fn_body
        .find("run_server_with_obs_and_driver(")
        .expect("run_server must call run_server_with_obs_and_driver");
    let fn_body = &fn_body[..fn_end_marker];

    let preflight_pos =
        fn_body.find("run_preflight()").expect("run_preflight() must be called inside run_server");

    let bootstrap_pos = fn_body
        .find("create_workloads_slice_with_controllers()")
        .expect("create_workloads_slice_with_controllers() must be called inside run_server");

    assert!(
        preflight_pos < bootstrap_pos,
        "ADR-0028 ordering violation: run_preflight() (byte {preflight_pos}) \
         must appear BEFORE create_workloads_slice_with_controllers() \
         (byte {bootstrap_pos}) in run_server — the preflight must execute \
         before any on-disk cgroup side effects"
    );
}
