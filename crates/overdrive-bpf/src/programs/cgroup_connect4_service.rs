//! `cgroup_connect4_service` — `BPF_CGROUP_INET4_CONNECT` program
//! per ADR-0053 § 1.
//!
//! Attached to the operator-configured cgroup ancestor (default
//! `/sys/fs/cgroup/overdrive.slice` — both the control plane and
//! every workload spawned via `ExecDriver` live as descendants).
//! Intercepts every IPv4 `connect(2)` from a process inside the
//! cgroup, looks up `(user_ip4, user_port)` against
//! `LOCAL_BACKEND_MAP`, and either:
//!
//! - Miss: returns 1 (allow connect unchanged; non-service traffic).
//! - Hit: overwrites `ctx->user_ip4` and `ctx->user_port` with the
//!   backend's address, returns 1 (allow connect to rewritten
//!   destination).
//!
//! Returns 1 on every code path — `0` (deny) is never returned. The
//! kernel proceeds with the (possibly-rewritten) destination. No
//! checksum work, no FIB lookup, no L2 rewrite — those are
//! wire-boundary concerns the cgroup hook never sees.
//!
//! Endianness lockstep per ADR-0041 / architecture.md § 11:
//! `bpf_sock_addr.user_ip4` and `bpf_sock_addr.user_port` carry
//! network-byte-order per kernel UAPI. We `u32::from_be(...)` /
//! `u16::from_be(...)` on read, look up against host-order map
//! storage, and `.to_be()` on write.

#![allow(dead_code)]

use aya_ebpf::{macros::cgroup_sock_addr, programs::SockAddrContext};

use crate::maps::local_backend_map::{LOCAL_BACKEND_MAP, LocalServiceKey};

/// `BPF_CGROUP_INET4_CONNECT` entry point. Returns 1 on every code
/// path — the hook only rewrites; it never denies.
///
/// On any internal error (bounds-check failure, missing context
/// fields), the inner `try_*` body returns `Err(())` and we fall
/// back to verdict 1 — allow the connect unchanged. The hook is
/// a same-host LB primitive, NOT a firewall: denying on internal
/// error would break non-service traffic for processes that
/// happen to live in the attach cgroup.
#[cgroup_sock_addr(connect4)]
pub fn cgroup_connect4_service(ctx: SockAddrContext) -> i32 {
    try_cgroup_connect4_service(&ctx).unwrap_or(1)
}

#[inline(always)]
fn try_cgroup_connect4_service(ctx: &SockAddrContext) -> Result<i32, ()> {
    // SAFETY: aya's `SockAddrContext` exposes `*mut bpf_sock_addr`
    // directly. The kernel guarantees the pointer is valid for the
    // duration of the program invocation; the bounds of the struct
    // are fixed by the in-tree UAPI definition. We read two u32
    // fields (`user_ip4`, `user_port`) and write the same two
    // back — all reads/writes are within the kernel-guaranteed
    // struct layout.
    let sock_addr = ctx.sock_addr;
    if sock_addr.is_null() {
        return Err(());
    }

    // Read connect destination — network-byte-order per kernel
    // UAPI. Convert to host-order for the map key.
    let user_ip4_nbo = unsafe { (*sock_addr).user_ip4 };
    let user_port_nbo = unsafe { (*sock_addr).user_port };

    let vip_host = u32::from_be(user_ip4_nbo);
    // `user_port` is stored as a u32 in `bpf_sock_addr`, but the
    // significant port bits are the high 16 bits in network-byte
    // order. After `from_be` we get the host-order port in the low
    // 16 bits.
    #[allow(clippy::cast_possible_truncation)]
    let port_host = u32::from_be(user_port_nbo) as u16;

    let key = LocalServiceKey { vip_host, port_host, _pad: 0 };

    // SAFETY: `LOCAL_BACKEND_MAP.get(...)` is the canonical
    // verifier-readable aya-ebpf map access shape. The verifier
    // validates the bounded operation; the returned reference is
    // valid for the duration of the program invocation.
    let entry = unsafe { LOCAL_BACKEND_MAP.get(&key) };
    let Some(entry) = entry else {
        // Miss — allow connect unchanged.
        return Ok(1);
    };

    // Hit — rewrite destination. Convert host-order map values
    // back to network-order for the syscall context.
    let backend_ip_nbo = entry.backend_ip_host.to_be();
    let backend_port_nbo = u32::from(entry.backend_port_host).to_be();

    // SAFETY: same as the read above — kernel-guaranteed struct
    // layout. The verifier permits in-place writes to specific
    // `bpf_sock_addr` fields documented as writable; `user_ip4`
    // and `user_port` are both in that set per the kernel UAPI.
    unsafe {
        (*sock_addr).user_ip4 = backend_ip_nbo;
        (*sock_addr).user_port = backend_port_nbo;
    }

    // Allow connect with rewritten destination.
    Ok(1)
}
