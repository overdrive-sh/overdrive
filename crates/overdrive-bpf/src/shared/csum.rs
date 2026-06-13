//! Shared L4 / IPv4 checksum helpers for kernel-side XDP programs.
//!
//! Both NAT programs rewrite a handful of header bytes per packet
//! (a 4-byte IP address + a 2-byte L4 port) and fix up the affected
//! checksums **incrementally** (RFC 1624) — never a payload walk.
//!
//! The L4 checksum covers the changed IP (via the TCP/UDP
//! pseudo-header) and the changed port (in the L4 header), so the
//! correct fixup folds *only* the deltas of those two fields into the
//! packet's existing valid L4 checksum. This is exactly Cilium's
//! production NAT (`bpf/lib/nat.h:489` — `csum_diff(&old_addr, 4,
//! &new_addr, 4, 0)`, chained with the port delta; it never walks the
//! payload) and the canonical RFC 1624 formula.
//!
//! # The incoming-checksum precondition (load-bearing)
//!
//! Incremental update requires the *incoming* packet's L4 checksum to
//! be **valid (FULL) at XDP ingress**. On a veth interface with
//! TX-checksum-offload enabled, a locally-generated packet arrives at
//! the XDP hook with `CHECKSUM_PARTIAL` — the on-wire L4 checksum
//! field holds only the pseudo-header partial, not a complete
//! checksum — and an incremental delta applied to that partial value
//! is garbage (every packet drops). The operational invariant
//! `ethtool -K <iface> tx off` forces the kernel to materialise the
//! FULL L4 checksum in software before the XDP hook, restoring a valid
//! base for the incremental update. The appliance OS owns every LB
//! veth and applies `tx off` at provisioning; the Tier-3 fixtures that
//! drive real sockets through veth apply it too. See
//! `docs/research/dataplane/bpf-verifier-complexity-and-perf-optimization-research.md`
//! § R-1 and `docs/research/dataplane/xdp-checksum-partial-veth-research.md`
//! (Approach F).
//!
//! # Byte-order domain (the trap this module avoids)
//!
//! Every helper here works **entirely in the big-endian-word domain**:
//! a checksum read via `read_u16_be` is the host-numeric value of the
//! two wire bytes, IP/port halves are extracted with `>> 16` /
//! `& 0xffff` (same encoding), and the folded result is written back
//! via `write_u16_be`. There is no `bpf_csum_diff` and no `swap_bytes`
//! — `bpf_csum_diff` / `csum_partial` accumulate in a *different*
//! (native-endian-of-16-bit-words) domain, and mixing the two is the
//! byte-swap trap that the prior full-recompute engine had to correct
//! with a trailing `swap_bytes`. A pure-incremental delta over
//! big-endian wire fields stays in one domain throughout, sidestepping
//! that trap entirely.

#[inline(always)]
fn csum_fold(mut csum: u32) -> u16 {
    csum = (csum & 0xffff) + (csum >> 16);
    csum = (csum & 0xffff) + (csum >> 16);
    csum = (csum & 0xffff) + (csum >> 16);
    #[allow(clippy::cast_possible_truncation)] // intentional checksum fold to u16
    let result = csum as u16;
    result
}

/// RFC 1624 incremental update — two (old, new) word pairs.
///   `new_csum = ~( ~old_csum + sum(~old_words) + sum(new_words) )`
/// All u16 inputs/outputs are big-endian (network order).
///
/// Inputs are individual words rather than slices so the verifier sees
/// a fully-unrolled body — slice iteration with a runtime length
/// explodes the verifier's path-walk budget.
///
/// Used for the IPv4 *header* checksum, where only the 4-byte IP
/// address changed (its two 16-bit halves).
#[inline(always)]
pub fn csum_incremental_2_2(
    old_csum: u16,
    old_lo: u16,
    old_hi: u16,
    new_lo: u16,
    new_hi: u16,
) -> u16 {
    let s: u32 = u32::from(!old_csum)
        + u32::from(!old_lo)
        + u32::from(!old_hi)
        + u32::from(new_lo)
        + u32::from(new_hi);
    !csum_fold(s)
}

/// RFC 1624 incremental update — three (old, new) word pairs.
///   `new_csum = ~( ~old_csum + sum(~old_words) + sum(new_words) )`
/// All u16 inputs/outputs are big-endian (network order).
///
/// Used for the L4 (TCP/UDP) checksum during a NAT rewrite, where the
/// changed fields are a 4-byte IP address (its two 16-bit halves, via
/// the pseudo-header) and a 2-byte L4 port — three words total.
///
/// Inputs are individual words rather than slices so the verifier sees
/// a fully-unrolled body; this is the same shape as
/// [`csum_incremental_2_2`] with one extra word pair for the port.
///
/// # Caller contract
///
/// 1. `old_csum` is the packet's **existing valid (FULL)** L4 checksum,
///    read via `read_u16_be` BEFORE any field is rewritten. (See the
///    module-level note on the `tx off` precondition for why it must
///    be FULL, not `CHECKSUM_PARTIAL`.)
/// 2. `old_ip_lo` / `old_ip_hi` are the low / high 16-bit halves of the
///    pre-rewrite IP that participates in the pseudo-header (dst IP on
///    the forward/DNAT path, src IP on the reverse/SNAT path), extracted
///    as `(ip & 0xffff)` / `(ip >> 16)` from the host-order IP. `new_*`
///    are the same halves of the post-rewrite IP.
/// 3. `old_port` / `new_port` are the pre / post-rewrite L4 port values
///    as read / to-be-written via `read_u16_be` / `write_u16_be`.
/// 4. The returned u16 is written back with `write_u16_be` (it is in
///    the same big-endian-word domain `old_csum` was read in).
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub fn csum_incremental_3_3(
    old_csum: u16,
    old_ip_lo: u16,
    old_ip_hi: u16,
    old_port: u16,
    new_ip_lo: u16,
    new_ip_hi: u16,
    new_port: u16,
) -> u16 {
    let s: u32 = u32::from(!old_csum)
        + u32::from(!old_ip_lo)
        + u32::from(!old_ip_hi)
        + u32::from(!old_port)
        + u32::from(new_ip_lo)
        + u32::from(new_ip_hi)
        + u32::from(new_port);
    !csum_fold(s)
}
