//! Host-side dispatcher for the `overdrive-fwd` module suite and the per-test
//! guest attempts the Metal test wrapper starts. The implementation lives in
//! [`overdrive_testing::guest_suite`] so the dispatcher and the
//! `#[overdrive_test(guest = "…")]` attribute share one guest boot, copy-in and
//! exec path. Test-support executable only; never shipped by a production crate.

#![cfg(target_os = "linux")]

fn main() {
    overdrive_testing::guest_suite::run_dispatcher();
}
