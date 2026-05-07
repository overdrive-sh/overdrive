//! `MaglevMapHandle` — userspace conceptual relabeling for Slice 04.
//!
//! Phase 2 implements the Maglev permutation table as the inner
//! ARRAY of [`crate::maps::hash_of_maps::HashOfMapsHandle`]
//! `<ServiceKey, BackendId>` (= the existing SERVICE_MAP outer HoM).
//! There is no separate userspace handle struct because the map
//! identity is shared with SERVICE_MAP — see
//! `crates/overdrive-bpf/src/maps/maglev_map.rs` for the full
//! rationale.
//!
//! `crate::EbpfDataplane::update_service` regenerates the Maglev
//! permutation on every backend-set change via
//! [`overdrive_core::maglev::permutation::generate`] and writes the resulting
//! `Vec<BackendId>` into a freshly-allocated inner ARRAY before the
//! atomic outer-pointer swap (step 3 of ADR-0040 § 2's 5-step swap
//! orchestration). The `HashOfMapsHandle` is the only typed handle
//! involved.
//!
//! Migration note — when aya 1.0 / PR #1446 lands and ServiceId-
//! keyed maps become ergonomic to declare, this module's name and
//! purpose may be re-purposed to wrap a separate MAGLEV_MAP HoM
//! distinct from SERVICE_MAP. Phase 2 does not need that split —
//! the single-HoM design ships.

#![allow(dead_code)]

#[cfg(target_os = "linux")]
pub use crate::maps::hash_of_maps::HashOfMapsHandle as MaglevMapHandle;

#[cfg(target_os = "linux")]
pub use overdrive_core::dataplane::MaglevTableSize;
