//! Node-local persistent volume (GH #348, ADR-0317, ADR-0318): a launch that
//! cannot obtain its volume is an ordinary start failure with a typed cause,
//! and the allocation's `Failed` row names it. Driven through the one total
//! driver-start conversion `TransitionReason::from(&DriverStartFailure)` and the
//! operator wording `human_readable()`.
//!
//! Evidence tier: unit (layer 1).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::TransitionReason;
use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::traits::driver::{DriverStartClass, DriverStartFailure, VmStartFailure};
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeUnavailableCause};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn failure(cause: VolumeUnavailableCause) -> DriverStartFailure {
    DriverStartFailure {
        class: DriverStartClass::Vm(VmStartFailure::VolumeUnavailable { cause }),
        detail: "volume".to_owned(),
    }
}

/// No volume pool on the node is recorded as the typed volume reason.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn pool_absent_maps_to_the_typed_volume_reason() {
    let reason = TransitionReason::from(&failure(VolumeUnavailableCause::PoolAbsent));
    assert!(
        matches!(
            &reason,
            TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::PoolAbsent }
        ),
        "a pool-absent start failure must map to VmVolumeUnavailable, got {reason:?}"
    );
}

/// A claim held past the bound maps to the typed volume reason.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn held_maps_to_the_typed_volume_reason() {
    let reason = TransitionReason::from(&failure(VolumeUnavailableCause::Held));
    assert!(
        matches!(
            &reason,
            TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::Held }
        ),
        "a held-claim start failure must map to VmVolumeUnavailable, got {reason:?}"
    );
}

/// The operator reads the volume, the cause and the remedy.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_operator_wording_names_the_volume_the_cause_and_the_remedy() {
    let held = TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::Held };
    let absent =
        TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::PoolAbsent };
    let (held_text, absent_text) = (held.human_readable(), absent.human_readable());
    assert!(held_text.contains("volume"), "the volume wording must name the cause: {held_text}");
    assert!(
        absent_text.contains("volume") && absent_text.to_lowercase().contains("pool"),
        "the volume wording must name the cause: {absent_text}"
    );
    assert_ne!(held_text, absent_text, "each cause reads differently");
}

/// A volume failure is a failure, never a progress marker.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guard_a_volume_failure_is_a_failure() {
    assert!(
        TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::PoolAbsent }
            .is_failure()
    );
}

/// S-VOL-26: not enough free extents is recorded as the typed volume reason, keeping the
/// declared size and the free space the operator needs to compare.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn insufficient_space_maps_to_the_typed_volume_reason_keeping_both_figures() {
    let needed = volume_size_gib(40);
    let free = ByteCount::new(4 * 1024 * 1024);
    let reason = TransitionReason::from(&failure(VolumeUnavailableCause::InsufficientSpace {
        needed,
        free,
    }));
    assert!(
        matches!(
            &reason,
            TransitionReason::VmVolumeUnavailable {
                cause: VolumeUnavailableCause::InsufficientSpace { needed: n, free: f }
            } if *n == needed && *f == free
        ),
        "an insufficient-space start failure must map to VmVolumeUnavailable got {reason:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: Some(Box::new(FileFailurePersistence::Off)), ..ProptestConfig::default() })]

    /// S-VOL-26: a command failure keeps the operation that was in progress, whichever it was.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_command_failure_maps_to_the_typed_volume_reason_keeping_the_operation(
        operation in prop_oneof![
            Just(VolumeOperation::Report),
            Just(VolumeOperation::Create),
            Just(VolumeOperation::Activate),
            Just(VolumeOperation::Claim),
        ],
    ) {
        let reason = TransitionReason::from(&failure(VolumeUnavailableCause::PoolCommandFailed { operation }));
        prop_assert!(
            matches!(
                &reason,
                TransitionReason::VmVolumeUnavailable { cause: VolumeUnavailableCause::PoolCommandFailed { operation: o } } if *o == operation
            ),
            "a command-failure start failure must map to VmVolumeUnavailable, got {reason:?}"
        );
    }
}

/// S-VOL-26: each cause reads differently and names its own remedy; the space and command
/// causes name the figures or the operation the operator acts on.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_space_and_command_causes_name_their_figures_and_operation() {
    let space = TransitionReason::VmVolumeUnavailable {
        cause: VolumeUnavailableCause::InsufficientSpace {
            needed: volume_size_gib(40),
            free: ByteCount::new(4_194_304),
        },
    }
    .human_readable();
    let command = TransitionReason::VmVolumeUnavailable {
        cause: VolumeUnavailableCause::PoolCommandFailed { operation: VolumeOperation::Create },
    }
    .human_readable();
    assert!(
        space.contains("40GiB") && space.contains("4194304"),
        "the space cause wording must name the declared size and the free bytes: {space}"
    );
    assert!(
        command.contains("create"),
        "the command cause wording must name the operation: {command}"
    );
    assert_ne!(space, command);
}
