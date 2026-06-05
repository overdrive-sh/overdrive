//! `build_local_service_key` — the single shared key-construction +
//! `user_port` low-16-NBO site for the three same-host cgroup hooks
//! (connect4 + sendmsg4 + recvmsg4) per ADR-0053 rev 2026-06-05 § D4
//! (Option 3, user-locked).
//!
//! **This helper does ONE thing: build a key from a `bpf_sock_addr`,
//! handling the `user_port` low-16-NBO hazard correctly.** It performs
//! NO map lookup and NO rewrite. Per F-1 (architect review) and DDD-4:
//!
//! - The **map lookup differs per hook** — connect4 + sendmsg4 look up
//!   `LOCAL_BACKEND_MAP` (forward); recvmsg4 looks up
//!   `REVERSE_LOCAL_MAP` (reverse). That lookup stays in each program
//!   body.
//! - The **rewrite direction differs per hook** — connect4/sendmsg4 do
//!   a forward DEST rewrite (`user_ip4`/`user_port` → backend);
//!   recvmsg4 does a reverse SOURCE rewrite (the source sockaddr → VIP
//!   / sentinel). That rewrite stays in each program body.
//!
//! ONE helper MUST NOT serve both rewrite directions. It builds the
//! `(addr_host, port_host, proto_byte)` triple; each hook composes the
//! map-specific key from that triple.
//!
//! ## `user_port` low-16-NBO hazard (the load-bearing reason this is shared)
//!
//! `bpf_sock_addr.user_port` is a `u32` whose LOW 16 bits carry the
//! network-byte-order port; the high 16 bits are undefined. The correct
//! read is `u16::from_be(user_port as u16)` — cast to u16 FIRST, then
//! `from_be`. `u32::from_be(user_port) as u16` swaps the whole u32 then
//! takes the wrong half (always 0). There is NO Tier-2 backstop for
//! this (`BPF_PROG_TEST_RUN` ENOTSUPP for `cgroup_sock_addr` ≤ 6.8) —
//! one correct shared site is the structural defense. See
//! `.claude/rules/development.md` § "`bpf_sock_addr.user_port`".
//!
#![allow(dead_code)]

/// The host-order address triple a same-host cgroup hook keys on,
/// extracted from a `bpf_sock_addr`. Each hook composes its
/// map-specific key (`LocalServiceKey` for the forward lookup,
/// `ReverseLocalKey` for the reverse lookup) from this triple.
#[derive(Clone, Copy)]
pub struct LocalServiceKeyParts {
    /// IPv4 address, host-order (`u32::from_be(user_ip4)`).
    pub addr_host: u32,
    /// Port, host-order (`u16::from_be(user_port as u16)` — low-16-NBO).
    pub port_host: u16,
    /// IANA L4 proto byte, zero-translation from `bpf_sock_addr.protocol`.
    pub proto_byte: u8,
}

/// Build the host-order `(addr, port, proto)` triple from a raw
/// `bpf_sock_addr` pointer, handling the `user_port` low-16-NBO hazard.
/// Returns `Err(())` on a null pointer (the caller falls back to its
/// hook-specific allow/pass verdict).
///
/// `#[inline(always)]` is required — the verifier needs the field
/// reads at the call site, not behind a function call.
///
/// # Safety
///
/// `sock_addr` must be the kernel-provided `*mut bpf_sock_addr` for the
/// current program invocation, valid for the duration of the call.
#[inline(always)]
pub unsafe fn build_local_service_key_parts(
    sock_addr: *const aya_ebpf::bindings::bpf_sock_addr,
) -> Result<LocalServiceKeyParts, ()> {
    if sock_addr.is_null() {
        return Err(());
    }

    // SAFETY: the caller's `# Safety` contract guarantees `sock_addr`
    // is the kernel-provided `*const bpf_sock_addr` for the current
    // program invocation; the kernel guarantees the pointer is valid
    // for the call duration and the struct bounds are fixed by the
    // in-tree UAPI. We read three fields (`user_ip4`, `user_port`,
    // `protocol`) — all within the kernel-guaranteed struct layout.
    let user_ip4_nbo = unsafe { (*sock_addr).user_ip4 };
    let user_port_nbo = unsafe { (*sock_addr).user_port };
    // `bpf_sock_addr.protocol` carries the IANA L4 proto number
    // (IPPROTO_TCP=6, IPPROTO_UDP=17) as a kernel-internal u32 in
    // host byte order — NOT network-order, and NOT a SOCK_* type
    // (that lives in `.type`). Zero translation: truncate the low
    // byte straight into the key. No SOCK_*→IPPROTO_* table; a single
    // byte so no endianness swap.
    let proto = unsafe { (*sock_addr).protocol };

    let addr_host = u32::from_be(user_ip4_nbo);
    // `user_port` is a u32 in `bpf_sock_addr` whose LOW 16 bits carry
    // the network-byte-order port (the kernel copies `inet_sk->inet_dport`
    // — already nbo — into the low 16 bits; the high 16 bits are
    // undefined). Truncate to u16 to obtain the nbo port, THEN
    // byte-swap to host order. Reading `u32::from_be(...) as u16`
    // would swap the whole u32 and then take the wrong half — always 0.
    // See `.claude/rules/development.md` § "`bpf_sock_addr.user_port`".
    #[allow(clippy::cast_possible_truncation)]
    let port_host = u16::from_be(user_port_nbo as u16);
    #[allow(clippy::cast_possible_truncation)]
    let proto_byte = proto as u8;

    Ok(LocalServiceKeyParts { addr_host, port_host, proto_byte })
}
