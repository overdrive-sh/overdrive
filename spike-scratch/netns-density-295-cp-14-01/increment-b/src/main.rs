//! One-shot STEP 14-01 control-plane-prefix diagnostic launcher.
//!
//! The current production runner keeps its exact GuestKit lifecycle private.
//! Generate that source without its file-level docs/attributes, then call its
//! existing private `run_nested_vm_test_binary` helper with the already-authored
//! first three control-plane cells. This crate adds no selector or dependency
//! to the workspace and is never built by a workspace gate.

#[allow(
    dead_code,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "the included production runner keeps its existing diagnostics and unused groups"
)]
mod existing_runner {
    include!(concat!(env!("OUT_DIR"), "/fwd_guest_suite_diagnostic.rs"));
}

fn main() {
    existing_runner::run_exact_control_plane_prefix();
}
