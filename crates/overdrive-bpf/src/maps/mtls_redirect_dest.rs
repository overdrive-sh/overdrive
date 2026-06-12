//! `MTLS_REDIRECT_DEST` — kernel-side `BPF_MAP_TYPE_HASH` keyed on
//! `MtlsDestKey { ip_host: u32, port_host: u16, _pad: u16 }` →
//! `MtlsAddrPort { ip_host: u32, port_host: u16, _pad: u16 }` for the
//! transparent-mTLS OUTBOUND intercept (ADR-0069, GH #26; D-MTLS-6).
//!
//! The `cgroup_connect4_mtls` program does one lookup per `connect(2)` against
//! this map. The userspace `HostMtlsEnforcement` adapter programs
//! `MTLS_REDIRECT_DEST[real_peer] = agent_leg_f_listener` before the workload
//! connects, so the workload's `connect()` is transparently rewritten to the
//! agent's leg-F listener (`findings-userspace-relay.md` Unknown 1).
//!
//! Distinct from `LOCAL_BACKEND_MAP` (the same-host LB rewrite): this is the mTLS
//! proxy intercept's OWN destination table, so the two interception concerns do
//! not collide on one map.
//!
//! Endianness lockstep per ADR-0041 / `.claude/rules/development.md`:
//! userspace writes host-order; the kernel-side `cgroup_connect4_mtls` program
//! reads `bpf_sock_addr.user_ip4` (NBO per UAPI), converts to host-order via
//! `u32::from_be(...)` at the boundary, then keys this map by host-order bytes.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Outer-map key — the real-peer destination the workload aimed at, host-order.
/// 8-byte POD; mirrors the userspace `MtlsDestKey` POD in
/// `crates/overdrive-dataplane/src/mtls`.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct MtlsDestKey {
    /// Real-peer IPv4, host-order. `u32::from(Ipv4Addr::new(a, b, c, d))`.
    pub ip_host: u32,
    /// Real-peer port, host-order.
    pub port_host: u16,
    /// Padding to 8-byte alignment. Always zero (the kernel hashes the full key
    /// bytes; uninitialised pad would split logically-equal keys).
    pub _pad: u16,
}

/// Compile-time guard: the key MUST stay 8 bytes — a drift fails the build here,
/// not silently at the next mis-keyed cgroup lookup.
const _: () = assert!(core::mem::size_of::<MtlsDestKey>() == 8);

/// Outer-map value — the agent's leg-F listener address the connect is rewritten
/// to. 8 bytes, host-order.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct MtlsAddrPort {
    /// Agent leg-F listener IPv4, host-order. The cgroup program writes
    /// `bpf_sock_addr.user_ip4 = ip_host.to_be()`.
    pub ip_host: u32,
    /// Agent leg-F listener port, host-order. Written as `user_port =
    /// u32::from(port_host.to_be())` — `to_be()` swaps the host-order `u16` into
    /// NBO, then widens into the low 16 bits of `user_port` (high half stays 0).
    pub port_host: u16,
    /// Padding for 8-byte alignment. Always zero.
    pub _pad: u16,
}

/// Capacity — one entry per programmed mTLS outbound destination. Sized
/// comfortably above any Phase 1 single-node deployment's concurrent
/// outbound-mTLS destination count.
pub const MAX_ENTRIES: u32 = 4096;

/// `MTLS_REDIRECT_DEST` — `BPF_MAP_TYPE_HASH` keyed on `MtlsDestKey` →
/// `MtlsAddrPort`. The `cgroup_connect4_mtls` program does one lookup per
/// `connect(2)`; miss → allow connect unchanged; hit → rewrite `(user_ip4,
/// user_port)` to the agent leg-F listener and allow.
#[map]
pub static MTLS_REDIRECT_DEST: HashMap<MtlsDestKey, MtlsAddrPort> =
    HashMap::with_max_entries(MAX_ENTRIES, 0);
