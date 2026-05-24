//! Compile-fail fixture: `ExecDriver::new` MUST reject a call that
//! omits the `fs: Arc<dyn CgroupFs>` parameter.
//!
//! # RED scaffold (step 01-01)
//!
//! This fixture is **not yet active** — see `tests/compile_fail.rs`
//! for the deferred-activation rationale. The fixture body below is
//! intentionally a stub; the real body (the call shape that must
//! fail to compile post-01-05) lands at step 01-05 alongside the
//! `ExecDriver::new` arity change.
//!
//! When activated, this file will attempt a 2-argument call against
//! the post-01-05 3-argument `ExecDriver::new` signature. The
//! `.stderr` companion pins the diagnostic shape so future arity
//! drift (e.g. an accidentally-defaulted `fs` parameter) is caught
//! at PR time.

fn main() {}
