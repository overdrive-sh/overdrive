//! Node-local persistent volume (GH #348, ADR-0310, ADR-0315, ADR-0318): the
//! value types the volume contract carries across the port, the operator wire
//! and the VMM argument. Driven through each type's own validating surface
//! (`FromStr`, `Display`, serde, `as_str`, `to_disk_arg`).
//!
//! Evidence tier: unit and property (layer 1, Mandate 9).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::path::PathBuf;

use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::vm::config::DiskAttachment;
use overdrive_core::volume::{ByteCount, VolumeOperation, VolumeSize, VolumeUnavailableCause};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

// ----- ByteCount: the free-space value `InsufficientSpace` carries --------------------

proptest! {
    #![proptest_config(config())]

    /// Any byte count, including 0 and `u64::MAX`, displays as `<n>B` and parses back; the
    /// unit letter is case-insensitive, as for `VolumeSize`.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn a_byte_count_round_trips_through_text_and_serde(
        bytes in any::<u64>(),
        unit in prop::sample::select(vec!["B", "b"]),
    ) {
        let count = ByteCount::new(bytes);
        prop_assert_eq!(count.to_string(), format!("{bytes}B"), "a byte count must display as <n>B");
        prop_assert_eq!(
            format!("{bytes}{unit}").parse::<ByteCount>().expect("either unit case must parse"),
            count
        );
        prop_assert_eq!(
            count.to_string().parse::<ByteCount>().expect("the displayed byte count must parse"),
            count
        );
        let json = serde_json::to_string(&count).expect("serialises");
        prop_assert_eq!(json, format!("\"{bytes}B\""));
        prop_assert_eq!(
            serde_json::from_str::<ByteCount>(&format!("\"{bytes}B\"")).expect("wire text parses"),
            count
        );
    }

    /// Sad: text that is not `<uint>B` (no unit, another unit, a sign, padding, non-digits) is refused.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn text_outside_the_byte_count_grammar_is_rejected(
        text in prop_oneof![
            "[0-9]{1,10}",
            "[0-9]{1,10}(GiB|MiB|KB|G|K)",
            "[-+ ][0-9]{1,10}B",
            "[0-9]{1,10}B ",
            "[a-z]{1,8}B",
            Just(String::new()),
            Just("B".to_owned()),
        ],
    ) {
        // The control: a grammar that refuses everything refuses the malformed text too.
        prop_assert!(
            "4096B".parse::<ByteCount>().is_ok(),
            "the grammar must accept <n>B while it refuses text outside it"
        );
        prop_assert!(text.parse::<ByteCount>().is_err(), "{text:?} must be refused");
    }
}

/// Sad: a byte count above `u64::MAX` is refused, never wrapped.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn sad_a_byte_count_above_the_u64_range_is_rejected() {
    assert_eq!(
        "18446744073709551615B".parse::<ByteCount>().expect("u64::MAX bytes must parse").get(),
        u64::MAX
    );
    assert!("18446744073709551616B".parse::<ByteCount>().is_err());
}

// ----- VolumeSize::bytes ------------------------------------------------------------------

/// The one accessor the host adapter and the sim read the size through yields the declared bytes.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_size_in_bytes_is_the_declared_binary_multiple() {
    assert_eq!(
        "40GiB".parse::<VolumeSize>().expect("40GiB must parse to 40 binary gibibytes").bytes(),
        40 * 1024 * 1024 * 1024
    );
    assert_eq!(
        "2TiB".parse::<VolumeSize>().expect("2TiB must parse to 2 binary tebibytes").bytes(),
        2 * 1024 * 1024 * 1024 * 1024
    );
}

// ----- VolumeOperation and the causes that carry it ----------------------------------

/// The operation labels are enum-owned and equal their serde spelling.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guard_volume_operation_labels_match_serde() {
    for operation in [
        VolumeOperation::Report,
        VolumeOperation::Create,
        VolumeOperation::Activate,
        VolumeOperation::Claim,
    ] {
        assert_eq!(
            serde_json::to_string(&operation).unwrap(),
            format!("\"{}\"", operation.as_str())
        );
    }
}

/// The free space on the wire is the `<n>B` string, never a bare number.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_insufficient_space_cause_carries_free_space_as_a_string() {
    let cause = VolumeUnavailableCause::InsufficientSpace {
        needed: volume_size_gib(40),
        free: ByteCount::new(4_194_304),
    };
    let json = serde_json::to_string(&cause).unwrap();
    assert!(
        json.contains("\"free\":\"4194304B\"") && json.contains("40GiB"),
        "the free space must travel as the <n>B string beside the declared size: {json}"
    );
    assert_eq!(serde_json::from_str::<VolumeUnavailableCause>(&json).unwrap(), cause);
}

// ----- DiskAttachment: the one render site of the volume's `--disk` (ADR-0315) -------

/// The volume disk renders with an explicit raw image type, direct I/O and the fixed
/// serial: the argv golden of the single render site.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn the_volume_disk_renders_the_golden_argument() {
    let device = PathBuf::from("/dev/mapper/overdrive-ovd--0123456789abcdef0123456789abcdef");
    assert_eq!(
        DiskAttachment::volume(device).to_disk_arg(),
        "path=/dev/mapper/overdrive-ovd--0123456789abcdef0123456789abcdef,image_type=raw,direct=on,serial=ovd-volume",
        "the volume --disk argument must render direct=on and serial=ovd-volume"
    );
}

/// The rootfs attachment keeps its shape: no serial, no direct I/O; the read-only flag is its only option.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn guard_the_rootfs_disk_argument_is_unchanged() {
    assert_eq!(
        DiskAttachment::new(PathBuf::from("/run/overdrive/vm/a/rootfs.img"), false).to_disk_arg(),
        "path=/run/overdrive/vm/a/rootfs.img,image_type=raw"
    );
    assert_eq!(
        DiskAttachment::new(PathBuf::from("/run/overdrive/vm/a/rootfs.img"), true).to_disk_arg(),
        "path=/run/overdrive/vm/a/rootfs.img,image_type=raw,readonly=on"
    );
}
