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
    // `cgroup_path_roundtrip` / `cgroup_path_validation` RELOCATED to
    // `overdrive-core/tests/acceptance/` (review remediation, step 01-01
    // F6) — `CgroupPath` itself lives in `overdrive_core::cgroup`
    // (ADR-0082 §D2, gap 1, GH #42); testing it only through this crate's
    // re-export left a scoped `-p overdrive-core` mutation run blind to
    // the `FromStr` rejection killers.
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
    // microvm-driver-cloud-hypervisor (GH #42), step 01-07 — S-VM-76 +
    // crafter-authored race-arm examples against SimVmm (ADR-0082
    // §§D3-D4).
    mod vm_driver_stop_totality;
}
