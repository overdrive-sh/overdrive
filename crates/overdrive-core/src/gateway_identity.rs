//! Dedicated non-allocation gateway identity values and the one hydration
//! read-port.
//!
//! The source-local properties at the foot of the file specify the epoch half
//! of S-PIG-44.

#![allow(
    clippy::todo,
    clippy::doc_markdown,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    reason = "public-ingress DISTILL scaffold: bodies are DELIVER's"
)]

use std::fmt::{self, Display, Formatter};
use std::num::NonZeroU64;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ca::issue_failure::SvidIssueFailure;
use crate::id::{CertSerial, SpiffeId};
use crate::wall_clock::UnixInstant;

/// Process-local desired-identity epoch. Constructor epoch one means enabled;
/// disable increments it (the slot's private checked add; the type pins no
/// increment operation).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct GatewayIdentityEpoch(NonZeroU64);

impl GatewayIdentityEpoch {
    /// Construct a nonzero epoch.
    pub fn new(raw: u64) -> Result<Self, GatewayIdentityEpochError> {
        NonZeroU64::new(raw).map(Self).ok_or(GatewayIdentityEpochError::Zero)
    }

    /// The initial enabled epoch, exactly 1.
    pub const fn first() -> Self {
        Self(NonZeroU64::MIN)
    }

    /// The nonzero value.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

impl Display for GatewayIdentityEpoch {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for GatewayIdentityEpoch {
    type Err = GatewayIdentityEpochError;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        if raw.is_empty() {
            return Err(GatewayIdentityEpochError::Empty);
        }
        let all_digits = raw.bytes().all(|b| b.is_ascii_digit());
        let leading_zero = raw.len() > 1 && raw.starts_with('0');
        if !all_digits || leading_zero {
            return Err(GatewayIdentityEpochError::InvalidDecimal);
        }
        let value: u64 = raw.parse().map_err(|_| GatewayIdentityEpochError::InvalidDecimal)?;
        Self::new(value)
    }
}

impl<'a> TryFrom<&'a str> for GatewayIdentityEpoch {
    type Error = GatewayIdentityEpochError;
    fn try_from(raw: &'a str) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl TryFrom<String> for GatewayIdentityEpoch {
    type Error = GatewayIdentityEpochError;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        raw.parse()
    }
}

impl Serialize for GatewayIdentityEpoch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for GatewayIdentityEpoch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Closed epoch parse failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GatewayIdentityEpochError {
    #[error("gateway identity epoch is empty")]
    Empty,
    #[error("gateway identity epoch is not canonical decimal")]
    InvalidDecimal,
    #[error("gateway identity epoch must be nonzero")]
    Zero,
}

/// Non-secret facts about the currently held gateway SVID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayIdentityFacts {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: SpiffeId,
    pub serial: CertSerial,
    pub not_after: UnixInstant,
}

/// Process-local issue-failure record of the gateway identity slot. The slot
/// holds `Option<GatewayIdentityIssueRecord>`: `None` is "no failure this
/// process", so a nonzero count without a `last` (or the reverse) is
/// unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayIdentityIssueRecord {
    pub count: NonZeroU64,
    pub last: SvidIssueFailure,
}

/// Desired gateway identity for one checked epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayIdentityDesired {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: Option<SpiffeId>,
}

/// The one gateway-identity hydration read-port (non-secret facts only). It
/// has no subscription method.
pub trait GatewayIdentityView: Send + Sync {
    /// Desired epoch and optional subject.
    fn desired(&self) -> GatewayIdentityDesired;
    /// Currently held facts, `None` while no SVID is held.
    fn current(&self) -> Option<GatewayIdentityFacts>;
}

#[cfg(test)]
mod properties {
    #![allow(
        clippy::expect_used,
        clippy::doc_markdown,
        reason = "fixture preconditions panic with their cause; CONTRACT_SHAPE is required metadata"
    )]

    use proptest::prelude::*;

    use super::*;

    /// S-PIG-44 (epoch half): the epoch is nonzero, starts at one and accepts
    /// every nonzero value, including the maximum.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn epoch_is_nonzero_and_starts_at_one() {
        assert_eq!(GatewayIdentityEpoch::first().get().get(), 1);
        assert_eq!(GatewayIdentityEpoch::new(2).expect("two is valid").get().get(), 2);
        assert_eq!(
            GatewayIdentityEpoch::new(u64::MAX).expect("maximum is nonzero").get().get(),
            u64::MAX,
        );
        assert_eq!(GatewayIdentityEpoch::new(0), Err(GatewayIdentityEpochError::Zero));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// S-PIG-44 (epoch half): the epoch round-trips its canonical decimal
        /// through `Display`, `FromStr`, `TryFrom`, string serde and rkyv.
        ///
        /// CONTRACT_SHAPE: pure-function.
        #[test]
            fn epoch_roundtrips_canonical_decimal(raw in 1u64..u64::MAX) {
            let epoch = GatewayIdentityEpoch::new(raw).expect("nonzero epoch");
            prop_assert_eq!(epoch.to_string(), raw.to_string());
            prop_assert_eq!(raw.to_string().parse::<GatewayIdentityEpoch>(), Ok(epoch));
            prop_assert_eq!(GatewayIdentityEpoch::try_from(raw.to_string().as_str()), Ok(epoch));
            prop_assert_eq!(GatewayIdentityEpoch::try_from(raw.to_string()), Ok(epoch));
            prop_assert_eq!(serde_json::from_str::<GatewayIdentityEpoch>(
                &serde_json::to_string(&epoch).expect("serialize"),
            ).expect("deserialize"), epoch);
            let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&epoch)
                .expect("epoch rkyv archive");
            prop_assert_eq!(
                rkyv::from_bytes::<GatewayIdentityEpoch, rkyv::rancor::Error>(&archived)
                    .expect("epoch rkyv valid trip"),
                epoch,
            );
        }
    }

    /// S-PIG-44 (epoch half): each noncanonical epoch partition is refused
    /// with its own closed variant.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn epoch_rejects_each_noncanonical_partition_exactly() {
        assert_eq!("".parse::<GatewayIdentityEpoch>(), Err(GatewayIdentityEpochError::Empty));
        for raw in ["+1", "-1", "01", " 1", "1 ", "18446744073709551616"] {
            assert_eq!(
                raw.parse::<GatewayIdentityEpoch>(),
                Err(GatewayIdentityEpochError::InvalidDecimal),
                "epoch partition {raw:?}",
            );
        }
        assert_eq!("0".parse::<GatewayIdentityEpoch>(), Err(GatewayIdentityEpochError::Zero));
    }
}
