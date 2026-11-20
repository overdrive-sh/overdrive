//! Test fixtures for the node-local persistent volume value types (GH #348).
//!
//! `VolumeSize` has exactly one public constructor in the design: its
//! `<uint><unit>` grammar. A test that is not about that grammar must not
//! depend on it being implemented, so these fixtures build a size the way the
//! intent store does: by reading its archived bytes (a `VolumeSize` archives as
//! its `u64` byte count, feature delta § Interface contracts). They add no
//! constructor to the type and are reachable only through the `test-utils`
//! feature, which production never enables.
//!
//! The parser tests (`vm_volume_size.rs`) are the only tests that build a
//! `VolumeSize` through `FromStr`.

#![allow(clippy::expect_used, reason = "test fixtures assert their preconditions")]

use crate::volume::VolumeSize;

const GIB: u64 = 1024 * 1024 * 1024;

/// A volume size of exactly `bytes` bytes, built from its archived form.
///
/// # Panics
///
/// When `VolumeSize` stops archiving as its byte count; the fixture is then to
/// be replaced by whatever constructor the design pins.
#[must_use]
pub fn volume_size_of_bytes(bytes: u64) -> VolumeSize {
    let archived = rkyv::to_bytes::<rkyv::rancor::Error>(&bytes).expect("a u64 archives");
    rkyv::from_bytes::<VolumeSize, rkyv::rancor::Error>(&archived)
        .expect("a VolumeSize archives as its byte count")
}

/// A volume size of `gib` GiB.
#[must_use]
pub fn volume_size_gib(gib: u64) -> VolumeSize {
    volume_size_of_bytes(gib * GIB)
}

/// A volume size of `tib` TiB.
#[must_use]
pub fn volume_size_tib(tib: u64) -> VolumeSize {
    volume_size_gib(tib * 1024)
}
