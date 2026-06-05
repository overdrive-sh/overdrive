//! `cgroup_recvmsg4_service` — `BPF_CGROUP_UDP4_RECVMSG` program per
//! ADR-0053 rev 2026-06-05 § D4 (GH #200).
//!
//! The reply-source rewrite that makes an unconnected same-host UDP
//! service reachable from a source-validating client. Without it, the
//! backend's reply carries the BACKEND IP as its source sockaddr, and
//! every DNS resolver DISCARDS a reply whose source ≠ the address it
//! queried (kernel commit `983695fa6765` is the documented proof).
//!
//! Fires on the unconnected `recvmsg`/`recvfrom` (non-NULL `msg_name`).
//! Builds its key via the shared `build_local_service_key` helper
//! (key-build + low-16-NBO ONLY), then does its OWN `REVERSE_LOCAL_MAP`
//! lookup (a DIFFERENT map from connect4/sendmsg4) and its OWN reverse
//! SOURCE rewrite (the source sockaddr the app reads → VIP). One helper
//! MUST NOT serve both rewrite directions (F-1).
//!
//! ## recvmsg4 CANNOT deny — `[1,1]` (load-bearing, DDD-3)
//!
//! The verifier restricts `BPF_CGROUP_UDP4_RECVMSG` to a return value of
//! exactly `[1,1]` — a program returning 0 is rejected at LOAD time. So
//! "drop on miss" is impossible at any layer.
//!
//! ## Miss is a pure no-op (ADR-0053 § D3 rev 2026-06-05b / UI-1)
//!
//! recvmsg4 is attached at a cgroup ANCESTOR and therefore fires on EVERY
//! unconnected-UDP `recvmsg`/`recvfrom` from any descendant — service
//! replies AND all unrelated same-host UDP (DNS clients, a backend reading
//! an inbound query). The `REVERSE_LOCAL_MAP` lookup, keyed on the
//! datagram's source identity, is the discriminator: a HIT means the
//! source is a registered backend (a service reply) → rewrite the source
//! to the VIP; a MISS means the source is NOT a registered backend (not a
//! service reply) → **pure no-op**, leave the real source byte-for-byte
//! intact and bump `REVERSE_LOCAL_MISS_COUNTER` for observability only.
//!
//! There is NO sentinel rewrite on the miss path. A source rewrite on a
//! miss would corrupt the sender address of every non-service datagram in
//! the cgroup (a backend reading a query would see a mangled sender and
//! reply to the wrong peer). The K5 no-leak guarantee — no backend IP ever
//! reaches the client app — is preserved by the D1 reverse-first
//! dual-write's always-hit property (every registered backend has a
//! reverse entry before its forward entry is usable, so a genuine service
//! reply ALWAYS hits and is ALWAYS rewritten to the VIP), NOT by a
//! miss-path sentinel. This is Cilium-aligned (`cil_sock4_recvmsg` returns
//! `SYS_PROCEED`, `__sock4_xlate_rev` leaves the source unchanged on a
//! reverse-SK miss), not weaker than it. The rejected sentinel-on-miss
//! design and the Tier-3-observed corruption it caused are recorded in
//! ADR-0053 § D3 (sub-revision 2026-06-05b) and the feature-delta CA-3.
//!
//! ## Layer: application sockaddr, NOT wire (DDD-3a)
//!
//! recvmsg4 fires inside `udp_recvmsg()` AFTER the kernel dequeued the
//! skb and populated the source sockaddr; a `tcpdump -i lo` shows the
//! backend source on every round-trip regardless. recvmsg4's domain is
//! the **application sockaddr** (`recvfrom`/`msg_name`) ONLY — the VIP-
//! sourced guarantee is asserted at the app layer, never the wire.
//!
//! Writable fields confirmed = `user_ip4` / `user_port` (`msg_src_ip4`
//! is sendmsg-only; DDD-5e). The reverse rewrite restores BOTH the
//! source address (`user_ip4 ← VIP`) AND the source port
//! (`user_port ← VIP_PORT`) per ADR-0053 §D4 — symmetric with the
//! forward path's full `(addr, port)` NAT. A source-validating resolver
//! (Unbound, BIND 9) discards a reply whose source ≠ the `(addr, port)`
//! it queried, so restoring the port is load-bearing for cross-port
//! services (DNS `VIP:53 → backend:5353`). Kernel floor:
//! `BPF_CGROUP_UDP4_RECVMSG` since 4.20 (below the 5.10 LTS floor — no
//! matrix bump).
//!
//! NO Tier-2 backstop (ENOTSUPP ≤ 6.8). Tier-3-only correctness +
//! Tier-1 reply-path equivalence invariant.
//!
//! # Scope split (Slice 01 vs Slice 03)
//!
//! Slice 01 lands the happy path: a `REVERSE_LOCAL_MAP` HIT rewrites the
//! source sockaddr to the VIP. The MISS branch is the pure no-op (source
//! untouched + `REVERSE_LOCAL_MISS_COUNTER` bump for observability only,
//! per UI-1) — structurally complete and verifier-legal. Its miss
//! BEHAVIOR (the counter bump under non-service traffic) is asserted in
//! Slice 03 (step 03-01); the happy-path round-trip never exercises it
//! under the ordered reverse-first dual-write (a genuine service reply
//! always hits).

#![allow(dead_code)]

use aya_ebpf::{macros::cgroup_sock_addr, programs::SockAddrContext};

use crate::maps::reverse_local_map::{REVERSE_LOCAL_MAP, ReverseLocalKey};
use crate::maps::reverse_local_miss_counter::{REVERSE_LOCAL_MISS_COUNTER, SLOT_REVERSE_MISS};
use crate::shared::build_local_service_key::build_local_service_key_parts;

/// `BPF_CGROUP_UDP4_RECVMSG` entry point. The verifier restricts the
/// return value to exactly 1 (`[1,1]` — recvmsg4 CANNOT deny; a program
/// returning 0 is rejected at LOAD time). Every code path returns 1.
///
/// On internal error the inner `try_*` body returns `Err(())` and we
/// fall back to verdict 1 with no rewrite — the reply reaches the app
/// with whatever source the kernel populated, the safe non-denying
/// default the `[1,1]` contract forces.
#[cgroup_sock_addr(recvmsg4)]
pub fn cgroup_recvmsg4_service(ctx: SockAddrContext) -> i32 {
    try_cgroup_recvmsg4_service(&ctx).unwrap_or(1)
}

#[inline(always)]
fn try_cgroup_recvmsg4_service(ctx: &SockAddrContext) -> Result<i32, ()> {
    let sock_addr = ctx.sock_addr;

    // On the recvmsg4 hook, the kernel has already populated the source
    // sockaddr (`user_ip4` / `user_port`) with the BACKEND identity the
    // datagram arrived from. The shared helper extracts that host-order
    // `(addr, port, proto)` triple (key-build + low-16-NBO ONLY); the
    // OWN reverse lookup + OWN reverse SOURCE rewrite stay here (F-1).
    //
    // SAFETY: aya's `SockAddrContext` exposes `*mut bpf_sock_addr`; the
    // kernel guarantees validity for the program invocation and the
    // struct bounds are fixed by the in-tree UAPI.
    let parts = unsafe { build_local_service_key_parts(sock_addr) }?;

    // The reverse key is the backend identity. Byte-parity with
    // `LocalServiceKey`, but the SEMANTICS are backend-side (DDD-2).
    let key = ReverseLocalKey {
        backend_ip_host: parts.addr_host,
        backend_port_host: parts.port_host,
        proto: parts.proto_byte,
        _pad: 0,
    };

    // SAFETY: canonical verifier-readable aya-ebpf map access shape.
    let entry = unsafe { REVERSE_LOCAL_MAP.get(&key) };

    // recvmsg4 is attached at a cgroup ANCESTOR and therefore fires on
    // EVERY unconnected UDP `recvmsg`/`recvfrom` from any descendant —
    // service replies AND all unrelated UDP (DNS clients, the backends'
    // own recvs, etc.). The map lookup is the discriminator: only a HIT
    // (the datagram's source is a registered backend identity) is a
    // service reply whose source must be rewritten back to the VIP.
    //
    // A MISS is the overwhelmingly common case — all non-service UDP —
    // and MUST be a pure no-op (leave the source untouched, return 1).
    // Rewriting the source on a miss would corrupt every unrelated UDP
    // recv in the cgroup (e.g. a backend reading a query would have its
    // sender address mangled, so its reply would target the wrong peer).
    // The miss counter is bumped for observability only; its BEHAVIOR
    // (and any sentinel-rewrite fail-safe scoped to service replies) is
    // Slice 03's concern (step 03-01) — NOT asserted here. recvmsg4
    // cannot deny (`[1,1]`); every path returns 1.
    let Some(entry) = entry else {
        record_reverse_miss();
        return Ok(1);
    };

    // HIT — this datagram came from a registered backend identity, so it
    // is a service reply. Rewrite the source the app reads back to the
    // VIP — BOTH the source address AND the source port (ADR-0053 §D4).
    // Convert the host-order VIP fields to network-order for the syscall
    // context. Writable fields on the recvmsg4 attach type are
    // `user_ip4` / `user_port` (`msg_src_ip4` is sendmsg-only; DDD-5e).
    //
    // `user_port`'s low 16 bits carry the network-byte-order port; widen
    // the nbo `u16` into the low 16 bits of the `u32` (high 16 bits stay
    // 0). This is the low-16-NBO idiom — `u32::from(port.to_be())`, the
    // SAME write shape sendmsg4/connect4 use on the forward path. Do NOT
    // use `from_be(..) as u16` (the silent-0 trap, `.claude/rules/
    // development.md` § "`bpf_sock_addr.user_port` — low-16-NBO in a
    // u32").
    let source_ip_nbo = entry.vip_host.to_be();
    let source_port_nbo = u32::from(entry.vip_port_host.to_be());

    // SAFETY: kernel-guaranteed struct layout; `user_ip4` / `user_port`
    // are both writable on the recvmsg4 attach type. Restoring the port
    // alongside the address makes the reply pass a source-validating
    // resolver's `(addr, port)` check for cross-port services (DNS
    // VIP:53 → backend:5353).
    unsafe {
        (*sock_addr).user_ip4 = source_ip_nbo;
        (*sock_addr).user_port = source_port_nbo;
    }

    Ok(1)
}

/// Bump the per-CPU reverse-miss counter (observability only). Called on
/// a `REVERSE_LOCAL_MAP` miss — the common non-service-reply case. Does
/// NOT rewrite the source: recvmsg4 fires on all cgroup UDP recvs, so a
/// miss is a pure no-op to avoid corrupting unrelated traffic (UI-1). The
/// counter is behaviorally inert — its incrementing has no effect on the
/// source the app reads; it exists only so an anomalous miss rate (which
/// should-never-happen for service replies under the reverse-first
/// dual-write) is observable.
///
/// `get_ptr_mut` returns the running CPU's slot pointer; the increment
/// is per-CPU-local and lock-free (single program context per CPU, no
/// re-entry on the recvmsg path). Mirrors the `DROP_COUNTER` precedent
/// in `programs/sanity.rs`.
#[inline(always)]
fn record_reverse_miss() {
    if let Some(counter) = REVERSE_LOCAL_MISS_COUNTER.get_ptr_mut(SLOT_REVERSE_MISS) {
        // SAFETY: per-CPU array point access validated by the verifier;
        // the pointer is valid for the program invocation and the write
        // is to this CPU's slot only.
        unsafe {
            *counter = (*counter).wrapping_add(1);
        }
    }
}
