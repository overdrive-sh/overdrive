//! Network newtypes (M-5) — vsock attachment replacement (GH #295).
//!
//! Interface contract: `docs/feature/netns-density-295/feature-delta.md`
//! § "[REF] Core vocabulary", **Newtypes (M-5)**: each has a validating
//! constructor returning `Result`, `FromStr`, `Display`, and `Serialize` /
//! `Deserialize` matching them, with a mandatory proptest roundtrip.
//!
//! The constructors, accessors and error types are pinned by DG-11
//! (`ListenPort::new` / `get`, `InterfaceIndex::new` / `get`,
//! `ListenPortError`, `InterfaceIndexError`); `FromStr` uses the same errors.
//!
//! `ListenPort` and `InterfaceIndex` are validated and carry one canonical
//! decimal text form shared by `Display`, `FromStr` and serde.

use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A kernel network interface index; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceIndex(NonZeroU32);

/// A TCP listen port an allocation declares or a guest reports; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListenPort(NonZeroU16);

impl ListenPort {
    /// Validating constructor: `0` → [`ListenPortError::Zero`].
    pub fn new(raw: u16) -> Result<Self, ListenPortError> {
        NonZeroU16::new(raw).map(Self).ok_or(ListenPortError::Zero)
    }

    /// The port number.
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

impl InterfaceIndex {
    /// Validating constructor: `0` → [`InterfaceIndexError::Zero`].
    pub fn new(raw: u32) -> Result<Self, InterfaceIndexError> {
        NonZeroU32::new(raw).map(Self).ok_or(InterfaceIndexError::Zero)
    }

    /// The interface index.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Why a value is not a [`ListenPort`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ListenPortError {
    /// The value 0.
    #[error("listen port 0 is not a port")]
    Zero,
    /// Not a decimal integer in `1..=65535`.
    #[error("listen port is not a decimal integer in 1..=65535")]
    Unparseable,
}

/// Why a value is not an [`InterfaceIndex`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InterfaceIndexError {
    /// The value 0.
    #[error("interface index 0 names no interface")]
    Zero,
    /// Not a decimal integer in `1..=4294967295`.
    #[error("interface index is not a decimal integer in 1..=4294967295")]
    Unparseable,
}

/// Decimal form.
impl fmt::Display for InterfaceIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decimal form; `0` is `Zero`, anything else not a decimal `u32` is
/// `Unparseable`.
impl FromStr for InterfaceIndex {
    type Err = InterfaceIndexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !is_decimal(s) {
            return Err(InterfaceIndexError::Unparseable);
        }
        let raw = s.parse::<u32>().map_err(|_| InterfaceIndexError::Unparseable)?;
        Self::new(raw)
    }
}

impl Serialize for InterfaceIndex {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for InterfaceIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Decimal form.
impl fmt::Display for ListenPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Decimal form; `0` is `Zero`, anything else not a decimal `u16` is
/// `Unparseable`.
impl FromStr for ListenPort {
    type Err = ListenPortError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !is_decimal(s) {
            return Err(ListenPortError::Unparseable);
        }
        let raw = s.parse::<u16>().map_err(|_| ListenPortError::Unparseable)?;
        Self::new(raw)
    }
}

impl Serialize for ListenPort {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ListenPort {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

fn is_decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}
