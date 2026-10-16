//! V-VALVE spike (netns-density-295). Aya Rust eBPF only.
//!
//! SK_SKB stream verdict on the SOURCE socket (VERD key 0). It redirects (egress, or BPF_F_INGRESS per
//! CFG[2]) to the TARGET socket (SOCKS key 1). fexit(skb_send_sock) counts the bytes that really reach the
//! target's send path. queued = FWD - SENT = bytes parked in the target psock->ingress_skb.
//!
//! The verdict program cannot remove its own source from the sockhash: kernel/bpf/verifier.c:10131-10167
//! (may_update_sockmap) does not list BPF_PROG_TYPE_SK_SKB, so bpf_map_delete_elem is rejected there.
//! Instead the program EMITS an event to the owner over a ring buffer when queued crosses HIGH, and the
//! fexit program emits one when queued falls to LOW or below while the valve is closed. The owner (user
//! space) acts on the events.
//!
//! CFG[0] mode: 0 always redirect, 1 return SK_PASS once queued >= HIGH (candidate 2 probe),
//!              3 park: SK_PASS while queued >= HIGH or while parked bytes are not yet forwarded by the owner,
//!              2 return SK_DROP once queued >= HARD (loses data; a bound demonstration only)
//! CFG[1] HIGH bytes (0 = no valve), CFG[2] redirect flags, CFG[3] LOW bytes, CFG[4] HARD bytes
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};

use aya_ebpf::{
    helpers::bpf_ktime_get_ns,
    macros::{fexit, kretprobe, map, stream_verdict},
    maps::{Array, RingBuf, SockHash},
    programs::{FExitContext, RetProbeContext, SkBuffContext},
};

#[map]
static VERD: SockHash<u32> = SockHash::with_max_entries(64, 0);
#[map]
static SOCKS: SockHash<u32> = SockHash::with_max_entries(64, 0);
#[map]
static STATE: Array<u64> = Array::with_max_entries(32, 0);
#[map]
static CFG: Array<u64> = Array::with_max_entries(8, 0);
#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(1 << 16, 0);

const FWD: u32 = 0;
const SENT: u32 = 1;
const VERDICTS: u32 = 2;
const SEND_OK: u32 = 3;
const SEND_FAIL: u32 = 4;
const LAST_ERRNO: u32 = 5;
const REFUSED: u32 = 6;
const PEAK: u32 = 7;
const H_EMITTED: u32 = 8;
const H_ACKED: u32 = 9;
const D_EMITTED: u32 = 10;
const PASSED: u32 = 11;
const HARD_DROPS: u32 = 12;
const RB_FAIL_V: u32 = 13;
const RB_FAIL_F: u32 = 14;
const PARKED: u32 = 16; // bytes the verdict returned SK_PASS for in mode 3 (verdict-only writer)
const FORWARDED: u32 = 17; // cumulative bytes the owner has received from the source and sent to the target (owner-only writer)
const HOLD: u32 = 18; // owner sets 1 while it holds received-but-unsent bytes (owner-only writer)
const RS_OK: u32 = 19; // kretprobe virtio_transport_read_skb: ret > 0
const RS_ZERO: u32 = 20; // ret == 0: an skb was dequeued and DROPPED by the verdict path (psock gone)
const RS_ERR: u32 = 21; // ret < 0
const RS_BYTES: u32 = 22;
const VR_OK: u32 = 23; // fexit sk_psock_verdict_recv: ret > 0
const VR_ZERO: u32 = 24; // ret == 0: sk_psock(sk) was NULL, the dequeued skb was dropped (skmsg.c:1103-1108)
const VR_ERR: u32 = 25;

#[repr(C)]
pub struct Ev {
    pub kind: u32, // 1 HIGH, 2 DRAINED
    pub pad: u32,
    pub fwd: u64,
    pub sent: u64,
    pub ktime: u64,
}

// Every STATE cell has exactly ONE kernel-side writer (the bpf target here has no atomic RMW):
//   verdict program (serialised per source socket by the socket lock): FWD, VERDICTS, REFUSED, PEAK, H_EMITTED,
//     PASSED, HARD_DROPS, RB_FAIL_V
//   fexit(skb_send_sock) (serialised per target psock by psock->work_mutex): SENT, SEND_OK, SEND_FAIL, LAST_ERRNO,
//     D_EMITTED, RB_FAIL_F
//   owner (user space): H_ACKED, and zeroing at trial start
#[inline(always)]
fn cell(i: u32) -> Option<*mut u64> {
    STATE.get_ptr_mut(i)
}
#[inline(always)]
fn cfg(i: u32) -> u64 {
    CFG.get(i).copied().unwrap_or(0)
}
#[inline(always)]
fn add(i: u32, v: u64) -> u64 {
    match cell(i) {
        Some(c) => unsafe {
            let n = read_volatile(c).wrapping_add(v);
            write_volatile(c, n);
            n
        },
        None => 0,
    }
}
#[inline(always)]
fn load(i: u32) -> u64 {
    match cell(i) {
        Some(c) => unsafe { read_volatile(c) },
        None => 0,
    }
}
#[inline(always)]
fn emit(kind: u32, fwd: u64, sent: u64, fail_cell: u32) {
    let ev = Ev { kind, pad: 0, fwd, sent, ktime: unsafe { bpf_ktime_get_ns() } };
    if EVENTS.output(&ev, 0).is_err() {
        add(fail_cell, 1);
    }
}

#[stream_verdict]
pub fn verdict(ctx: SkBuffContext) -> u32 {
    let len = ctx.len() as u64;
    add(VERDICTS, 1);
    let q = load(FWD).saturating_sub(load(SENT));
    let high = cfg(1);
    if cfg(0) == 3 && high != 0 {
        let pending = load(PARKED).saturating_sub(load(FORWARDED)) > 0 || load(HOLD) != 0;
        if pending || q.wrapping_add(len) >= high {
            if load(H_EMITTED) == load(H_ACKED) {
                add(H_EMITTED, 1);
                emit(1, load(FWD), load(SENT), RB_FAIL_V);
            }
            add(PARKED, len);
            add(PASSED, 1);
            return 1; // SK_PASS: the skb stays charged to the source socket
        }
    }
    if high != 0 && q.wrapping_add(len) >= high && cfg(0) != 3 {
        // emit HIGH once per cycle: only while the owner has acked every HIGH emitted so far
        if load(H_EMITTED) == load(H_ACKED) {
            add(H_EMITTED, 1);
            emit(1, load(FWD), load(SENT), RB_FAIL_V);
        }
        if cfg(0) == 1 {
            add(PASSED, 1);
            return 1; // SK_PASS
        }
    }
    let hard = cfg(4);
    if cfg(0) == 2 && hard != 0 && q >= hard {
        add(HARD_DROPS, 1);
        return 0; // SK_DROP
    }
    let nq = add(FWD, len).saturating_sub(load(SENT));
    if let Some(p) = cell(PEAK) {
        unsafe {
            if nq > read_volatile(p) {
                write_volatile(p, nq);
            }
        }
    }
    let mut key = 1u32;
    let v = SOCKS.redirect_skb(&ctx, &mut key, cfg(2)) as u32;
    if v == 0 {
        add(REFUSED, 1);
    }
    v
}

#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    let ret: i32 = unsafe { ctx.arg(4) };
    if ret > 0 {
        let sent = add(SENT, ret as u64);
        add(SEND_OK, 1);
        // DRAINED once per HIGH: a HIGH was emitted that has no matching DRAINED yet, and the queue is at or below LOW
        let he = load(H_EMITTED);
        if he > load(D_EMITTED) {
            let fwd = load(FWD);
            if fwd.saturating_sub(sent) <= cfg(3) {
                if let Some(p) = cell(D_EMITTED) {
                    unsafe { write_volatile(p, he) };
                }
                emit(2, fwd, sent, RB_FAIL_F);
            }
        }
    } else {
        add(SEND_FAIL, 1);
        if let Some(p) = cell(LAST_ERRNO) {
            unsafe { write_volatile(p, (-ret) as u64) };
        }
    }
    0
}

#[fexit(function = "sk_psock_verdict_recv")]
pub fn vr_ret(ctx: FExitContext) -> i32 {
    let ret: i32 = unsafe { ctx.arg(2) };
    if ret > 0 {
        add(VR_OK, 1);
    } else if ret == 0 {
        add(VR_ZERO, 1);
    } else {
        add(VR_ERR, 1);
    }
    0
}

#[kretprobe]
pub fn rs_ret(ctx: RetProbeContext) -> u32 {
    let r: i64 = ctx.ret().unwrap_or(0);
    let r = r as i32;
    if r > 0 {
        add(RS_OK, 1);
        add(RS_BYTES, r as u64);
    } else if r == 0 {
        add(RS_ZERO, 1);
    } else {
        add(RS_ERR, 1);
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
