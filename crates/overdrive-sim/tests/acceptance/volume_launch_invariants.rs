//! The seeded node-local volume launch invariants through the DST harness (GH #348, ADR-0317,
//! ADR-0318; S-VOL-48 `single_writer`, S-VOL-49 `data_survives_restart`, S-VOL-50
//! `no_drain_failure_charges_budget`).
//!
//! The driving port is the harness (`Harness::new().only(..).run(seed)`), the same surface the
//! catalogue's other pending invariants use: none of the three is in `Invariant::ALL` until the
//! volume launch path and `SimVolumeBacking` land, so the default catalogue and `cargo dst` are
//! unchanged and `--only` cannot name them yet. The evaluators drive `VmDriver::start` over
//! `SimVmm`, `SimClock` and a stepped `SimBeaconIo`, with the pool reaching the driver through
//! `SimVolumePoolProbe::probe` and `VmDriver::new(.., volumes, ..)`; the faults are `SimVolumeBacking`
//! scripting (a claim held until a drain, a crash between the create and the attach, a failing
//! command, a node reboot) and `SimClock`. The seed names the run: it is printed on every run and
//! the failure carries it with the schedule, so `Harness::new().only(<invariant>).run(<seed>)`
//! reproduces it.
//!
//! Pending marker: each test is `#[should_panic]` with the oracle the invariant fails on today:
//! the driver does not yet read `VmPayload.volume` and `SimVolumeBacking` is a scaffold, so a launch
//! that declares a volume boots a VMM with no second disk. DELIVER removes the marker when the
//! launch path and the sim land and the invariant holds at every seed.

#![allow(
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "CONTRACT_SHAPE metadata, fixture preconditions and the printed seed"
)]

use overdrive_sim::harness::{Harness, InvariantStatus};
use overdrive_sim::invariants::Invariant;
use overdrive_testing::placement;

placement!(host_pure);

/// Seeds covering the schedule space; each is one printed, reproducible run.
const SEEDS: std::ops::RangeInclusive<u64> = 1..=8;

fn assert_holds_at_every_seed(invariant: Invariant) {
    for seed in SEEDS {
        eprintln!("volume launch invariant {invariant} seed={seed}");
        let report = Harness::new().only(invariant).run(seed).expect("harness composes");
        let result = report
            .invariants
            .iter()
            .find(|result| result.name == invariant.as_canonical())
            .expect("the selected invariant ran");
        assert_eq!(
            result.status,
            InvariantStatus::Pass,
            "{invariant} must hold at seed {seed}: {}",
            result.cause.as_deref().unwrap_or("no cause reported")
        );
    }
}

/// S-VOL-48: no launch reaches the VMM while the claim is held, and at most one VMM is alive, for
/// any seeded schedule of launches, kills, delayed claim releases, reboots, control-plane restarts,
/// a crash between the create and the attach, and a failing command.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn single_writer_holds_for_every_seeded_schedule() {
    assert_holds_at_every_seed(Invariant::VolumeSingleWriter);
}

/// S-VOL-49: one volume is ever created for the workload and every launch attaches its one
/// device, for any seeded schedule.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn data_survives_restart_holds_for_every_seeded_schedule() {
    assert_holds_at_every_seed(Invariant::VolumeDataSurvivesRestart);
}

/// S-VOL-50: a successor that finds the claim still held waits for it (for a drain delay inside the
/// 30 s bound chosen by the seed): it neither fails nor creates a second VMM, and it eventually
/// attaches once the claim drains.
///
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn no_drain_failure_charges_the_budget_and_the_replacement_eventually_attaches() {
    assert_holds_at_every_seed(Invariant::VolumeNoDrainFailureChargesBudget);
}
