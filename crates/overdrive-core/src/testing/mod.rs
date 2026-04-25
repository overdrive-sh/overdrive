//! Trait-conformance harnesses for `overdrive-core` injectable boundaries.
//!
//! Each submodule exposes a generic `run_*_conformance<T: Trait>(t: &T)`
//! function exercising the full contract of a trait against any
//! implementation. Adapter test suites invoke the matching harness so a
//! divergence between adapter implementations is caught at the trait
//! level — see
//! `docs/feature/fix-observation-lww-merge/deliver/rca.md` Cause B
//! (test coverage shape: invariant asserted at impl level, never at
//! trait level) for the motivation.

pub mod observation_store;
