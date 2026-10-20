//! Integration-test entrypoint for `overdrive-init` (a binary crate: these
//! tests drive its built artefacts, never its private items).
//!
//! The only binary here is the in-guest lane (GH #295, T-2): it boots the
//! guest-kit microVM with the production `overdrive-init` as PID 1 and runs
//! the crate's own in-guest real-kernel cells as that guest's operator
//! command. Gated `kvm-tests` (native `x86_64` metal,
//! `cargo xtask metal run --`); the inline `mod integration` shifts the
//! module lookup base into `integration/` (`.claude/rules/testing.md` § "Integration vs unit
//! gating").

#![cfg(all(feature = "integration-tests", feature = "kvm-tests"))]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod integration {
    mod in_guest_lane;
}
