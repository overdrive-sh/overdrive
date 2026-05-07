//! `BACKEND_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `BackendId` (u32) → `BackendEntry { ipv4: u32, port: u16,
//! weight: u16, healthy: u8, _pad: [u8; 3] }`. Single global;
//! backends shared across services. 8-byte aligned. `max_entries
//! = 65_536` per architecture.md § 10.
//!
//! The kernel-side `BackendId` is the raw `u32` wire-shape — the
//! `overdrive-core::id::BackendId` newtype is a userspace
//! type-system distinctness layer; the kernel keys the map by
//! raw bytes via `bpf_map_lookup_elem`. Userspace converts at
//! the write boundary by `BackendId::get()`.
//!
//! `BackendEntry` is host-order on every numeric field, matching
//! the `service_map.rs` precedent (architecture.md § 11). The
//! XDP fast path does its own NBO ⇄ host conversion against
//! incoming packets; userspace stores host-order without
//! flipping.
//!
//! Capacity: `max_entries = 65_536` per architecture.md § 10 —
//! sized to comfortably hold every backend a single node will
//! route across all services. Slice 03 ships the kernel-side
//! `#[map]` declaration; the userspace `BackendMapHandle` lands
//! alongside in step 03-02.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Outer-map value — the resolved backend. 12 bytes, host-order.
/// Mirrors the `BackendEntry` shape in
/// `crates/overdrive-bpf/src/maps/service_map.rs` so kernel-side
/// `SERVICE_MAP` and `BACKEND_MAP` carry identical layout for a
/// resolved backend record.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BackendEntry {
    pub ipv4_host: u32,
    pub port_host: u16,
    pub weight: u16,
    pub healthy: u8,
    pub _pad: [u8; 3],
}

/// Capacity per architecture.md § 10. Sized to comfortably hold
/// every backend a single node will route across all services.
pub const MAX_ENTRIES: u32 = 65_536;

/// `BACKEND_MAP` — `BPF_MAP_TYPE_HASH` keyed on `BackendId`
/// (raw `u32`) → `BackendEntry`. Single global; backends shared
/// across services. The XDP fast path resolves a `(ServiceId,
/// MaglevSlot)` lookup against `MAGLEV_MAP` into a `BackendId`,
/// then resolves the `BackendId` against this map to obtain
/// the per-backend `BackendEntry`.
#[map]
pub static BACKEND_MAP: HashMap<u32, BackendEntry> = HashMap::with_max_entries(MAX_ENTRIES, 0);
