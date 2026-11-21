//! Node-local persistent volume (GH #348, ADR-0312, ADR-0317, ADR-0318): the
//! `VolumeBacking` port contract, exercised on its simulation twin. The same
//! scenarios run against `LvmVolumeBacking` at Tier 3
//! (`crates/overdrive-worker/tests/integration/volume_pool/`), so a divergence
//! between the twins surfaces as one failing pair (development.md, "The DST
//! equivalence test is the structural guard").
//!
//! Driving port: the `VolumeBacking` trait only (`ensure`, `claim_state`).
//! Faults enter through `SimVolumeBacking`'s scripting (`hold_claim`, `drain`,
//! `node_reboot`, a small pool, a failing `claim_state`). Every test is pending: the
//! sim is a RED scaffold whose `ensure` fails with a typed `PoolCommand` naming the scaffold,
//! whose `claim_state` reports `NotFound` and whose fault methods are ignored.
//!
//! Each pending marker names the assertion the test exists to make, or the precondition the
//! scaffold's failing `ensure` denies it. Sizes are built through `testing::volume`, never the
//! grammar, so no test here depends on a parser that is not written.
//!
//! Evidence tier: in-memory acceptance (layer 2) with a layer-1 model property.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::id::WorkloadId;
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::volume_backing::{ClaimState, VolumeBacking, VolumeBackingError};
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeSize};
use overdrive_sim::SimVolumeBacking;
use overdrive_testing::placement;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

placement!(host_pure);

fn gib(count: u64) -> VolumeSize {
    volume_size_gib(count)
}

fn workload(id: &str) -> WorkloadId {
    WorkloadId::new(id).expect("a valid workload id")
}

fn pool(capacity_gib: u64) -> SimVolumeBacking {
    SimVolumeBacking::with_capacity(gib(capacity_gib))
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime").block_on(f)
}

/// Gate available (boundary 1): a new volume is created, activated, returned, and unclaimed.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ensure_creates_the_volume_and_leaves_the_claim_free() {
    let sim = pool(1024);
    let w = workload("dev-box");
    let device =
        run(sim.ensure(&w, gib(40))).expect("ensure on a pool with space must create the volume");
    assert!(device.starts_with("/dev/mapper"), "the device is a mapper node: {device:?}");
    assert_eq!(sim.volumes_created(), 1);
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Free,
        "a new volume's claim is free"
    );
}

/// A repeat `ensure` (a relaunch, or the retry after a control-plane crash)
/// returns the same device and creates nothing new; the existing size is not compared.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn ensure_is_idempotent_and_does_not_compare_the_existing_size() {
    let sim = pool(1024);
    let w = workload("dev-box");
    let first = run(sim.ensure(&w, gib(40))).expect("the first ensure must create the volume");
    let again = run(sim.ensure(&w, gib(40))).expect("a repeat ensure must return the volume");
    let other_size =
        run(sim.ensure(&w, gib(80))).expect("an ensure with another size must return the volume");
    assert_eq!((first.clone(), first), (again, other_size));
    assert_eq!(sim.volumes_created(), 1, "one workload, one volume");
}

/// Two workloads get two distinct volumes.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn each_workload_gets_its_own_volume() {
    let sim = pool(1024);
    let a = run(sim.ensure(&workload("alpha"), gib(10)))
        .expect("each workload's ensure must create its own volume");
    let b = run(sim.ensure(&workload("beta"), gib(10)))
        .expect("each workload's ensure must create its own volume");
    assert_ne!(a, b);
    assert_eq!(sim.volumes_created(), 2);
}

/// Sad (boundary 2): not enough free extents fails typed, naming the declared
/// size, and creates nothing.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn sad_insufficient_space_fails_typed_and_creates_nothing() {
    let sim = pool(10);
    let err = run(sim.ensure(&workload("dev-box"), gib(40))).expect_err("40GiB does not fit 10GiB");
    assert!(
        matches!(err, VolumeBackingError::InsufficientSpace { needed, free }
            if needed == gib(40) && free == ByteCount::new(10 * 1024 * 1024 * 1024)),
        "an ensure that does not fit must fail InsufficientSpace naming both figures, got {err:?}"
    );
    assert_eq!(sim.volumes_created(), 0);
}

/// Sad (F-3): an `open(2)` failure other than `EBUSY` and `ENOENT` on the device node is
/// `PoolCommand { operation: Claim }`, not `NotFound` and not `Held`; the fault is one-shot, so
/// the volume's claim reads `Free` on the very next call.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn sad_a_claim_state_open_failure_is_a_claim_pool_command_error_and_the_next_call_recovers() {
    let sim = pool(1024);
    let w = workload("dev-box");
    run(sim.ensure(&w, gib(40)))
        .expect("ensure must create the volume whose claim_state fault is armed");
    sim.fail_next_claim_state();
    let err = run(sim.claim_state(&w))
        .expect_err("an armed claim_state fault must fail as a Claim pool command error");
    assert!(
        matches!(err, VolumeBackingError::PoolCommand { operation: VolumeOperation::Claim, .. }),
        "an armed claim_state fault must fail as a Claim pool command error, got {err:?}"
    );
    assert_eq!(
        run(sim.claim_state(&w)).expect("the fault was one-shot"),
        ClaimState::Free,
        "the call after the armed one answers normally"
    );
}

/// Sad: asking for the claim of a volume nobody created is `NotFound`.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_claim_state_of_an_absent_volume_is_not_found() {
    let sim = pool(1024);
    // The control: a sim that reports NotFound for everything also reports it for the ghost.
    let real = workload("dev-box");
    let created = run(sim.ensure(&real, gib(40))).and_then(|_| run(sim.claim_state(&real))).ok();
    assert_eq!(
        created,
        Some(ClaimState::Free),
        "a created volume's claim must be Free while an uncreated one's is NotFound"
    );
    let err = run(sim.claim_state(&workload("ghost"))).expect_err("no such volume");
    assert!(matches!(err, VolumeBackingError::NotFound { .. }), "got {err:?}");
}

/// The claim is `Held` while a VMM has the device open and `Free` once its
/// in-flight I/O drained (spike B2b: the claim outlives the process).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn the_claim_is_held_until_the_old_writer_drains() {
    let sim = pool(1024);
    let w = workload("dev-box");
    run(sim.ensure(&w, gib(40))).expect("ensure must create the volume whose claim is read");
    sim.hold_claim(&w);
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Held,
        "a held claim must read Held"
    );
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Held,
        "reading the claim never releases it"
    );
    sim.drain(&w);
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Free,
        "a drained claim must read Free"
    );
}

/// A claim linked to a VMM process (test support for a crashed control plane's orphan) reads
/// `Held` while the process is live and `Free` once it is gone, with no `drain` step.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn a_claim_linked_to_a_vmm_is_released_when_the_vmm_dies() {
    let sim = pool(1024);
    let w = workload("dev-box");
    run(sim.ensure(&w, gib(40)))
        .expect("ensure must create the volume whose claim is linked to a VMM");
    let process_live = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let probe = std::sync::Arc::clone(&process_live);
    sim.hold_claim_while_live(&w, move || probe.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Held,
        "a claim linked to a live VMM must read Held"
    );
    process_live.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        run(sim.claim_state(&w)).expect("a created volume has a claim state"),
        ClaimState::Free,
        "the death of the linked VMM must release its claim without a drain step"
    );
}

/// Node reboot (boundary 5): the volume is present but inactive, so it has no
/// device node and `claim_state` reports `NotFound`; `ensure` activates the same
/// volume without creating another.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn after_a_node_reboot_the_volume_is_inactive_until_ensure_reactivates_it() {
    let sim = pool(1024);
    let w = workload("dev-box");
    let before =
        run(sim.ensure(&w, gib(40))).expect("ensure must create the volume the reboot deactivates");
    sim.hold_claim(&w);
    sim.node_reboot();
    let err = run(sim.claim_state(&w)).expect_err("an inactive volume has no device node");
    assert!(matches!(err, VolumeBackingError::NotFound { .. }), "got {err:?}");
    let after =
        run(sim.ensure(&w, gib(40))).expect("ensure after a reboot must reactivate the volume");
    assert_eq!(before, after);
    assert_eq!(sim.volumes_created(), 1, "reactivation creates nothing");
    assert_eq!(
        run(sim.claim_state(&w)).expect("a reactivated volume has a claim state"),
        ClaimState::Free,
        "a reboot released every claim"
    );
}

#[derive(Debug, Clone)]
enum Op {
    Ensure(usize),
    Hold(usize),
    Drain(usize),
    Reboot,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0usize..3).prop_map(Op::Ensure),
        (0usize..3).prop_map(Op::Hold),
        (0usize..3).prop_map(Op::Drain),
        Just(Op::Reboot),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: Some(Box::new(FileFailurePersistence::Off)), ..ProptestConfig::default() })]

    /// For any sequence of ensure, hold, drain and reboot: a volume is created exactly once per
    /// ensured workload, a claim is `Held` only on an active volume (an inactive or never-created
    /// one reads `NotFound`, never `Held`), and a successful `ensure` always leaves an active,
    /// claimable volume.
    /// CONTRACT_SHAPE: bounded-change.
    #[test]
    fn model_volumes_are_created_once_and_claims_follow_activity(ops in prop::collection::vec(op_strategy(), 0..24)) {
        let sim = pool(1024);
        let ids = [workload("alpha"), workload("beta"), workload("gamma")];
        // The model: a volume is `ensured` once created, `active` while it has a device node (every
        // reboot clears it, `ensure` sets it), and `held` while a VMM has it open (a reboot or a
        // drain clears it; holding needs an active volume).
        let mut ensured = [false; 3];
        let mut active = [false; 3];
        let mut held = [false; 3];
        for op in ops {
            match op {
                Op::Ensure(i) => {
                    run(sim.ensure(&ids[i], gib(10))).expect("1TiB holds three 10GiB volumes");
                    ensured[i] = true;
                    active[i] = true;
                }
                Op::Hold(i) if active[i] && !held[i] => {
                    sim.hold_claim(&ids[i]);
                    held[i] = true;
                }
                Op::Drain(i) if ensured[i] => {
                    sim.drain(&ids[i]);
                    held[i] = false;
                }
                Op::Reboot => {
                    sim.node_reboot();
                    active = [false; 3];
                    held = [false; 3];
                }
                _ => {}
            }
            for i in 0..3 {
                let state = run(sim.claim_state(&ids[i]));
                if active[i] {
                    let expected = if held[i] { ClaimState::Held } else { ClaimState::Free };
                    prop_assert_eq!(
                        state.ok(),
                        Some(expected),
                        "an active volume reads Held iff a holder has it open, volume {}", i
                    );
                } else {
                    prop_assert!(
                        matches!(state, Err(VolumeBackingError::NotFound { .. })),
                        "a claim is held only on an active volume: volume {} is inactive or never created, got {:?}", i, state
                    );
                }
            }
            prop_assert_eq!(sim.volumes_created(), ensured.iter().filter(|e| **e).count());
        }
    }
}

// ----- S-VOL-39: the command failure and the deadline ---------------------------------------------

/// Sad: an `lvm` command that fails is `PoolCommand` naming the operation in progress, creates
/// nothing, and leaves the claim state untouched; the very next `ensure` (no fault armed) succeeds.
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn sad_a_failing_command_is_a_pool_command_error_and_the_next_ensure_recovers() {
    let sim = pool(1024);
    let w = workload("dev-box");
    sim.fail_next_command();
    let err = run(sim.ensure(&w, gib(40))).expect_err("the armed command fails");
    assert!(
        matches!(err, VolumeBackingError::PoolCommand { operation, .. }
            if matches!(operation, VolumeOperation::Report | VolumeOperation::Create | VolumeOperation::Activate)),
        "got {err:?}"
    );
    assert_eq!(sim.volumes_created(), 0, "a failed command leaves nothing behind");
    run(sim.ensure(&w, gib(40))).expect("the fault was one-shot");
    assert_eq!(sim.volumes_created(), 1);
}

/// A failure after the create (the activate step) still leaves the volume: the retry reuses it
/// rather than creating a second one (`I_ENSURE_IDEMPOTENT_UNDER_CRASH`).
/// CONTRACT_SHAPE: bounded-change.
#[test]
fn an_ensure_that_failed_after_the_create_is_reused_by_the_retry() {
    let sim = pool(1024);
    let w = workload("dev-box");
    sim.pause_ensure_after_create(&w);
    let (stopped, parked_after_create) = run(async {
        let mut ensure = Box::pin(sim.ensure(&w, gib(40)));
        let stopped = tokio::select! {
            result = &mut ensure => Some(result),
            () = tokio::time::sleep(std::time::Duration::from_millis(20)) => None,
        };
        // Observed while the ensure future is still alive, before it is dropped as a crash would.
        (stopped.is_none(), sim.ensure_parked(&w))
    });
    assert!(stopped && parked_after_create, "the first ensure is parked after its create");
    assert!(!sim.ensure_parked(&w), "dropping the parked ensure clears its parked state");
    sim.resume_ensure(&w);
    run(sim.ensure(&w, gib(40))).expect("the retry finds the volume and activates it");
    assert_eq!(sim.volumes_created(), 1, "never a second volume");
}

// ----- S-VOL-26b: the volume's name is a pure function of the workload id (ADR-0312) -------------
//
// The derivation has no shared core function: each adapter derives the name itself, so this golden
// (the same "dev-box" vector the host adapter's Tier-3 test pins) is the cross-adapter equivalence.

/// The ADR-0312 golden vectors: the logical volume is `ovd-` followed by the first 32 hexadecimal
/// characters of the SHA-256 of the workload id, and its device is the device-mapper node
/// `/dev/mapper/overdrive-ovd--<hex>` (dm name `<vg>-<lv>` with every `-` doubled; the hex has none).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_volume_name_is_ovd_and_the_sha256_prefix_of_the_workload_id() {
    let sim = pool(1024);
    for (id, hex) in [
        ("dev-box", "8cc272549bf232c0ea5e367b6b44997d"),
        ("alpha", "8ed3f6ad685b959ead7022518e1af76c"),
        ("a", "ca978112ca1bbdcafac231b39a23dc4d"),
        ("payments-db", "5b3e0ea9e1f88fbab9173631e763409d"),
    ] {
        let device =
            run(sim.ensure(&workload(id), gib(1))).expect("ensure must return the volume's device");
        assert_eq!(
            device,
            std::path::PathBuf::from(format!("/dev/mapper/overdrive-ovd--{hex}")),
            "{id}"
        );
    }
}
