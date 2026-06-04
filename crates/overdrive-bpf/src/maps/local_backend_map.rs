//! `LOCAL_BACKEND_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `LocalServiceKey { vip_host: u32, port_host: u16, proto: u8, _pad: u8 }`
//! → `LocalBackendEntry { backend_ip_host: u32, backend_port_host: u16,
//! _pad: u16 }`. Single global; one entry per `(VIP, vip_port, proto)`
//! per ADR-0053 § 1 (rev 2026-06-03 — IPVS-style proto-keyed so a
//! service co-locating tcp/53 + udp/53 on one VIP routes each protocol
//! to its own backend).
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
///
/// Step 02-02 (ADR-0053 rev 2026-06-03) widened the key from
/// `(vip, port)` to `(vip, port, proto)`: the IANA L4 proto byte
/// (TCP=6, UDP=17) absorbs one reserved pad byte; the 8-byte key
/// envelope is preserved. The trailing `_pad` is zeroed for
/// deterministic BPF hashing (the kernel hashes the full key bytes,
/// uninitialised pad would split logically-equal keys).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LocalServiceKey {
    /// VIP IPv4, host-order. `u32::from(Ipv4Addr::new(a, b, c, d))`.
    pub vip_host: u32,
    /// VIP port, host-order.
    pub port_host: u16,
    /// IANA L4 protocol byte — TCP=6, UDP=17. Sourced cgroup-side
    /// from `bpf_sock_addr.protocol` (zero translation; single byte
    /// so no endianness swap).
    pub proto: u8,
    /// Padding to 8-byte alignment. Always zero.
    pub _pad: u8,
}

/// Compile-time guard: the proto-widened key MUST stay 8 bytes
/// (ADR-0053 rev Amendment 1). A drift off 8 — e.g. promoting `_pad`
/// back to `u16` or adding a field — fails the build here, not silently
/// at the next mis-keyed cgroup lookup.
const _: () = assert!(core::mem::size_of::<LocalServiceKey>() == 8);

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
    /// u32::from(backend_port_host.to_be())` — `to_be()` swaps the
    /// host-order `u16` into network-byte-order, then widens into
    /// the low 16 bits of `user_port` (high 16 bits stay zero).
    /// Per the in-tree `bpf_sock_addr` definition, only the low 16
    /// bits of `user_port` carry the NBO port; the high half is
    /// undefined. See `.claude/rules/development.md` §
    /// "`bpf_sock_addr.user_port` — low-16-NBO in a u32".
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
