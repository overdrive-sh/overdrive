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

use aya_ebpf::{bindings::xdp_action, macros::xdp, programs::XdpContext};

use crate::maps::service_map::{BackendEntry, SERVICE_MAP, ServiceKey};

// Header offsets / constants.
const ETH_HDR_LEN: usize = 14;
const ETH_TYPE_OFFSET: usize = 12;
const ETH_TYPE_IPV4: u16 = 0x0800;

const IPV4_HDR_LEN: usize = 20;
const IPV4_PROTO_OFFSET: usize = 9;
const IPV4_CSUM_OFFSET: usize = 10;
const IPV4_DST_IP_OFFSET: usize = 16;
const IPV4_PROTO_TCP: u8 = 6;
const IPV4_PROTO_UDP: u8 = 17;

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

    // (4) Bounds-check L4 header + read dest port + checksum.
    let l4_off = ETH_HDR_LEN + IPV4_HDR_LEN;
    let (l4_csum_off, l4_hdr_len) =
        if is_tcp { (TCP_CSUM_OFFSET, TCP_HDR_LEN) } else { (UDP_CSUM_OFFSET, UDP_HDR_LEN) };
    let _l4_bounds: *const u8 = unsafe { ptr_at(ctx, l4_off + l4_hdr_len - 1)? };
    let dst_port = unsafe { read_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET)? };
    let l4_csum = unsafe { read_u16_be(ctx, l4_off + l4_csum_off)? };

    // (5) Build SERVICE_MAP key in host order. The userspace
    // handle stores `vip_host = u32::from(Ipv4Addr::new(a,b,c,d))`,
    // which is the same numeric value as `u32::from_be_bytes([a,b,c,d])`
    // — see architecture.md § 11.
    let key = ServiceKey { vip_host: dst_ip, port_host: dst_port, _pad: 0 };

    // (6) Lookup. Miss ⇒ XDP_PASS (S-2.2-05).
    let backend = match unsafe { SERVICE_MAP.get(&key) } {
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
