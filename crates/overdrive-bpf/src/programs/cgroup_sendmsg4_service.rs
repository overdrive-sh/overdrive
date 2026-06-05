//! `cgroup_sendmsg4_service` — `BPF_CGROUP_UDP4_SENDMSG` program per
//! ADR-0053 rev 2026-06-05 § D4 (GH #200).
//!
//! The per-datagram analogue of `cgroup_connect4_service`. Attached to
//! the same `cgroup_attach_path` (`overdrive.slice`), it fires on every
//! IPv4 `sendmsg`/`sendto` from a process inside the cgroup that did NOT
//! first `connect()` — the canonical DNS-resolver idiom (`dig`, glibc
//! `getaddrinfo`, musl: a `sendto(VIP)` per query). connect4 never sees
//! these datagrams (it fires only at `connect()` time), which is the
//! exact gap #200 closes.
//!
//! Forward rewrite: builds its key via the shared
//! `build_local_service_key` helper (key-build + low-16-NBO ONLY), then
//! does its OWN `LOCAL_BACKEND_MAP` lookup and its OWN forward DEST
//! rewrite (`user_ip4`/`user_port` → backend). Miss → allow unchanged
//! (verdict 1). Hit → rewrite + verdict 1. Like connect4, returns 1 on
//! every path — the hook only rewrites, never denies.
//!
//! proto read zero-translation from `bpf_sock_addr.protocol` (ADR-0053
//! Amd 2). Kernel floor: `BPF_CGROUP_UDP4_SENDMSG` since 4.18 (below the
//! 5.10 LTS floor — no matrix bump).
//!
//! There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for
//! `cgroup_sock_addr` (ENOTSUPP ≤ 6.8). Correctness is a Tier-3-only
//! gate (the unconnected round-trip) with the Tier-1 `SimDataplane`
//! reply-path equivalence invariant as the structural defense below it.

#![allow(dead_code)]

use aya_ebpf::{macros::cgroup_sock_addr, programs::SockAddrContext};

use crate::maps::local_backend_map::{LOCAL_BACKEND_MAP, LocalServiceKey};
use crate::shared::build_local_service_key::build_local_service_key_parts;

/// `BPF_CGROUP_UDP4_SENDMSG` entry point. Returns 1 on every code path —
/// the hook only rewrites the datagram destination; it never denies.
///
/// The per-datagram forward analogue of `cgroup_connect4_service`: on
/// any internal error the inner `try_*` body returns `Err(())` and we
/// fall back to verdict 1 (allow the `sendmsg` unchanged). The hook is a
/// same-host LB primitive, NOT a firewall — denying on internal error
/// would break non-service traffic for processes that happen to live in
/// the attach cgroup.
#[cgroup_sock_addr(sendmsg4)]
pub fn cgroup_sendmsg4_service(ctx: SockAddrContext) -> i32 {
    try_cgroup_sendmsg4_service(&ctx).unwrap_or(1)
}

#[inline(always)]
fn try_cgroup_sendmsg4_service(ctx: &SockAddrContext) -> Result<i32, ()> {
    let sock_addr = ctx.sock_addr;

    // Build the host-order `(addr, port, proto)` triple via the shared
    // key-build helper — the single site handling the `user_port`
    // low-16-NBO hazard (ADR-0053 § D4 / DDD-4, Option 3). The helper
    // does key-build + NBO ONLY; sendmsg4's OWN `LOCAL_BACKEND_MAP`
    // forward lookup and OWN forward DEST rewrite stay below (F-1: one
    // helper must not serve both rewrite directions).
    //
    // SAFETY: aya's `SockAddrContext` exposes `*mut bpf_sock_addr`
    // directly. The kernel guarantees the pointer is valid for the
    // duration of the program invocation; the struct bounds are fixed by
    // the in-tree UAPI definition. The helper reads `user_ip4` /
    // `user_port` / `protocol` within that layout.
    let parts = unsafe { build_local_service_key_parts(sock_addr) }?;

    let key = LocalServiceKey {
        vip_host: parts.addr_host,
        port_host: parts.port_host,
        proto: parts.proto_byte,
        _pad: 0,
    };

    // SAFETY: `LOCAL_BACKEND_MAP.get(...)` is the canonical
    // verifier-readable aya-ebpf map access shape. The verifier
    // validates the bounded operation; the returned reference is valid
    // for the duration of the program invocation.
    let entry = unsafe { LOCAL_BACKEND_MAP.get(&key) };
    let Some(entry) = entry else {
        // Miss — allow the datagram unchanged (non-service traffic).
        return Ok(1);
    };

    // Hit — forward DEST rewrite. Convert host-order map values back to
    // network-order for the syscall context. `user_port`'s low 16 bits
    // carry the network-byte-order port; widen the nbo u16 into the low
    // 16 bits of the u32 (high 16 bits stay 0). Same NBO write idiom as
    // connect4 (DDD-5e).
    let backend_ip_nbo = entry.backend_ip_host.to_be();
    let backend_port_nbo = u32::from(entry.backend_port_host.to_be());

    // SAFETY: same as the read above — kernel-guaranteed struct layout.
    // The verifier permits in-place writes to specific `bpf_sock_addr`
    // fields documented as writable; `user_ip4` / `user_port` are both in
    // that set per the kernel UAPI for the sendmsg4 attach type.
    unsafe {
        (*sock_addr).user_ip4 = backend_ip_nbo;
        (*sock_addr).user_port = backend_port_nbo;
    }

    // Allow the datagram with rewritten destination.
    Ok(1)
}
