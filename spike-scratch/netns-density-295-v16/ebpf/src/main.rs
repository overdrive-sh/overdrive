//! V-16 throwaway probe (netns-density-295). Aya Rust eBPF only.
//! The D5a fragment-aware, tuple-gated egress unframe, copied VERBATIM (logic unchanged) from
//! spike-scratch/netns-density-295-v11-vip-v14/bpf/src/main.rs (`eg_decode_frag` / `decode_frag`),
//! reduced to the maps and counters that program touches. Only the counter *set* is smaller.
#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{TC_ACT_OK, TC_ACT_SHOT},
    macros::{classifier, map},
    maps::{HashMap, PerCpuArray},
    programs::TcContext,
};

pub const MAGIC: [u8; 4] = *b"ZUD1";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Tuple {
    pub src: u32,
    pub dst: u32,
    pub sport: u16,
    pub dport: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FragKey {
    pub src: u32,
    pub dst: u32,
    pub id: u32,
}

#[map]
static TUPLES: HashMap<Tuple, u32> = HashMap::with_max_entries(4096, 0);
#[map]
static FRAGS: HashMap<FragKey, u32> = HashMap::with_max_entries(4096, 0);
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(64, 0);

const C_TC_DECODED: u32 = 13;
const C_TC_DECODED_EMPTY: u32 = 15;
const C_FRAG_FIRST: u32 = 32;
const C_FRAG_SHIFT: u32 = 33;
const C_TC_ERR_FRAG: u32 = 37;
const C_TC_LENIENT_PASS: u32 = 39;
// V-16 additions (observation only): packets that arrived as GSO skbs.
const C_GSO_SEEN: u32 = 50;
const C_GSO_UNFRAMED: u32 = 51;
const C_SEEN_UDP: u32 = 52;

#[inline(always)]
fn count(i: u32) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) }
    }
}

#[inline(always)]
fn fail(slot: u32, e: i64) {
    count(slot);
    if let Some(p) = CNT.get_ptr_mut(63) {
        unsafe { *p = (*p).wrapping_add((-e) as u64) }
    }
}

#[classifier]
pub fn eg_decode_frag(ctx: TcContext) -> i32 {
    match decode_frag(ctx) {
        Ok(x) => x,
        Err(_) => {
            count(C_TC_ERR_FRAG);
            TC_ACT_SHOT
        }
    }
}

fn decode_frag(mut ctx: TcContext) -> Result<i32, i64> {
    let ip = 14usize;
    let udp = 34usize;
    if ctx.len() < 34 || ctx.load::<u8>(ip).unwrap_or(0) != 0x45 || ctx.load::<u8>(ip + 9).unwrap_or(0) != 17 {
        return Ok(TC_ACT_OK);
    }
    let gso = unsafe { (*ctx.skb.skb).gso_segs } > 1;
    let src = ctx.load::<u32>(ip + 12)?;
    let dst = ctx.load::<u32>(ip + 16)?;
    let fragw = u16::from_be(ctx.load::<u16>(ip + 6)?);
    let off = fragw & 0x1fff;
    let mf = fragw & 0x2000 != 0;
    if off != 0 {
        let id = u16::from_be(ctx.load::<u16>(ip + 4)?) as u32;
        let key = FragKey { src, dst, id };
        if unsafe { FRAGS.get(&key) }.is_none() {
            return Ok(TC_ACT_OK);
        }
        let neww = (fragw & 0xe000) | (off - 1);
        ctx.l3_csum_replace(ip + 10, fragw.to_be() as u64, neww.to_be() as u64, 2)?;
        ctx.store(ip + 6, &neww.to_be(), 0)?;
        if !mf {
            let _ = FRAGS.remove(&key);
        }
        count(C_FRAG_SHIFT);
        return Ok(TC_ACT_OK);
    }
    if ctx.len() < 42 {
        return Ok(TC_ACT_OK);
    }
    count(C_SEEN_UDP);
    if gso {
        count(C_GSO_SEEN);
    }
    let sport = ctx.load::<u16>(udp)?;
    let dport = ctx.load::<u16>(udp + 2)?;
    let tuple = Tuple { src, dst, sport, dport };
    let Some(mode) = (unsafe { TUPLES.get(&tuple).copied() }) else { return Ok(TC_ACT_OK) };
    if mode != 2 && mode != 3 {
        return Ok(TC_ACT_OK);
    }
    let lenient = mode == 3;
    let old_ip = ctx.load::<u16>(ip + 2)?;
    let old_udp = ctx.load::<u16>(udp + 4)?;
    let plen = u16::from_be(old_udp) as u32; // whole datagram (UDP length)
    let ilen = u16::from_be(old_ip) as u32; // this fragment
    let shape_ok = plen >= 16 && ctx.len() == ilen + 14 && ilen >= 36 && (mf || ilen == plen + 20);
    if !shape_ok {
        if lenient {
            count(C_TC_LENIENT_PASS);
            return Ok(TC_ACT_OK);
        }
        return Err(-2);
    }
    let head = ctx.load::<[u8; 8]>(udp + 8)?;
    let logical = plen - 16;
    if head[0] != MAGIC[0] || head[1] != MAGIC[1] || head[2] != MAGIC[2] || head[3] != MAGIC[3]
        || u32::from_be_bytes([head[4], head[5], head[6], head[7]]) != logical
    {
        if lenient {
            count(C_TC_LENIENT_PASS);
            return Ok(TC_ACT_OK);
        }
        return Err(-3);
    }
    let udp_header = ctx.load::<[u8; 8]>(udp)?;
    // V-16 instrumentation (observation only): which step fails, and with which errno, on a GSO skb.
    // Variant `fgso` (V-16 follow-up experiment only): BPF_F_ADJ_ROOM_FIXED_GSO (1<<0) keeps gso_size on UDP GSO skbs.
    #[cfg(feature = "fgso")]
    let adj_flags: u64 = if gso { 1 } else { 0 };
    #[cfg(not(feature = "fgso"))]
    let adj_flags: u64 = 0;
    if let Err(e) = ctx.adjust_room(-8, 0, adj_flags) { fail(60, e); return Err(e); }
    if let Err(e) = ctx.store(udp, &udp_header, 0) { fail(61, e); return Err(e); }
    let new_ip = (ilen - 8) as u16;
    let new_udp = (plen - 8) as u16;
    if let Err(e) = ctx.l3_csum_replace(ip + 10, old_ip as u64, new_ip.to_be() as u64, 2) { fail(62, e); return Err(e); }
    ctx.store(ip + 2, &new_ip.to_be(), 0)?;
    ctx.store(udp + 4, &new_udp.to_be(), 0)?;
    ctx.store(udp + 6, &0u16, 0)?;
    if mf {
        let id = u16::from_be(ctx.load::<u16>(ip + 4)?) as u32;
        let _ = FRAGS.insert(&FragKey { src, dst, id }, &1, 0);
        count(C_FRAG_FIRST);
    }
    if gso {
        count(C_GSO_UNFRAMED);
    }
    count(C_TC_DECODED);
    if logical == 0 {
        count(C_TC_DECODED_EMPTY);
    }
    Ok(TC_ACT_OK)
}

#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
