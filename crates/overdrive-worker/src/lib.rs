//! Overdrive worker subsystem — `ProcessDriver`, workload-cgroup
//! management, and the boot-time `node_health` row writer.
//!
//! Per ADR-0029, the worker subsystem is its own crate (class
//! `adapter-host`) so that the boundary between the control-plane and
//! the worker is enforced at compile time. The control-plane crate
//! sees only the `Driver` trait surface (from `overdrive-core`); the
//! impl is plugged in by the binary at AppState construction time.
//!
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 2 (US-02) and slice 4 (US-04
//! node_health writer half).
//! Wave: DISTILL. Every body in this crate is `panic!("Not yet
//! implemented -- RED scaffold")` per `.claude/rules/testing.md` §
//! RED scaffolds. The DELIVER crafter implements
//! `tokio::process::Command::spawn`, the five cgroupfs operations,
//! the SIGTERM-then-SIGKILL `stop` flow, and the boot-time
//! `node_health` write.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

/// SCAFFOLD marker — see this file's module docs.
pub const SCAFFOLD: bool = true;

pub mod cgroup_manager;
pub mod driver;
pub mod node_health;

pub use cgroup_manager::CgroupPath;
pub use driver::ProcessDriver;
