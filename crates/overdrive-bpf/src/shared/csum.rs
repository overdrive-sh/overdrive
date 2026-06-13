//! Shared L4 checksum helpers for kernel-side XDP programs.
//!
//! See `docs/research/dataplane/xdp-checksum-partial-veth-research.md`

use aya_ebpf::helpers::bpf_csum_diff;
use aya_ebpf::programs::XdpContext;

const MAX_L4_LEN: usize = 1500;

/// Number of 64-byte chunks needed to cover `MAX_L4_LEN` (1500).
/// `24 * 64 = 1536 ≥ 1500` — the loop bound is sized so a valid
/// in-range L4 segment is never truncated by the `break` guard;
/// the guard is a defensive invariant against future grammar drift,
/// not a routine path (per `.claude/rules/development.md`
/// § "One shared length ceiling for label-shaped ids").
const MAX_64_CHUNKS: usize = 24;

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

/// Accumulate a `N`-byte packet chunk at `off` into `seed` via
/// `bpf_csum_diff`. `N` is a const generic — every call site below
/// passes a literal (64/32/16/8/4), so the `to_size` argument the
/// helper sees is a compile-time constant. That constancy is what
/// lets the verifier track the packet pointer through the helper
/// call (the variable-length form is rejected; aya-rs/aya#1562).
///
/// `N` MUST be a multiple of 4 — `bpf_csum_diff` operates on a
/// `__be32` (u32) array. The bounds check (`s + off + N > e`) is
/// the `ptr_at` discipline applied per chunk: it re-reads
/// `data`/`data_end` volatilely so the verifier sees a fresh
/// bounded pointer for the helper's `to`/`to_size` pair.
#[inline(always)]
fn csum_diff_chunk<const N: u32>(ctx: &XdpContext, off: usize, seed: u32) -> Result<u32, ()> {
    let s = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data) } as usize;
    let e = unsafe { core::ptr::read_volatile(&raw const (*ctx.ctx).data_end) } as usize;
    if s + off + (N as usize) > e {
        return Err(());
    }
    let ptr = (s + off) as *mut u32;
    // SAFETY: `ptr` points at `N` in-bounds packet bytes (checked
    // above); `from = null, from_size = 0` accumulates `ptr[..N]`
    // into `seed`. The helper returns a `__s64` whose low 32 bits
    // hold the running one's-complement partial sum (`csum_partial`
    // never produces a negative value for a non-empty buffer).
    let diff = unsafe { bpf_csum_diff(core::ptr::null_mut(), 0, ptr, N, seed) };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // intentional: low-32-bit partial-sum accumulator chained as the next seed
    let folded = diff as u32;
    Ok(folded)
}

/// Full L4 checksum recomputation. Sums the L4 segment in
/// fixed-size power-of-two chunks (64/32/16/8/4 bytes) through
/// `bpf_csum_diff`, each call with a **compile-time-constant**
/// `to_size`. Constant `to_size` is what the verifier needs to
/// track the packet pointer — a *variable*-length `to_size` is
/// verifier-rejected (operand ordering / `pkt_ptr` tracking;
/// aya-rs/aya#1562), which is why the prior implementation walked
/// the segment word-by-word in a 750-iteration bounded loop that
/// the verifier unrolled to ~150K verified instructions. The
/// chunked engine (technique from aya#1562, comment by `CaioBrz1`)
/// collapses that to the low thousands while computing the
/// byte-identical checksum: the pseudo-header partial sum seeds the
/// first `bpf_csum_diff`, the seed chains across chunks, and the
/// final fold + invert are unchanged.
///
/// `bpf_csum_diff`'s `to_size` must be a multiple of 4, so the
/// chunk tree reaches every multiple of 4; a trailing 1–3-byte
/// residue is summed by hand (one 16-bit word + one odd byte, per
/// RFC 1071) exactly as the prior odd-byte path did.
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

    // Pseudo-header checksum, seeded through `bpf_csum_diff` itself
    // (Katran pattern — Finding 5 of
    // `docs/research/dataplane/xdp-checksum-partial-veth-research.md`).
    // Feeding the pseudo-header to the SAME helper that sums the L4
    // segment keeps the whole computation in one `csum_partial`
    // byte-order domain — building the seed in host arithmetic and
    // chaining it into `bpf_csum_diff` would byte-swap it on
    // little-endian and produce a wrong checksum.
    //
    // Wire layout, 12 bytes (a multiple of 4 — required by
    // `bpf_csum_diff`): src_ip(4) | dst_ip(4) | 0x00 | proto |
    // l4_len(2), all network-order. `src_ip` / `dst_ip` / `l4_len`
    // arrive host-order, so `to_be_bytes` yields the wire bytes.
    #[allow(clippy::cast_possible_truncation)] // intentional: BPF packet length fits u16
    let l4_len_u16 = l4_len as u16;
    let src_be = src_ip.to_be_bytes();
    let dst_be = dst_ip.to_be_bytes();
    let len_be = l4_len_u16.to_be_bytes();
    let pseudo: [u8; 12] = [
        src_be[0], src_be[1], src_be[2], src_be[3], //
        dst_be[0], dst_be[1], dst_be[2], dst_be[3], //
        0x00, proto, //
        len_be[0], len_be[1],
    ];
    // `bpf_csum_diff` wants `*mut __be32` (= `*mut u32`). The kernel
    // helper reads the buffer BYTEWISE (`csum_partial`), so the
    // pointer is never dereferenced as a `u32` and its alignment is
    // immaterial — hence the scoped `cast_ptr_alignment` allow.
    // `.cast().cast_mut()` keeps `ptr_as_ptr` / `as_ptr_cast_mut`
    // happy.
    #[allow(clippy::cast_ptr_alignment)] // helper reads bytewise; alignment immaterial
    let pseudo_ptr = pseudo.as_ptr().cast::<u32>().cast_mut();
    // SAFETY: `pseudo` is a 12-byte stack buffer fully initialised
    // above; `from = null, from_size = 0` accumulates `pseudo[..12]`
    // into seed 0. Stack pointer + constant length is trivially
    // verifier-accepted.
    let pseudo_sum = unsafe { bpf_csum_diff(core::ptr::null_mut(), 0, pseudo_ptr, 12, 0) };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // intentional: low-32-bit partial-sum accumulator (csum_partial is non-negative)
    let mut sum: u32 = pseudo_sum as u32;

    // Sum the L4 segment in fixed-size power-of-two chunks via
    // `bpf_csum_diff`. `sum` (the pseudo-header partial) seeds the
    // accumulator; each chunk's return chains into the next chunk's
    // seed. `bpf_csum_diff(NULL, 0, ptr, N, seed)` computes the
    // one's-complement 16-bit-word sum of `ptr[..N]` accumulated
    // into `seed`, all in the same byte-order domain as the
    // pseudo-header seed above, so the folded result is correct.
    //
    // Every `to_size` below (64/32/16/8/4) is a compile-time
    // constant — the load-bearing property that lets the verifier
    // track the packet pointer (a *variable* `to_size` is rejected;
    // aya#1562). `csum_diff_chunk` bounds-checks `[off, off+N)`
    // against `data_end` before handing the pointer to the helper.
    let mut off = l4_off;
    let mut remaining = l4_len;

    // 64-byte main loop. `MAX_64_CHUNKS` (24) covers MAX_L4_LEN
    // (24*64 = 1536 ≥ 1500); the `break` is a defensive invariant,
    // never reached for an in-range segment.
    let mut i: usize = 0;
    while i < MAX_64_CHUNKS {
        if remaining < 64 {
            break;
        }
        sum = csum_diff_chunk::<64>(ctx, off, sum)?;
        off += 64;
        remaining -= 64;
        i += 1;
    }
    // Descending power-of-two tail tree — reaches every multiple of
    // 4 from 60 down to 4, leaving a 0–3-byte residue.
    if remaining >= 32 {
        sum = csum_diff_chunk::<32>(ctx, off, sum)?;
        off += 32;
        remaining -= 32;
    }
    if remaining >= 16 {
        sum = csum_diff_chunk::<16>(ctx, off, sum)?;
        off += 16;
        remaining -= 16;
    }
    if remaining >= 8 {
        sum = csum_diff_chunk::<8>(ctx, off, sum)?;
        off += 8;
        remaining -= 8;
    }
    if remaining >= 4 {
        sum = csum_diff_chunk::<4>(ctx, off, sum)?;
        off += 4;
        remaining -= 4;
    }

    // Trailing 1–3-byte residue (`bpf_csum_diff` cannot take a
    // non-multiple-of-4 `to_size`). Copy the residue into a
    // zero-padded 4-byte stack buffer — in wire order — and sum it
    // through `bpf_csum_diff` so the tail stays in the SAME
    // byte-order domain as the chunks and the pseudo-header seed.
    // RFC 1071 left-justifies the final partial word: residue bytes
    // occupy the leading positions, trailing bytes are zero — so a
    // lone odd byte `b` becomes the word `(b, 0x00)` = `b << 8`,
    // matching the prior hand-rolled odd-byte path. Byte reads are
    // per-byte bounds-checked via `pkt_read_u8`.
    if remaining >= 1 {
        // `remaining` is 1..=3 here (the chunk tree consumed every
        // multiple of 4). Copy each residue byte with its own bounds
        // check, so the verifier sees fresh in-bounds accesses rather
        // than a variable-length packet read.
        let mut tail: [u8; 4] = [0; 4];
        tail[0] = unsafe { pkt_read_u8(ctx, off)? };
        if remaining >= 2 {
            tail[1] = unsafe { pkt_read_u8(ctx, off + 1)? };
        }
        if remaining >= 3 {
            tail[2] = unsafe { pkt_read_u8(ctx, off + 2)? };
        }
        // Bytewise-read by the kernel helper — alignment immaterial
        // (see the pseudo-header call site for the full rationale).
        #[allow(clippy::cast_ptr_alignment)] // helper reads bytewise; alignment immaterial
        let tail_ptr = tail.as_ptr().cast::<u32>().cast_mut();
        // SAFETY: `tail` is a fully-initialised 4-byte stack buffer;
        // `from = null, from_size = 0` accumulates `tail[..4]` into
        // `sum`. Stack pointer + constant length 4 is
        // verifier-accepted.
        let with_tail = unsafe { bpf_csum_diff(core::ptr::null_mut(), 0, tail_ptr, 4, sum) };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // intentional: low-32-bit partial-sum accumulator
        let folded = with_tail as u32;
        sum = folded;
    }

    // `sum` is in `bpf_csum_diff` / `csum_partial`'s byte-order domain
    // (the kernel sums native-endian 16-bit words, so `csum_fold`
    // yields the checksum in *network* byte order). The prior
    // word-by-word path summed `from_be_bytes` words — its fold was in
    // the host-number-of-big-endian-word domain — and the two callers
    // write the returned value with `write_u16_be` (a host→wire swap).
    // To preserve that caller contract byte-for-byte (return the SAME
    // u16 the old path did, so `write_u16_be` still lands the correct
    // wire bytes) swap the folded result out of the `csum_partial`
    // domain. The two domains differ by exactly `swap_bytes` — a wrong
    // (byte-swapped) L4 checksum silently drops every packet, caught by
    // the Tier-3 `real_tcp_connection_*` e2e gate.
    let folded = csum_fold(sum);
    Ok((!folded).swap_bytes())
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
