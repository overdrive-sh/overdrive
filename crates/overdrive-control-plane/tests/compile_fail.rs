//! Trybuild harness for compile-fail invariants on `overdrive-control-plane`.
//!
//! Every file under `tests/compile_fail/*.rs` is compiled and its
//! stderr compared against the sibling `*.stderr` fixture. Regenerate
//! with `TRYBUILD=overwrite cargo test -p overdrive-control-plane
//! --test compile_fail` after an intentional compiler-diagnostic
//! change; the pinned trybuild version in the workspace manifest
//! exists to make that regeneration a deliberate act.
//!
//! # What this asserts
//!
//! * `LifecycleEvent` does NOT carry an `AllocStatusRow` field. This
//!   is the architecture.md §8 invariant: the broadcast-channel
//!   payload is a wire-shape projection (typed `from`/`to` states,
//!   typed `reason`, typed `source`) — NOT the raw observation row.
//!   Conflating the two would let a future refactor leak rkyv-archive
//!   types into the streaming-handler context.
//! * `ServerConfig::new` cannot be called with the KEK alone: the
//!   `MtlsIntercept` and `GuestDnsFactory` ports are required parameters
//!   (S-ND295-65; D-295-R16, E16), so no server is configured without
//!   protection and DNS.

#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
