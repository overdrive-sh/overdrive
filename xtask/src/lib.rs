//! Library surface for `xtask` — exposes modules that integration tests
//! need to reach without going through the subprocess boundary.
//!
//! The binary entry point (`cargo xtask <cmd>`) lives in `src/main.rs`;
//! the shared implementations live here.

#![allow(clippy::expect_used, clippy::print_stderr, clippy::unnecessary_wraps)]

pub mod dev_setup;
pub mod dst;
pub mod dst_lint;
pub mod mutants;
pub mod openapi;
pub mod yaml_free_cli;
