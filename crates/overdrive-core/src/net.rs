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
//! RED scaffold (DISTILL, Mandate 7): the constructor, accessor, `FromStr` /
//! `Display` / serde bodies panic until DELIVER step 2 lands them.
//! The error enums are real.

// SCAFFOLD: true — netns-density-295 DISTILL (DELIVER step 2).
#![expect(
    clippy::unused_self,
    reason = "RED scaffold bodies panic before reading their inputs; the lint self-trips when DELIVER lands the bodies"
)]

use std::fmt;
use std::num::{NonZeroU16, NonZeroU32};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const SCAFFOLD: &str = "Not yet implemented -- RED scaffold";

/// A kernel network interface index; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceIndex(NonZeroU32);

/// A TCP listen port an allocation declares or a guest reports; never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListenPort(NonZeroU16);

impl ListenPort {
    /// Validating constructor: `0` → [`ListenPortError::Zero`].
    pub fn new(_raw: u16) -> Result<Self, ListenPortError> {
        panic!("{SCAFFOLD}")
    }

    /// The port number.
    pub const fn get(self) -> u16 {
        panic!("Not yet implemented -- RED scaffold")
    }
}

impl InterfaceIndex {
    /// Validating constructor: `0` → [`InterfaceIndexError::Zero`].
    pub fn new(_raw: u32) -> Result<Self, InterfaceIndexError> {
        panic!("{SCAFFOLD}")
    }

    /// The interface index.
    pub const fn get(self) -> u32 {
        panic!("Not yet implemented -- RED scaffold")
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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; `0` is `Zero`, anything else not a decimal `u32` is
/// `Unparseable`.
impl FromStr for InterfaceIndex {
    type Err = InterfaceIndexError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

impl Serialize for InterfaceIndex {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

impl<'de> Deserialize<'de> for InterfaceIndex {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form.
impl fmt::Display for ListenPort {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("{SCAFFOLD}")
    }
}

/// Decimal form; `0` is `Zero`, anything else not a decimal `u16` is
/// `Unparseable`.
impl FromStr for ListenPort {
    type Err = ListenPortError;

    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        panic!("{SCAFFOLD}")
    }
}

impl Serialize for ListenPort {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        panic!("{SCAFFOLD}")
    }
}

impl<'de> Deserialize<'de> for ListenPort {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        panic!("{SCAFFOLD}")
    }
}
