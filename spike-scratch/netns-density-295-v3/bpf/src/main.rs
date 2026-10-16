//! V-3 spike (netns-density-295). Aya Rust eBPF only.
//! SK_SKB stream verdict: every skb arriving on a socket in VERD is redirected (egress, flags=0)
//! to the socket stored at key 1 of SOCKS. fexit(skb_send_sock) counts the bytes that really reach
//! the target socket's send path. queued = fwd - sent = bytes parked in sk_psock->ingress_skb.
#![no_std]
#![no_main]

use aya_ebpf::{
    macros::{fexit, map, stream_verdict},
    maps::{PerCpuArray, SockHash},
    programs::{FExitContext, SkBuffContext},
};

#[map]
static VERD: SockHash<u32> = SockHash::with_max_entries(64, 0);
#[map]
static SOCKS: SockHash<u32> = SockHash::with_max_entries(64, 0);
// 0 fwd bytes entering the verdict, 1 verdict calls, 2 redirect refused (SK_DROP),
// 3 bytes sent by skb_send_sock, 4 successful skb_send_sock calls, 5 failed/zero calls, 6 last errno
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(16, 0);

#[inline(always)]
fn add(i: u32, v: u64) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(v) }
    }
}

#[stream_verdict]
pub fn verdict(ctx: SkBuffContext) -> u32 {
    add(0, ctx.len() as u64);
    add(1, 1);
    let mut key = 1u32;
    let v = SOCKS.redirect_skb(&ctx, &mut key, 0) as u32;
    if v == 0 {
        add(2, 1);
    }
    v
}

#[fexit(function = "skb_send_sock")]
pub fn sent(ctx: FExitContext) -> i32 {
    let ret: i32 = unsafe { ctx.arg(4) };
    if ret > 0 {
        add(3, ret as u64);
        add(4, 1);
    } else {
        add(5, 1);
        if let Some(p) = CNT.get_ptr_mut(6) {
            unsafe { *p = (-ret) as u64 }
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
