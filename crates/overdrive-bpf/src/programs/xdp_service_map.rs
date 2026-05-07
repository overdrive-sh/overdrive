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
    EbpfContext,
    bindings::{BPF_FIB_LKUP_RET_SUCCESS, bpf_fib_lookup as bpf_fib_lookup_params, xdp_action},
    cty::c_void,
    helpers::{bpf_fib_lookup, bpf_map_lookup_elem, bpf_redirect},
    macros::xdp,
    programs::XdpContext,
};

use crate::maps::backend_map::{BACKEND_MAP, BackendEntry};
use crate::maps::service_map::{INNER_TABLE_SIZE, SERVICE_MAP, ServiceKey};
use crate::programs::sanity::{Verdict as SanityVerdict, sanity_check};
use crate::shared::csum::recompute_l4_csum;

// Header offsets / constants.
const ETH_HDR_LEN: usize = 14;
const ETH_DST_OFFSET: usize = 0;
const ETH_SRC_OFFSET: usize = 6;
const ETH_TYPE_OFFSET: usize = 12;
const ETH_TYPE_IPV4: u16 = 0x0800;
const ETH_ALEN: usize = 6;

// AF_INET — kernel-stable address-family constant (sys/socket.h). Not
// re-exported by aya-ebpf-bindings; declared here as a local constant
// since it is part of the in-kernel `bpf_fib_lookup` UAPI contract and
// will not change.
const AF_INET: u8 = 2;

const IPV4_HDR_LEN: usize = 20;
const IPV4_TOS_OFFSET: usize = 1;
const IPV4_TOT_LEN_OFFSET: usize = 2;
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
    // (0) Sanity prologue per Slice 06 / ADR-0040 Q3=C — five
    //     Cloudflare-order checks, first failure short-circuits.
    //     The helper's `Verdict::Drop` arm has already incremented
    //     `DROP_COUNTER[MalformedHeader]`; we just translate the
    //     three-way decision into the XDP verdict.
    //
    //     Sanity bounds-check for the full IPv4 header (offsets
    //     0..IPV4_HDR_LEN-1 from `ETH_HDR_LEN`) is the IP version
    //     check's prerequisite; for the L4 flag-byte read we need
    //     at least the L4 flags byte (offset 13 from L4 start) to
    //     be in-range. We bounds-check the FULL fixed-min L4 header
    //     here (TCP_HDR_LEN; UDP is shorter so it'd be a tighter
    //     bound but the read of TCP_FLAGS_OFFSET=13 only fires for
    //     TCP frames — a UDP frame's flag-byte read is gated by the
    //     proto check inside `sanity_check`).
    let _ipv4_bounds_pre: *const u8 = unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN - 1)? };
    let _l4_bounds_pre: *const u8 =
        unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN + TCP_HDR_LEN - 1)? };
    let packet_len: usize = ctx.data_end().saturating_sub(ctx.data());
    let sanity = sanity_check(
        ETH_HDR_LEN,
        ETH_HDR_LEN + IPV4_HDR_LEN,
        packet_len,
        |off| unsafe { read_u8(ctx, off) },
        |off| unsafe { read_u16_be(ctx, off) },
    );
    match sanity {
        SanityVerdict::Continue => {}
        SanityVerdict::Drop => return Ok(xdp_action::XDP_DROP),
        SanityVerdict::PassToKernel => return Ok(xdp_action::XDP_PASS),
    }

    // (1) Read fields needed for the SERVICE_MAP forward path. The
    //     sanity prologue has already validated EtherType, IPv4
    //     header bounds, version+IHL, total_length, and proto.
    let proto = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_PROTO_OFFSET)? };
    let dst_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET)? };
    let ip_csum = unsafe { read_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET)? };

    let is_tcp = proto == IPV4_PROTO_TCP;
    let is_udp = proto == IPV4_PROTO_UDP;

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

    // Hit ⇒ rewrite + (FIB-resolved L2 MAC rewrite) + XDP_TX
    // (S-2.2-04 / S-2.2-17). The L2 MAC rewrite via `bpf_fib_lookup`
    // is required for `XDP_TX` delivery into a non-local backend
    // veth peer — without it, the receiving veth's `eth_type_trans`
    // sets `pkt_type = PACKET_OTHERHOST` and `ip_rcv` drops the
    // packet before it reaches the backend's listener. See
    // `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
    // (Option α — kernel-tree `samples/bpf/xdp_fwd_kern.c` shape).
    rewrite_and_tx(
        ctx,
        src_ip,
        dst_ip,
        ip_csum,
        l4_off,
        l4_csum_off,
        dst_port,
        l4_csum,
        proto,
        &backend,
        is_udp,
    )
}

/// Rewrite IPv4 dst IP + L4 dst port; incrementally update IP +
/// L4 checksums; resolve next-hop L2 MAC via `bpf_fib_lookup`;
/// rewrite `eth->h_dest` / `eth->h_source`; return `XDP_TX` on
/// success or `XDP_PASS` on FIB-lookup failure (so the kernel
/// stack can do ARP / handle the edge case gracefully).
///
/// The ordering is load-bearing per
/// `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
/// Finding 2.3: rewrite L3 + L4 + checksums first; THEN call
/// `bpf_fib_lookup` against the post-rewrite IPv4 header — the FIB
/// lookup must resolve the *backend's* next-hop, not the VIP's.
#[inline(always)]
fn rewrite_and_tx(
    ctx: &XdpContext,
    src_ip_host: u32,
    old_dst_ip: u32,
    old_ip_csum: u16,
    l4_off: usize,
    l4_csum_off: usize,
    _old_dst_port: u16,
    old_l4_csum: u16,
    proto: u8,
    backend: &BackendEntry,
    is_udp: bool,
) -> Result<u32, ()> {
    let new_dst_ip: u32 = backend.ipv4_host;
    let new_dst_port: u16 = backend.port_host;

    // Split 32-bit IPs into two 16-bit big-endian words for the
    // IPv4 header checksum incremental update.
    let old_ip_hi = (old_dst_ip >> 16) as u16;
    let old_ip_lo = (old_dst_ip & 0xffff) as u16;
    let new_ip_hi = (new_dst_ip >> 16) as u16;
    let new_ip_lo = (new_dst_ip & 0xffff) as u16;

    // IPv4 header checksum: only the dst-IP changed (2 words).
    // IP header checksums are always FULL on the wire — incremental
    // update is safe here.
    let new_ip_csum = csum_incremental_2_2(old_ip_csum, old_ip_lo, old_ip_hi, new_ip_lo, new_ip_hi);

    // Write IP header csum, new dst IP, and new dst port FIRST.
    // Then zero the L4 csum field and recompute from scratch.
    //
    // Full L4 checksum recomputation replaces RFC 1624 incremental
    // update because veth with TX-checksum-offload emits
    // CHECKSUM_PARTIAL — incremental update on partial input
    // produces garbage. Full recomputation via bpf_csum_diff is
    // correct for both PARTIAL and FULL input. See
    // docs/research/dataplane/xdp-checksum-partial-veth-research.md.
    unsafe {
        write_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET, new_ip_csum)?;
        write_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET, new_dst_ip)?;
        write_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET, new_dst_port)?;
        write_u16_be(ctx, l4_off + l4_csum_off, 0)?;
    }

    // RFC 768 (UDP): csum=0x0000 means "no checksum computed" —
    // if the original L4 csum was 0 (UDP with no checksum),
    // leave untouched.
    if is_udp && old_l4_csum == 0 {
        unsafe {
            write_u16_be(ctx, l4_off + l4_csum_off, 0)?;
        }
    } else {
        // Pass host-order IPs — `recompute_l4_csum` extracts
        // pseudo-header u16 words via `>> 16` / `& 0xffff`, which
        // produces the correct host-order-of-network-order encoding
        // matching `pkt_read_u16`. src_ip_host is already host-order;
        // new_dst_ip is host-order from BackendEntry.
        let new_l4_csum = recompute_l4_csum(ctx, src_ip_host, new_dst_ip, proto, l4_off)?;

        // RFC 768: UDP csum of 0 means "no checksum"; if our
        // computation yields 0, write 0xffff instead.
        let final_l4_csum = if is_udp && new_l4_csum == 0 { 0xffff } else { new_l4_csum };
        unsafe {
            write_u16_be(ctx, l4_off + l4_csum_off, final_l4_csum)?;
        }
    }

    // L2 MAC rewrite via `bpf_fib_lookup` — see
    // `docs/research/dataplane/cilium-bpf-fib-lookup-l2-mac-rewrite-comprehensive-research.md`
    // (Option α). Without this, `XDP_TX` into a non-local backend's
    // veth peer is dropped at `ip_rcv` due to `PACKET_OTHERHOST`
    // classification — the symptom that blocked Slice 05-04's
    // first GREEN attempt.
    fib_resolve_and_rewrite_mac(ctx, src_ip_host, new_dst_ip, proto, l4_off)
}

/// Build a `bpf_fib_lookup` parameter block from the post-rewrite
/// IPv4 + L4 header, call the helper, and on success rewrite the
/// Ethernet src/dst MAC. Returns `XDP_TX` on success and `XDP_PASS`
/// on any FIB-lookup non-success (most commonly
/// `BPF_FIB_LKUP_RET_NO_NEIGH` — the kernel stack will then resolve
/// ARP and the next packet hits the populated neighbour table). See
/// the kernel-tree reference `samples/bpf/xdp_fwd_kern.c` for the
/// canonical shape.
#[inline(always)]
fn fib_resolve_and_rewrite_mac(
    ctx: &XdpContext,
    src_ip_host: u32,
    new_dst_ip_host: u32,
    proto: u8,
    l4_off: usize,
) -> Result<u32, ()> {
    // Read the post-rewrite IPv4 tot_len and tos. tot_len is
    // network-order on the wire; the FIB UAPI declares tot_len as
    // `__u16` (host-order in the bpf_fib_lookup struct, despite
    // sitting alongside `__be*` neighbours — the kernel reads it
    // as a host-order length). `read_u16_be` returns host-order
    // already, which is what the helper wants.
    //
    // SAFETY: bounds-checked by `read_u16_be` / `read_u8`.
    let tot_len = unsafe { read_u16_be(ctx, ETH_HDR_LEN + IPV4_TOT_LEN_OFFSET)? };
    let tos = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_TOS_OFFSET)? };

    // Read the post-rewrite L4 dst_port. The FIB struct's `dport`
    // field is `__be16` — pass network-order bytes. `read_u16_be`
    // returns host-order; we re-byte-swap via `to_be()` (a no-op
    // re-roundtrip via raw bytes).
    //
    // SAFETY: bounds-checked.
    let dst_port_host = unsafe { read_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET)? };
    let src_port_host = unsafe { read_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET)? };

    // Read the xdp_md ingress_ifindex. This is the iface the SYN
    // arrived on; for `XDP_TX` (same-iface bounce) it is also the
    // egress iface — but a real production network may route the
    // backend out a *different* iface, in which case `bpf_redirect`
    // is the correct return. Phase 2.2 sticks with `XDP_TX` because
    // the production deployment shape is single-iface; the FIB
    // lookup remains correct because it resolves the next-hop MAC
    // regardless of which iface ends up being the egress.
    //
    // SAFETY: `xdp_md` is a kernel-managed pointer; `ingress_ifindex`
    // is a plain `__u32` field at a stable offset.
    let ingress_ifindex = unsafe { (*ctx.ctx).ingress_ifindex };

    // Build the parameter block. Every field is initialised to 0
    // first (zeroed struct), then the inputs are written. The FIB
    // helper ignores fields it does not consume on input and
    // overwrites the smac/dmac/ifindex on output.
    //
    // SAFETY: zeroed struct is a valid initial state for
    // `bpf_fib_lookup` per the kernel UAPI — every union variant
    // accepts zero as a placeholder.
    let mut fib: bpf_fib_lookup_params = unsafe { core::mem::zeroed() };
    fib.family = AF_INET;
    fib.l4_protocol = proto;
    fib.sport = src_port_host.to_be();
    fib.dport = dst_port_host.to_be();
    fib.__bindgen_anon_1.tot_len = tot_len;
    fib.ifindex = ingress_ifindex;
    fib.__bindgen_anon_2.tos = tos;
    // ipv4_src / ipv4_dst are `__be32` per UAPI. The kernel-side host
    // representation of the post-rewrite dst is `new_dst_ip_host`
    // (e.g. `0x0a010005` for 10.1.0.5 per architecture.md § 11);
    // `to_be()` flips it to network-order for the helper.
    fib.__bindgen_anon_3.ipv4_src = src_ip_host.to_be();
    fib.__bindgen_anon_4.ipv4_dst = new_dst_ip_host.to_be();

    // SAFETY: `bpf_fib_lookup` helper takes the XDP ctx pointer +
    // `bpf_fib_lookup` parameter block. The verifier validates the
    // call. Returns 0 on success, > 0 for `BPF_FIB_LKUP_RET_*`
    // non-success codes, < 0 for invalid input. The helper
    // overwrites `fib.smac`/`fib.dmac` (and `fib.ifindex`) on
    // success.
    let rc = unsafe {
        bpf_fib_lookup(
            ctx.as_ptr(),
            &mut fib as *mut _,
            core::mem::size_of::<bpf_fib_lookup_params>() as i32,
            0u32,
        )
    };

    if rc as u32 != BPF_FIB_LKUP_RET_SUCCESS {
        // Non-success: `RET_NO_NEIGH` (ARP not yet resolved — let
        // the kernel do ARP), `RET_NOT_FWDED` (no route — let the
        // kernel handle it), `RET_FRAG_NEEDED`, etc. The L3+L4
        // rewrite already happened to the packet buffer — `XDP_PASS`
        // hands the now-rewritten packet to the kernel stack, which
        // will route it normally and ARP for the new dst IP. This
        // is the canonical pattern from `xdp_fwd_kern.c`.
        return Ok(xdp_action::XDP_PASS);
    }

    // Success: write the resolved smac/dmac into the eth header.
    //
    // SAFETY: `mut_ptr_at` performs the bounds check; the
    // ETH_DST/ETH_SRC offsets are within the validated eth header
    // (offset 0..14 was already bounds-checked at program entry via
    // `read_u16_be(ETH_TYPE_OFFSET)`).
    unsafe {
        let dst_mac: *mut [u8; ETH_ALEN] = mut_ptr_at(ctx, ETH_DST_OFFSET)?;
        let src_mac: *mut [u8; ETH_ALEN] = mut_ptr_at(ctx, ETH_SRC_OFFSET)?;
        *dst_mac = fib.dmac;
        *src_mac = fib.smac;
    }

    // Egress decision: when the FIB-resolved egress iface matches
    // the ingress iface, `XDP_TX` is the optimal same-iface bounce.
    // When the egress iface differs (e.g. a 3-iface transit topology
    // where `lb_veth_a` is ingress and `lb_veth_b` egresses to the
    // backend), use `bpf_redirect` to route to the resolved iface.
    // This matches `samples/bpf/xdp_fwd_kern.c`'s shape, which always
    // uses redirect (XDP_TX is the degenerate same-iface case).
    if fib.ifindex == ingress_ifindex {
        Ok(xdp_action::XDP_TX)
    } else {
        // SAFETY: `bpf_redirect` is the standard XDP redirect helper.
        // Returns the action code the program should return; on
        // success this is `XDP_REDIRECT` (= 4). The verifier accepts
        // this pattern as the canonical XDP-redirect shape.
        let action = unsafe { bpf_redirect(fib.ifindex, 0) };
        Ok(action as u32)
    }
}
