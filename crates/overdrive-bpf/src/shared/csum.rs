//! Shared L4 checksum helpers for kernel-side XDP programs.
//!
//! See `docs/research/dataplane/xdp-checksum-partial-veth-research.md`

use aya_ebpf::programs::XdpContext;

const MAX_L4_LEN: usize = 1500;

#[inline(always)]
fn csum_fold(mut csum: u32) -> u16 {
    csum = (csum & 0xffff) + (csum >> 16);
    csum = (csum & 0xffff) + (csum >> 16);
    csum = (csum & 0xffff) + (csum >> 16);
    #[allow(clippy::cast_possible_truncation)] // intentional checksum fold to u16
    let result = csum as u16;
    result
}

/// Read one byte from the packet with a bounds check.
/// Uses volatile reads of `xdp_md.data`/`data_end` to prevent
/// the compiler from CSE-ing the packet pointer across calls
/// (which would cause the verifier to see stale `r` values).
#[inline(always)]
unsafe fn pkt_read_u8(ctx: &XdpContext, off: usize) -> Result<u8, ()> {
    let s = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data) } as usize;
    let e = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data_end) } as usize;
    if s + off + 1 > e {
        return Err(());
    }
    Ok(unsafe { *((s + off) as *const u8) })
}

/// Read two bytes (BE u16) from the packet with a bounds check.
/// Volatile reads prevent CSE of the packet pointer.
#[inline(always)]
unsafe fn pkt_read_u16(ctx: &XdpContext, off: usize) -> Result<u16, ()> {
    let s = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data) } as usize;
    let e = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data_end) } as usize;
    if s + off + 2 > e {
        return Err(());
    }
    let p = (s + off) as *const [u8; 2];
    Ok(u16::from_be_bytes(unsafe { *p }))
}

/// Full L4 checksum recomputation by reading packet data word-by-word
/// through per-access bounds-checked pointer reads (`ptr_at`-style).
///
/// Each u16 read re-derives the packet pointer from `ctx.data()`
/// with a fresh bounds check, satisfying the verifier without
/// requiring `bpf_csum_diff`'s `pkt_access` (which has
/// operand-ordering issues with the Rust BPF backend).
///
/// # Caller contract
///
/// 1. Zero the L4 checksum field in the packet BEFORE calling this.
/// 2. Write the rewritten IP/port values BEFORE calling this.
/// 3. Pass **host-order** IP addresses (`u32::from(Ipv4Addr)` —
///    the same encoding stored in `SERVICE_MAP` / `BACKEND_MAP` /
///    `REVERSE_NAT_MAP` and returned by `read_u32_be`). The `>> 16`
///    / `& 0xffff` extraction produces pseudo-header u16 words in
///    the same host-order-of-network-order encoding as
///    `pkt_read_u16`.
/// 4. Pass `l4_payload_len` as the L4 segment length derived from
///    the IP header (`ip_total_length - ip_header_length`), NOT
///    from `ctx.data_end()`. On hardware NICs, `data_end` can
///    include Ethernet minimum-frame padding beyond the IP payload;
///    using it overcounts `l4_len` and produces a checksum over
///    padding bytes the remote stack does not expect.
#[inline(always)]
pub fn recompute_l4_csum(
    ctx: &XdpContext,
    src_ip: u32,
    dst_ip: u32,
    proto: u8,
    l4_off: usize,
    l4_payload_len: usize,
) -> Result<u16, ()> {
    let start = ctx.data();
    let end = ctx.data_end();

    if start + l4_off >= end {
        return Err(());
    }
    // Cap at the buffer's actual extent so we never read past
    // `data_end`, but prefer the IP-header-derived length to
    // exclude Ethernet minimum-frame padding.
    let buf_len = end - start - l4_off;
    let l4_len = buf_len.min(l4_payload_len);
    if l4_len > MAX_L4_LEN || l4_len == 0 {
        return Err(());
    }

    // Pseudo-header checksum (inline). All values are the
    // host-order interpretation of their network-order u16
    // representation — the same encoding `pkt_read_u16` returns
    // (i.e. `u16::from_be_bytes([wire_hi, wire_lo])`). The IP
    // address halves come from `src_ip_be >> 16` and `& 0xffff`
    // which already produces that encoding. Protocol and length
    // are simply their numeric value (protocol=6 is `[0x00,0x06]`
    // on wire → `u16::from_be_bytes` → 6; length=40 is `[0x00,
    // 0x28]` → 40). No `.to_be()` — that would byte-swap and
    // produce the wrong sum.
    let src_hi = (src_ip >> 16) as u16;
    let src_lo = (src_ip & 0xffff) as u16;
    let dst_hi = (dst_ip >> 16) as u16;
    let dst_lo = (dst_ip & 0xffff) as u16;

    let mut sum: u32 = 0;
    sum += u32::from(src_hi);
    sum += u32::from(src_lo);
    sum += u32::from(dst_hi);
    sum += u32::from(dst_lo);
    sum += u32::from(u16::from(proto));
    #[allow(clippy::cast_possible_truncation)] // intentional: BPF packet length fits u16
    let l4_len_u16 = l4_len as u16;
    sum += u32::from(l4_len_u16);

    // Sum the L4 segment word-by-word. Each `pkt_read_u16` call
    // re-reads ctx.data()/ctx.data_end() and performs a fresh
    // bounds check, satisfying the verifier on every kernel in the
    // matrix (5.10+). The bounded loop counter (i < MAX_L4_LEN/2)
    // is a compile-time constant the verifier can prove finite.
    let num_words = l4_len / 2;
    let mut i: usize = 0;
    while i < num_words {
        if i >= MAX_L4_LEN / 2 {
            break;
        }
        let w = unsafe { pkt_read_u16(ctx, l4_off + i * 2)? };
        sum += u32::from(w);
        i += 1;
    }

    // Odd trailing byte (left-padded with zero per RFC 1071).
    // Use `num_words * 2` as the byte offset rather than
    // `l4_len - 1` — the verifier loses `l4_len`'s scalar range
    // after the loop body's state merge. Re-cap `num_words` with
    // a redundant bound so the verifier re-establishes the range
    // for the pointer arithmetic in `pkt_read_u8`.
    if l4_len & 1 != 0 && num_words < MAX_L4_LEN / 2 {
        let b = unsafe { pkt_read_u8(ctx, l4_off + num_words * 2)? };
        sum += u32::from(b) << 8;
    }

    let folded = csum_fold(sum);
    Ok(!folded)
}

/// RFC 1624 incremental update — two (old, new) word pairs.
///   `new_csum = ~( ~old_csum + sum(~old_words) + sum(new_words) )`
/// All u16 inputs/outputs are big-endian (network order).
///
/// Inputs are individual words rather than slices so the verifier sees
/// a fully-unrolled body — slice iteration with a runtime length
/// explodes the verifier's path-walk budget.
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
