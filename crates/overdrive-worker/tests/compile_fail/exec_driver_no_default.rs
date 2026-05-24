//! Compile-fail fixture: `ExecDriver` MUST NOT implement `Default`.
//!
//! Per `.claude/rules/development.md` § "Port-trait dependencies":
//! a defaulted constructor silently inherits production wiring
//! (wall-clock, real cgroupfs) into tests that forgot to override —
//! exactly the failure mode the trait surface exists to prevent.
//! The fixture asserts the structural invariant at compile time;
//! drift (e.g. a `#[derive(Default)]` slipped onto `ExecDriver`)
//! surfaces as a green PR check turning red.

use overdrive_worker::ExecDriver;

fn _no_default() {
    let _driver = ExecDriver::default();
}

fn main() {}
