//! `xdp_service_map_lookup` — kernel-side XDP program for Phase 2.2
//! SERVICE_MAP forward path (US-02; S-2.2-04 / S-2.2-05 / S-2.2-08
//! flip GREEN here).
//!
//! Lookup pipeline per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10:
//!
//! 1. Bounds-check + parse Eth header. Non-IPv4 EtherType ⇒
//!    `XDP_PASS`.
//! 2. Bounds-check + parse IPv4 header. Truncated frames ⇒
//!    `XDP_PASS` via the wrapper's `Err(_)` branch (S-2.2-08).
//! 3. Non-{TCP,UDP} ⇒ `XDP_PASS`.
//! 4. Bounds-check + parse L4 header (only dest port + csum
//!    matter for Slice 02).
//! 5. Build host-order `(VIP, port)` key, look up SERVICE_MAP.
//! 6. Miss ⇒ `XDP_PASS` (S-2.2-05). Hit ⇒ rewrite dest IP +
//!    dest port, incrementally update IPv4 + L4 checksums per
//!    RFC 1624, return `XDP_TX` (S-2.2-04).
//!
//! # Why incremental checksum fold (not `bpf_l*_csum_replace`)
//!
//! `bpf_l3_csum_replace` / `bpf_l4_csum_replace` require an
//! `*mut sk_buff` — they are TC-only. XDP has no skb; the
//! canonical XDP NAT pattern is the one's-complement incremental
//! fold (RFC 1624) applied directly to the checksum bytes in the
//! packet buffer. This is what Cilium's XDP load-balancer uses
//! and what the verifier accepts on every kernel in the matrix.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Wire bytes are network-order (big-endian). The packet bytes we
//! read via `read_u32_be` give us a `u32` whose host-order value
//! mirrors the wire bytes — i.e. the IP address `10.0.0.1` on the
//! wire as `[10, 0, 0, 1]` reads back as `0x0a000001` in host
//! order, which is `u32::from(Ipv4Addr::new(10, 0, 0, 1))`. This
//! is the userspace handle's host-order convention; SERVICE_MAP
//! is keyed identically. Writing `value.to_be_bytes()` puts the
//! same 4 bytes back on the wire. No `htonl` / `ntohl` needed.

#![allow(dead_code)]

use aya_ebpf::{
    bindings::xdp_action, cty::c_void, helpers::bpf_map_lookup_elem, macros::xdp,
    programs::XdpContext,
};

use crate::maps::backend_map::{BACKEND_MAP, BackendEntry};
use crate::maps::service_map::{INNER_TABLE_SIZE, SERVICE_MAP, ServiceKey};

// Header offsets / constants.
const ETH_HDR_LEN: usize = 14;
const ETH_TYPE_OFFSET: usize = 12;
const ETH_TYPE_IPV4: u16 = 0x0800;

const IPV4_HDR_LEN: usize = 20;
const IPV4_PROTO_OFFSET: usize = 9;
const IPV4_CSUM_OFFSET: usize = 10;
const IPV4_SRC_IP_OFFSET: usize = 12;
const IPV4_DST_IP_OFFSET: usize = 16;
const IPV4_PROTO_TCP: u8 = 6;
const IPV4_PROTO_UDP: u8 = 17;

const L4_SRC_PORT_OFFSET: usize = 0; // same for TCP and UDP
const L4_DST_PORT_OFFSET: usize = 2; // same for TCP and UDP
const TCP_CSUM_OFFSET: usize = 16;
const UDP_CSUM_OFFSET: usize = 6;
const TCP_HDR_LEN: usize = 20;
const UDP_HDR_LEN: usize = 8;

// ---------- bounds-checked pointer access ----------
//
// SAFETY notes apply to every helper here: each `unsafe` block
// inside the body relies on the bounds check immediately above
// it. A `start + offset + len > end` test that succeeds means
// `(start + offset)` references at least `len` bytes of valid
// packet data — exactly the verifier's required shape.

#[inline(always)]
unsafe fn ptr_at<T>(ctx: &XdpContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = core::mem::size_of::<T>();
    if start + offset + len > end {
        return Err(());
    }
    Ok((start + offset) as *const T)
}

#[inline(always)]
unsafe fn mut_ptr_at<T>(ctx: &XdpContext, offset: usize) -> Result<*mut T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = core::mem::size_of::<T>();
    if start + offset + len > end {
        return Err(());
    }
    Ok((start + offset) as *mut T)
}

#[inline(always)]
unsafe fn read_u8(ctx: &XdpContext, offset: usize) -> Result<u8, ()> {
    // SAFETY: `ptr_at` performs the verifier-required bounds
    // check; the dereference is therefore in-range.
    let p: *const u8 = unsafe { ptr_at(ctx, offset) }?;
    Ok(unsafe { *p })
}

#[inline(always)]
unsafe fn read_u16_be(ctx: &XdpContext, offset: usize) -> Result<u16, ()> {
    // SAFETY: bounds-checked by `ptr_at`.
    let p: *const [u8; 2] = unsafe { ptr_at(ctx, offset) }?;
    Ok(u16::from_be_bytes(unsafe { *p }))
}

#[inline(always)]
unsafe fn read_u32_be(ctx: &XdpContext, offset: usize) -> Result<u32, ()> {
    // SAFETY: bounds-checked by `ptr_at`.
    let p: *const [u8; 4] = unsafe { ptr_at(ctx, offset) }?;
    Ok(u32::from_be_bytes(unsafe { *p }))
}

#[inline(always)]
unsafe fn write_u16_be(ctx: &XdpContext, offset: usize, val: u16) -> Result<(), ()> {
    // SAFETY: bounds-checked by `mut_ptr_at`.
    let p: *mut [u8; 2] = unsafe { mut_ptr_at(ctx, offset) }?;
    unsafe { *p = val.to_be_bytes() };
    Ok(())
}

#[inline(always)]
unsafe fn write_u32_be(ctx: &XdpContext, offset: usize, val: u32) -> Result<(), ()> {
    // SAFETY: bounds-checked by `mut_ptr_at`.
    let p: *mut [u8; 4] = unsafe { mut_ptr_at(ctx, offset) }?;
    unsafe { *p = val.to_be_bytes() };
    Ok(())
}

// ---------- one's-complement incremental checksum fold ----------
//
// The verifier rejects unbounded loops. Our sum has at most ~16
// terms (each `u32::from(u16)` ≤ 0xffff), so the running u32 is
// bounded by 16 * 0xffff < 0x100000 — i.e. the high half is at
// most 16. Two carry-folds is therefore *always* sufficient: the
// first fold reduces (carry ≤ 16) + (low ≤ 0xffff) ≤ 0x10010,
// which fits in 17 bits; the second fold reduces ≤ 1 + 0xffff =
// 0x10000, which fits in 17 bits; one final fold trims it to 16.
// Three unrolled folds is the minimum bounded loop the verifier
// accepts and is what Cilium's XDP fast path uses for the same
// shape of sum.

#[inline(always)]
fn fold32(s: u32) -> u16 {
    let s = (s & 0xffff) + (s >> 16);
    let s = (s & 0xffff) + (s >> 16);
    let s = (s & 0xffff) + (s >> 16);
    s as u16
}

/// RFC 1624 incremental update:
///   new_csum = ~( ~old_csum + sum(~old_words) + sum(new_words) )
/// All u16 inputs/outputs are big-endian (network order).
///
/// Inputs use fixed-size arrays rather than slices so the verifier
/// sees a fully-unrolled loop body — slice iteration with a
/// runtime length explodes the verifier's path-walk budget.
#[inline(always)]
fn csum_incremental_2_2(old_csum: u16, old_lo: u16, old_hi: u16, new_lo: u16, new_hi: u16) -> u16 {
    let s: u32 = u32::from(!old_csum)
        + u32::from(!old_lo)
        + u32::from(!old_hi)
        + u32::from(new_lo)
        + u32::from(new_hi);
    !fold32(s)
}

#[inline(always)]
fn csum_incremental_3_3(
    old_csum: u16,
    old_a: u16,
    old_b: u16,
    old_c: u16,
    new_a: u16,
    new_b: u16,
    new_c: u16,
) -> u16 {
    let s: u32 = u32::from(!old_csum)
        + u32::from(!old_a)
        + u32::from(!old_b)
        + u32::from(!old_c)
        + u32::from(new_a)
        + u32::from(new_b)
        + u32::from(new_c);
    !fold32(s)
}

// ---------- FNV-1a 32-bit over the 5-tuple ----------
//
// FNV-1a is the canonical Maglev hash choice (Cilium / Katran use
// the same family). 32-bit width is sufficient: we reduce mod M ≤
// 131_071 (largest ALLOWED_PRIMES entry). The constants are the
// IETF FNV spec values.
//
// Why 32-bit, not 64-bit: the verifier's instruction-count budget
// is tighter for u64 multiplies on some kernels; 32-bit FNV-1a
// gives us identical dispersion at lower complexity. The userspace
// counterpart in `crate::maglev::permutation` uses 64-bit FNV-1a
// for table-generation seeding only — that path is not on the
// per-packet hot path, so the bigger hash is free there.

const FNV32_OFFSET: u32 = 0x811c_9dc5;
const FNV32_PRIME: u32 = 0x0100_0193;

#[inline(always)]
fn fnv1a_5tuple_slot(src_ip: u32, dst_ip: u32, src_port: u16, dst_port: u16, proto: u8) -> u32 {
    // Unrolled FNV-1a over the canonical 5-tuple byte order:
    //   src_ip[0..4], dst_ip[0..4], src_port[0..2], dst_port[0..2], proto[0]
    // All "host-order" — we feed the same in-register representation
    // userspace would compute. The XOR-multiply pair is the FNV-1a
    // step; doing it inline (rather than in a loop) keeps the
    // verifier's complexity walk bounded.
    let mut h: u32 = FNV32_OFFSET;
    let src_b = src_ip.to_ne_bytes();
    let dst_b = dst_ip.to_ne_bytes();
    let sp_b = src_port.to_ne_bytes();
    let dp_b = dst_port.to_ne_bytes();
    h = (h ^ u32::from(src_b[0])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(src_b[1])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(src_b[2])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(src_b[3])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dst_b[0])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dst_b[1])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dst_b[2])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dst_b[3])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(sp_b[0])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(sp_b[1])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dp_b[0])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(dp_b[1])).wrapping_mul(FNV32_PRIME);
    h = (h ^ u32::from(proto)).wrapping_mul(FNV32_PRIME);
    h % INNER_TABLE_SIZE
}

// ---------- main program ----------

#[xdp]
pub fn xdp_service_map_lookup(ctx: XdpContext) -> u32 {
    match try_xdp_service_map_lookup(&ctx) {
        Ok(action) => action,
        Err(()) => xdp_action::XDP_PASS,
    }
}

#[inline(always)]
fn try_xdp_service_map_lookup(ctx: &XdpContext) -> Result<u32, ()> {
    // (1) Bounds-check Eth header + read EtherType.
    let eth_type = unsafe { read_u16_be(ctx, ETH_TYPE_OFFSET)? };
    if eth_type != ETH_TYPE_IPV4 {
        return Ok(xdp_action::XDP_PASS);
    }

    // (2) Bounds-check full IPv4 header + read fields. A truncated
    // frame (S-2.2-08) fails here; `?` propagates Err(()) and the
    // wrapper returns XDP_PASS.
    let _ipv4_bounds: *const u8 = unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN - 1)? };
    let proto = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_PROTO_OFFSET)? };
    let dst_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET)? };
    let ip_csum = unsafe { read_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET)? };

    // (3) Filter to TCP / UDP.
    let is_tcp = proto == IPV4_PROTO_TCP;
    let is_udp = proto == IPV4_PROTO_UDP;
    if !is_tcp && !is_udp {
        return Ok(xdp_action::XDP_PASS);
    }

    // (4) Bounds-check L4 header + read source/dest ports + checksum.
    let l4_off = ETH_HDR_LEN + IPV4_HDR_LEN;
    let (l4_csum_off, l4_hdr_len) =
        if is_tcp { (TCP_CSUM_OFFSET, TCP_HDR_LEN) } else { (UDP_CSUM_OFFSET, UDP_HDR_LEN) };
    let _l4_bounds: *const u8 = unsafe { ptr_at(ctx, l4_off + l4_hdr_len - 1)? };
    let src_port = unsafe { read_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET)? };
    let dst_port = unsafe { read_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET)? };
    let l4_csum = unsafe { read_u16_be(ctx, l4_off + l4_csum_off)? };

    // (5) Build SERVICE_MAP key in host order. The userspace
    // handle stores `vip_host = u32::from(Ipv4Addr::new(a,b,c,d))`,
    // which is the same numeric value as `u32::from_be_bytes([a,b,c,d])`
    // — see architecture.md § 11.
    let key = ServiceKey { vip_host: dst_ip, port_host: dst_port, _pad: 0 };

    // (6) Two-step HoM lookup per kernel.org map_of_maps doc + research
    // § D.6:
    //   step a: outer SERVICE_MAP[key] → inner ARRAY pointer
    //   step b: inner ARRAY[slot] → BackendId (raw u32)
    //   step c: BACKEND_MAP[BackendId] → BackendEntry
    //
    // NULL-check between step a and step b is verifier-mandatory —
    // the outer-lookup return is type-tagged `inner_map`; the verifier
    // rejects unconditional dereference. The Option representation
    // makes the check load-bearing in the type system. Same shape for
    // step b → step c.
    let inner_ptr = match SERVICE_MAP.lookup_inner(&key) {
        Some(p) => p,
        None => return Ok(xdp_action::XDP_PASS),
    };

    // Slot index — Slice 04 Maglev-table indexing. Hash the 5-tuple
    // (src_ip, dst_ip, src_port, dst_port, proto) via FNV-1a 32-bit
    // and reduce modulo `INNER_TABLE_SIZE` (= MaglevTableSize::DEFAULT
    // = 16_381). The Maglev permutation populated from userspace into
    // the inner ARRAY guarantees ±5 % distribution evenness across
    // backends and ≤ 1 % flow-shift on backend-set churn (ASR-2.2-02).
    //
    // Source ip is read host-order (we already converted dst_ip
    // above); src_port / dst_port are host-order from `read_u16_be`.
    // FNV-1a treats them as raw bytes — the host-order representation
    // is canonical for our slot-keying purpose because BOTH endpoints
    // of a connection see the same 5-tuple bytes regardless of
    // endianness, AND the FNV-1a hash is deterministic over any
    // consistent byte stream.
    let src_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_SRC_IP_OFFSET)? };
    let slot: u32 = fnv1a_5tuple_slot(src_ip, dst_ip, src_port, dst_port, proto);

    // SAFETY: `inner_ptr` is verifier-tagged `inner_map` from the
    // outer lookup above; the chained lookup is the canonical
    // verifier-accepted shape. `slot` is bounded by
    // `INNER_TABLE_SIZE - 1` so the inner ARRAY (size
    // INNER_TABLE_SIZE) cannot reject as out-of-range.
    let bid_ptr =
        unsafe { bpf_map_lookup_elem(inner_ptr.as_ptr(), &slot as *const u32 as *const c_void) };
    if bid_ptr.is_null() {
        return Ok(xdp_action::XDP_PASS);
    }
    // SAFETY: `bid_ptr` non-null (NULL-checked above); the inner
    // ARRAY's value is `BackendId` = raw `u32`, value_size matches.
    let backend_id: u32 = unsafe { *(bid_ptr as *const u32) };

    // step c: resolve BackendId → BackendEntry via BACKEND_MAP.
    // SAFETY: `BACKEND_MAP.get` is unsafe per aya-ebpf API; the
    // returned pointer is verifier-checked, NULL-check via Option.
    let backend = match unsafe { BACKEND_MAP.get(&backend_id) } {
        Some(b) => *b,
        None => return Ok(xdp_action::XDP_PASS),
    };

    // Hit ⇒ rewrite + XDP_TX (S-2.2-04).
    rewrite_and_tx(ctx, dst_ip, ip_csum, l4_off, l4_csum_off, dst_port, l4_csum, &backend, is_udp)
}

/// Rewrite IPv4 dst IP + L4 dst port; incrementally update IP +
/// L4 checksums; return `XDP_TX`.
#[inline(always)]
fn rewrite_and_tx(
    ctx: &XdpContext,
    old_dst_ip: u32,
    old_ip_csum: u16,
    l4_off: usize,
    l4_csum_off: usize,
    old_dst_port: u16,
    old_l4_csum: u16,
    backend: &BackendEntry,
    is_udp: bool,
) -> Result<u32, ()> {
    let new_dst_ip: u32 = backend.ipv4_host;
    let new_dst_port: u16 = backend.port_host;

    // Split 32-bit IPs into two 16-bit big-endian words for the
    // RFC 1624 fold.
    let old_ip_hi = (old_dst_ip >> 16) as u16;
    let old_ip_lo = (old_dst_ip & 0xffff) as u16;
    let new_ip_hi = (new_dst_ip >> 16) as u16;
    let new_ip_lo = (new_dst_ip & 0xffff) as u16;

    // IPv4 header checksum: only the dst-IP changed (2 words).
    let new_ip_csum = csum_incremental_2_2(old_ip_csum, old_ip_lo, old_ip_hi, new_ip_lo, new_ip_hi);

    // L4 checksum covers the pseudo-header (which uses dst IP)
    // AND the L4 dst port — 3 words changed.
    let new_l4_csum = csum_incremental_3_3(
        old_l4_csum,
        old_ip_hi,
        old_ip_lo,
        old_dst_port,
        new_ip_hi,
        new_ip_lo,
        new_dst_port,
    );

    // RFC 768 (UDP): csum=0x0000 means "no checksum computed" —
    // leave untouched if it started as 0; if our fold produced 0,
    // transmit as 0xffff so the receiver doesn't interpret it as
    // "skip validation".
    let final_l4_csum = if is_udp && old_l4_csum == 0 {
        0
    } else if new_l4_csum == 0 && is_udp {
        0xffff
    } else {
        new_l4_csum
    };

    unsafe {
        write_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET, new_ip_csum)?;
        write_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET, new_dst_ip)?;
        write_u16_be(ctx, l4_off + l4_csum_off, final_l4_csum)?;
        write_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET, new_dst_port)?;
    }

    Ok(xdp_action::XDP_TX)
}
