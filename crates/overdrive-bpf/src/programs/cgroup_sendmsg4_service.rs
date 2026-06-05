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
//!
//! # RED scaffold (Slice 01 / S-01-01)
//!
//! Per the kernel-side RED convention (`programs/mod.rs`), the RED
//! signal is the ABSENCE of the `#[cgroup_sock_addr(sendmsg4)]`
//! attribute — `panic!`/`todo!` cannot expand cleanly inside the
//! handler (the `panic_handler` is `loop {}`). Adding the attribute +
//! the body is DELIVER's GREEN pass (Slice 01).

#![allow(dead_code)]

use aya_ebpf::programs::SockAddrContext;

/// `BPF_CGROUP_UDP4_SENDMSG` entry point. Returns 1 on every code path.
///
/// RED scaffold: the `#[cgroup_sock_addr(sendmsg4)]` attribute is NOT
/// yet present — that absence is the kernel-side RED signal. DELIVER
/// adds the attribute and fills the body (Slice 01 GREEN): build key via
/// the shared helper → own `LOCAL_BACKEND_MAP` lookup → own forward
/// dest-rewrite → return 1.
// __SCAFFOLD__ — add `#[cgroup_sock_addr(sendmsg4)]` in DELIVER (Slice 01 GREEN).
pub fn cgroup_sendmsg4_service(ctx: SockAddrContext) -> i32 {
    // Allow-unchanged is the safe RED placeholder verdict: with no
    // attribute the function is never invoked by the kernel; if it were,
    // verdict 1 ("allow, no rewrite") is the non-denying default the
    // hook contract requires. The forward rewrite lands in GREEN.
    let _ = ctx;
    1
}
