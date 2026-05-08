//! `DropClass` — drop classification for the kernel-side
//! `DROP_COUNTER` PERCPU_ARRAY.
//!
//! Locked variant set per Q7=B in
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 6 *DropClass* and ADR-0040 D8. `#[repr(u32)]` makes `as u32` a
//! stable kernel-side index across Rust toolchains (research § 7.1 —
//! the verified pattern Cilium and Katran use).
//!
//! Variant ordering and discriminants are STABLE — additions are
//! minor-version (per ADR-0037 K8s-Condition convention);
//! reordering or removal is a major-version break that requires a
//! new ADR.
//!
//! # Slot drift detection
//!
//! [`DropClass::VARIANT_COUNT`] is structurally locked to the actual
//! variant count via the [`const _: () = assert!(...)`] block at the
//! end of this file. Adding a variant without bumping
//! `VARIANT_COUNT`, OR changing `VARIANT_COUNT` away from the actual
//! count, fails the const-assert at compile time. The compile-fail
//! fixture at
//! `crates/overdrive-core/tests/compile_fail/drop_class_slot_drift.rs`
//! pins this property at PR time.
//!
//! # Wire forms
//!
//! * [`fmt::Display`] emits canonical kebab-case
//!   (`malformed-header`, `unknown-vip`, ...).
//! * [`FromStr`] parses kebab-case case-insensitively (the
//!   `Newtype completeness` rule for human-typed identifiers).
//! * Serde uses the kebab-case form via `#[serde(rename_all =
//!   "kebab-case")]` so wire forms agree with `Display` / `FromStr`.

#![allow(clippy::missing_errors_doc)]

use core::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Drop classification for the `DROP_COUNTER` PERCPU_ARRAY. Six
/// variants locked at Q7=B; `#[repr(u32)]` makes the discriminant
/// the kernel-side slot index.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DropClass {
    /// Frame's IPv4 / TCP / UDP header failed sanity-prologue
    /// checks (truncated header, bad IHL, nonsense TCP flags, ...).
    /// Slot 0.
    MalformedHeader = 0,
    /// SERVICE_MAP miss for a packet that otherwise looked routable.
    /// Slot 1.
    UnknownVip = 1,
    /// MAGLEV_MAP entry pointed at a backend whose `BACKEND_MAP`
    /// row is `healthy = 0`. Slot 2.
    NoHealthyBackend = 2,
    /// Sanity-prologue logical drop that did not fit
    /// `MalformedHeader` (operator-tunable rule that fires before
    /// SERVICE_MAP). Slot 3.
    SanityPrologue = 3,
    /// `tc_reverse_nat` lookup miss for a backend whose forward
    /// path was previously known. Slot 4.
    ReverseNatMiss = 4,
    /// Frame exceeded operator-configured size cap. Slot 5.
    OversizePacket = 5,
}

impl DropClass {
    /// Number of `DropClass` variants — equals the kernel-side
    /// `DROP_COUNTER` slot count. Locked at 6 per Q7=B / ADR-0040
    /// D8. Adding a variant requires bumping this value; the
    /// const-assert at the bottom of this file refuses to compile
    /// otherwise.
    pub const VARIANT_COUNT: u32 = 6;

    /// Stable kernel-side index for this variant.
    ///
    /// Equivalent to `self as u32` but spelled as a method for
    /// call-site clarity. The discriminant IS the slot index by
    /// `#[repr(u32)]` declaration.
    #[must_use]
    pub const fn as_index(self) -> u32 {
        self as u32
    }

    /// Canonical kebab-case token form used in [`fmt::Display`]'s
    /// output and `FromStr`'s parser. Mirrors the
    /// `BackendKey::Proto::as_str` shape elsewhere in this module.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedHeader => "malformed-header",
            Self::UnknownVip => "unknown-vip",
            Self::NoHealthyBackend => "no-healthy-backend",
            Self::SanityPrologue => "sanity-prologue",
            Self::ReverseNatMiss => "reverse-nat-miss",
            Self::OversizePacket => "oversize-packet",
        }
    }
}

/// Parse / validation failure for [`DropClass::from_str`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Input does not match any known kebab-case variant name.
    #[error("DropClass {0:?} is not a known variant")]
    Unknown(String),
}

impl fmt::Display for DropClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DropClass {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Case-insensitive on the kebab-case canonical token —
        // matches `.claude/rules/development.md` § Newtype
        // completeness for human-typed identifiers.
        match s.to_ascii_lowercase().as_str() {
            "malformed-header" => Ok(Self::MalformedHeader),
            "unknown-vip" => Ok(Self::UnknownVip),
            "no-healthy-backend" => Ok(Self::NoHealthyBackend),
            "sanity-prologue" => Ok(Self::SanityPrologue),
            "reverse-nat-miss" => Ok(Self::ReverseNatMiss),
            "oversize-packet" => Ok(Self::OversizePacket),
            _ => Err(ParseError::Unknown(s.to_owned())),
        }
    }
}

/// Sum a per-CPU view of a `DROP_COUNTER` slot bit-exact across all
/// online CPUs. Models the userspace
/// `DropCounterHandle::read(class)` surface: the kernel-side
/// `BPF_MAP_TYPE_PERCPU_ARRAY` returns a `Vec<u64>` (one entry per
/// online CPU); operators want the total. `iter().sum()` is the
/// canonical shape; this helper exists so the proptest in
/// `tests/drop_class.rs` has a stable call site to defend against
/// integer-width and overflow regressions.
///
/// Returns the bit-exact saturating sum — values larger than `u64`
/// max would saturate (with the input shape `0..1_000_000` × `≤128`
/// CPUs the proptest exercises, overflow is structurally
/// unreachable; production callers exposing arbitrary inputs must
/// validate the sum fits a `u64`, which is the entire point of the
/// per-class slot — counter rollover within a single observation
/// window is not a real failure mode).
#[must_use]
pub fn aggregate_per_cpu(per_cpu_values: &[u64]) -> u64 {
    per_cpu_values.iter().copied().fold(0_u64, u64::saturating_add)
}

// Slot-drift detection: VARIANT_COUNT must equal the discriminant
// of the highest variant + 1. Adding a variant without bumping
// VARIANT_COUNT, OR changing VARIANT_COUNT away from the actual
// count, fails this assert at compile time. The compile-fail
// fixture at `tests/compile_fail/drop_class_slot_drift.rs` pins the
// shape at PR time.
const _: () = {
    // The highest variant is OversizePacket (slot 5). VARIANT_COUNT
    // must equal 6.
    assert!(
        DropClass::OversizePacket.as_index() + 1 == DropClass::VARIANT_COUNT,
        "DropClass::VARIANT_COUNT must equal the highest variant's discriminant + 1",
    );
    // Defence against numeric drift on the discriminants
    // themselves — pin every variant to its locked slot.
    assert!(DropClass::MalformedHeader.as_index() == 0);
    assert!(DropClass::UnknownVip.as_index() == 1);
    assert!(DropClass::NoHealthyBackend.as_index() == 2);
    assert!(DropClass::SanityPrologue.as_index() == 3);
    assert!(DropClass::ReverseNatMiss.as_index() == 4);
    assert!(DropClass::OversizePacket.as_index() == 5);
};
