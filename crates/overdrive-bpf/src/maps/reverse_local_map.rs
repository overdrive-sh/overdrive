//! `REVERSE_LOCAL_MAP` — kernel-side `BPF_MAP_TYPE_HASH` keyed on the
//! backend identity `ReverseLocalKey { backend_ip_host: u32,
//! backend_port_host: u16, proto: u8, _pad: u8 }` → `vip_host: u32`
//! (the original VIP). The `cgroup_recvmsg4_service` program does one
//! lookup per unconnected `recvmsg(2)` to rewrite the reply *source*
//! the app reads (`recvfrom`/`msg_name`) backend→VIP.
//!
//! ADR-0053 revision 2026-06-05 (GH #200) — the reply store for the
//! UNCONNECTED-UDP same-host cgroup path. DISTINCT from the XDP
//! `REVERSE_NAT_MAP` (the connected/remote wire path): different hook,
//! different key envelope value semantics. The key reuses the byte
//! layout of `LocalServiceKey` (the SAME 8-byte POD shape) so the
//! userspace `BackendKey {ip, port, proto}` newtype lowers to it with
//! byte-parity (DDD-2).
//!
//! Written **ordered (reverse-first)** by the same
//! `register_local_backend` call that writes `LOCAL_BACKEND_MAP` — two
//! BPF map syscalls, the guarantee is ordering (no observer sees a
//! forward entry without its reverse), not atomicity (DDD-1, F-2).
//!
//! On a miss, recvmsg4 rewrites the source to the sentinel `192.0.2.1`
//! (RFC 5737) and bumps `REVERSE_LOCAL_MISS_COUNTER` — recvmsg4 CANNOT
//! deny (verifier `[1,1]`, research Q1), so the fail-safe is a source
//! rewrite, not a drop (DDD-3).
//!
//! Endianness lockstep per ADR-0041 / architecture.md § 11: userspace
//! writes host-order; the kernel-side program converts at the boundary.
//!
#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Reverse-map key — the backend identity. 8-byte POD, host-order on
/// every numeric field. Byte-parity with `LocalServiceKey` so the
/// userspace `BackendKey {ip, port, proto}` lowers to it directly
/// (DDD-2). The trailing `_pad` is zeroed for deterministic BPF hashing.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ReverseLocalKey {
    /// Backend IPv4, host-order. `u32::from(Ipv4Addr)`.
    pub backend_ip_host: u32,
    /// Backend port, host-order.
    pub backend_port_host: u16,
    /// IANA L4 protocol byte — TCP=6, UDP=17. The unconnected path is
    /// UDP-only in practice, but the key carries proto for byte-parity
    /// with the three existing keys and to disambiguate a backend
    /// socket shared across protos.
    pub proto: u8,
    /// Padding to 8-byte alignment. Always zero.
    pub _pad: u8,
}

/// Compile-time guard: the reverse key MUST stay 8 bytes (byte-parity
/// with `LocalServiceKey`). A drift fails the build here, not silently
/// at the next mis-keyed recvmsg4 lookup.
const _: () = assert!(core::mem::size_of::<ReverseLocalKey>() == 8);

/// Capacity per ADR-0053 rev — same envelope as `LOCAL_BACKEND_MAP`
/// (one reverse entry per forward entry).
pub const MAX_ENTRIES: u32 = 4096;

/// `REVERSE_LOCAL_MAP` — `BPF_MAP_TYPE_HASH` keyed on
/// `ReverseLocalKey` → `vip_host: u32`. One lookup per unconnected
/// `recvmsg(2)`; hit → rewrite reply source to the VIP; miss → rewrite
/// to the sentinel `192.0.2.1` + bump the miss counter.
#[map]
pub static REVERSE_LOCAL_MAP: HashMap<ReverseLocalKey, u32> =
    HashMap::with_max_entries(MAX_ENTRIES, 0);
