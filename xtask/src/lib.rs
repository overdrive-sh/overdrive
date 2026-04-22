//! Library surface for `xtask` — exposes modules that integration tests
//! need to reach without going through the subprocess boundary.
//!
//! The binary entry point (`cargo xtask <cmd>`) lives in `src/main.rs`;
//! the shared implementations live here.

#![allow(clippy::expect_used, clippy::print_stderr, clippy::unnecessary_wraps)]

pub mod dst_lint;
