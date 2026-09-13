//! Dedicated non-allocation gateway identity contracts.
//!
//! SCAFFOLD: true. Constructors remain RED until DELIVER.

#![expect(clippy::todo, reason = "public-ingress DISTILL RED scaffolds")]

use std::num::NonZeroU64;
use std::str::FromStr;

use thiserror::Error;

use crate::{CertSerial, SpiffeId, UnixInstant};

/// Process-local desired-identity epoch.
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
    pub fn new(_raw: u64) -> Result<Self, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: GatewayIdentityEpoch::new")
    }

    /// Initial enabled epoch.
    pub const fn first() -> Self {
        Self(NonZeroU64::MIN)
    }

    /// Advance without wrapping.
    pub fn checked_next(self) -> Result<Self, GatewayIdentityEpochError> {
        todo!("SCAFFOLD: GatewayIdentityEpoch::checked_next")
    }

    /// Inner nonzero value.
    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

impl std::fmt::Display for GatewayIdentityEpoch {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("SCAFFOLD: GatewayIdentityEpoch Display")
    }
}

impl FromStr for GatewayIdentityEpoch {
    type Err = GatewayIdentityEpochError;
    fn from_str(_raw: &str) -> Result<Self, Self::Err> {
        todo!("SCAFFOLD: GatewayIdentityEpoch FromStr")
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

impl serde::Serialize for GatewayIdentityEpoch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for GatewayIdentityEpoch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Closed epoch parse/arithmetic failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GatewayIdentityEpochError {
    #[error("gateway identity epoch is empty")]
    Empty,
    #[error("gateway identity epoch is not canonical decimal")]
    InvalidDecimal,
    #[error("gateway identity epoch must be nonzero")]
    Zero,
    #[error("gateway identity epoch exhausted")]
    Exhausted,
}

/// Non-secret facts about the currently held gateway SVID.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayIdentityFacts {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: SpiffeId,
    pub serial: CertSerial,
    pub not_after: UnixInstant,
}

/// Desired gateway identity for one checked epoch.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct GatewayIdentityDesired {
    pub epoch: GatewayIdentityEpoch,
    pub spiffe_id: Option<SpiffeId>,
}

/// Read-only desired gateway identity port.
pub trait GatewayIdentityDesiredRead: Send + Sync {
    fn desired(&self) -> GatewayIdentityDesired;
}

/// Read-only current gateway identity facts port.
pub trait GatewayIdentityCurrentRead: Send + Sync {
    fn current(&self) -> Option<GatewayIdentityFacts>;
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER gateway identity epoch step"]
    fn epoch_is_nonzero_monotone_and_refuses_wrap() {
        assert_eq!(GatewayIdentityEpoch::first().get().get(), 1);
        assert_eq!(
            GatewayIdentityEpoch::new(1)
                .expect("one is valid")
                .checked_next()
                .expect("two is representable")
                .get()
                .get(),
            2,
        );
        assert_eq!(
            GatewayIdentityEpoch::new(u64::MAX).expect("maximum is nonzero").checked_next(),
            Err(GatewayIdentityEpochError::Exhausted),
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// CONTRACT_SHAPE: pure-function.
        #[test]
        #[ignore = "pending DELIVER canonical gateway identity epoch"]
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

    /// CONTRACT_SHAPE: pure-function.
    #[test]
    #[ignore = "pending DELIVER canonical gateway identity epoch"]
    fn epoch_rejects_each_noncanonical_partition_exactly() {
        assert_eq!("".parse::<GatewayIdentityEpoch>(), Err(GatewayIdentityEpochError::Empty));
        for raw in ["+1", "-1", "01", " 1", "1 ", "18446744073709551616"] {
            assert_eq!(
                raw.parse::<GatewayIdentityEpoch>(),
                Err(GatewayIdentityEpochError::InvalidDecimal),
            );
        }
        assert_eq!("0".parse::<GatewayIdentityEpoch>(), Err(GatewayIdentityEpochError::Zero));
    }
}
