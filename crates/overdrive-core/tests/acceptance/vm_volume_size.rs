//! Node-local persistent volume (GH #348, ADR-0310): the `VolumeSize` value an
//! operator writes as `[vm.volume] size = "40GiB"`.
//!
//! Scenarios: the grammar `<uint><unit>` accepts `GiB` and `TiB` in any case,
//! rejects everything else with a typed error, round-trips through its
//! canonical text and through serde, and renders canonically (`GiB` unless an
//! exact `TiB` multiple). Every test here is pending: the value type is a RED
//! scaffold (`overdrive_core::volume`), so each body panics at the missing
//! grammar. DELIVER removes the pending marker one test at a time.
//!
//! Every test here is about the grammar, so each one fails on its own accept or reject
//! property while the scaffold refuses every input. The tests that reject text carry a
//! positive control (a valid size the grammar must accept), so a scaffold that refuses
//! everything cannot satisfy them. The one test about `Display` builds its sizes through
//! `testing::volume`, so it fails on the rendering, not on the parser.
//!
//! Evidence tier: unit/property (layer 1, PBT). Production owner: the
//! `VolumeSize` value, reached through `FromStr`/`Display`/serde, which is
//! the only way the spec parser and the wire layer build one.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use overdrive_core::testing::volume::volume_size_gib;
use overdrive_core::volume::VolumeSize;
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    }
}

fn parse(text: &str) -> Result<VolumeSize, overdrive_core::volume::VolumeSizeParseError> {
    text.parse::<VolumeSize>()
}

proptest! {
    #![proptest_config(config())]

    /// A declared size survives text and serde unchanged, in any unit case.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "a positive count of GiB or TiB must parse")]
    fn a_declared_size_round_trips_through_text_and_serde_in_any_unit_case(
        n in 1u64..=1_000_000,
        unit in prop::sample::select(vec!["GiB", "TiB", "gib", "tib", "GIB", "TIB"]),
    ) {
        let text = format!("{n}{unit}");
        let size = parse(&text).expect("a positive count of GiB or TiB must parse");
        let canonical = size.to_string();
        prop_assert_eq!(parse(&canonical).expect("canonical text must parse"), size);
        let json = serde_json::to_string(&size).expect("serialises");
        prop_assert_eq!(json, format!("\"{canonical}\""));
        let back: VolumeSize = serde_json::from_str(&format!("\"{text}\"")).expect("wire text parses");
        prop_assert_eq!(back, size);
    }

    /// A count of GiB equals the same size written in TiB when it is an exact multiple.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "a TiB spelling and its GiB spelling must be the same size")]
    fn tib_and_gib_spellings_of_one_size_are_equal(t in 1u64..=4096) {
        let tib = parse(&format!("{t}TiB")).ok();
        let gib = parse(&format!("{}GiB", t * 1024)).ok();
        prop_assert!(
            tib.is_some() && tib == gib,
            "a TiB spelling and its GiB spelling must be the same size: {tib:?} vs {gib:?}"
        );
    }

    /// Anything outside `<uint><GiB|TiB>` is a typed error, never a size.
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[should_panic(expected = "the grammar must accept 40GiB while it refuses the near-miss")]
    fn text_outside_the_grammar_is_rejected(
        text in "[0-9]{0,6}(MiB|GB|KiB|PiB|G|T| GiB|GiB | TiB|\\.5GiB|)",
    ) {
        // The control: a grammar that refuses everything refuses the near-miss too.
        prop_assert!(
            parse("40GiB").is_ok(),
            "the grammar must accept 40GiB while it refuses the near-miss"
        );
        // The strategy only yields near-misses: a unit outside GiB/TiB, padding, a fraction, or no unit.
        prop_assert!(parse(&text).is_err(), "{text:?} must not parse");
    }
}

/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "40 GiB must display as 40GiB")]
fn display_prefers_tib_only_for_exact_multiples() {
    let shown = |gib: u64| volume_size_gib(gib).to_string();
    assert_eq!(shown(40), "40GiB", "40 GiB must display as 40GiB");
    assert_eq!(shown(1024), "1TiB", "1024 GiB must display as 1TiB");
    assert_eq!(shown(2048), "2TiB", "2048 GiB must display as 2TiB");
    assert_eq!(shown(1025), "1025GiB", "1025 GiB is not a TiB multiple and must display in GiB");
    assert_eq!(shown(3072), "3TiB", "3072 GiB must display as 3TiB");
}

/// Sad: each malformed declaration an operator can plausibly write.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(
    expected = "the grammar must accept a valid size while it refuses each malformed declaration"
)]
fn sad_malformed_declarations_are_rejected_with_a_typed_error() {
    assert!(
        parse("40GiB").is_ok(),
        "the grammar must accept a valid size while it refuses each malformed declaration"
    );
    for bad in [
        "",
        "0GiB",
        "0TiB",
        "GiB",
        "40",
        "40GB",
        "40MiB",
        "40 GiB",
        "-1GiB",
        "4.5GiB",
        "40GiB ",
        " 40GiB",
        "+40GiB",
        "٤٠GiB",
        "18446744073709551616TiB",
        "40GiBGiB",
    ] {
        assert!(parse(bad).is_err(), "{bad:?} must be rejected");
    }
}

/// Sad: the value is bytes; a size whose byte count overflows u64 is rejected, not wrapped.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the largest size whose bytes fit u64 must parse")]
fn sad_a_size_whose_bytes_overflow_is_rejected() {
    assert_eq!(
        parse("16777215TiB").expect("the largest size whose bytes fit u64 must parse").bytes(),
        u64::MAX - (1 << 40) + 1,
        "16777215 TiB is 2^64 - 2^40 bytes"
    );
    assert!(parse("16777216TiB").is_err(), "2^24 TiB is exactly 2^64 bytes and must not wrap");
    assert!(parse("17179869184TiB").is_err(), "2^34 TiB is 2^74 bytes and must not wrap");
}

/// Sad: a wire value that is not a valid size fails deserialisation.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[should_panic(expected = "the wire form of a valid size must deserialise")]
fn sad_serde_rejects_an_invalid_wire_size() {
    serde_json::from_str::<VolumeSize>("\"40GiB\"")
        .expect("the wire form of a valid size must deserialise");
    assert!(serde_json::from_str::<VolumeSize>("\"0GiB\"").is_err());
    assert!(serde_json::from_str::<VolumeSize>("40").is_err());
}
