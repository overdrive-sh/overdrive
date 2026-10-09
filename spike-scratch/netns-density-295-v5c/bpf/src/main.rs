//! Throwaway spike (netns-density-295, V-11 / VIP / V-14). Aya Rust eBPF only.
//! Derived from spike-scratch/netns-density-295-guest-vsock-capture/bpf (reused, extended):
//! * forward modes 3/5/6: unframe in the host verdict (D5a option b / hybrid);
//! * eg_decode_frag: fragment-aware tuple-gated unframe for host egress interfaces (D5a a2);
//! * vip_connect4: mirror of production cgroup_connect4_service (ADR-0053) on the owner cgroup;
//! * listen_start / listen_stop: fexit listen-state wake events to a ring buffer (V-14).
//!
//! One ELF, loaded by both owners:
//! * guest (stand-in for overdrive-init): cgroup sock_addr capture hooks, sock_ops,
//!   sock_release, SK_SKB verdicts, strparser parking cell, TCX frame decode on lo,
//!   fexit drain counter.
//! * host (guest-flow owner stand-in): SK_SKB verdicts, TCX frame decode on lo,
//!   fexit drain counter. The host never loads the cgroup/sock_ops programs.
//!
//! Payload only ever moves via SK_SKB redirect to a destination socket's egress.
#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{TC_ACT_OK, TC_ACT_SHOT},
    helpers::{bpf_get_socket_cookie, bpf_ktime_get_ns, bpf_probe_read_kernel, bpf_skb_adjust_room},
    macros::{cgroup_sock, cgroup_sock_addr, classifier, fexit, map, sock_ops, stream_parser, stream_verdict},
    maps::{Array, HashMap, PerCpuArray, PerCpuHashMap, Queue, RingBuf, SockHash},
    programs::{FExitContext, SkBuffContext, SockAddrContext, SockContext, SockOpsContext, TcContext},
};

pub const MAGIC: [u8; 4] = *b"ZUD1";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Route {
    pub target: u64,
    pub mode: u32, // 0 plain, 1 encode (+8 frame), 2 check frame
    pub pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Dst {
    pub ip: u32,   // network order
    pub port: u32, // host order
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SlotKey {
    pub cookie: u64,
    pub ip: u32,
    pub port: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Slot {
    pub cookie: u64,
    pub ip: u32,       // original destination, network order
    pub port: u32,     // original destination port, host order
    pub app_port: u32, // app source port seen on lo (host order), 0 = unknown
    pub gen: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Tuple {
    pub src: u32,
    pub dst: u32,
    pub sport: u16,
    pub dport: u16,
}

// ---- forwarding maps (both sides)
#[map]
static SOCKS: SockHash<u64> = SockHash::with_max_entries(65536, 0); // redirect targets, no programs
#[map]
static VERD: SockHash<u64> = SockHash::with_max_entries(65536, 0); // stream_verdict attached
#[map]
static STRP: SockHash<u64> = SockHash::with_max_entries(4096, 0); // parser + verdict attached
#[map]
static ROUTE: HashMap<u64, Route> = HashMap::with_max_entries(65536, 0);
// K2 accounting. NOT LRU: an evicted live counter breaks the drain check (increment-g).
// The owner deletes a socket's entries when it closes the socket.
#[map]
static FWD: PerCpuHashMap<u64, u64> = PerCpuHashMap::with_max_entries(262144, 0);
#[map]
static SENT: PerCpuHashMap<u64, u64> = PerCpuHashMap::with_max_entries(262144, 0);
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(64, 0);
#[map]
static TUPLES: HashMap<Tuple, u32> = HashMap::with_max_entries(4096, 0);

// ---- guest capture maps
// CFG: 0 workload ip (NBO), 1 tcp intake port, 2 udp base port, 3 udp slots,
//      4 dns ip (NBO), 5 dns port, 6 capture enabled
#[map]
static CFG: Array<u32> = Array::with_max_entries(8, 0);
#[map]
static ORIG: HashMap<u64, Dst> = HashMap::with_max_entries(65536, 0);
#[map]
static FLOWP: HashMap<u32, Dst> = HashMap::with_max_entries(65536, 0);
#[map]
static SLOTC: HashMap<SlotKey, u32> = HashMap::with_max_entries(4096, 0);
#[map]
static COOKIE_SLOT: HashMap<u64, u32> = HashMap::with_max_entries(4096, 0);
#[map]
static SLOT: Array<Slot> = Array::with_max_entries(256, 0);
#[map]
static FREE: Queue<u32> = Queue::with_max_entries(256, 0);
#[map]
static RELEASED: Queue<u32> = Queue::with_max_entries(1024, 0);

// ---- establishment-time install (sock_ops), both sides
// CFG 7: park passive intake children at PASSIVE_ESTABLISHED
#[map]
static INTAKE: HashMap<u32, u32> = HashMap::with_max_entries(64, 0); // local port (host order) -> 1
#[map]
static TCPPARK: Array<u64> = Array::with_max_entries(1024, 0); // park slot -> P1 cookie
#[map]
static TCPFREE: Queue<u32> = Queue::with_max_entries(1024, 0);
#[map]
static PARK_OF: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0); // intake child cookie -> slot
#[map]
static ARM: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0); // active socket cookie armed for install

// counter indices
const C_TCP_CAPTURE: u32 = 0;
const C_UDP_CAPTURE: u32 = 1;
const C_UDP_NOSLOT: u32 = 2;
const C_PEER_REWRITE: u32 = 3;
const C_RECV_REWRITE: u32 = 4;
const C_SOCKOPS_FLOW: u32 = 5;
const C_NOROUTE: u32 = 8;
const C_HELPER_FAIL: u32 = 9;
const C_BAD_FRAME: u32 = 10;
const C_REDIRECT_FAIL: u32 = 11;
const C_ENCODED: u32 = 12;
const C_TC_DECODED: u32 = 13;
const C_ENCODED_EMPTY: u32 = 14;
const C_TC_DECODED_EMPTY: u32 = 15;
const C_REDIRECT_OK: u32 = 16;
const C_STRP_FRAMES: u32 = 17;
const C_RELEASE: u32 = 18;
const C_TC_ERR: u32 = 20;
const C_TC_APPPORT: u32 = 21;
const C_SENT_CALLS: u32 = 22;
const C_ZERO_LEN_DROP: u32 = 23;
const C_PARKED: u32 = 24;
const C_PARK_EMPTY: u32 = 25;
const C_ARMED_INSTALL: u32 = 26;
const C_SOCKOPS_INSERT_FAIL: u32 = 27;
const C_VSTRIP: u32 = 28; // verdict stripped a nonempty frame
const C_VSTRIP_EMPTY_DROP: u32 = 29; // mode 3: empty datagram dropped (cannot be sent from a verdict)
const C_VSTRIP_EMPTY_FWD: u32 = 30; // mode 5: zero-length skb redirected anyway
const C_HYBRID_EMPTY_FRAMED: u32 = 31; // mode 6: empty passed framed to the egress unframe
const C_FRAG_FIRST: u32 = 32; // eg_decode_frag: first fragment unframed
const C_FRAG_SHIFT: u32 = 33; // eg_decode_frag: later fragment offset shifted by 8 bytes
const C_VIP_HIT: u32 = 34;
const C_LISTEN_EV: u32 = 35;
const C_LISTEN_EV_LOST: u32 = 36;
const C_TC_ERR_FRAG: u32 = 37;
const C_HYBRID_ESCAPED: u32 = 38; // mode 6: payload itself parses as a frame -> kept framed
const C_TC_LENIENT_PASS: u32 = 39; // tuple mode 3: unframed datagram passed untouched

// ---- V-11 fragment tracking: (src, dst, ip id) of a datagram whose first fragment lost 8 bytes
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FragKey {
    pub src: u32,
    pub dst: u32,
    pub id: u32,
}
#[map]
static FRAGS: HashMap<FragKey, u32> = HashMap::with_max_entries(4096, 0);

// ---- VIP mirror (ADR-0053 LOCAL_BACKEND_MAP layout, host order)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LocalServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LocalBackendEntry {
    pub backend_ip_host: u32,
    pub backend_port_host: u16,
    pub _pad: u16,
}
#[map]
static VIPMAP: HashMap<LocalServiceKey, LocalBackendEntry> = HashMap::with_max_entries(4096, 0);

// ---- V-14 listen-state wake events
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ListenEv {
    pub kind: u32, // 1 listen started, 2 listen stopped
    pub port: u32, // sock_common.skc_num (host order)
    pub cookie: u64,
    pub ktime: u64,
    pub family: u32, // sock_common.skc_family
    pub saddr: u32,  // sock_common.skc_rcv_saddr (network order; IPv4 only)
}
#[map]
static LEV: RingBuf = RingBuf::with_byte_size(1 << 16, 0);
// V-14 "map" mode: the kernel-maintained set of reachable listening sockets (cookie -> port).
// The ring buffer is only a wake hint; a lost record cannot lose state.
#[map]
static LISTENERS: HashMap<u64, u32> = HashMap::with_max_entries(8192, 0);
const C_LISTENERS_FULL: u32 = 40;

#[inline(always)]
fn count(i: u32) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) }
    }
}

#[inline(always)]
fn cfg(i: u32) -> u32 {
    CFG.get(i).copied().unwrap_or(0)
}

#[inline(always)]
fn add(map: &PerCpuHashMap<u64, u64>, k: u64, v: u64) {
    match map.get_ptr_mut(&k) {
        Some(p) => unsafe { *p = (*p).wrapping_add(v) },
        None => {
            let _ = map.insert(&k, &v, 0);
        }
    }
}

// ------------------------------------------------------------------ capture hooks
#[cgroup_sock_addr(connect4)]
pub fn connect4(ctx: SockAddrContext) -> i32 {
    capture(&ctx, false)
}

#[cgroup_sock_addr(sendmsg4)]
pub fn sendmsg4(ctx: SockAddrContext) -> i32 {
    capture(&ctx, true)
}

#[inline(always)]
fn capture(ctx: &SockAddrContext, sendmsg: bool) -> i32 {
    if cfg(6) == 0 {
        return 1;
    }
    let sa = unsafe { &mut *ctx.sock_addr };
    let w = cfg(0);
    let ip = sa.user_ip4;
    let port = u16::from_be(sa.user_port as u16) as u32;
    let dns = ip == cfg(4) && port == cfg(5);
    let first = (u32::from_be(ip) >> 24) as u8;
    if !dns && (first == 127 || first == 0 || ip == w) {
        return 1;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr as *mut _) };
    let proto = sa.protocol;
    if proto == 6 {
        if sendmsg {
            return 1;
        }
        if ORIG.insert(&cookie, &Dst { ip, port }, 0).is_err() {
            return 0;
        }
        sa.user_ip4 = w;
        sa.user_port = u32::from((cfg(1) as u16).to_be());
        count(C_TCP_CAPTURE);
        return 1;
    }
    if proto == 17 {
        let key = SlotKey { cookie, ip, port };
        let slot = match unsafe { SLOTC.get(&key) } {
            Some(s) => *s,
            None => {
                let Some(s) = FREE.pop() else {
                    count(C_UDP_NOSLOT);
                    return 0;
                };
                if let Some(p) = SLOT.get_ptr_mut(s) {
                    unsafe {
                        (*p).cookie = cookie;
                        (*p).ip = ip;
                        (*p).port = port;
                        (*p).app_port = 0;
                        (*p).gen = (*p).gen.wrapping_add(1);
                    }
                }
                let _ = SLOTC.insert(&key, &s, 0);
                let _ = COOKIE_SLOT.insert(&cookie, &s, 0);
                s
            }
        };
        sa.user_ip4 = w;
        sa.user_port = u32::from(((cfg(2) + slot) as u16).to_be());
        count(C_UDP_CAPTURE);
        return 1;
    }
    1
}

#[inline(always)]
fn slot_of(ip: u32, port: u32) -> Option<u32> {
    let base = cfg(2);
    if ip == cfg(0) && port >= base && port < base + cfg(3) {
        Some(port - base)
    } else {
        None
    }
}

#[cgroup_sock_addr(recvmsg4)]
pub fn recvmsg4(ctx: SockAddrContext) -> i32 {
    let sa = unsafe { &mut *ctx.sock_addr };
    let port = u16::from_be(sa.user_port as u16) as u32;
    if let Some(s) = slot_of(sa.user_ip4, port) {
        if let Some(d) = SLOT.get(s) {
            sa.user_ip4 = d.ip;
            sa.user_port = u32::from((d.port as u16).to_be());
            count(C_RECV_REWRITE);
        }
    }
    1
}

#[cgroup_sock_addr(getpeername4)]
pub fn getpeername4(ctx: SockAddrContext) -> i32 {
    let sa = unsafe { &mut *ctx.sock_addr };
    let port = u16::from_be(sa.user_port as u16) as u32;
    if sa.user_ip4 == cfg(0) && port == cfg(1) {
        let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr as *mut _) };
        if let Some(d) = unsafe { ORIG.get(&cookie) } {
            sa.user_ip4 = d.ip;
            sa.user_port = u32::from((d.port as u16).to_be());
            count(C_PEER_REWRITE);
        }
    } else if let Some(s) = slot_of(sa.user_ip4, port) {
        if let Some(d) = SLOT.get(s) {
            sa.user_ip4 = d.ip;
            sa.user_port = u32::from((d.port as u16).to_be());
            count(C_PEER_REWRITE);
        }
    }
    1
}

#[cgroup_sock(sock_release)]
pub fn sock_release(ctx: SockContext) -> i32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock as *mut _) };
    let _ = ORIG.remove(&cookie);
    if let Some(s) = unsafe { COOKIE_SLOT.get(&cookie) } {
        let s = *s;
        if let Some(d) = SLOT.get(s) {
            let _ = SLOTC.remove(&SlotKey { cookie, ip: d.ip, port: d.port });
        }
        let _ = COOKIE_SLOT.remove(&cookie);
        let _ = RELEASED.push(&s, 0);
        count(C_RELEASE);
    }
    1
}

#[sock_ops]
pub fn flow_sockops(ctx: SockOpsContext) -> u32 {
    let op = ctx.op();
    // BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB = 4, BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB = 5
    if op != 4 && op != 5 {
        return 1;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.ops as *mut _) };
    if op == 4 {
        if let Some(d) = unsafe { ORIG.get(&cookie) } {
            let lp = ctx.local_port();
            let _ = FLOWP.insert(&lp, d, 0);
            count(C_SOCKOPS_FLOW);
        }
        if unsafe { ARM.get(&cookie) }.is_some() {
            install(&ctx, cookie);
            count(C_ARMED_INSTALL);
        }
    } else if cfg(7) != 0 && unsafe { INTAKE.get(&ctx.local_port()) }.is_some() {
        // Intake child: install at establishment, before any data or FIN can be
        // queued, and route its bytes into a pre-made kernel TCP parking cell.
        let Some(slot) = TCPFREE.pop() else {
            count(C_PARK_EMPTY);
            return 1;
        };
        let p1 = TCPPARK.get(slot).copied().unwrap_or(0);
        let _ = ROUTE.insert(&cookie, &Route { target: p1, mode: 0, pad: 0 }, 0);
        let _ = PARK_OF.insert(&cookie, &slot, 0);
        install(&ctx, cookie);
        count(C_PARKED);
    }
    1
}

#[inline(always)]
fn install(ctx: &SockOpsContext, cookie: u64) {
    let ops = unsafe { &mut *ctx.ops };
    let mut k = cookie;
    if SOCKS.update(&mut k, ops, 0).is_err() {
        count(C_SOCKOPS_INSERT_FAIL);
    }
    let mut k = cookie;
    if VERD.update(&mut k, ops, 0).is_err() {
        count(C_SOCKOPS_INSERT_FAIL);
    }
}

// ------------------------------------------------------------------ SK_SKB forwarding
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
    if ctx.len() < 8 {
        return 0;
    }
    let Ok(h) = ctx.load::<[u8; 8]>(0) else { return 0 };
    if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3] {
        count(C_BAD_FRAME);
        return u32::MAX; // -1: abort the stream (never expected)
    }
    8 + u32::from_be_bytes([h[4], h[5], h[6], h[7]])
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
    if n == 0 && (mode == 0 || mode == 4) {
        // A TCP FIN-only skb: redirecting it to egress makes the target psock
        // report EPIPE and stop TX (skb_send_sock returns 0). FIN is propagated
        // by the owner with shutdown() after drain.
        count(C_ZERO_LEN_DROP);
        return 0;
    }
    add(&FWD, cookie, n as u64);
    if mode == 1 {
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
        count(C_ENCODED);
        if n == 0 {
            count(C_ENCODED_EMPTY);
        }
        if c.pull_data(c.len()).is_err() {
            count(C_HELPER_FAIL);
            return 0;
        }
        return redirect(&c, &mut target);
    }
    if mode == 2 {
        if n < 8 {
            count(C_BAD_FRAME);
            return 0;
        }
        let Ok(h) = ctx.load::<[u8; 8]>(0) else {
            count(C_BAD_FRAME);
            return 0;
        };
        if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3]
            || u32::from_be_bytes([h[4], h[5], h[6], h[7]]) != n - 8
        {
            count(C_BAD_FRAME);
            return 0;
        }
    }
    if mode == 3 || mode == 5 || mode == 6 {
        // D5a (b): verify the frame, remove it here, send the bare datagram.
        if n < 8 {
            count(C_BAD_FRAME);
            return 0;
        }
        let Ok(h) = ctx.load::<[u8; 8]>(0) else {
            count(C_BAD_FRAME);
            return 0;
        };
        if h[0] != MAGIC[0] || h[1] != MAGIC[1] || h[2] != MAGIC[2] || h[3] != MAGIC[3]
            || u32::from_be_bytes([h[4], h[5], h[6], h[7]]) != n - 8
        {
            count(C_BAD_FRAME);
            return 0;
        }
        if n == 8 {
            if mode == 3 {
                count(C_VSTRIP_EMPTY_DROP);
                return 0;
            }
            if mode == 6 {
                // hybrid: an empty datagram stays framed (8 bytes, never fragmented);
                // the tuple-gated egress unframe turns it into a zero-length datagram.
                count(C_HYBRID_EMPTY_FRAMED);
                if ctx.pull_data(n).is_err() {
                    count(C_HELPER_FAIL);
                    return 0;
                }
                return redirect(&ctx, &mut target);
            }
            count(C_VSTRIP_EMPTY_FWD);
        }
        if mode == 6 && n >= 16 {
            // Escape: if the application payload itself parses as a frame, keep our frame so
            // the egress unframe (which strips any valid frame on a hybrid tuple) removes
            // exactly one layer. Every frame on the wire is then one we added.
            if let Ok(q) = ctx.load::<[u8; 8]>(8) {
                if q[0] == MAGIC[0] && q[1] == MAGIC[1] && q[2] == MAGIC[2] && q[3] == MAGIC[3]
                    && u32::from_be_bytes([q[4], q[5], q[6], q[7]]) == n - 16
                {
                    count(C_HYBRID_ESCAPED);
                    if ctx.pull_data(n).is_err() {
                        count(C_HELPER_FAIL);
                        return 0;
                    }
                    return redirect(&ctx, &mut target);
                }
            }
        }
        if unsafe { bpf_skb_adjust_room(ctx.skb.skb, -8, 0, 0) } != 0 {
            count(C_HELPER_FAIL);
            return 0;
        }
        let c = ctx;
        if c.len() > 0 && c.pull_data(c.len()).is_err() {
            count(C_HELPER_FAIL);
            return 0;
        }
        count(C_VSTRIP);
        return redirect(&c, &mut target);
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
        count(C_REDIRECT_FAIL);
    } else {
        count(C_REDIRECT_OK);
    }
    r
}

// ------------------------------------------------------------------ K2 drain signal
#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    let sk: *const core::ffi::c_void = unsafe { ctx.arg(0) };
    let ret: i32 = unsafe { ctx.arg(4) };
    if ret > 0 {
        let cookie = unsafe { bpf_get_socket_cookie(sk as *mut _) };
        add(&SENT, cookie, ret as u64);
        count(C_SENT_CALLS);
    }
    0
}

// ------------------------------------------------------------------ TCX decode on lo
#[classifier]
pub fn lo_decode(ctx: TcContext) -> i32 {
    match decode(ctx) {
        Ok(x) => x,
        Err(_) => {
            count(C_TC_ERR);
            TC_ACT_SHOT
        }
    }
}

fn decode(mut ctx: TcContext) -> Result<i32, i64> {
    let ip = 14usize;
    let udp = 34usize;
    if ctx.len() < 42 || ctx.load::<u8>(ip).unwrap_or(0) != 0x45 || ctx.load::<u8>(ip + 9).unwrap_or(0) != 17 {
        return Ok(TC_ACT_OK);
    }
    let src = ctx.load::<u32>(ip + 12)?;
    let dst = ctx.load::<u32>(ip + 16)?;
    let sport = ctx.load::<u16>(udp)?;
    let dport = ctx.load::<u16>(udp + 2)?;
    // guest: record the application port that addressed a UDP slot (no payload access)
    if cfg(6) != 0 {
        if let Some(s) = slot_of(dst, u16::from_be(dport) as u32) {
            if let Some(p) = SLOT.get_ptr_mut(s) {
                unsafe {
                    if (*p).app_port != u16::from_be(sport) as u32 {
                        (*p).app_port = u16::from_be(sport) as u32;
                        count(C_TC_APPPORT);
                    }
                }
            }
            return Ok(TC_ACT_OK);
        }
    }
    let tuple = Tuple { src, dst, sport, dport };
    let Some(mode) = (unsafe { TUPLES.get(&tuple).copied() }) else { return Ok(TC_ACT_OK) };
    if mode != 2 && mode != 3 {
        return Ok(TC_ACT_OK);
    }
    // mode 2: every datagram on the tuple is framed (strict). mode 3 (hybrid): only frames
    // we kept are framed; anything that is not a valid frame passes untouched.
    let lenient = mode == 3;
    let old_ip = ctx.load::<u16>(ip + 2)?;
    let old_udp = ctx.load::<u16>(udp + 4)?;
    let plen = u16::from_be(old_udp) as u32;
    let ilen = u16::from_be(old_ip) as u32;
    if plen < 16 || ilen != plen + 20 || ctx.len() != ilen + 14 {
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
    ctx.adjust_room(-8, 0, 0)?;
    ctx.store(udp, &udp_header, 0)?;
    let new_ip = (ilen - 8) as u16;
    let new_udp = (plen - 8) as u16;
    ctx.l3_csum_replace(ip + 10, old_ip as u64, new_ip.to_be() as u64, 2)?;
    ctx.store(ip + 2, &new_ip.to_be(), 0)?;
    ctx.store(udp + 4, &new_udp.to_be(), 0)?;
    ctx.store(udp + 6, &0u16, 0)?;
    count(C_TC_DECODED);
    if logical == 0 {
        count(C_TC_DECODED_EMPTY);
    }
    Ok(TC_ACT_OK)
}

// ------------------------------------------------------------------ D5a (a2): fragment-aware egress unframe
// Tuple-gated like lo_decode. A framed datagram larger than the egress MTU is fragmented
// by ip_fragment() BEFORE TC egress, so the first fragment carries UDP header + frame and
// later fragments carry neither. Remove the 8 frame bytes from the first fragment and move
// every later fragment of the same (src, dst, id) back by one 8-byte offset unit.
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
    count(C_TC_DECODED);
    if logical == 0 {
        count(C_TC_DECODED_EMPTY);
    }
    Ok(TC_ACT_OK)
}

// ------------------------------------------------------------------ VIP (ADR-0053 mirror)
// Same lookup and rewrite as crates/overdrive-bpf cgroup_connect4_service: key
// (vip host order, port host order, IANA proto), miss -> unchanged, hit -> rewrite. Always 1.
#[cgroup_sock_addr(connect4)]
pub fn vip_connect4(ctx: SockAddrContext) -> i32 {
    let sa = unsafe { &mut *ctx.sock_addr };
    let key = LocalServiceKey {
        vip_host: u32::from_be(sa.user_ip4),
        port_host: u16::from_be(sa.user_port as u16),
        proto: sa.protocol as u8,
        _pad: 0,
    };
    if let Some(e) = unsafe { VIPMAP.get(&key) } {
        sa.user_ip4 = e.backend_ip_host.to_be();
        sa.user_port = u32::from(e.backend_port_host.to_be());
        count(C_VIP_HIT);
    }
    1
}

// ------------------------------------------------------------------ V-14 listen-state wake
// fexit runs after the function body: on listen_start the socket is already hashed in
// the listening table; on listen_stop it has already been unhashed (tcp_set_state(CLOSE)
// runs first in __tcp_close / tcp_disconnect). The event is a wake-up only: the owner
// re-derives the per-port state from the kernel's socket table (level-triggered).
#[inline(always)]
fn listen_ev(sk: *const u8, kind: u32) {
    // struct sock_common: skc_rcv_saddr @4, skc_num @14, skc_family @16 (offsets pinned from
    // this kernel's BTF in the evidence).
    let port = unsafe { bpf_probe_read_kernel(sk.add(14) as *const u16) }.unwrap_or(0) as u32;
    let family = unsafe { bpf_probe_read_kernel(sk.add(16) as *const u16) }.unwrap_or(0) as u32;
    let saddr = unsafe { bpf_probe_read_kernel(sk.add(4) as *const u32) }.unwrap_or(0);
    let cookie = unsafe { bpf_get_socket_cookie(sk as *mut _) };
    let reachable = family == 10 || saddr == 0 || saddr == cfg(0);
    if kind == 1 && reachable {
        if LISTENERS.insert(&cookie, &port, 0).is_err() {
            count(C_LISTENERS_FULL);
        }
    } else if kind == 2 {
        let _ = LISTENERS.remove(&cookie);
    }
    let ev = ListenEv { kind, port, cookie, ktime: unsafe { bpf_ktime_get_ns() }, family, saddr };
    if LEV.output(&ev, 0).is_err() {
        count(C_LISTEN_EV_LOST);
    } else {
        count(C_LISTEN_EV);
    }
}

#[fexit(function = "inet_csk_listen_start")]
pub fn listen_start(ctx: FExitContext) -> i32 {
    let sk: *const u8 = unsafe { ctx.arg(0) };
    let ret: i32 = unsafe { ctx.arg(1) };
    if ret == 0 {
        listen_ev(sk, 1);
    }
    0
}

#[fexit(function = "inet_csk_listen_stop")]
pub fn listen_stop(ctx: FExitContext) -> i32 {
    let sk: *const u8 = unsafe { ctx.arg(0) };
    listen_ev(sk, 2);
    0
}

#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
