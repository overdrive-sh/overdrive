//! Throwaway spike (netns-density-295, v31 "R2": no control-session half-close). Aya Rust eBPF only.
//!
//! One ELF, loaded by both roles. It holds:
//! * SK_SKB verdict: redirect a source socket's bytes to its routed target through a SockHash
//!   (egress), counting the verdict-forwarded bytes (`fwd`) in the direction's cell.
//! * two independent fexit(skb_send_sock) programs. Each reads a CUMULATIVE kernel quantity of the
//!   target socket at exit (TCP: tcp_sock.write_seq minus snd_nxt-at-establish; vsock:
//!   virtio_vsock_sock.tx_cnt minus the owner's own bytes) instead of adding its own per-call counter.
//!   When the direction's cell is armed and `sent >= goal`, a fire-once compare-and-swap decides who
//!   reports completion; the winner writes ONE record into a BPF ringbuf.
//! * fentry(sock_poll): the level backstop -- the same cumulative read, run when the owner polls the
//!   target socket (poll(fd, 0, 0)); independent of the fexit programs having run for the last call.
//! * sock_ops: establishment-time install (passive intake children park into a loopback cell, active
//!   legs are installed when armed) and the TCP send-sequence base.
//! * tracepoint vsock:virtio_transport_recv_pkt: SHUTDOWN/RST ops with flags, one ring per role.
#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use core::arch::asm;

use aya_ebpf::{
    EbpfContext,
    helpers::{bpf_get_prandom_u32, bpf_get_socket_cookie, bpf_ktime_get_ns, bpf_probe_read_kernel},
    macros::{fentry, fexit, map, raw_tracepoint, sock_ops, stream_verdict, tracepoint},
    maps::{Array, HashMap, PerCpuArray, Queue, RingBuf, SockHash},
    programs::{FEntryContext, FExitContext, RawTracePointContext, SkBuffContext, SockOpsContext, TracePointContext},
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

/// What a socket is the FINAL-HOP TARGET of: its cell, its kind (1 = TCP, 2 = vsock) and, for a vsock,
/// the bytes the owner itself wrote on it outside the redirect path (the 16-byte flow header).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Dst {
    pub cell: u32,
    pub kind: u32,
    pub base: u32,
    pub _pad: u32,
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
static DSTCELL: HashMap<u64, Dst> = HashMap::with_max_entries(65536, 0);
/// TCP legs: snd_nxt at establishment (== write_seq, nothing queued yet). Written by sock_ops.
#[map]
static SNDBASE: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0);
#[map]
static SOCKS: SockHash<u64> = SockHash::with_max_entries(65536, 0);
#[map]
static VERD: SockHash<u64> = SockHash::with_max_entries(65536, 0);
// one completion ring per role: cells [0,2048) are the guest role's, [2048,4096) the host role's
#[map]
static RB_G: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
#[map]
static RB_H: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
// SHUTDOWN/RST packets received by this kernel: those addressed to the host role's listener port go to
// SHUTRB_H, everything else (guest role's flow vsocks, control session) to SHUTRB_G
#[map]
static SHUTRB_G: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
#[map]
static SHUTRB_H: RingBuf = RingBuf::with_byte_size(1 << 22, 0);
const HOST_VPORT: u32 = 7900;
// CFG slots
const CFG_DIAG_SEQ: u32 = 1;
const CFG_SKIP_A: u32 = 2; // fault injection: program A skips this permille of calls (counted as an injected miss)
const CFG_SKIP_B: u32 = 3;
const CFG_INCR: u32 = 4; // 1 = v27 per-call increment counters (control), 0 = cumulative kernel quantity
const CFG_OFF_WSEQ: u32 = 5; // tcp_sock.write_seq
const CFG_OFF_TRANS: u32 = 6; // vsock_sock.trans
const CFG_OFF_TXCNT: u32 = 7; // virtio_vsock_sock.tx_cnt
const CFG_OFF_FPRIV: u32 = 8; // file.private_data
const CFG_OFF_SKT: u32 = 9; // socket.sk
const CFG_OFF_COOKIE: u32 = 10; // sock.__sk_common.skc_cookie
const CFG_POKE: u32 = 11; // 1 = the fentry(sock_poll) level read is active
#[map]
static CFG: Array<u32> = Array::with_max_entries(16, 0);
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(48, 0);
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
const C_INJ_MISS: u32 = 29;
const C_POKE_RUN: u32 = 30;
const C_FIRE_POKE: u32 = 31;
const C_LEVEL_UNREADABLE: u32 = 32;
const C_SHUT_DROP_H: u32 = 33;
const C_SHUT_REC_H: u32 = 34;

#[inline(always)]
fn cfg(i: u32) -> u32 {
    CFG.get(i).copied().unwrap_or(0)
}

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
    if cfg(CFG_DIAG_SEQ) != 0 && r.cell != NONE {
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
/// armed/goal were published by the owner before `armed` was set; `sent` was just advanced. Exactly
/// one claimant wins `fired`.
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
    match kind {
        1 => count(C_FIRE_FEXIT),
        4 => count(C_FIRE_POKE),
        _ => count(C_FIRE_STATECB),
    }
}

/// General 64-bit compare-and-swap (BPF_CMPXCHG, fully ordered). Returns the value found.
#[inline(always)]
unsafe fn cas(p: *mut u64, old: u64, new: u64) -> u64 {
    let found: u64;
    asm!("r0 = cmpxchg_64({p} + 0, r0, {n})", p = in(reg) p, n = in(reg) new, inout("r0") old => found);
    found
}

/// Monotone store: *p = max(*p, v), bounded retries.
#[inline(always)]
unsafe fn amax(p: *mut u64, v: u64) {
    let mut old = core::ptr::read_volatile(p);
    let mut i = 0;
    while i < 6 {
        if old >= v {
            return;
        }
        let f = cas(p, old, v);
        if f == old {
            return;
        }
        old = f;
        i += 1;
    }
}

/// Cumulative bytes the target socket has ACCEPTED since the direction's base, modulo 2^32:
///  * TCP:   tcp_sock.write_seq (net/ipv4/tcp.c:1390 @ v7.0, advanced by every byte tcp_sendmsg_locked
///           copies into the send queue; a FIN adds one, which only happens after completion)
///           minus snd_nxt recorded at establishment;
///  * vsock: virtio_vsock_sock.tx_cnt (net/vmw_vsock/virtio_transport_common.c:504, advanced by
///           virtio_transport_get_credit() per packet, rolled back by put_credit() for what was not sent)
///           minus the bytes the owner wrote itself (the 16-byte flow header).
#[inline(always)]
unsafe fn level_of(sk: u64, cookie: u64, d: &Dst) -> Option<u32> {
    if d.kind == 1 {
        let w: u32 = bpf_probe_read_kernel((sk + cfg(CFG_OFF_WSEQ) as u64) as *const u32).ok()?;
        let base = *SNDBASE.get(&cookie)?;
        Some(w.wrapping_sub(base))
    } else {
        let t: u64 = bpf_probe_read_kernel((sk + cfg(CFG_OFF_TRANS) as u64) as *const u64).ok()?;
        if t == 0 {
            return None;
        }
        let c: u32 = bpf_probe_read_kernel((t + cfg(CFG_OFF_TXCNT) as u64) as *const u32).ok()?;
        Some(c.wrapping_sub(d.base))
    }
}

/// Extend a 32-bit level to the cell's 64-bit monotone `sent` (signed 32-bit delta: a stale read that
/// is behind the cell's value changes nothing), store it with a monotone max, then try to complete.
#[inline(always)]
unsafe fn publish(idx: u32, c: *mut Cell, lvl: u32, second: bool, kind: u32) {
    let sa = core::ptr::read_volatile(&(*c).sent);
    let sb = core::ptr::read_volatile(&(*c).sent_b);
    let e = if sa > sb { sa } else { sb };
    let delta = lvl.wrapping_sub(e as u32) as i32;
    let nv = if delta > 0 { e + delta as u64 } else { e };
    let field = if second { &mut (*c).sent_b as *mut u64 } else { &mut (*c).sent as *mut u64 };
    amax(field, nv);
    try_complete(idx, c, nv, kind);
}

#[inline(always)]
fn skipped(second: bool) -> bool {
    let p = cfg(if second { CFG_SKIP_B } else { CFG_SKIP_A });
    if p == 0 {
        return false;
    }
    let r = unsafe { bpf_get_prandom_u32() } % 1000;
    if r < p {
        count(C_INJ_MISS);
        true
    } else {
        false
    }
}

#[inline(always)]
fn account_send(ctx: &FExitContext, second: bool) {
    if skipped(second) {
        return;
    }
    let sk: *const core::ffi::c_void = unsafe { ctx.arg(0) };
    let ret: i32 = unsafe { ctx.arg(4) };
    let cookie = unsafe { bpf_get_socket_cookie(sk as *mut _) };
    let Some(d) = (unsafe { DSTCELL.get(&cookie) }) else {
        if !second {
            if ret < 0 {
                count(C_SEND_ERR);
            } else if ret == 0 {
                count(C_SEND_ZERO);
            } else {
                count(C_SEND_UNTRACKED);
            }
        }
        return;
    };
    if ret < 0 && !second {
        let key = ((-ret) as u32) | (1 << 16);
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
    }
    let idx = d.cell;
    let Some(c) = CELLS.get_ptr_mut(idx) else { return };
    if !second {
        count(C_SEND_CALLS);
    }
    unsafe {
        if cfg(CFG_INCR) != 0 {
            // v27 control: per-call increment counters
            if ret > 0 {
                let field = if second { &mut (*c).sent_b } else { &mut (*c).sent };
                let prev = xadd(field, ret as u64);
                try_complete(idx, c, prev + ret as u64, 1);
            }
            return;
        }
        match level_of(sk as u64, cookie, d) {
            Some(lvl) => publish(idx, c, lvl, second, 1),
            None => count(C_LEVEL_UNREADABLE),
        }
    }
}

#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    account_send(&ctx, false);
    0
}

/// Second, independent program: its own recursion guard, so a call skipped by `sent` is still seen here.
#[fexit(function = "skb_send_sock")]
pub fn sent_b(ctx: FExitContext) -> i32 {
    account_send(&ctx, true);
    0
}

/// Level backstop: the same cumulative read, run when the owner polls the target socket.
#[fentry(function = "sock_poll")]
pub fn poke(ctx: FEntryContext) -> i32 {
    if cfg(CFG_POKE) == 0 {
        return 0;
    }
    unsafe {
        let file: u64 = ctx.arg(0);
        let Ok(sock) = bpf_probe_read_kernel((file + cfg(CFG_OFF_FPRIV) as u64) as *const u64) else { return 0 };
        if sock == 0 {
            return 0;
        }
        let Ok(sk) = bpf_probe_read_kernel((sock + cfg(CFG_OFF_SKT) as u64) as *const u64) else { return 0 };
        if sk == 0 {
            return 0;
        }
        let Ok(cookie) = bpf_probe_read_kernel((sk + cfg(CFG_OFF_COOKIE) as u64) as *const u64) else { return 0 };
        let Some(d) = DSTCELL.get(&cookie) else { return 0 };
        let idx = d.cell;
        let Some(c) = CELLS.get_ptr_mut(idx) else { return 0 };
        if core::ptr::read_volatile(&(*c).armed) == 0 || core::ptr::read_volatile(&(*c).fired) != 0 {
            return 0;
        }
        count(C_POKE_RUN);
        match level_of(sk, cookie, d) {
            Some(lvl) => publish(idx, c, lvl, false, 4),
            None => count(C_LEVEL_UNREADABLE),
        }
    }
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
                    let snd_nxt = unsafe { (*ctx.ops).snd_nxt };
                    let _ = SNDBASE.insert(&cookie, &snd_nxt, 0);
                    install(&ctx, cookie);
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
                let snd_nxt = unsafe { (*ctx.ops).snd_nxt };
                let _ = SNDBASE.insert(&cookie, &snd_nxt, 0);
                install(&ctx, cookie);
                count(C_PARKED);
            }
        }
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

// ------------------------------------------------------------------ vsock SHUTDOWN / RST classification input
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
        let to_host = rec.dst_port == HOST_VPORT;
        let r = if to_host { SHUTRB_H.output(&rec, RB_FORCE_WAKEUP) } else { SHUTRB_G.output(&rec, RB_FORCE_WAKEUP) };
        if r.is_err() {
            count(if to_host { C_SHUT_DROP_H } else { C_SHUT_DROP });
        } else {
            count(if to_host { C_SHUT_REC_H } else { C_SHUT_REC });
        }
    }
    0
}


/// Same classification input as `vsock_recv`, but as a RAW tracepoint. `vsock_recv` is a perf-attached tracepoint program and
/// runs through `trace_call_bpf` (kernel/trace/bpf_trace.c:109-127 @ v7.0), which SKIPS the program (counted only as a miss)
/// whenever `bpf_prog_active != 1` on the CPU -- and every bpf() map-update/delete syscall the owner issues raises it
/// (kernel/bpf/syscall.c:1862 `bpf_disable_instrumentation()` @ v7.0). Raw tracepoints run despite bpf_prog_active
/// (bpf_trace.c:1568 @ v7.0).
#[raw_tracepoint(tracepoint = "virtio_transport_recv_pkt")]
pub fn vsock_recv_raw(ctx: RawTracePointContext) -> i32 {
    unsafe {
        let a = ctx.as_ptr() as *const u64;
        let op = (*a.add(6)) as u32 & 0xffff;
        if op != 3 && op != 4 {
            return 0;
        }
        let rec = ShutRec {
            t_ns: bpf_ktime_get_ns(),
            src_cid: *a.add(0) as u32,
            src_port: *a.add(1) as u32,
            dst_cid: *a.add(2) as u32,
            dst_port: *a.add(3) as u32,
            op,
            flags: *a.add(7) as u32,
        };
        let to_host = rec.dst_port == HOST_VPORT;
        let r = if to_host { SHUTRB_H.output(&rec, RB_FORCE_WAKEUP) } else { SHUTRB_G.output(&rec, RB_FORCE_WAKEUP) };
        if r.is_err() {
            count(if to_host { C_SHUT_DROP_H } else { C_SHUT_DROP });
        } else {
            count(if to_host { C_SHUT_REC_H } else { C_SHUT_REC });
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
