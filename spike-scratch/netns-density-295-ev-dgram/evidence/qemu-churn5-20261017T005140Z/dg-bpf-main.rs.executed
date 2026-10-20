//! V-EV-DGRAM spike (netns-density-295). Aya Rust eBPF only. Models the guest's existing UDP rewrite pair:
//!   cgroup/sendmsg4: an application sending an UNCONNECTED datagram to the original destination O is steered to the slot socket S
//!   cgroup/recvmsg4: the reply's source (S, as the forwarder's send on the slot socket carries it) is rewritten back to O
//! the same shape as crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs (key = datagram source identity, hit rewrites
//! user_ip4/user_port, miss is a pure no-op, return 1: recvmsg4 cannot deny).
//! Plus cgroup_skb/egress that drops UDP to CFG[4] (fallback EPERM source if nft is unavailable in the VM).
//! CFG (u32 values, NETWORK byte order as bpf_sock_addr carries them): 0 O.ip, 1 O.port, 2 S.ip, 3 S.port, 4 egress-drop dport (host order, 0 = off)
#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{cgroup_sock_addr, cgroup_skb, map},
    maps::Array,
    programs::{SockAddrContext, SkBuffContext},
};

#[map]
static CFG: Array<u32> = Array::with_max_entries(8, 0);
#[map]
static HITS: Array<u32> = Array::with_max_entries(8, 0);

#[inline(always)]
fn cfg(i: u32) -> u32 {
    CFG.get(i).copied().unwrap_or(0)
}
#[inline(always)]
fn bump(i: u32) {
    if let Some(p) = HITS.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) };
    }
}
/// bpf_sock_addr.user_port: only the low 16 bits carry the port (network order); compare the low halves.
#[inline(always)]
fn port16(p: u32) -> u32 {
    p & 0xffff
}

#[cgroup_sock_addr(sendmsg4)]
pub fn dg_sendmsg4(ctx: SockAddrContext) -> i32 {
    let sa = ctx.sock_addr;
    unsafe {
        if (*sa).user_ip4 == cfg(0) && port16((*sa).user_port) == port16(cfg(1)) {
            (*sa).user_ip4 = cfg(2);
            (*sa).user_port = port16(cfg(3));
            bump(0);
        } else {
            bump(1);
        }
    }
    1
}

#[cgroup_sock_addr(recvmsg4)]
pub fn dg_recvmsg4(ctx: SockAddrContext) -> i32 {
    let sa = ctx.sock_addr;
    unsafe {
        if (*sa).user_ip4 == cfg(2) && port16((*sa).user_port) == port16(cfg(3)) {
            (*sa).user_ip4 = cfg(0);
            (*sa).user_port = port16(cfg(1));
            bump(2);
        } else {
            bump(3);
        }
    }
    1
}

#[cgroup_skb]
pub fn dg_egress(ctx: SkBuffContext) -> i32 {
    let want = cfg(4);
    if want == 0 {
        return 1;
    }
    // IPv4 header at offset 0; protocol at 9; assume IHL = 5 (spike) -> UDP dport at 22
    let proto: u8 = match ctx.load(9) {
        Ok(v) => v,
        Err(_) => return 1,
    };
    if proto != 17 {
        return 1;
    }
    let dport: u16 = match ctx.load(22) {
        Ok(v) => u16::from_be(v),
        Err(_) => return 1,
    };
    if dport as u32 == want {
        bump(4);
        return 0;
    }
    1
}

#[no_mangle]
#[link_section = "license"]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
