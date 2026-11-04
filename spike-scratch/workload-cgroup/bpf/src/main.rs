//! workload-cgroup spike (V42): the small listen object PR-2 attaches to the workload cgroup.
//! One sockops program (TCP_LISTEN_CB arm only) and two CgroupSock post_bind programs that
//! filter to protocol 17 and always return 1. The ring, the counters, the event type and the
//! TCP report are the listen-hooks spike's `listen.rs`, reused by path (not copied).
#![no_std]
#![no_main]
#![allow(clippy::all, dead_code)]

#[cfg(feature = "plus")]
#[path = "../../../listen-hooks/bpf/src/guest_capture.rs"]
mod guest_capture;
#[cfg(feature = "plus")]
#[path = "../../../listen-hooks/bpf/src/tcp_correlation.rs"]
mod tcp_correlation;
#[path = "../../../listen-hooks/bpf/src/listen.rs"]
mod listen;

use aya_ebpf::{
    helpers::bpf_get_current_pid_tgid,
    macros::{cgroup_sock, map, sock_ops},
    maps::PerCpuArray,
    programs::{SockContext, SockOpsContext},
};
use listen::{KIND_UDP_BIND, LISTEN_EVENTS, ListenEvent};

/// [0] = UDP reports emitted, [1] = UDP reports dropped because the ring was full.
#[map]
pub static WC_UDP: PerCpuArray<u64> = PerCpuArray::with_max_entries(2, 0);

#[inline(always)]
fn bump_udp(i: u32) {
    if let Some(slot) = WC_UDP.get_ptr_mut(i) {
        unsafe { *slot += 1 };
    }
}

#[sock_ops]
pub fn wc_listen(ctx: SockOpsContext) -> u32 {
    if ctx.op() == aya_ebpf::bindings::BPF_SOCK_OPS_TCP_LISTEN_CB {
        listen::report_tcp_listen(&ctx);
    }
    1
}

#[inline(always)]
fn emit_udp(sock: &aya_ebpf::bindings::bpf_sock, addr4: u32, addr6: [u32; 4]) {
    let event = ListenEvent {
        kind: KIND_UDP_BIND,
        family: sock.family,
        port: sock.src_port,
        pid: (bpf_get_current_pid_tgid() >> 32) as u32,
        addr4,
        addr6,
        protocol: sock.protocol,
        sock_type: sock.type_,
        aux: 0,
    };
    bump_udp(0);
    if LISTEN_EVENTS.output(&event, 0).is_err() {
        bump_udp(1);
    }
}

#[cgroup_sock(post_bind4)]
pub fn wc_post_bind4(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    if sock.protocol == 17 {
        emit_udp(sock, sock.src_ip4, [0; 4]);
    }
    1
}

#[cgroup_sock(post_bind6)]
pub fn wc_post_bind6(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    if sock.protocol == 17 {
        let a = [sock.src_ip6[0], sock.src_ip6[1], sock.src_ip6[2], sock.src_ip6[3]];
        emit_udp(sock, 0, a);
    }
    1
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
