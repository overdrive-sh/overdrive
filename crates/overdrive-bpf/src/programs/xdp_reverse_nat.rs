//! `xdp_reverse_nat_lookup` — kernel-side XDP program for the
//! Phase 2.2 `REVERSE_NAT` response path (US-05; S-2.2-32 / ADR-0045
//! § Decision § 2 — replaces `tc_reverse_nat` at TCX-egress).
//!
//! Per ADR-0045 § 2, the response path moves from TCX-egress on the
//! client-facing veth to XDP-ingress on the backend-facing veth.
//! When a backend's response packet enters the lb-ns through
//! `lb_veth_b`, this program runs as the first kernel-level hook on
//! that ingress and:
//!
//! 1. Sanity prologue (Slice 06-02 shared helper; XDP-ingress scope
//!    per ADR-0040 Q3 amendment + ADR-0045 § 4).
//! 2. `REVERSE_NAT_MAP` lookup keyed on
//!    `(backend_ip, backend_port, proto)` — the response's source
//!    3-tuple. Miss ⇒ `XDP_PASS` (the kernel networking stack
//!    handles non-LB traffic; no `DROP_COUNTER` slot).
//! 3. L3 rewrite — source `(backend_ip, backend_port)` →
//!    `(VIP, vip_port)`; incremental IPv4 + L4 checksum update
//!    via the same RFC 1624 fold the forward path uses. Note: the
//!    XDP attach point has no `__sk_buff`, so the TC-only
//!    `bpf_l3_csum_replace` / `bpf_l4_csum_replace` helpers are not
//!    available — the inline RFC 1624 fold is the canonical XDP
//!    pattern (research § 4.1 / § 4.2; identical reasoning as
//!    `xdp_service_map.rs`).
//! 4. `bpf_fib_lookup` against the post-rewrite `(src_ip, dst_ip)` —
//!    src=VIP (just rewritten), `dst=client_ip`. Resolves the egress
//!    iface index + next-hop MAC for the response's path back to
//!    the client.
//! 5. L2 rewrite — `eth_hdr->h_dest` ← FIB-resolved `dmac`,
//!    `eth_hdr->h_source` ← FIB-resolved `smac`. ADR-0045 §
//!    Decision § 2 step 5 mandates this happen in-program before
//!    the redirect.
//! 6. `bpf_redirect(fib.ifindex, 0)` — the kernel verifier rejects
//!    `bpf_redirect_neigh` on XDP (TC-only helper; ADR-0045
//!    amendment 2026-05-07). Since L2 MACs are written by step 5,
//!    `bpf_redirect` is functionally equivalent.
//!
//! On `bpf_fib_lookup` non-success (`RET_NO_NEIGH`, `RET_NOT_FWDED`,
//! `RET_BLACKHOLE`, …) the program returns `XDP_PASS` after the L3+L4
//! rewrite has committed; the kernel networking stack handles the
//! packet through ARP / its own routing table. No `DROP_COUNTER` slot
//! is consumed (ADR-0040 Q7 preserved).
//!
//! # Why a separate program (not folded into `xdp_service_map`)
//!
//! Per ADR-0045 § Decision § 3: the forward path attaches to the
//! client-facing veth ingress; the reverse path attaches to the
//! backend-facing veth ingress. They never see the same packets;
//! per-direction logic IS per-iface logic. Splitting matches
//! Cilium's `bpf_lxc.c` / `bpf_overlay.c` shape (research § Q4,
//! Finding 4.1) and keeps each program's verifier-budget delta well
//! below the ≤ 60% absolute ceiling.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Wire bytes are network-order (big-endian). The packet bytes we
//! read via `read_u32_be` give us a `u32` whose host-order numeric
//! mirrors the wire bytes — i.e. the IP address `10.1.0.5` on the
//! wire as `[10, 1, 0, 5]` reads back as `0x0a010005` in host
//! order, which is `u32::from(Ipv4Addr::new(10, 1, 0, 5))`. This is
//! the userspace handle's host-order convention; `REVERSE_NAT_MAP` is
//! keyed identically. The Slice 05-03 lockstep proptest still
//! applies — the read-side conversion is preserved verbatim from
//! `tc_reverse_nat.rs`.

#![allow(dead_code)]

use aya_ebpf::{
    EbpfContext,
    bindings::{BPF_FIB_LKUP_RET_SUCCESS, bpf_fib_lookup as bpf_fib_lookup_params, xdp_action},
    helpers::{bpf_fib_lookup, bpf_redirect},
    macros::xdp,
    programs::XdpContext,
};

use crate::maps::reverse_nat_map::{BackendKey, REVERSE_NAT_MAP, Vip};
use crate::programs::sanity::{Verdict as SanityVerdict, sanity_check};
use crate::shared::csum::recompute_l4_csum;

// Header offsets / constants — same shape as `xdp_service_map.rs`.
const ETH_HDR_LEN: usize = 14;
const ETH_DST_OFFSET: usize = 0;
const ETH_SRC_OFFSET: usize = 6;
const ETH_ALEN: usize = 6;

// AF_INET — kernel-stable address-family constant (sys/socket.h).
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
    // SAFETY: bounds-checked by `ptr_at`.
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
// Mirrors `xdp_service_map.rs` — same verifier discipline (three
// unrolled folds is the verifier-accepted minimum).

#[inline(always)]
#[allow(clippy::cast_possible_truncation)] // Intentional: three folds guarantee s fits in u16.
fn fold32(s: u32) -> u16 {
    let s = (s & 0xffff) + (s >> 16);
    let s = (s & 0xffff) + (s >> 16);
    let s = (s & 0xffff) + (s >> 16);
    s as u16
}

/// RFC 1624 incremental update — two (old, new) word pairs. Used
/// for the IPv4 header checksum where only the source IP (2 words)
/// changes.
#[inline(always)]
fn csum_incremental_2_2(old_csum: u16, old_lo: u16, old_hi: u16, new_lo: u16, new_hi: u16) -> u16 {
    let s: u32 = u32::from(!old_csum)
        + u32::from(!old_lo)
        + u32::from(!old_hi)
        + u32::from(new_lo)
        + u32::from(new_hi);
    !fold32(s)
}

// ---------- main program ----------

#[xdp]
pub fn xdp_reverse_nat_lookup(ctx: XdpContext) -> u32 {
    match try_xdp_reverse_nat_lookup(&ctx) {
        Ok(action) => action,
        Err(()) => xdp_action::XDP_PASS,
    }
}

#[inline(always)]
fn try_xdp_reverse_nat_lookup(ctx: &XdpContext) -> Result<u32, ()> {
    // (0) Sanity prologue — XDP-ingress scope per ADR-0040 Q3
    // amendment + ADR-0045 § 4. Pre-bounds-check the IPv4 header
    // and the minimum L4 header (UDP_HDR_LEN = 8, the smaller of
    // TCP/UDP) before invoking the helper. `sanity_check`'s TCP
    // flags read at l4_offset+13 is self-guarded via its own
    // `read_u8` → `ptr_at` bounds check; the per-protocol L4
    // bounds check after `sanity_check` returns validates the full
    // TCP header when needed.
    let _ipv4_bounds_pre: *const u8 = unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN - 1)? };
    let _l4_bounds_pre: *const u8 =
        unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN + UDP_HDR_LEN - 1)? };
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

    // (1) Read fields needed for the REVERSE_NAT_MAP lookup. The
    // sanity prologue has already validated EtherType, IPv4 header
    // bounds, version+IHL, total_length, and proto.
    let proto = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_PROTO_OFFSET)? };
    let src_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_SRC_IP_OFFSET)? };
    let ip_csum = unsafe { read_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET)? };

    let is_tcp = proto == IPV4_PROTO_TCP;
    let is_udp = proto == IPV4_PROTO_UDP;

    // (2) Bounds-check L4 header + read source port + L4 checksum.
    let l4_off = ETH_HDR_LEN + IPV4_HDR_LEN;
    let (l4_csum_off, l4_hdr_len) =
        if is_tcp { (TCP_CSUM_OFFSET, TCP_HDR_LEN) } else { (UDP_CSUM_OFFSET, UDP_HDR_LEN) };
    let _l4_bounds: *const u8 = unsafe { ptr_at(ctx, l4_off + l4_hdr_len - 1)? };
    let src_port = unsafe { read_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET)? };
    let l4_csum = unsafe { read_u16_be(ctx, l4_off + l4_csum_off)? };

    // (3) Build REVERSE_NAT_MAP key in host order. The userspace
    // handle stores `ip_host = u32::from(Ipv4Addr::new(a,b,c,d))`,
    // which is the same numeric value as
    // `u32::from_be_bytes([a,b,c,d])` — see architecture.md § 11.
    // The key represents the response's source 3-tuple
    // (backend → VIP map direction).
    let key = BackendKey { ip_host: src_ip, port_host: src_port, proto, _pad: 0 };

    // (4) REVERSE_NAT_MAP lookup. Miss ⇒ `XDP_PASS` (not LB
    // traffic; the kernel networking stack handles it).
    //
    // SAFETY: `REVERSE_NAT_MAP.get` is `unsafe` per aya-ebpf API;
    // the returned pointer is verifier-checked, NULL-check via
    // Option.
    let vip = match unsafe { REVERSE_NAT_MAP.get(&key) } {
        Some(v) => *v,
        None => return Ok(xdp_action::XDP_PASS),
    };

    // (5) Hit ⇒ rewrite source IP/port, incremental checksum
    // update, FIB lookup, L2 rewrite, redirect.
    rewrite_and_redirect(
        ctx,
        src_ip,
        ip_csum,
        l4_off,
        l4_csum_off,
        src_port,
        l4_csum,
        proto,
        &vip,
        is_udp,
    )
}

/// Rewrite IPv4 src IP + L4 src port; incrementally update IP +
/// L4 checksums; resolve next-hop L2 MAC via `bpf_fib_lookup`;
/// rewrite `eth->h_dest` / `eth->h_source`; return whatever
/// `bpf_redirect` returns on success or `XDP_PASS` on FIB-lookup
/// failure.
///
/// Mirrors `xdp_service_map.rs::rewrite_and_tx` /
/// `fib_resolve_and_rewrite_mac` ordering — L3 + L4 rewrite +
/// checksums first; THEN call `bpf_fib_lookup` against the
/// post-rewrite IPv4 header — the FIB lookup must resolve the
/// VIP-side (post-rewrite source) next-hop, NOT the backend's.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn rewrite_and_redirect(
    ctx: &XdpContext,
    old_src_ip: u32,
    old_ip_csum: u16,
    l4_off: usize,
    l4_csum_off: usize,
    _old_src_port: u16,
    old_l4_csum: u16,
    proto: u8,
    vip: &Vip,
    is_udp: bool,
) -> Result<u32, ()> {
    let new_src_ip: u32 = vip.ip_host;
    let new_src_port: u16 = vip.port_host;

    // Split 32-bit IPs into two 16-bit big-endian words for the
    // IPv4 header checksum incremental update.
    let old_ip_hi = (old_src_ip >> 16) as u16;
    let old_ip_lo = (old_src_ip & 0xffff) as u16;
    let new_ip_hi = (new_src_ip >> 16) as u16;
    let new_ip_lo = (new_src_ip & 0xffff) as u16;

    // IPv4 header checksum: only the src-IP changed (2 words).
    // IP header checksums are always FULL on the wire — incremental
    // update is safe here.
    let new_ip_csum = csum_incremental_2_2(old_ip_csum, old_ip_lo, old_ip_hi, new_ip_lo, new_ip_hi);

    // Read dst IP (unchanged by reverse-NAT) for the pseudo-header.
    let dst_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET)? };

    // Write IP header csum, new src IP, and new src port FIRST.
    // Then zero the L4 csum field and recompute from scratch.
    //
    // Full L4 checksum recomputation replaces RFC 1624 incremental
    // update — same rationale as `xdp_service_map.rs::rewrite_and_tx`.
    unsafe {
        write_u16_be(ctx, ETH_HDR_LEN + IPV4_CSUM_OFFSET, new_ip_csum)?;
        write_u32_be(ctx, ETH_HDR_LEN + IPV4_SRC_IP_OFFSET, new_src_ip)?;
        write_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET, new_src_port)?;
        write_u16_be(ctx, l4_off + l4_csum_off, 0)?;
    }

    // RFC 768 (UDP): csum=0x0000 means "no checksum computed" —
    // if the original L4 csum was 0 (UDP with no checksum),
    // the zero written above is already correct; skip recomputation.
    if !is_udp || old_l4_csum != 0 {
        // Pass host-order IPs — `recompute_l4_csum` extracts
        // pseudo-header u16 words via `>> 16` / `& 0xffff`.
        // new_src_ip is host-order from Vip; dst_ip is host-order
        // from `read_u32_be`.
        let new_l4_csum = recompute_l4_csum(ctx, new_src_ip, dst_ip, proto, l4_off)?;

        // RFC 768: UDP csum of 0 means "no checksum"; write 0xffff
        // instead.
        let final_l4_csum = if is_udp && new_l4_csum == 0 { 0xffff } else { new_l4_csum };
        unsafe {
            write_u16_be(ctx, l4_off + l4_csum_off, final_l4_csum)?;
        }
    }

    // L2 MAC rewrite via `bpf_fib_lookup` — see ADR-0045 §
    // Decision § 2 step 4–5 and the canonical
    // `samples/bpf/xdp_fwd_kern.c` shape. Reads the post-rewrite
    // src/dst IPs from the packet (already committed above) and
    // calls the helper against them.
    fib_resolve_and_rewrite_mac(ctx, new_src_ip, proto, l4_off)
}

/// Build a `bpf_fib_lookup` parameter block from the post-rewrite
/// IPv4 + L4 header, call the helper, and on success rewrite the
/// Ethernet src/dst MAC. Returns the action code from
/// `bpf_redirect` on success and `XDP_PASS` on any FIB-lookup
/// non-success. Mirrors `xdp_service_map.rs::fib_resolve_and_rewrite_mac`.
#[inline(always)]
fn fib_resolve_and_rewrite_mac(
    ctx: &XdpContext,
    new_src_ip_host: u32,
    proto: u8,
    l4_off: usize,
) -> Result<u32, ()> {
    // SAFETY: bounds-checked by `read_u16_be` / `read_u8`.
    let tot_len = unsafe { read_u16_be(ctx, ETH_HDR_LEN + IPV4_TOT_LEN_OFFSET)? };
    let tos = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_TOS_OFFSET)? };

    // Read the post-rewrite L4 src_port + dst_port. For the FIB
    // lookup the helper consumes them as `__be16` — we pass
    // network-order bytes via `to_be()`.
    //
    // SAFETY: bounds-checked.
    let src_port_host = unsafe { read_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET)? };
    let dst_port_host = unsafe { read_u16_be(ctx, l4_off + L4_DST_PORT_OFFSET)? };
    // Read the post-rewrite IPv4 dst (the response destination —
    // unchanged by reverse-NAT, but the FIB lookup needs it).
    let dst_ip_host = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_DST_IP_OFFSET)? };

    // SAFETY: `xdp_md` is a kernel-managed pointer; `ingress_ifindex`
    // is a plain `__u32` field at a stable offset.
    let ingress_ifindex = unsafe { (*ctx.ctx).ingress_ifindex };

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
    // ipv4_src / ipv4_dst are `__be32` per UAPI. The kernel-side
    // host representation of the post-rewrite src is
    // `new_src_ip_host`; `to_be()` flips it to network-order.
    fib.__bindgen_anon_3.ipv4_src = new_src_ip_host.to_be();
    fib.__bindgen_anon_4.ipv4_dst = dst_ip_host.to_be();

    // SAFETY: `bpf_fib_lookup` helper takes the XDP ctx pointer +
    // `bpf_fib_lookup` parameter block. The verifier validates
    // the call.
    // BPF kernel ABI: bpf_fib_lookup takes *mut params, i32 size, u32 flags;
    // returns i64. All casts below are intentional kernel-ABI conversions.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let param_size = core::mem::size_of::<bpf_fib_lookup_params>() as i32;
    let rc = unsafe { bpf_fib_lookup(ctx.as_ptr(), &raw mut fib, param_size, 0u32) };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if rc as u32 != BPF_FIB_LKUP_RET_SUCCESS {
        // Non-success: the L3+L4 rewrite already happened; hand
        // the now-rewritten packet to the kernel stack via
        // `XDP_PASS` so it can ARP / route it normally. ADR-0045
        // § 5 — canonical pattern, no DROP_COUNTER slot consumed.
        return Ok(xdp_action::XDP_PASS);
    }

    // Success: write the resolved smac/dmac into the eth header.
    //
    // SAFETY: `mut_ptr_at` performs the bounds check; the
    // ETH_DST/ETH_SRC offsets are within the validated eth header
    // (offset 0..14 was already bounds-checked at program entry
    // via the sanity prologue's read_u16_be(ETH_TYPE_OFFSET)).
    unsafe {
        let dst_mac: *mut [u8; ETH_ALEN] = mut_ptr_at(ctx, ETH_DST_OFFSET)?;
        let src_mac: *mut [u8; ETH_ALEN] = mut_ptr_at(ctx, ETH_SRC_OFFSET)?;
        *dst_mac = fib.dmac;
        *src_mac = fib.smac;
    }

    // Per ADR-0045 amendment 2026-05-07: `bpf_redirect_neigh` is
    // TC-only; the kernel verifier rejects on XDP. Since L2 MACs
    // have already been written above, `bpf_redirect` is
    // functionally equivalent — it just delivers the rewritten
    // frame to the resolved egress iface's tx queue.
    //
    // SAFETY: `bpf_redirect` is the standard XDP redirect helper.
    // Returns the action code the program should return; on
    // success this is `XDP_REDIRECT` (= 4).
    let action = unsafe { bpf_redirect(fib.ifindex, 0) };
    // BPF helper returns i64; XDP actions are u32 constants — intentional ABI cast.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(action as u32)
}
