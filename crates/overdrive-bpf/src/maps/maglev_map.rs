//! `MAGLEV_MAP` — kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` outer
//! keyed on `ServiceId` (u64) → inner-map fd. Inner is
//! `BPF_MAP_TYPE_ARRAY` of `BackendId` slots, size =
//! `MaglevTableSize::DEFAULT.get()` (16_381) per Q5=A / Q6=A.
//!
//! One inner per service. Atomic swap on backend-set change
//! per Slice 04 (US-04; S-2.2-12..14).
//!
//! **RED scaffold** — `#[map]` declaration not yet emitted.
//! DELIVER lands it per Slice 04.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
