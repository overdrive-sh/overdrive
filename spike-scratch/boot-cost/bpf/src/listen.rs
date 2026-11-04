//! Listener observation: TCP via sockops TCP_LISTEN_CB, UDP via post_bind4/6.
use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, r#gen::bpf_getsockopt},
    macros::{cgroup_sock, map},
    maps::{PerCpuArray, RingBuf},
    programs::{SockContext, SockOpsContext},
};

pub const KIND_TCP_LISTEN: u32 = 1;
pub const KIND_UDP_BIND: u32 = 2;
pub const KIND_OTHER_BIND: u32 = 3;

const C_SEEN: u32 = 0;
const C_DROPS: u32 = 1;
const C_LISTEN_ONLY: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ListenEvent {
    pub kind: u32,
    pub family: u32,
    /// Host byte order.
    pub port: u32,
    pub pid: u32,
    /// Network byte order word.
    pub addr4: u32,
    pub addr6: [u32; 4],
    pub protocol: u32,
    pub sock_type: u32,
    /// TCP arm: bpf_getsockopt(IPV6_V6ONLY) return code for AF_INET6, else 0.
    pub aux: u32,
}

#[map]
pub static LISTEN_EVENTS: RingBuf = RingBuf::with_byte_size(1 << 20, 0);
#[map]
pub static LISTEN_COUNTERS: PerCpuArray<u64> = PerCpuArray::with_max_entries(4, 0);

#[inline(always)]
fn bump(index: u32) {
    if let Some(slot) = LISTEN_COUNTERS.get_ptr_mut(index) {
        unsafe { *slot += 1 };
    }
}

#[inline(always)]
fn emit(event: &ListenEvent) {
    bump(C_SEEN);
    if LISTEN_EVENTS.output(event, 0).is_err() {
        bump(C_DROPS);
    }
}

#[inline(always)]
pub fn report_tcp_listen(ctx: &SockOpsContext) {
    let ops = unsafe { &*ctx.ops };
    let mut aux: u32 = 0;
    if ops.family == 10 {
        let mut value: i32 = -1;
        let rc = unsafe {
            bpf_getsockopt(
                ctx.ops.cast(),
                41, /* SOL_IPV6 */
                26, /* IPV6_V6ONLY */
                (&raw mut value).cast(),
                4,
            )
        };
        aux = rc as u32;
    }
    let event = ListenEvent {
        kind: KIND_TCP_LISTEN,
        family: ops.family,
        port: ops.local_port,
        pid: (bpf_get_current_pid_tgid() >> 32) as u32,
        addr4: ops.local_ip4,
        addr6: [ops.local_ip6[0], ops.local_ip6[1], ops.local_ip6[2], ops.local_ip6[3]],
        protocol: 6,
        sock_type: 1,
        aux,
    };
    emit(&event);
}

// src_ip4 is readable only from post_bind4 and src_ip6 only from post_bind6
// (filter.c:8976-8989), so the two programs cannot share one reader.
#[inline(always)]
fn emit_bind(sock: &aya_ebpf::bindings::bpf_sock, addr4: u32, addr6: [u32; 4]) {
    let event = ListenEvent {
        kind: if sock.protocol == 17 { KIND_UDP_BIND } else { KIND_OTHER_BIND },
        family: sock.family,
        port: sock.src_port,
        pid: (bpf_get_current_pid_tgid() >> 32) as u32,
        addr4,
        addr6,
        protocol: sock.protocol,
        sock_type: sock.type_,
        aux: 0,
    };
    emit(&event);
}

// A post_bind program that returns 0 makes bind() fail (af_inet.c:550-556).
#[cgroup_sock(post_bind4)]
pub fn post_bind4(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    emit_bind(sock, sock.src_ip4, [0; 4]);
    1
}

#[cgroup_sock(post_bind6)]
pub fn post_bind6(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    let a = [sock.src_ip6[0], sock.src_ip6[1], sock.src_ip6[2], sock.src_ip6[3]];
    emit_bind(sock, 0, a);
    1
}
