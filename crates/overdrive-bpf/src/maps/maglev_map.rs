//! `MAGLEV_MAP` — kernel-side conceptual relabeling for Slice 04.
//!
//! In Phase 2 the architectural concept of a "Maglev map" is
//! implemented by [`crate::maps::service_map::SERVICE_MAP`] — a
//! `BPF_MAP_TYPE_HASH_OF_MAPS` keyed by `ServiceKey` (= `(VIP, port)`)
//! whose inner ARRAY is the Maglev permutation table. The single
//! kernel-side map serves both roles (service lookup + Maglev slot
//! resolution) in one chained `bpf_map_lookup_elem` chain — the
//! hot-path packet flow is:
//!
//!     SERVICE_MAP[(vip, port)]    → inner ARRAY ptr  (verifier-tagged inner_map)
//!     inner_ARRAY[FNV-1a(5tuple) % M] → BackendId
//!     BACKEND_MAP[BackendId]      → BackendEntry
//!     XDP_TX with rewritten dest IP+port
//!
//! Inner ARRAY size `M` = `MaglevTableSize::DEFAULT.get()` = 16_381
//! per architecture.md § 5 Q-Sig D6 / ADR-0041 (Cilium's prime list).
//! Slot population is the deterministic Maglev permutation produced
//! by `crates/overdrive-core/src/maglev/permutation.rs::generate`.
//!
//! # Why no separate map declaration here
//!
//! The DESIGN-wave artifacts envisioned a 3-map architecture:
//!
//!   SERVICE_MAP   `(vip, port) → ServiceId`         (flat HASH)
//!   MAGLEV_MAP    `ServiceId → ARRAY[16_381] of BackendId` (HoM)
//!   BACKEND_MAP   `BackendId → BackendEntry`        (flat HASH)
//!
//! Phase 2 collapses the first two into a single HoM (the existing
//! SERVICE_MAP) keyed directly by `ServiceKey`. The collapse is
//! semantically equivalent for the Phase 2 scope (one ServiceId per
//! VIP/port) and saves a chained kernel lookup. ADR-0041's
//! AC #4 ("the swap orchestration is unchanged; only the inner-ARRAY
//! contents differ") and ADR-0040's atomic-swap shape both ride on
//! this single-HoM design.
//!
//! Future work (out of Phase 2 scope) — splitting SERVICE_MAP into
//! a flat `(vip,port)→ServiceId` HASH and a separate
//! `ServiceId→ARRAY` HoM would decouple service registration from
//! backend-set churn and is the natural shape for ServiceId-keyed
//! tooling (per-service ingress/egress counters, observability
//! tagged by ServiceId rather than ServiceKey). Tracked outside of
//! Phase 2.

#![allow(dead_code)]

// Re-export for consumers that prefer the `MAGLEV_MAP` name in code.
// The kernel-side `#[map]` static is `SERVICE_MAP`; the alias is a
// type-system breadcrumb for future cross-crate plumbing (e.g.
// observability counters keyed by service identity).
pub use crate::maps::service_map::{
    INNER_TABLE_SIZE as MAGLEV_TABLE_SIZE, SERVICE_MAP as MAGLEV_MAP,
};
