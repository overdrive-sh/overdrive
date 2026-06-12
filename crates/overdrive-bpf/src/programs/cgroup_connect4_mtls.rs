//! `cgroup_connect4_mtls` — `BPF_CGROUP_INET4_CONNECT` program for the
//! transparent-mTLS OUTBOUND intercept (ADR-0069, GH #26; D-MTLS-6).
//!
//! The OUTBOUND mirror of `cgroup_connect4_service`, proven in
//! `findings-userspace-relay.md` Unknown 1: the workload `connect()`s to a real
//! peer `(ip, port)`; this hook rewrites the destination to the AGENT's leg-F
//! listener so the workload — unaware — lands on the agent (transparent
//! interception). The userspace adapter programs `MTLS_REDIRECT_DEST[peer_key] =
//! agent_listener` before the workload connects.
//!
//! Distinct from `cgroup_connect4_service` (the same-host LB rewrite keyed on
//! `LOCAL_BACKEND_MAP`): this program is the mTLS proxy intercept, keyed on its
//! OWN `MTLS_REDIRECT_DEST` map, so the two interception concerns do not collide.
//! Both attach to the workload cgroup subtree; the agent's own leg-B dial is
//! exempted (F5 — `SO_MARK` / cgroup scoping; the agent attaches the program to
//! the *workload* subtree, not its own, so the agent's dial is never
//! re-intercepted).
//!
//! Returns 1 on every code path — the hook only rewrites; it never denies (it is a
//! proxy intercept, NOT a firewall). On a miss (no programmed redirect for the
//! dest) the connect proceeds unchanged.
//!
//! Endianness lockstep per `.claude/rules/development.md` § "`bpf_sock_addr`
//! `user_port` low-16-NBO in a u32": `user_ip4` / `user_port` are
//! network-byte-order per UAPI; `u32::from_be` / `u16::from_be` on read against
//! host-order map storage, `.to_be()` on the rewrite-write.

#![allow(dead_code)]

use aya_ebpf::{macros::cgroup_sock_addr, programs::SockAddrContext};

use crate::maps::mtls_redirect_dest::{MTLS_REDIRECT_DEST, MtlsDestKey};

/// `BPF_CGROUP_INET4_CONNECT` entry point for the mTLS outbound intercept.
/// Returns 1 on every code path — the hook only rewrites the destination to the
/// agent's leg-F listener; it never denies.
#[cgroup_sock_addr(connect4)]
pub fn cgroup_connect4_mtls(ctx: SockAddrContext) -> i32 {
    try_cgroup_connect4_mtls(&ctx).unwrap_or(1)
}

#[inline(always)]
fn try_cgroup_connect4_mtls(ctx: &SockAddrContext) -> Result<i32, ()> {
    let sock_addr = ctx.sock_addr;

    // SAFETY: aya's `SockAddrContext` exposes `*mut bpf_sock_addr` directly. The
    // kernel guarantees the pointer is valid for the program invocation; the
    // struct bounds are fixed by the in-tree UAPI. `user_ip4` is NBO; `user_port`
    // is low-16-NBO in a u32 (the development.md hazard — u16-truncate THEN from_be).
    let dst_ip_host = u32::from_be(unsafe { (*sock_addr).user_ip4 });
    // The `as u16` truncation is DELIBERATE: `user_port`'s low 16 bits carry the
    // NBO port, the high half is undefined (`.claude/rules/development.md` §
    // "`bpf_sock_addr.user_port` — low-16-NBO in a u32"). Truncate THEN from_be.
    #[allow(clippy::cast_possible_truncation)]
    let dst_port_host = u16::from_be(unsafe { (*sock_addr).user_port } as u16);

    let key = MtlsDestKey { ip_host: dst_ip_host, port_host: dst_port_host, _pad: 0 };

    // SAFETY: canonical aya map lookup shape; the returned reference is valid for
    // the program invocation.
    let entry = unsafe { MTLS_REDIRECT_DEST.get(&key) };
    let Some(dest) = entry else {
        // Miss — allow the connect unchanged (not a programmed mTLS destination).
        return Ok(1);
    };

    // Hit — rewrite the destination to the agent's leg-F listener. Convert
    // host-order map values back to network-order for the syscall context.
    // `user_port`'s low 16 bits carry the NBO port; widen the NBO u16 into the
    // low 16 bits of the u32 (high 16 bits stay 0).
    let agent_ip_nbo = dest.ip_host.to_be();
    let agent_port_nbo = u32::from(dest.port_host.to_be());

    // SAFETY: kernel-guaranteed struct layout; `user_ip4` and `user_port` are both
    // in the writable set per the kernel UAPI.
    unsafe {
        (*sock_addr).user_ip4 = agent_ip_nbo;
        (*sock_addr).user_port = agent_port_nbo;
    }

    Ok(1)
}
