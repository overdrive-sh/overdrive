//! Spike (d): smallest cgroup/connect4 backend rewrite (aya Rust). VIP:port -> backend ip:port from CFG.
#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{cgroup_sock_addr, map},
    maps::Array,
    programs::SockAddrContext,
};

/// [0]=vip (network-order u32 as stored in user_ip4), [1]=vip port (host), [2]=backend ip (network-order), [3]=backend port (host)
#[map]
static CFG: Array<u32> = Array::with_max_entries(8, 0);
/// [0]=rewrites performed, [1]=connects seen
#[map]
static HITS: Array<u32> = Array::with_max_entries(4, 0);

#[cgroup_sock_addr(connect4)]
pub fn gw_connect4(ctx: SockAddrContext) -> i32 {
    unsafe {
        let sa = ctx.sock_addr;
        if let Some(c) = HITS.get_ptr_mut(1) {
            *c += 1;
        }
        let (Some(vip), Some(vport), Some(bip), Some(bport)) = (CFG.get(0), CFG.get(1), CFG.get(2), CFG.get(3)) else {
            return 1;
        };
        let port = u32::from(u16::from_be((*sa).user_port as u16));
        if (*sa).user_ip4 == *vip && port == *vport {
            (*sa).user_ip4 = *bip;
            (*sa).user_port = u32::from((*bport as u16).to_be());
            if let Some(c) = HITS.get_ptr_mut(0) {
                *c += 1;
            }
        }
    }
    1
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
