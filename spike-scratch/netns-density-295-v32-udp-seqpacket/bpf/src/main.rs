//! Throwaway spike v32 (netns-density-295): guest UDP strparser cell -> vsock SEQPACKET/STREAM.
//! Aya Rust eBPF only. One ELF loaded by both owners (guest and host stand-ins).
//!
//! Guest: UDP intake (mode 1: frame + redirect to loopback TCP cell P1), P2 strparser (one frame per
//!        verdict, mode 0: redirect to the vsock leg), fexit(skb_send_sock) outcome histogram on the
//!        vsock leg, tracepoint vsock:virtio_transport_alloc_pkt (EOM / non-EOM packet counts).
//! Host:  vsock SEQPACKET leg (mode 2: ADR-0149 length check, then redirect to the sink) or vsock
//!        STREAM leg (mode 0 -> host loopback TCP cell H1; H2 strparser mode 2 -> sink),
//!        tracepoint vsock:virtio_transport_recv_pkt.
//! Payload only ever moves by SK_SKB redirect; userspace reads only the final sink.
#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_socket_cookie, bpf_skb_adjust_room},
    macros::{fexit, map, stream_parser, stream_verdict, tracepoint},
    maps::{Array, HashMap, PerCpuArray, RingBuf, SockHash},
    programs::{FExitContext, SkBuffContext, TracePointContext},
};

pub const MAGIC: [u8; 4] = *b"ZUD1";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Route {
    pub target: u64,
    pub mode: u32, // 0 plain, 1 encode (+8 frame), 2 check frame (ADR-0149 length check)
    pub pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Evt {
    pub kind: u32, // 1 short (n-8 < declared), 2 long (n-8 > declared), 3 n<8, 4 bad magic
    pub n: u32,
    pub declared: u32,
    pub second_magic: u32,
    pub seq: u64,
}

#[map]
static SOCKS: SockHash<u64> = SockHash::with_max_entries(1024, 0); // redirect targets
#[map]
static VERD: SockHash<u64> = SockHash::with_max_entries(1024, 0); // stream_verdict attached
#[map]
static STRP: SockHash<u64> = SockHash::with_max_entries(1024, 0); // parser + verdict attached
#[map]
static ROUTE: HashMap<u64, Route> = HashMap::with_max_entries(1024, 0);
#[map]
static TRACK: HashMap<u64, u32> = HashMap::with_max_entries(64, 0); // vsock tx leg cookies (fexit)
#[map]
static ERRS: HashMap<u32, u64> = HashMap::with_max_entries(64, 0); // fexit outcome histogram
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(64, 0);
// CFG: 0 payload bound (0 = none), 1 check enabled (mode 2), 2 data vsock port, 3 last host buf_alloc seen, 4 parse log on
#[map]
static CFG: Array<u32> = Array::with_max_entries(8, 0);
#[map]
static EVT: RingBuf = RingBuf::with_byte_size(1 << 20, 0);

pub const C_ENC: u32 = 0;
pub const C_OVER_BOUND: u32 = 1;
pub const C_HELPER_FAIL: u32 = 2;
pub const C_NOROUTE: u32 = 3;
pub const C_REDIR_OK: u32 = 4;
pub const C_REDIR_FAIL: u32 = 5;
pub const C_STRP_FRAMES: u32 = 6;
pub const C_STRP_BAD: u32 = 7;
pub const C_RX_OK: u32 = 8;
pub const C_RX_LT8: u32 = 9;
pub const C_RX_BADMAGIC: u32 = 10;
pub const C_RX_SHORT: u32 = 11;
pub const C_RX_LONG: u32 = 12;
pub const C_RX_LONG_2MAGIC: u32 = 13;
pub const C_RX_PASSED_BAD: u32 = 14; // mismatched record passed on because the check is off
pub const C_ZERO_LEN: u32 = 15;
pub const C_FX_CALLS: u32 = 16;
pub const C_FX_BYTES: u32 = 17;
pub const C_EVT_DROP: u32 = 18;
pub const C_PARSE_CALLS: u32 = 19;
pub const C_TXA_SEQ_EOM: u32 = 20;
pub const C_TXA_SEQ_NOEOM: u32 = 21;
pub const C_TXA_SEQ_NOEOM_BYTES: u32 = 22;
pub const C_TXA_STREAM: u32 = 23;
pub const C_TXA_STREAM_BYTES: u32 = 24;
pub const C_TXA_SEQ_EOM_BYTES: u32 = 25;
pub const C_RXP_SEQ_EOM: u32 = 30;
pub const C_RXP_SEQ_NOEOM: u32 = 31;
pub const C_RXP_SEQ_NOEOM_BYTES: u32 = 32;
pub const C_RXP_STREAM: u32 = 33;
pub const C_RXP_STREAM_BYTES: u32 = 34;
pub const C_RXP_SEQ_EOM_BYTES: u32 = 35;

#[inline(always)]
fn add(i: u32, v: u64) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(v) }
    }
}

#[inline(always)]
fn count(i: u32) {
    add(i, 1)
}

#[inline(always)]
fn cfg(i: u32) -> u32 {
    CFG.get(i).copied().unwrap_or(0)
}

#[stream_verdict]
pub fn verdict(ctx: SkBuffContext) -> u32 {
    forward(ctx)
}

#[stream_verdict]
pub fn strp_verdict(ctx: SkBuffContext) -> u32 {
    count(C_STRP_FRAMES);
    forward(ctx)
}

#[stream_parser]
pub fn strp_parse(ctx: SkBuffContext) -> u32 {
    count(C_PARSE_CALLS);
    if ctx.len() < 8 {
        return 0;
    }
    let Ok(h) = ctx.load::<[u8; 8]>(0) else { return 0 };
    if cfg(4) != 0 {
        // diagnostic: log the first parser inputs (skb length + first 8 bytes the parser can read at offset 0)
        if let Some(p) = CNT.get_ptr(C_PARSE_CALLS) {
            if unsafe { *p } <= 48 {
                evt(5, ctx.len(), 0, 0, u64::from_be_bytes(h));
            }
        }
    }
    if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3] {
        count(C_STRP_BAD);
        evt(6, ctx.len(), 0, 0, u64::from_be_bytes(h));
        return u32::MAX; // -1: abort the stream (never expected)
    }
    8 + u32::from_be_bytes([h[4], h[5], h[6], h[7]])
}

#[inline(always)]
fn evt(kind: u32, n: u32, declared: u32, second_magic: u32, seq: u64) {
    let e = Evt { kind, n, declared, second_magic, seq };
    if EVT.output(&e, 0).is_err() {
        count(C_EVT_DROP);
    }
}

#[inline(always)]
fn forward(ctx: SkBuffContext) -> u32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.skb.skb as *mut _) };
    let Some(r) = (unsafe { ROUTE.get(&cookie) }) else {
        count(C_NOROUTE);
        return 0;
    };
    let mut target = r.target;
    let mode = r.mode;
    let n = ctx.len();
    if n == 0 {
        // a FIN-only skb redirected to egress would disable the target psock's TX
        count(C_ZERO_LEN);
        return 0;
    }
    if mode == 1 {
        let bound = cfg(0);
        if bound != 0 && n > bound {
            count(C_OVER_BOUND);
            return 0;
        }
        let header = [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], (n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8];
        if unsafe { bpf_skb_adjust_room(ctx.skb.skb, 8, 0, 0) } != 0 {
            count(C_HELPER_FAIL);
            return 0;
        }
        let mut c = ctx;
        if c.store(0, &header, 0).is_err() {
            count(C_HELPER_FAIL);
            return 0;
        }
        count(C_ENC);
        if c.pull_data(c.len()).is_err() {
            count(C_HELPER_FAIL);
            return 0;
        }
        return redirect(&c, &mut target);
    }
    if mode == 5 {
        // strparser verdict on the host stream cell: the parser already cut the frame, and the redirect
        // sends exactly [strp offset, strp full_len). ctx.len() is the accumulated skb (may reach into the
        // next frame), so the ADR-0149 "n - 8 == declared" test is not applicable here.
        if n < 8 {
            count(C_RX_LT8);
            return 0;
        }
        let Ok(h) = ctx.load::<[u8; 8]>(0) else {
            count(C_HELPER_FAIL);
            return 0;
        };
        if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3] {
            count(C_RX_BADMAGIC);
            return 0;
        }
        let frame = 8 + u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
        if frame > n {
            count(C_RX_SHORT);
            return 0;
        }
        count(C_RX_OK);
        if ctx.pull_data(frame).is_err() {
            count(C_HELPER_FAIL);
            return 0;
        }
        return redirect(&ctx, &mut target);
    }
    if mode == 2 {
        let enforce = cfg(1) != 0;
        let mut bad = false;
        if n < 8 {
            count(C_RX_LT8);
            evt(3, n, 0, 0, u64::MAX);
            bad = true;
        } else {
            let Ok(h) = ctx.load::<[u8; 8]>(0) else {
                count(C_HELPER_FAIL);
                return 0;
            };
            if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3] {
                count(C_RX_BADMAGIC);
                evt(4, n, 0, 0, u64::MAX);
                bad = true;
            } else {
                let declared = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
                let seq = if n >= 16 {
                    match ctx.load::<[u8; 8]>(8) {
                        Ok(s) => u64::from_be_bytes(s),
                        Err(_) => u64::MAX,
                    }
                } else {
                    u64::MAX
                };
                let body = n - 8;
                if body == declared {
                    count(C_RX_OK);
                } else if body < declared {
                    count(C_RX_SHORT);
                    evt(1, n, declared, 0, seq);
                    bad = true;
                } else {
                    count(C_RX_LONG);
                    let off = declared as u64 + 8;
                    let mut second = 0u32;
                    if off + 4 <= n as u64 {
                        if let Ok(m) = ctx.load::<[u8; 4]>(off as usize) {
                            if m == MAGIC {
                                second = 1;
                                count(C_RX_LONG_2MAGIC);
                            }
                        }
                    }
                    evt(2, n, declared, second, seq);
                    bad = true;
                }
            }
        }
        if bad {
            if enforce {
                return 0;
            }
            count(C_RX_PASSED_BAD);
        }
    }
    if mode != 0 && ctx.pull_data(n).is_err() {
        count(C_HELPER_FAIL);
        return 0;
    }
    redirect(&ctx, &mut target)
}

#[inline(always)]
fn redirect(ctx: &SkBuffContext, target: &mut u64) -> u32 {
    let r = SOCKS.redirect_skb(ctx, target, 0) as u32;
    if r == 0 {
        count(C_REDIR_FAIL);
    } else {
        count(C_REDIR_OK);
    }
    r
}

// ------------------------------------------------------------------ guest: outcome of every skb_send_sock
// on the tracked vsock leg: key 1 = full, 2 = partial (0 < ret < len), 100+errno = error, 99 = zero.
#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    let sk: *const core::ffi::c_void = unsafe { ctx.arg(0) };
    let len: i32 = unsafe { ctx.arg(3) };
    let ret: i32 = unsafe { ctx.arg(4) };
    let cookie = unsafe { bpf_get_socket_cookie(sk as *mut _) };
    if unsafe { TRACK.get(&cookie) }.is_none() {
        return 0;
    }
    count(C_FX_CALLS);
    let key: u32 = if ret > 0 {
        add(C_FX_BYTES, ret as u64);
        if ret == len {
            1
        } else {
            2
        }
    } else if ret == 0 {
        99
    } else {
        100 + (-ret) as u32
    };
    match ERRS.get_ptr_mut(&key) {
        Some(p) => unsafe { *p += 1 },
        None => {
            let _ = ERRS.insert(&key, &1u64, 0);
        }
    }
    0
}

// ------------------------------------------------------------------ guest: packets allocated for the data port
// tracepoint layout (8-byte common header): src_cid@8 src_port@12 dst_cid@16 dst_port@20 len@24 type@28(u16)
// op@30(u16) flags@32 ; recv_pkt adds buf_alloc@36 fwd_cnt@40.   op 5 = RW, type 1 = STREAM, 2 = SEQPACKET.
#[tracepoint(category = "vsock", name = "virtio_transport_alloc_pkt")]
pub fn tp_alloc(ctx: TracePointContext) -> u32 {
    unsafe {
        let op: u16 = ctx.read_at(30).unwrap_or(0);
        if op != 5 {
            return 0;
        }
        let dport: u32 = ctx.read_at(20).unwrap_or(0);
        if dport != cfg(2) {
            return 0;
        }
        let ty: u16 = ctx.read_at(28).unwrap_or(0);
        let flags: u32 = ctx.read_at(32).unwrap_or(0);
        let len: u32 = ctx.read_at(24).unwrap_or(0);
        if ty == 2 {
            if flags & 1 != 0 {
                count(C_TXA_SEQ_EOM);
                add(C_TXA_SEQ_EOM_BYTES, len as u64);
            } else {
                count(C_TXA_SEQ_NOEOM);
                add(C_TXA_SEQ_NOEOM_BYTES, len as u64);
            }
        } else {
            count(C_TXA_STREAM);
            add(C_TXA_STREAM_BYTES, len as u64);
        }
    }
    0
}

#[tracepoint(category = "vsock", name = "virtio_transport_recv_pkt")]
pub fn tp_recv(ctx: TracePointContext) -> u32 {
    unsafe {
        let op: u16 = ctx.read_at(30).unwrap_or(0);
        let dport: u32 = ctx.read_at(20).unwrap_or(0);
        if dport != cfg(2) {
            return 0;
        }
        let buf: u32 = ctx.read_at(36).unwrap_or(0);
        if let Some(p) = CFG.get_ptr_mut(3) {
            *p = buf;
        }
        if op != 5 {
            return 0;
        }
        let ty: u16 = ctx.read_at(28).unwrap_or(0);
        let flags: u32 = ctx.read_at(32).unwrap_or(0);
        let len: u32 = ctx.read_at(24).unwrap_or(0);
        if ty == 2 {
            if flags & 1 != 0 {
                count(C_RXP_SEQ_EOM);
                add(C_RXP_SEQ_EOM_BYTES, len as u64);
            } else {
                count(C_RXP_SEQ_NOEOM);
                add(C_RXP_SEQ_NOEOM_BYTES, len as u64);
            }
        } else {
            count(C_RXP_STREAM);
            add(C_RXP_STREAM_BYTES, len as u64);
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
