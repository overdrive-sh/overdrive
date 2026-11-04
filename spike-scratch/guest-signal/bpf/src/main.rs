//! guest-signal spike: the listen hooks as init would attach them at the cgroup2 root.
//! sockops TCP_LISTEN_CB -> ringbuf event(kind=1); cgroup_sock post_bind4/6 (UDP only) -> event(kind=2).
#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::r#gen::bpf_ktime_get_ns,
    macros::{cgroup_sock, map, sock_ops},
    maps::RingBuf,
    programs::{SockContext, SockOpsContext},
};

const BPF_SOCK_OPS_TCP_LISTEN_CB: u32 = 11;
const IPPROTO_UDP: u32 = 17;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Event {
    pub kind: u32, // 1 = tcp listen, 2 = udp bind
    pub port: u32, // host order
    pub ktime_ns: u64,
}

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(65_536, 0);

#[inline(always)]
fn emit(kind: u32, port: u32) {
    let ev = Event { kind, port, ktime_ns: unsafe { bpf_ktime_get_ns() } };
    let _ = EVENTS.output(&ev, 0);
}

#[sock_ops]
pub fn listen_cb(ctx: SockOpsContext) -> u32 {
    if ctx.op() == BPF_SOCK_OPS_TCP_LISTEN_CB {
        emit(1, ctx.local_port());
    }
    0
}

#[inline(always)]
fn post_bind(ctx: &SockContext) -> i32 {
    let (proto, port) = unsafe { ((*ctx.sock).protocol, (*ctx.sock).src_port) };
    if proto == IPPROTO_UDP {
        emit(2, port);
    }
    1
}

#[cgroup_sock(post_bind4)]
pub fn post_bind4(ctx: SockContext) -> i32 {
    post_bind(&ctx)
}

#[cgroup_sock(post_bind6)]
pub fn post_bind6(ctx: SockContext) -> i32 {
    post_bind(&ctx)
}

#[panic_handler]
fn panic(_i: &core::panic::PanicInfo) -> ! {
    loop {}
}
