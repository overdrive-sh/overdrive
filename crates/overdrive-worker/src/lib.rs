//! Overdrive worker subsystem — `ProcessDriver`, workload-cgroup
//! management, and the boot-time `node_health` row writer.
//!
//! Per ADR-0029, the worker subsystem is its own crate (class
//! `adapter-host`) so that the boundary between the control-plane and
//! the worker is enforced at compile time. The control-plane crate
//! sees only the `Driver` trait surface (from `overdrive-core`); the
//! impl is plugged in by the binary at `AppState` construction time.
//!
//! # Status
//!
//! Phase: phase-1-first-workload, slice 2 (US-02) GREEN landed; slice
//! 4 (US-04 `node_health` writer half) remains RED scaffold until
//! delivered.

// `forbid(unsafe_code)` is intentionally NOT set: `Driver::stop` on
// Linux invokes `libc::kill(pid, SIGTERM)`, which requires `unsafe`.
// Per `.claude/rules/development.md`, the worker crate is class
// `adapter-host` — host-OS interaction is its raison d'être. The
// workspace-wide `unsafe_op_in_unsafe_fn = deny` lint still requires
// every `unsafe { ... }` block to be explicit, with a `// SAFETY:`
// comment documenting the precondition.
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

/// SCAFFOLD marker — see this file's module docs.
pub const SCAFFOLD: bool = true;

pub mod cgroup_manager;
pub mod driver;
pub mod node_health;

pub use cgroup_manager::CgroupPath;
pub use driver::ProcessDriver;
