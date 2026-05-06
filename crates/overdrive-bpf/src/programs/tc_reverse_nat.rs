//! `tc_reverse_nat` — kernel-side TC egress program for Phase 2.2
//! REVERSE_NAT path (US-05; ADR-0041 Q2=A locked TC egress over
//! XDP-egress).
//!
//! Lookup pipeline per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10:
//!
//! 1. Parse Eth + IPv4 + TCP/UDP headers (bounds-checked).
//! 2. Build `BackendKey { ip_host, port_host, proto, _pad: 0 }`
//!    host-order at the kernel boundary (architecture.md § 11
//!    endianness lockstep).
//! 3. REVERSE_NAT_MAP lookup → `Vip { ip_host, port_host }`
//!    (host-order).
//! 4. On miss → `TC_ACT_OK` (pass-through, not LB traffic).
//! 5. On hit: rewrite source IP / source port back to the VIP,
//!    recompute IP + L4 checksums via `bpf_l3_csum_replace` /
//!    `bpf_l4_csum_replace` (Q1 = A locked — TC-only kernel helpers
//!    that operate on `__sk_buff` directly).
//! 6. Return `TC_ACT_OK` so the kernel networking stack sees the
//!    rewritten packet on egress.
//!
//! # Why `bpf_l*_csum_replace` and not the RFC 1624 fold
//!
//! `xdp_service_map_lookup` uses an inline RFC 1624 incremental fold
//! because XDP has no `__sk_buff` and the helpers reject. TC operates
//! on `__sk_buff` and the helpers are the canonical Cilium / Katran
//! pattern (research § 4.1, § 4.2). Using the helpers here keeps
//! verifier-budget delta below the 20% gate (ASR-2.2-03).
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Wire bytes are network-order (big-endian). The packet bytes we
//! read via `read_u32_be` give us a `u32` whose host-order value
//! mirrors the wire bytes — i.e. the IP address `10.1.0.5` on the
//! wire as `[10, 1, 0, 5]` reads back as `0x0a010005` in host order,
//! which is `u32::from(Ipv4Addr::new(10, 1, 0, 5))`. This is the
//! userspace handle's host-order convention; REVERSE_NAT_MAP is
//! keyed identically.

#![allow(dead_code)]

use aya_ebpf::{bindings::BPF_F_PSEUDO_HDR, macros::classifier, programs::TcContext};

use crate::maps::reverse_nat_map::{BackendKey, REVERSE_NAT_MAP, Vip};

// TC verdict constants (kernel ABI; <linux/pkt_cls.h>).
const TC_ACT_OK: i32 = 0;

// Header offsets / constants — same shape as `xdp_service_map.rs`.
const ETH_HDR_LEN: usize = 14;
const ETH_TYPE_OFFSET: usize = 12;
const ETH_TYPE_IPV4: u16 = 0x0800;

const IPV4_HDR_LEN: usize = 20;
const IPV4_PROTO_OFFSET: usize = 9;
const IPV4_CSUM_OFFSET: usize = 10;
const IPV4_SRC_IP_OFFSET: usize = 12;
const IPV4_PROTO_TCP: u8 = 6;
const IPV4_PROTO_UDP: u8 = 17;

const L4_SRC_PORT_OFFSET: usize = 0; // same for TCP and UDP
const TCP_CSUM_OFFSET: usize = 16;
const UDP_CSUM_OFFSET: usize = 6;
const TCP_HDR_LEN: usize = 20;
const UDP_HDR_LEN: usize = 8;

// ---------- bounds-checked pointer access ----------
//
// Mirrors `xdp_service_map.rs`'s helpers but against `TcContext`.
// `TcContext::data()` / `data_end()` expose the same skb data range
// the verifier requires bounds checks against.

#[inline(always)]
unsafe fn ptr_at<T>(ctx: &TcContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = core::mem::size_of::<T>();
    if start + offset + len > end {
        return Err(());
    }
    Ok((start + offset) as *const T)
}

#[inline(always)]
unsafe fn read_u8(ctx: &TcContext, offset: usize) -> Result<u8, ()> {
    // SAFETY: `ptr_at` performs the verifier-required bounds check.
    let p: *const u8 = unsafe { ptr_at(ctx, offset) }?;
    Ok(unsafe { *p })
}

#[inline(always)]
unsafe fn read_u16_be(ctx: &TcContext, offset: usize) -> Result<u16, ()> {
    // SAFETY: bounds-checked by `ptr_at`.
    let p: *const [u8; 2] = unsafe { ptr_at(ctx, offset) }?;
    Ok(u16::from_be_bytes(unsafe { *p }))
}

#[inline(always)]
unsafe fn read_u32_be(ctx: &TcContext, offset: usize) -> Result<u32, ()> {
    // SAFETY: bounds-checked by `ptr_at`.
    let p: *const [u8; 4] = unsafe { ptr_at(ctx, offset) }?;
    Ok(u32::from_be_bytes(unsafe { *p }))
}

// ---------- main program ----------

#[classifier]
pub fn tc_reverse_nat(mut ctx: TcContext) -> i32 {
    match try_tc_reverse_nat(&mut ctx) {
        Ok(action) => action,
        Err(()) => TC_ACT_OK,
    }
}

#[inline(always)]
fn try_tc_reverse_nat(ctx: &mut TcContext) -> Result<i32, ()> {
    // (1) Bounds-check Eth header + read EtherType.
    let eth_type = unsafe { read_u16_be(ctx, ETH_TYPE_OFFSET)? };
    if eth_type != ETH_TYPE_IPV4 {
        return Ok(TC_ACT_OK);
    }

    // (2) Bounds-check full IPv4 header + read fields.
    let _ipv4_bounds: *const u8 = unsafe { ptr_at(ctx, ETH_HDR_LEN + IPV4_HDR_LEN - 1)? };
    let proto = unsafe { read_u8(ctx, ETH_HDR_LEN + IPV4_PROTO_OFFSET)? };
    let src_ip = unsafe { read_u32_be(ctx, ETH_HDR_LEN + IPV4_SRC_IP_OFFSET)? };

    // (3) Filter to TCP / UDP.
    let is_tcp = proto == IPV4_PROTO_TCP;
    let is_udp = proto == IPV4_PROTO_UDP;
    if !is_tcp && !is_udp {
        return Ok(TC_ACT_OK);
    }

    // (4) Bounds-check L4 header + read source port.
    let l4_off = ETH_HDR_LEN + IPV4_HDR_LEN;
    let (l4_csum_off, l4_hdr_len) =
        if is_tcp { (TCP_CSUM_OFFSET, TCP_HDR_LEN) } else { (UDP_CSUM_OFFSET, UDP_HDR_LEN) };
    let _l4_bounds: *const u8 = unsafe { ptr_at(ctx, l4_off + l4_hdr_len - 1)? };
    let src_port = unsafe { read_u16_be(ctx, l4_off + L4_SRC_PORT_OFFSET)? };

    // (5) Build REVERSE_NAT_MAP key in host order. Backend = source
    // of the egress response. The userspace handle stores the same
    // host-order shape — see architecture.md § 11.
    let key = BackendKey { ip_host: src_ip, port_host: src_port, proto, _pad: 0 };

    // (6) REVERSE_NAT_MAP lookup. Miss ⇒ TC_ACT_OK pass-through.
    // SAFETY: `REVERSE_NAT_MAP.get` is `unsafe` per aya-ebpf API; the
    // returned pointer is verifier-checked, NULL-check via Option.
    let vip = match unsafe { REVERSE_NAT_MAP.get(&key) } {
        Some(v) => *v,
        None => return Ok(TC_ACT_OK),
    };

    // (7) Hit ⇒ rewrite source IP + source port to the VIP and
    // recompute checksums via TC kernel helpers.
    rewrite_source_to_vip(ctx, src_ip, src_port, &vip, l4_off, l4_csum_off, is_udp)
}

/// Rewrite source IP + source port to `vip`; update IPv4 + L4
/// checksums via `bpf_l3_csum_replace` / `bpf_l4_csum_replace`.
///
/// All values passed to the kernel helpers are network-order. The
/// helpers read the existing checksum from the packet at `offset`,
/// fold in the difference, and write the result back.
#[inline(always)]
fn rewrite_source_to_vip(
    ctx: &mut TcContext,
    old_src_ip_host: u32,
    old_src_port_host: u16,
    vip: &Vip,
    l4_off: usize,
    l4_csum_off: usize,
    is_udp: bool,
) -> Result<i32, ()> {
    let new_src_ip_host: u32 = vip.ip_host;
    let new_src_port_host: u16 = vip.port_host;

    // Convert host-order map values to network-order for the wire +
    // kernel-helper inputs. `bpf_l*_csum_replace` operates on
    // big-endian bytes via the `from`/`to` arguments, matching the
    // Cilium/Katran pattern.
    let old_src_ip_be: u32 = old_src_ip_host.to_be();
    let new_src_ip_be: u32 = new_src_ip_host.to_be();
    let old_src_port_be: u16 = old_src_port_host.to_be();
    let new_src_port_be: u16 = new_src_port_host.to_be();

    // (a) IPv4 header checksum: only the source IP changed (4 bytes).
    //     `size = 4` → helper folds in 32-bit IP delta.
    ctx.l3_csum_replace(
        ETH_HDR_LEN + IPV4_CSUM_OFFSET,
        u64::from(old_src_ip_be),
        u64::from(new_src_ip_be),
        4,
    )
    .map_err(|_| ())?;

    // (b) L4 checksum: source IP is part of the pseudo-header, so the
    //     IP change affects the L4 checksum too. `flags = BPF_F_PSEUDO_HDR | 4`
    //     tells the helper this is a pseudo-header field of width 4.
    ctx.l4_csum_replace(
        l4_off + l4_csum_off,
        u64::from(old_src_ip_be),
        u64::from(new_src_ip_be),
        u64::from(BPF_F_PSEUDO_HDR) | 4,
    )
    .map_err(|_| ())?;

    // (c) L4 checksum: source port is in the L4 header itself
    //     (not the pseudo-header). `flags = 2` → 2-byte field.
    //
    //     RFC 768 (UDP): csum=0x0000 means "no checksum computed".
    //     `bpf_l4_csum_replace` preserves the 0 sentinel automatically
    //     when `BPF_F_MARK_MANGLED_0` is NOT set — passing only the
    //     2-byte size flag keeps that protective behaviour.
    let _ = is_udp; // documented above; helper handles the 0 sentinel.
    ctx.l4_csum_replace(
        l4_off + l4_csum_off,
        u64::from(old_src_port_be),
        u64::from(new_src_port_be),
        2,
    )
    .map_err(|_| ())?;

    // (d) Write the new source IP + source port bytes into the packet.
    //     `TcContext::store` calls `bpf_skb_store_bytes` under the hood;
    //     it preserves skb linearity and recomputes the L3 hardware
    //     checksum offload metadata if any. We pass network-order bytes
    //     directly so the helpers see the wire format.
    ctx.store(ETH_HDR_LEN + IPV4_SRC_IP_OFFSET, &new_src_ip_be, 0).map_err(|_| ())?;
    ctx.store(l4_off + L4_SRC_PORT_OFFSET, &new_src_port_be, 0).map_err(|_| ())?;

    Ok(TC_ACT_OK)
}
