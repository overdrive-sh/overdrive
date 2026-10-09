//! V-17 probe program (aya-rs, Rust). The design's egress unframe program (ADR-0165, D5a hybrid
//! TC half): fragment-aware, tuple-gated, lenient (mode 3). Lifted from the V-11 spike's
//! `eg_decode_frag` (spike-scratch/netns-density-295-v11-vip-v14/bpf/src/main.rs), reduced to the
//! unframe program, its tuple map, its fragment-tracking map and counters. Also `v17_nop`, a
//! trivial TC program used to fill an interface's TCX mprog set (refusal provocation).
#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{TC_ACT_OK, TC_ACT_SHOT},
    helpers::{bpf_get_current_pid_tgid, bpf_override_return, bpf_probe_read_kernel},
    macros::{classifier, kprobe, map},
    maps::{Array, HashMap, PerCpuArray},
    programs::{ProbeContext, TcContext},
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
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(16, 0);

const C_DECODED: u32 = 0;
const C_DECODED_EMPTY: u32 = 1;
const C_FRAG_FIRST: u32 = 2;
const C_FRAG_SHIFT: u32 = 3;
const C_LENIENT_PASS: u32 = 4;
const C_ERR: u32 = 5;
const C_IDEM_SKIP: u32 = 7; // idempotent variant: pass skipped (skb already marked)
const C_SEEN: u32 = 6; // IPv4 UDP packets examined

#[inline(always)]
fn count(i: u32) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) }
    }
}

#[classifier]
pub fn v17_unframe(ctx: TcContext) -> i32 {
    match decode_frag(ctx, false) {
        Ok(x) => x,
        Err(_) => {
            count(C_ERR);
            TC_ACT_SHOT
        }
    }
}

/// MITIGATION CANDIDATE (not the design): the same program, made idempotent per skb. After it
/// modifies a packet it sets skb->cb[4] = IDEM_MARK; a later pass on the same skb (a stacked
/// device's lower interface) sees the mark and passes.
#[classifier]
pub fn v17_unframe_idem(ctx: TcContext) -> i32 {
    match decode_frag(ctx, true) {
        Ok(x) => x,
        Err(_) => {
            count(C_ERR);
            TC_ACT_SHOT
        }
    }
}

const IDEM_MARK: u32 = 0x5a17;

#[inline(always)]
fn mark(ctx: &TcContext, idem: bool) {
    if idem {
        unsafe { (*ctx.skb.skb).cb[4] = IDEM_MARK };
    }
}

/// Fault injection for the ENOMEM refusal: overrides the bpf(2) syscall return for BPF_LINK_CREATE
/// issued by the target thread group, only while enabled. CFGK: [0]=offset of the first syscall
/// argument inside the user pt_regs (arm64 0, x86_64 112), [1]=enabled, [2]=tgid, [3]=injected count.
#[map]
static CFGK: Array<u64> = Array::with_max_entries(4, 0);

#[kprobe]
pub fn v17_enomem(ctx: ProbeContext) -> u32 {
    let (Some(off), Some(en), Some(tg)) = (CFGK.get(0).copied(), CFGK.get(1).copied(), CFGK.get(2).copied()) else { return 0 };
    if en == 0 || (bpf_get_current_pid_tgid() >> 32) != tg {
        return 0;
    }
    let Some(inner) = ctx.arg::<*const u8>(0) else { return 0 };
    let cmd = unsafe { bpf_probe_read_kernel::<u64>(inner.add(off as usize) as *const u64) }.unwrap_or(0) as u32;
    if cmd != 28 {
        return 0; // BPF_LINK_CREATE
    }
    if let Some(p) = CFGK.get_ptr_mut(3) {
        unsafe { *p += 1 };
    }
    unsafe { bpf_override_return(ctx.regs as *mut _, (-12i64) as u64) };
    0
}

#[classifier]
pub fn v17_nop(_ctx: TcContext) -> i32 {
    TC_ACT_OK
}

fn decode_frag(mut ctx: TcContext, idem: bool) -> Result<i32, i64> {
    if idem && unsafe { (*ctx.skb.skb).cb[4] } == IDEM_MARK {
        count(C_IDEM_SKIP);
        return Ok(TC_ACT_OK);
    }
    let ip = 14usize;
    let udp = 34usize;
    if ctx.len() < 34 || ctx.load::<u8>(ip).unwrap_or(0) != 0x45 || ctx.load::<u8>(ip + 9).unwrap_or(0) != 17 {
        return Ok(TC_ACT_OK);
    }
    count(C_SEEN);
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
        mark(&ctx, idem);
        return Ok(TC_ACT_OK);
    }
    if ctx.len() < 42 {
        return Ok(TC_ACT_OK);
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
    let plen = u16::from_be(old_udp) as u32;
    let ilen = u16::from_be(old_ip) as u32;
    let shape_ok = plen >= 16 && ctx.len() == ilen + 14 && ilen >= 36 && (mf || ilen == plen + 20);
    if !shape_ok {
        if lenient {
            count(C_LENIENT_PASS);
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
            count(C_LENIENT_PASS);
            return Ok(TC_ACT_OK);
        }
        return Err(-3);
    }
    let udp_header = ctx.load::<[u8; 8]>(udp)?;
    ctx.adjust_room(-8, 0, 0)?;
    ctx.store(udp, &udp_header, 0)?;
    let new_ip = (ilen - 8) as u16;
    let new_udp = (plen - 8) as u16;
    ctx.l3_csum_replace(ip + 10, old_ip as u64, new_ip.to_be() as u64, 2)?;
    ctx.store(ip + 2, &new_ip.to_be(), 0)?;
    ctx.store(udp + 4, &new_udp.to_be(), 0)?;
    ctx.store(udp + 6, &0u16, 0)?;
    if mf {
        let id = u16::from_be(ctx.load::<u16>(ip + 4)?) as u32;
        let _ = FRAGS.insert(&FragKey { src, dst, id }, &1, 0);
        count(C_FRAG_FIRST);
    }
    count(C_DECODED);
    if logical == 0 {
        count(C_DECODED_EMPTY);
    }
    mark(&ctx, idem);
    Ok(TC_ACT_OK)
}

#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
