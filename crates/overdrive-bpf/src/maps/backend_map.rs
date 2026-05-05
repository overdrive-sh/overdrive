//! `BACKEND_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `BackendId` (u32) → `BackendEntry { ipv4: u32, port: u16,
//! weight: u16, healthy: u8, _pad: [u8; 3] }`. Single global;
//! backends shared across services. 8-byte aligned. `max_entries
//! = 65_536` per architecture.md § 10.
//!
//! **RED scaffold** — `#[map]` declaration not yet emitted.
//! DELIVER lands it per Slice 03.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
