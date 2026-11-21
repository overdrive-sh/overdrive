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

/// Operator wording for one cause, as the operator reads it on the allocation's `Failed` row.
fn wording(cause: VolumeUnavailableCause) -> String {
    TransitionReason::VmVolumeUnavailable { cause }.human_readable()
}

/// Phrases that send the operator to a node-local command. The appliance has no SSH and no
/// operator shell (CLAUDE.md), so none of these remedies can be performed.
const NODE_LOCAL_REMEDIES: [&str; 7] = [
    "lvm",
    "lvextend",
    "vgextend",
    "on the node",
    "check the pool",
    "create the volume pool",
    "free pool space",
];

/// The operator reads the volume, the cause and a remedy performable on the appliance: with no
/// pool the node is re-provisioned with one (VD-4, ADR-0313).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_pool_absent_wording_names_the_volume_the_cause_and_the_remedy() {
    let text = wording(VolumeUnavailableCause::PoolAbsent);
    let lower = text.to_lowercase();
    assert!(
        lower.contains("volume") && lower.contains("no volume pool"),
        "the pool-absent wording must name the volume and its cause: {text}"
    );
    assert!(
        lower.contains("re-provision"),
        "the pool-absent wording must carry an appliance remedy (re-provision the node with a volume pool): {text}"
    );
}

/// Both figures in their canonical forms, and a remedy performable on the appliance: declare a
/// size no larger than the free space, or re-provision with a larger pool (L4: the pool only
/// shrinks, nothing is ever deleted).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_insufficient_space_wording_names_both_figures_and_the_remedy() {
    let text = wording(VolumeUnavailableCause::InsufficientSpace {
        needed: volume_size_gib(40),
        free: ByteCount::new(4_194_304),
    });
    let lower = text.to_lowercase();
    assert!(
        text.contains("40GiB") && text.contains("4194304B"),
        "the insufficient-space wording must carry the declared size and the free bytes: {text}"
    );
    assert!(
        lower.contains("smaller") && lower.contains("re-provision"),
        "the insufficient-space wording must carry an appliance remedy (declare a smaller size, or re-provision with a larger pool): {text}"
    );
}

/// The failed operation is named, and the remedy is re-provisioning if it persists (the launch
/// is retried under the restart budget).
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_command_failure_wording_names_the_operation_and_the_remedy() {
    for operation in [
        VolumeOperation::Report,
        VolumeOperation::Create,
        VolumeOperation::Activate,
        VolumeOperation::Claim,
    ] {
        let text = wording(VolumeUnavailableCause::PoolCommandFailed { operation });
        let lower = text.to_lowercase();
        assert!(
            lower.contains("volume") && lower.contains(operation.as_str()),
            "the command-failure wording must name the volume and the operation: {text}"
        );
        assert!(
            lower.contains("retried") && lower.contains("re-provision"),
            "the command-failure wording must carry an appliance remedy (retried under the restart budget; re-provision if it persists): {text}"
        );
    }
}

/// A claim held past the wait bound names the holder remedy with the verb the CLI has.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_held_wording_names_the_volume_the_cause_and_the_remedy() {
    let text = wording(VolumeUnavailableCause::Held);
    let lower = text.to_lowercase();
    assert!(
        lower.contains("volume") && lower.contains("held"),
        "the held wording must name the volume and its cause: {text}"
    );
    assert!(
        lower.contains("overdrive job stop"),
        "the held wording must carry an appliance remedy (stop the workload holding the volume with `overdrive job stop`): {text}"
    );
}

/// No cause tells the operator to run a node-local command: there is no operator shell.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn no_volume_wording_sends_the_operator_to_a_node_local_command() {
    let causes = [
        VolumeUnavailableCause::PoolAbsent,
        VolumeUnavailableCause::InsufficientSpace {
            needed: volume_size_gib(40),
            free: ByteCount::new(4_194_304),
        },
        VolumeUnavailableCause::PoolCommandFailed { operation: VolumeOperation::Create },
        VolumeUnavailableCause::Held,
    ];
    for cause in causes {
        let text = wording(cause);
        let lower = text.to_lowercase();
        for phrase in NODE_LOCAL_REMEDIES {
            assert!(
                !lower.contains(phrase),
                "the volume wording must not tell the operator to run a node-local command (`{phrase}`): {text}"
            );
        }
    }
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
