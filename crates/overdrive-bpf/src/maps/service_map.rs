//! `SERVICE_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on the
//! 8-byte host-order `(VIP, port)` POD per architecture.md § 10
//! and the userspace handle in
//! `crates/overdrive-dataplane/src/maps/service_map_handle.rs`.
//!
//! Phase 2.2 Slice 02 ships a flat `HashMap<ServiceKey,
//! BackendEntry>` matching the userspace handle's in-memory
//! shape — single backend per `(VIP, port)`. Slice 03 (US-03;
//! S-2.2-09..11) reshapes this into the
//! `BPF_MAP_TYPE_HASH_OF_MAPS` outer / inner topology with
//! atomic-swap semantics; the kernel-side rewrite that flips
//! Slice 03 GREEN replaces this declaration with the outer-map
//! variant. The current shape is the minimum viable for S-2.2-04
//! / S-2.2-05 / S-2.2-08 to flip GREEN under Slice 02 scope.
//!
//! `max_entries = 4096` per architecture.md § 10 (matches the
//! userspace handle's expected capacity per the aya-rs
//! kernel-side patterns rule in `.claude/rules/development.md`
//! § "Map access from XDP context").
//!
//! Endianness lockstep (architecture.md § 11): both `ServiceKey`
//! and `BackendEntry` are stored host-order. The kernel-side
//! lookup converts wire-order packet fields into host-order
//! before keying SERVICE_MAP. Userspace stores host-order without
//! flipping. The S-2.2-04 proptest in the userspace handle is
//! the byte-level pin.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Outer-map key for SERVICE_MAP. 8 bytes, host-order. Matches
/// `crates/overdrive-dataplane/src/maps/service_map_handle.rs`
/// `ServiceKey` byte-for-byte (vip_host: u32, port_host: u16,
/// _pad: u16). Kept `#[repr(C)]` so the kernel-side and
/// userspace structs share an identical memory layout — aya's
/// `bpf_map_lookup_elem` keys the map by raw bytes.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub _pad: u16,
}

/// Inner-map value — the resolved backend. 12 bytes, host-order.
/// Matches `BackendEntry` in the userspace handle.
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
/// every `(VIP, port)` tuple a single node will route in Phase 2.
pub const MAX_ENTRIES: u32 = 4096;

/// `SERVICE_MAP` — flat `BPF_MAP_TYPE_HASH` for Slice 02. The
/// XDP fast path looks up `(dest_ip_host, dest_port_host, 0)` and
/// rewrites IP+port from the resulting `BackendEntry`.
#[map]
pub static SERVICE_MAP: HashMap<ServiceKey, BackendEntry> =
    HashMap::with_max_entries(MAX_ENTRIES, 0);
