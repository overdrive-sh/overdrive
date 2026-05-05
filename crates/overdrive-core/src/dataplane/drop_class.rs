//! `DropClass` — drop classification for the kernel-side
//! `DROP_COUNTER` PERCPU_ARRAY.
//!
//! Locked variant set per Q7=B in
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 6 *DropClass*. `#[repr(u32)]` makes `as u32` a stable
//! kernel-side index across Rust toolchains (research § 7.1).
//!
//! Variant ordering and discriminants are STABLE — additions are
//! minor-version (per ADR-0037 K8s-Condition convention);
//! reordering or removal is a major-version break that requires a
//! new ADR.
//!
//! **RED scaffold** — `FromStr`, `Display`, serde shapes panic
//! until DELIVER fills them (Slice 06 per the carpaccio plan).

#![allow(clippy::missing_errors_doc)]

use core::fmt;
use std::str::FromStr;

/// Drop classification for the `DROP_COUNTER` PERCPU_ARRAY. Six
/// variants locked at Q7=B; `#[repr(u32)]` makes the discriminant
/// the kernel-side slot index.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DropClass {
    /// Frame's IPv4 / TCP / UDP header failed sanity-prologue
    /// checks (truncated header, bad IHL, nonsense TCP flags, …).
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
    /// `DROP_COUNTER` slot count.
    pub const VARIANT_COUNT: u32 = 6;

    /// Stable kernel-side index for this variant.
    pub fn as_index(self) -> u32 {
        self as u32
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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // RED scaffold — DELIVER fills this body per Slice 06.
        // See test-scenarios.md S-2.2-19, S-2.2-20.
        todo!("RED scaffold: DropClass::fmt — see Slice 06 / S-2.2-19")
    }
}

impl FromStr for DropClass {
    type Err = ParseError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        // RED scaffold — DELIVER fills this body per Slice 06.
        todo!("RED scaffold: DropClass::from_str — see Slice 06 / S-2.2-19")
    }
}
