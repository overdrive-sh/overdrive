//! Node-local persistent volume value types (GH #348, ADR-0310, ADR-0317).
//!
//! The grammars (`VolumeSize`, `ByteCount`), the operation label and the
//! unavailability cause the volume contract carries across the port, the
//! operator wire and the VMM argument.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

use crate::byte_size::{BYTES_PER_GIB, ByteSize};

/// A validated volume size in bytes: the `ByteSize` grammar refined to a
/// non-zero whole number of GiB. `Display` is the `ByteSize` canonical form,
/// `GiB` unless the value is an exact `TiB` multiple (ADR-0310).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct VolumeSize(u64);

/// A volume size that failed the `<uint><unit>` grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid volume size {input:?}: {reason}")]
pub struct VolumeSizeParseError {
    input: String,
    reason: String,
}

impl VolumeSize {
    /// The size in bytes: the one accessor the host adapter and the sim read the size through.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }
}

/// Splits `<uint><unit>` into its ASCII-digit count and the remaining unit text.
fn split_count_and_unit(s: &str) -> Option<(u64, &str)> {
    let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (digits, unit) = s.split_at(digits_end);
    digits.parse::<u64>().ok().map(|count| (count, unit))
}

impl FromStr for VolumeSize {
    type Err = VolumeSizeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let reject = |reason: String| VolumeSizeParseError { input: s.to_owned(), reason };
        let size = s.parse::<ByteSize>().map_err(|e| reject(e.cause().to_owned()))?;
        if size.bytes() == 0 {
            return Err(reject("the size must be at least 1GiB".to_owned()));
        }
        if !size.bytes().is_multiple_of(BYTES_PER_GIB) {
            return Err(reject("the size must be a whole number of GiB".to_owned()));
        }
        Ok(Self(size.bytes()))
    }
}

impl fmt::Display for VolumeSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        ByteSize::new(self.0).fmt(f)
    }
}

impl Serialize for VolumeSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VolumeSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// A validated byte count (free extents in `InsufficientSpace`). Any `u64`,
/// including 0. `Display` emits `<n>B`; `FromStr` parses `<uint>B`; serde is
/// that string (feature delta § Port).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct ByteCount(u64);

impl ByteCount {
    /// Total: no `u64` is invalid.
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self(bytes)
    }

    /// The count in bytes.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A byte count that failed the `<uint>B` grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid byte count {input:?}")]
pub struct ByteCountParseError {
    input: String,
}

impl FromStr for ByteCount {
    type Err = ByteCountParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        split_count_and_unit(s)
            .filter(|(_, unit)| unit.eq_ignore_ascii_case("B"))
            .map(|(count, _)| Self(count))
            .ok_or_else(|| ByteCountParseError { input: s.to_owned() })
    }
}

impl fmt::Display for ByteCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}B", self.0)
    }
}

impl Serialize for ByteCount {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ByteCount {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// The `lvm` or device operation that failed (`VolumeBackingError::PoolCommand`,
/// `VolumeUnavailableCause::PoolCommandFailed`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum VolumeOperation {
    /// An `lvs`/`vgs` read that observes presence, activation or free extents.
    Report,
    /// `lvcreate`.
    Create,
    /// `lvchange -ay`.
    Activate,
    /// The `O_EXCL` open of the device node in `claim_state` (a failure other
    /// than `EBUSY` and `ENOENT`).
    Claim,
}

impl VolumeOperation {
    /// The kebab-case label; the enum owns its vocabulary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Create => "create",
            Self::Activate => "activate",
            Self::Claim => "claim",
        }
    }
}

/// Why a launch could not obtain its volume (ADR-0317, ADR-0318).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum VolumeUnavailableCause {
    /// No volume pool is composed on this node.
    PoolAbsent,
    /// Not enough free extents for the declared size.
    InsufficientSpace {
        /// The declared size.
        #[schema(value_type = String)]
        needed: VolumeSize,
        /// Free space observed before the create.
        #[schema(value_type = String)]
        free: ByteCount,
    },
    /// An `lvm` command or device operation failed, or the `ensure` deadline passed.
    PoolCommandFailed {
        /// The operation in progress when it failed.
        operation: VolumeOperation,
    },
    /// The volume's kernel claim stayed held past the 30 s bound (a constant;
    /// the cause carries nothing).
    Held,
}
