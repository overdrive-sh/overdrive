//! Workload listener observation for D13.

use aya_ebpf::{
    macros::{cgroup_sock, map, sock_ops},
    maps::{PerCpuArray, RingBuf},
    programs::{SockContext, SockOpsContext},
};

const IPPROTO_TCP: u32 = 6;
const IPPROTO_UDP: u32 = 17;
const AF_INET: u32 = 2;
const AF_INET6: u32 = 10;
const BPF_SOCK_OPS_TCP_LISTEN_CB: u32 = aya_ebpf::bindings::BPF_SOCK_OPS_TCP_LISTEN_CB;
const DROP_COUNT: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ListenEvent {
    pub protocol: u32,
    /// Host byte order.
    pub port: u32,
}

#[map]
pub static LISTEN_EVENTS: RingBuf = RingBuf::with_byte_size(1 << 20, 0);

#[map]
pub static LISTEN_COUNTERS: PerCpuArray<u64> = PerCpuArray::with_max_entries(1, 0);

#[inline(always)]
fn count_drop() {
    if let Some(counter) = LISTEN_COUNTERS.get_ptr_mut(DROP_COUNT) {
        unsafe { *counter += 1 };
    }
}

#[inline(always)]
fn emit(protocol: u32, port: u32) {
    let event = ListenEvent { protocol, port };
    if LISTEN_EVENTS.output(&event, 0).is_err() {
        count_drop();
    }
}

#[inline(always)]
fn is_loopback4(address: u32) -> bool {
    // The address word is in network byte order. On bpfel, its first octet is
    // the low byte of the loaded word.
    address & 0xff == 127
}

#[inline(always)]
fn is_loopback6(word0: u32, word1: u32, word2: u32, word3: u32) -> bool {
    let loopback = word0 == 0 && word1 == 0 && word2 == 0 && word3 == 0x0100_0000;
    let mapped_loopback = word0 == 0 && word1 == 0 && word2 == 0xffff_0000 && word3 & 0xff == 127;
    loopback || mapped_loopback
}

#[sock_ops]
pub fn listen_tcp(ctx: SockOpsContext) -> u32 {
    if ctx.op() == BPF_SOCK_OPS_TCP_LISTEN_CB {
        let ops = unsafe { &*ctx.ops };
        let address0 = ops.local_ip6[0];
        let address1 = ops.local_ip6[1];
        let address2 = ops.local_ip6[2];
        let address3 = ops.local_ip6[3];
        let loopback = match ops.family {
            AF_INET => is_loopback4(ops.local_ip4),
            AF_INET6 => is_loopback6(address0, address1, address2, address3),
            _ => false,
        };
        if !loopback {
            emit(IPPROTO_TCP, ops.local_port);
        }
    }
    1
}

#[cgroup_sock(post_bind4)]
pub fn post_bind4(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    if sock.protocol == IPPROTO_UDP && !is_loopback4(sock.src_ip4) {
        emit(IPPROTO_UDP, sock.src_port);
    }
    1
}

#[cgroup_sock(post_bind6)]
pub fn post_bind6(ctx: SockContext) -> i32 {
    let sock = unsafe { &*ctx.sock };
    if sock.protocol == IPPROTO_UDP {
        let address0 = sock.src_ip6[0];
        let address1 = sock.src_ip6[1];
        let address2 = sock.src_ip6[2];
        let address3 = sock.src_ip6[3];
        if !is_loopback6(address0, address1, address2, address3) {
            emit(IPPROTO_UDP, sock.src_port);
        }
    }
    1
}
