//! Acceptance-test entrypoint for `overdrive-worker`.
//!
//! Default-lane tests only — pure-Rust, in-process. Runs on every
//! `cargo nextest run -p overdrive-worker` invocation without
//! `--features integration-tests`. Real-process / real-cgroup tests
//! live under `tests/integration/`.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod acceptance {
    mod cgroup_manager;
    mod cgroup_path_roundtrip;
    mod cgroup_path_validation;
    mod sim_cgroup_fs;
    mod sim_driver_only_in_default_lane;

    // service-health-check-probes — Tier 1 acceptance (Sim adapters)
    // for the ProbeRunner subsystem per ADR-0054. Slices 01 / 02 / 03.
    // RED scaffolds — production bodies land in DELIVER.
    mod probe_runner_exec_outcome;
    mod probe_runner_http_outcome;
    // GAP-7 closure — `ProbeRunner::start_alloc` spawns per-descriptor
    // supervised tick tasks. See
    // `.context/01-03-structural-gap-audit.md` GAP-7.
    mod probe_runner_supervised_tick;
    mod probe_runner_tcp_outcome;
}
