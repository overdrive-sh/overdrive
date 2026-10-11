//! Throwaway spike (netns-density-295, drain probes T1-T6). Aya Rust eBPF only.
//!
//! One ELF, loaded by both roles. It holds:
//! * SK_SKB verdict: redirect a source socket's bytes to its routed target (SockHash,
//!   egress), counting the verdict-forwarded bytes (`fwd`) in the direction's cell.
//! * fexit(skb_send_sock): the final-hop sent counter. When the direction's cell is armed
//!   and `sent >= goal`, a fire-once compare-and-swap decides who reports completion; the
//!   winner in the kernel writes ONE record into a BPF ringbuf (BPF_RB_FORCE_WAKEUP).
//! * sock_ops: establishment-time install (passive intake children park into a
//!   loopback cell, active legs are installed when armed) and, optionally (R1-alt),
//!   STATE_CB on ESTABLISHED->CLOSE_WAIT arming the goal in the kernel.
//! * tracepoint vsock:virtio_transport_recv_pkt: records SHUTDOWN/RST ops (T5).
#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use core::arch::asm;

use aya_ebpf::{
    helpers::{bpf_get_socket_cookie, bpf_ktime_get_ns},
    macros::{fexit, map, sock_ops, stream_verdict, tracepoint},
    maps::{Array, HashMap, PerCpuArray, Queue, RingBuf, SockHash},
    programs::{FExitContext, SkBuffContext, SockOpsContext, TracePointContext},
};

pub const NONE: u32 = u32::MAX;
const RB_FORCE_WAKEUP: u64 = 2;
const MMAPABLE: u32 = 1 << 10;

/// One cell per (flow, direction). Owner and kernel both use atomics on the shared page.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cell {
    pub sent: u64,  // exact final-hop bytes sent (fexit skb_send_sock ret)
    pub goal: u64,  // bytes the target must have accepted
    pub armed: u64, // 1 once goal is valid
    pub fired: u64, // fire-once claim: 0 -> 1
    pub fwd: u64,   // verdict-forwarded bytes (source side)
    pub gen: u64,   // epoch, set by the owner on (re)assignment
    pub sent_b: u64, // second, independent fexit counter (its own recursion guard)
    pub drops: u64, // verdict redirects refused by the helper (SK_DROP): bytes eaten from the source, never sent
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Route {
    pub target: u64,
    pub cell: u32, // cell this source's verdicts account to; NONE = none
    pub nofwd: u32, // 1: account drops only, do not add to fwd
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Rec {
    pub cell: u32,
    pub kind: u32, // 1 = fexit completion, 2 = sock_ops STATE_CB completion
    pub gen: u64,
    pub sent: u64,
    pub goal: u64,
    pub t_ns: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShutRec {
    pub t_ns: u64,
    pub src_cid: u32,
    pub src_port: u32,
    pub dst_cid: u32,
    pub dst_port: u32,
    pub op: u32,
    pub flags: u32,
}

#[map]
static CELLS: Array<Cell> = Array::with_max_entries(4096, MMAPABLE);
#[map]
static ROUTE: HashMap<u64, Route> = HashMap::with_max_entries(65536, 0);
#[map]
static DSTCELL: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0);
#[map]
static SOCKS: SockHash<u64> = SockHash::with_max_entries(65536, 0);
#[map]
static VERD: SockHash<u64> = SockHash::with_max_entries(65536, 0);
// one completion ring per role: cells [0,2048) are the guest role's, [2048,4096) the host role's
#[map]
static RB_G: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
#[map]
static RB_H: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
#[map]
static SHUTRB: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
// CFG: 0 = kernel arms TCP-source goals in sock_ops STATE_CB (R1-alt)
#[map]
static CFG: Array<u32> = Array::with_max_entries(4, 0);
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(32, 0);
// establishment-time install
#[map]
static INTAKE: HashMap<u32, u32> = HashMap::with_max_entries(64, 0); // local port -> 1
#[map]
static TCPPARK: Array<u64> = Array::with_max_entries(4096, 0); // park slot -> P1 cookie
#[map]
static TCPFREE: Queue<u32> = Queue::with_max_entries(4096, 0);
#[map]
static PARK_OF: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0); // child cookie -> slot
#[map]
static ARM: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0); // active socket to install

const C_ZERO_LEN: u32 = 0;
const C_NOROUTE: u32 = 1;
const C_REDIR_FAIL: u32 = 2;
const C_REDIR_OK: u32 = 3;
const C_SEND_CALLS: u32 = 4;
const C_FIRE_FEXIT: u32 = 5;
const C_FIRE_STATECB: u32 = 6;
const C_RB_FAIL: u32 = 7;
const C_PARKED: u32 = 8;
const C_PARK_EMPTY: u32 = 9;
const C_INSTALL_FAIL: u32 = 10;
const C_STATECB: u32 = 11;
const C_STATECB_NOCELL: u32 = 12;
const C_SHUT_REC: u32 = 13;
// diagnostic: per-source-socket next expected TCP sequence (TCP_SKB_CB(skb)->seq/end_seq are the first
// two words of skb->cb while the skb sits in the receive queue) and overlap bytes seen
#[map]
static SEQNEXT: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0);
#[map]
static OVL: HashMap<u64, u64> = HashMap::with_max_entries(65536, 0);

#[map]
static ERRS: HashMap<u32, u64> = HashMap::with_max_entries(64, 0);

const C_SHUT_DROP: u32 = 14;
const C_SEND_ERR: u32 = 15;
const C_SEND_ZERO: u32 = 16;
const C_SEND_ERR_LAST: u32 = 17;
const C_NOCELL_VERDICT: u32 = 18;
const C_SEND_UNTRACKED: u32 = 19;
const C_OVERLAP_SKBS: u32 = 20;
const C_OVERLAP_BYTES: u32 = 21;
const C_SC_EST_CW: u32 = 22;
const C_SC_ARMED: u32 = 23;
const C_SC_ARM_NO_ROUTE_OR_CELL: u32 = 24;
const C_FLAGS_SET: u32 = 25;
const C_FLAGS_SET_ERR: u32 = 26;
const C_SC_FW1_CLOSING: u32 = 27;
const C_SC_FW2_CLOSE: u32 = 28;

#[inline(always)]
unsafe fn xadd(p: *mut u64, v: u64) -> u64 {
    // fully ordered 64-bit fetch-and-add (BPF_ATOMIC | BPF_FETCH), needs cpu v3
    let mut old = v;
    asm!("{old} = atomic_fetch_add((u64 *)({p} + 0), {old})", old = inout(reg) old, p = in(reg) p);
    old
}

/// 0 -> 1 compare-and-swap (BPF_CMPXCHG, always fully ordered). True iff this caller won.
#[inline(always)]
unsafe fn claim(p: *mut u64) -> bool {
    let old: u64;
    asm!("r0 = cmpxchg_64({p} + 0, r0, {n})", p = in(reg) p, n = in(reg) 1u64, inout("r0") 0u64 => old);
    old == 0
}

#[inline(always)]
fn count(i: u32) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) }
    }
}

// ------------------------------------------------------------------ verdict
#[stream_verdict]
pub fn verdict(ctx: SkBuffContext) -> u32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.skb.skb as *mut _) };
    let Some(r) = (unsafe { ROUTE.get(&cookie) }) else {
        count(C_NOROUTE);
        return 0;
    };
    let mut target = r.target;
    let n = ctx.len();
    if n == 0 {
        // A FIN-only skb redirected to egress makes skb_send_sock return 0 -> EPIPE.
        count(C_ZERO_LEN);
        return 0;
    }
    // diagnostic: detect a receive-queue skb whose head overlaps bytes already handed to a verdict
    if CFG.get(1).copied().unwrap_or(0) != 0 && r.cell != NONE {
        let (seq, end_seq) = unsafe { ((*ctx.skb.skb).cb[0], (*ctx.skb.skb).cb[1]) };
        match unsafe { SEQNEXT.get(&cookie) } {
            Some(next) => {
                let next = *next;
                let ov = next.wrapping_sub(seq) as i32;
                if ov > 0 {
                    count(C_OVERLAP_SKBS);
                    if let Some(c) = CNT.get_ptr_mut(C_OVERLAP_BYTES) {
                        unsafe { *c = (*c).wrapping_add(ov as u64) }
                    }
                    match OVL.get_ptr_mut(&cookie) {
                        Some(p) => {
                            unsafe { xadd(p, ov as u64) };
                        }
                        None => {
                            let _ = OVL.insert(&cookie, &(ov as u64), 0);
                        }
                    }
                }
                if (end_seq.wrapping_sub(next) as i32) > 0 {
                    let _ = SEQNEXT.insert(&cookie, &end_seq, 0);
                }
            }
            None => {
                let _ = SEQNEXT.insert(&cookie, &end_seq, 0);
            }
        }
    }
    if r.cell == NONE {
        count(C_NOCELL_VERDICT);
    } else if r.nofwd == 0 {
        if let Some(c) = CELLS.get_ptr_mut(r.cell) {
            unsafe { xadd(&mut (*c).fwd, n as u64) };
        }
    }
    let v = SOCKS.redirect_skb(&ctx, &mut target, 0) as u32;
    if v == 0 {
        count(C_REDIR_FAIL);
        if r.cell != NONE {
            if let Some(c) = CELLS.get_ptr_mut(r.cell) {
                // the helper refused the redirect (target not ESTABLISHED, or not in the map): the
                // bytes were eaten from the source and will never be sent. Tell the owner once.
                let prev = unsafe { xadd(&mut (*c).drops, 1) };
                if prev == 0 {
                    let rec = Rec { cell: r.cell, kind: 3, gen: unsafe { (*c).gen }, sent: n as u64, goal: 0, t_ns: unsafe { bpf_ktime_get_ns() } };
                    let _ = if r.cell < 2048 { RB_G.output(&rec, RB_FORCE_WAKEUP) } else { RB_H.output(&rec, RB_FORCE_WAKEUP) };
                }
            }
        }
    } else {
        count(C_REDIR_OK);
    }
    v
}

// ------------------------------------------------------------------ completion
/// armed/goal were published by the owner (or STATE_CB) before `armed` was set; `sent` was
/// just advanced with a fully ordered fetch_add. Exactly one claimant wins `fired`.
#[inline(always)]
unsafe fn try_complete(idx: u32, c: *mut Cell, sent: u64, kind: u32) {
    let armed = core::ptr::read_volatile(&(*c).armed);
    if armed == 0 {
        return;
    }
    let goal = core::ptr::read_volatile(&(*c).goal);
    if sent < goal {
        return;
    }
    if !claim(&mut (*c).fired) {
        return;
    }
    let rec = Rec { cell: idx, kind, gen: (*c).gen, sent, goal, t_ns: bpf_ktime_get_ns() };
    let r = if idx < 2048 { RB_G.output(&rec, RB_FORCE_WAKEUP) } else { RB_H.output(&rec, RB_FORCE_WAKEUP) };
    if r.is_err() {
        count(C_RB_FAIL);
    }
    if kind == 1 {
        count(C_FIRE_FEXIT);
    } else {
        count(C_FIRE_STATECB);
    }
}

#[inline(always)]
fn account_send(ctx: &FExitContext, second: bool) {
    let sk: *const core::ffi::c_void = unsafe { ctx.arg(0) };
    let ret: i32 = unsafe { ctx.arg(4) };
    if ret < 0 {
        if second {
            return;
        }
        let tracked = unsafe { DSTCELL.get(&bpf_get_socket_cookie(sk as *mut _)) }.is_some() as u32;
        let key = ((-ret) as u32) | (tracked << 16);
        match ERRS.get_ptr_mut(&key) {
            Some(p) => {
                unsafe { xadd(p, 1) };
            }
            None => {
                let _ = ERRS.insert(&key, &1u64, 0);
            }
        }
        count(C_SEND_ERR);
        if let Some(p) = CNT.get_ptr_mut(C_SEND_ERR_LAST) {
            unsafe { *p = (-ret) as u64 }
        }
        return;
    }
    if ret == 0 {
        if !second {
            count(C_SEND_ZERO);
        }
        return;
    }
    let cookie = unsafe { bpf_get_socket_cookie(sk as *mut _) };
    let Some(idx) = (unsafe { DSTCELL.get(&cookie) }) else {
        if !second {
            count(C_SEND_UNTRACKED);
        }
        return;
    };
    let idx = *idx;
    let Some(c) = CELLS.get_ptr_mut(idx) else { return };
    if !second {
        count(C_SEND_CALLS);
    }
    unsafe {
        let field = if second { &mut (*c).sent_b } else { &mut (*c).sent };
        let prev = xadd(field, ret as u64);
        try_complete(idx, c, prev + ret as u64, 1);
    }
}

#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    account_send(&ctx, false);
    0
}

/// Second, independent counter: a separate program has its own recursion guard, so a call skipped
/// by `sent` (the program was already running on this CPU) is still counted here, and vice versa.
#[fexit(function = "skb_send_sock")]
pub fn sent_b(ctx: FExitContext) -> i32 {
    account_send(&ctx, true);
    0
}

// ------------------------------------------------------------------ sock_ops
#[sock_ops]
pub fn flow_sockops(ctx: SockOpsContext) -> u32 {
    let op = ctx.op();
    match op {
        4 | 5 => {
            let cookie = unsafe { bpf_get_socket_cookie(ctx.ops as *mut _) };
            if op == 4 {
                if unsafe { ARM.get(&cookie) }.is_some() {
                    install(&ctx, cookie);
                    if ctx.set_cb_flags(4).is_ok() {
                        count(C_FLAGS_SET);
                    } else {
                        count(C_FLAGS_SET_ERR);
                    }
                }
            } else if unsafe { INTAKE.get(&ctx.local_port()) }.is_some() {
                let Some(slot) = TCPFREE.pop() else {
                    count(C_PARK_EMPTY);
                    return 1;
                };
                let p1 = TCPPARK.get(slot).copied().unwrap_or(0);
                // dir0 cell of a park slot is 2*slot (guest-role cell range starts at 0)
                let _ = ROUTE.insert(&cookie, &Route { target: p1, cell: slot * 2, nofwd: 0 }, 0);
                let _ = PARK_OF.insert(&cookie, &slot, 0);
                install(&ctx, cookie);
                if ctx.set_cb_flags(4).is_ok() {
                    count(C_FLAGS_SET);
                } else {
                    count(C_FLAGS_SET_ERR);
                }
                count(C_PARKED);
            }
        }
        10 => state_cb(&ctx),
        _ => {}
    }
    1
}

#[inline(always)]
fn install(ctx: &SockOpsContext, cookie: u64) {
    let ops = unsafe { &mut *ctx.ops };
    let mut k = cookie;
    if SOCKS.update(&mut k, ops, 0).is_err() {
        count(C_INSTALL_FAIL);
    }
    let mut k = cookie;
    if VERD.update(&mut k, ops, 0).is_err() {
        count(C_INSTALL_FAIL);
    }
}

/// R1-alt: tcp_fin() -> tcp_set_state(CLOSE_WAIT) runs this inside the softirq that received
/// the FIN: bytes_received already includes the FIN, the verdict has not yet run for the
/// final skb.
#[inline(always)]
fn state_cb(ctx: &SockOpsContext) {
    count(C_STATECB);
    // The peer's FIN is consumed in tcp_fin(): ESTABLISHED->CLOSE_WAIT, or (the leg was already
    // half-closed by us) FIN_WAIT1->CLOSING, or FIN_WAIT2->CLOSE via tcp_time_wait()/tcp_done().
    let (old, new) = (ctx.arg(0), ctx.arg(1));
    let fin_seen = (old == 1 && new == 8) || (old == 4 && new == 11) || (old == 5 && new == 7);
    if !fin_seen {
        return;
    }
    if old == 1 {
        count(C_SC_EST_CW);
    } else if old == 4 {
        count(C_SC_FW1_CLOSING);
    } else {
        count(C_SC_FW2_CLOSE);
    }
    if CFG.get(0).copied().unwrap_or(0) == 0 {
        return;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.ops as *mut _) };
    let Some(r) = (unsafe { ROUTE.get(&cookie) }) else {
        count(C_STATECB_NOCELL);
        return;
    };
    let idx = r.cell;
    let Some(c) = CELLS.get_ptr_mut(idx) else {
        count(C_STATECB_NOCELL);
        return;
    };
    let br = unsafe { (*ctx.ops).bytes_received };
    if br == 0 {
        count(C_SC_ARM_NO_ROUTE_OR_CELL);
        return;
    }
    count(C_SC_ARMED);
    let goal = br - 1;
    unsafe {
        core::ptr::write_volatile(&mut (*c).goal, goal);
        xadd(&mut (*c).armed, 1); // locked op: publishes goal, full barrier before the sent load
        let sent_a = core::ptr::read_volatile(&(*c).sent);
        let sent_b = core::ptr::read_volatile(&(*c).sent_b);
        try_complete(idx, c, if sent_a > sent_b { sent_a } else { sent_b }, 2);
    }
}

// ------------------------------------------------------------------ T5 tracepoint
#[tracepoint(category = "vsock", name = "virtio_transport_recv_pkt")]
pub fn vsock_recv(ctx: TracePointContext) -> u32 {
    unsafe {
        let op: u16 = ctx.read_at(30).unwrap_or(0);
        if op != 3 && op != 4 {
            return 0;
        }
        let rec = ShutRec {
            t_ns: bpf_ktime_get_ns(),
            src_cid: ctx.read_at(8).unwrap_or(0),
            src_port: ctx.read_at(12).unwrap_or(0),
            dst_cid: ctx.read_at(16).unwrap_or(0),
            dst_port: ctx.read_at(20).unwrap_or(0),
            op: op as u32,
            flags: ctx.read_at(32).unwrap_or(0),
        };
        if SHUTRB.output(&rec, RB_FORCE_WAKEUP).is_err() {
            count(C_SHUT_DROP);
        } else {
            count(C_SHUT_REC);
        }
    }
    0
}

#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
