//! `REVERSE_NAT_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `BackendKey { ip: u32, port: u16, proto: u8, _pad: u8 }` →
//! `Vip { ip: u32, port: u16, _pad: u16 }`.
//!
//! The third map of the Cilium three-map split (`SERVICE_MAP` +
//! `BACKEND_MAP` + `REVERSE_NAT_MAP`) per architecture.md § 10. The
//! egress reverse-NAT path uses this map to rewrite the source
//! address of backend response packets back to the original VIP
//! the client connected to: when a backend responds, the kernel
//! looks up `(backend_ip, backend_port, proto) → Vip` and rewrites
//! the source 5-tuple before the kernel networking stack sees the
//! packet.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! All values stored host-order. The kernel-side egress program
//! converts at the read boundary against incoming wire-order packets.
//! Userspace stores host-order without flipping — the same lockstep
//! contract `ServiceMapHandle` and `BackendMapHandle` carry. The
//! kernel-side `BackendKey` is the raw POD wire-shape — the
//! `overdrive-core::dataplane::backend_key::BackendKey` newtype is a
//! userspace type-system distinctness layer; the kernel keys the map
//! by raw bytes via `bpf_map_lookup_elem`. Userspace converts at the
//! write boundary by destructuring the newtype into its three fields.
//!
//! # IANA proto codes
//!
//! `proto = 6` for TCP, `proto = 17` for UDP per RFC 1700 / IANA
//! protocol-numbers registry. Phase 2.2 supports exactly these two
//! L4 protocols (architecture.md § 6).
//!
//! # Capacity
//!
//! `max_entries = 1_048_576` (operator-tunable in future; Phase 2.2
//! fixed) per architecture.md § 10.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Outer-map key — the backend-side 3-tuple of an in-flight egress
/// response. 8 bytes, host-order. Matches the userspace
/// `overdrive_core::dataplane::backend_key::BackendKey` newtype's
/// wire-shape: 4 bytes IPv4 (host-order `u32`), 2 bytes port
/// (host-order `u16`), 1 byte proto (IANA assignment), 1 byte pad
/// for 8-byte alignment in BPF map storage.
///
/// `#[repr(C)]` so the kernel-side and userspace structs share an
/// identical memory layout — aya's `bpf_map_lookup_elem` keys the
/// map by raw bytes.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct BackendKey {
    /// Backend IPv4 address. Host-order.
    pub ip_host: u32,
    /// Backend port. Host-order.
    pub port_host: u16,
    /// L4 protocol — IANA proto number (TCP=6, UDP=17).
    pub proto: u8,
    /// Pad byte for 8-byte alignment in BPF map storage. Always 0.
    pub _pad: u8,
}

/// Outer-map value — the original VIP the client connected to.
/// 8 bytes, host-order. The egress reverse-NAT program rewrites the
/// source 5-tuple of a backend response packet to `(ip, port)` so
/// the client sees the packet as coming from the VIP, not from the
/// backend.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vip {
    /// Original VIP IPv4 address. Host-order.
    pub ip_host: u32,
    /// Original VIP port. Host-order.
    pub port_host: u16,
    /// Pad bytes for 8-byte alignment in BPF map storage. Always 0.
    pub _pad: u16,
}

/// Capacity per architecture.md § 10. Sized to comfortably hold every
/// backend × proto entry a single node will route across all services.
pub const MAX_ENTRIES: u32 = 1_048_576;

/// `REVERSE_NAT_MAP` — `BPF_MAP_TYPE_HASH` keyed on `BackendKey` →
/// `Vip`. The egress reverse-NAT path looks up
/// `(backend_ip, backend_port, proto)` and rewrites the source
/// 5-tuple of the response packet to `(vip.ip, vip.port)` before
/// the kernel networking stack sees the packet.
///
/// Slice 05 ships the kernel-side `#[map]` declaration and the
/// userspace lockstep-write contract; the egress XDP / TC program
/// itself lands in subsequent slice work per
/// `docs/feature/phase-2-xdp-service-map/discuss/story-map.md`.
#[map]
pub static REVERSE_NAT_MAP: HashMap<BackendKey, Vip> = HashMap::with_max_entries(MAX_ENTRIES, 0);
