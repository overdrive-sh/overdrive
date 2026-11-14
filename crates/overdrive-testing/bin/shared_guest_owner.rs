//! The shared-guest owner: one process per `shared_guest!` declaration per run,
//! started by the first test of the declaration to arrive and serving the tests
//! that follow. `overdrive_testing::placement::owner` is the implementation;
//! `cargo xtask guest-kit-prepare` builds this binary at
//! `<target>/debug/overdrive-shared-guest-owner`. Test-support executable only;
//! never shipped by a production crate.

#![cfg(target_os = "linux")]

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    overdrive_testing::placement::owner::run(&arguments);
}
