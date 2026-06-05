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
//! "drop on miss" is impossible at any layer. On a `REVERSE_LOCAL_MAP`
//! miss the program rewrites the source to the sentinel `192.0.2.1`
//! (RFC 5737 — never the backend IP) and bumps
//! `REVERSE_LOCAL_MISS_COUNTER` (US-03 / K5). Strictly stronger than
//! Cilium's pass-through-leak.
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
//! is sendmsg-only; DDD-5e). Kernel floor: `BPF_CGROUP_UDP4_RECVMSG`
//! since 4.20 (below the 5.10 LTS floor — no matrix bump).
//!
//! NO Tier-2 backstop (ENOTSUPP ≤ 6.8). Tier-3-only correctness +
//! Tier-1 reply-path equivalence invariant.
//!
//! # RED scaffold (Slice 01 / S-01-01)
//!
//! Per the kernel-side RED convention, the RED signal is the ABSENCE of
//! the `#[cgroup_sock_addr(recvmsg4)]` attribute. DELIVER adds it + the
//! body (Slice 01 GREEN); the sentinel-miss branch lands with Slice 03.

#![allow(dead_code)]

use aya_ebpf::programs::SockAddrContext;

/// `BPF_CGROUP_UDP4_RECVMSG` entry point. The verifier requires the
/// return value to be exactly 1 (`[1,1]` — cannot deny).
///
/// RED scaffold: the `#[cgroup_sock_addr(recvmsg4)]` attribute is NOT
/// yet present — that absence is the kernel-side RED signal. DELIVER
/// adds the attribute and fills the body (Slice 01 GREEN): build key via
/// the shared helper → own `REVERSE_LOCAL_MAP` lookup → reverse source-
/// rewrite to VIP (hit) or sentinel 192.0.2.1 + miss-counter (miss) →
/// return 1.
// __SCAFFOLD__ — add `#[cgroup_sock_addr(recvmsg4)]` in DELIVER (Slice 01 GREEN).
pub fn cgroup_recvmsg4_service(ctx: SockAddrContext) -> i32 {
    // Return 1 unconditionally — the only verifier-legal verdict for a
    // recvmsg4 program (`[1,1]`). The source rewrite (hit → VIP, miss →
    // sentinel) lands in GREEN; with no attribute the kernel never
    // invokes this scaffold.
    let _ = ctx;
    1
}
