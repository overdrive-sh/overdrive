//! `LOCAL_BACKEND_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `LocalServiceKey { vip_host: u32, port_host: u16, _pad: u16 }` →
//! `LocalBackendEntry { backend_ip_host: u32, backend_port_host: u16,
//! _pad: u16 }`. Single global; one entry per `(VIP, vip_port)` per
//! ADR-0053 § 1.
//!
//! Endianness lockstep per ADR-0041 / architecture.md § 11:
//! userspace writes host-order; the kernel-side `cgroup_connect4_service`
//! program reads `bpf_sock_addr.user_ip4` (network-order per kernel
//! UAPI), converts to host-order via `u32::from_be(...)` at the
//! boundary, then keys this map by host-order bytes. Userspace
//! `LocalBackendMapHandle::upsert(vip, port, backend)` produces the
//! same host-order key from `u32::from(Ipv4Addr)`.
//!
//! Capacity per ADR-0053 § 1: `MAX_ENTRIES = 4096`, sized comfortably
//! above any Phase 1 deployment's expected service-count.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Outer-map key. 8-byte POD, host-order on every numeric field.
/// Mirrors the userspace `LocalServiceKey` POD in
/// `crates/overdrive-dataplane/src/maps/wire`.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LocalServiceKey {
    /// VIP IPv4, host-order. `u32::from(Ipv4Addr::new(a, b, c, d))`.
    pub vip_host: u32,
    /// VIP port, host-order.
    pub port_host: u16,
    /// Padding to 8-byte alignment. Always zero.
    pub _pad: u16,
}

/// Outer-map value — the resolved local backend. 8 bytes,
/// host-order. The cgroup program rewrites `bpf_sock_addr.user_ip4`
/// and `bpf_sock_addr.user_port` from these fields.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LocalBackendEntry {
    /// Backend IPv4, host-order. The cgroup program writes
    /// `bpf_sock_addr.user_ip4 = backend_ip_host.to_be()` to push
    /// network-order bytes back onto the syscall context.
    pub backend_ip_host: u32,
    /// Backend port, host-order. Written as `user_port =
    /// (backend_port_host as u32).to_be()` — the kernel UAPI
    /// stores the port in the upper 16 bits of `user_port` per the
    /// in-tree `bpf_sock_addr` definition.
    pub backend_port_host: u16,
    /// Padding for 8-byte alignment. Always zero.
    pub _pad: u16,
}

/// Capacity per ADR-0053 § 1. Same envelope as the `SERVICE_MAP`
/// outer-HoM cap; Phase 1 deployments sit far below this.
pub const MAX_ENTRIES: u32 = 4096;

/// `LOCAL_BACKEND_MAP` — `BPF_MAP_TYPE_HASH` keyed on
/// `LocalServiceKey` → `LocalBackendEntry`. The
/// `cgroup_connect4_service` program does one lookup per
/// `connect(2)` syscall against this map; miss → allow connect
/// unchanged; hit → rewrite `(user_ip4, user_port)` and allow.
#[map]
pub static LOCAL_BACKEND_MAP: HashMap<LocalServiceKey, LocalBackendEntry> =
    HashMap::with_max_entries(MAX_ENTRIES, 0);
