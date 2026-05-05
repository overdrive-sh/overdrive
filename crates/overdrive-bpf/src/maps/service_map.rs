//! `SERVICE_MAP` — kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` outer
//! map keyed on `(ServiceVip, u16 port)` (host-order; converted at
//! the kernel boundary per architecture.md § 11). Inner is
//! `BPF_MAP_TYPE_HASH` keyed on `BackendId` → `BackendEntry`,
//! `max_entries = 256` per Q5=A.
//!
//! Atomic swap via outer-map fd replace per Slice 03 (US-03;
//! S-2.2-09).
//!
//! **RED scaffold** — `#[map]` declaration not yet emitted.
//! DELIVER lands it per Slice 02 / 03.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
