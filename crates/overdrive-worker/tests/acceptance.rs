//! Acceptance-test entrypoint for `overdrive-worker`.
//!
//! Default-lane tests only — pure-Rust, in-process. Runs on every
//! `cargo nextest run -p overdrive-worker` invocation without
//! `--features integration-tests`. Real-process / real-cgroup tests
//! live under `tests/integration/`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod acceptance {
    mod cgroup_path_roundtrip;
    mod cgroup_path_validation;
    mod sim_driver_only_in_default_lane;
}
