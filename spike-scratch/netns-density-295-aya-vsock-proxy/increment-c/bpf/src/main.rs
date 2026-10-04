#![no_std]
#![no_main]
use aya_ebpf::{macros::{map,stream_verdict},maps::{HashMap,SockMap,Array},programs::SkBuffContext,helpers::bpf_get_socket_cookie};
#[map] static SOCKETS: SockMap=SockMap::with_max_entries(65536,0);
#[map] static ROUTES: HashMap<u64,u32>=HashMap::with_max_entries(65536,0);
#[map] static COUNTERS: Array<u64>=Array::with_max_entries(65536,0);
#[stream_verdict]
pub fn route(ctx:SkBuffContext)->u32 {
 let cookie=unsafe{bpf_get_socket_cookie(ctx.skb.skb as *mut _)};
 let Some(key)= (unsafe{ROUTES.get(&cookie)}) else {return 0};
 if let Some(ptr)=COUNTERS.get_ptr_mut(*key){unsafe{core::sync::atomic::AtomicU64::from_ptr(ptr).fetch_add(1,core::sync::atomic::Ordering::Relaxed);}}
 unsafe { SOCKETS.redirect_skb(&ctx,*key,0) as u32 }
}
#[panic_handler] fn panic(_: &core::panic::PanicInfo)->!{loop{}}
