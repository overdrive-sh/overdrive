#![no_std]
#![no_main]
use aya_ebpf::{macros::{map,stream_verdict},maps::{HashMap,SockHash,PerCpuArray},programs::SkBuffContext,helpers::bpf_get_socket_cookie};
#[map] static SOCKETS: SockHash<u32>=SockHash::with_max_entries(65536,0);
#[map] static ROUTES: HashMap<u64,u32>=HashMap::with_max_entries(65536,0);
#[map] static FAILURES: PerCpuArray<u64>=PerCpuArray::with_max_entries(4,0);
#[map] static COUNTERS: PerCpuArray<u64>=PerCpuArray::with_max_entries(65536,0);
#[stream_verdict]
pub fn route(ctx:SkBuffContext)->u32 {
 let cookie=unsafe{bpf_get_socket_cookie(ctx.skb.skb as *mut _)};
 let Some(key)= (unsafe{ROUTES.get(&cookie)}) else {if let Some(ptr)=FAILURES.get_ptr_mut(0){unsafe{*ptr=(*ptr).wrapping_add(1);}}return 0};
 if let Some(ptr)=COUNTERS.get_ptr_mut(*key){unsafe{*ptr=(*ptr).wrapping_add(1);}}
 let mut target=*key;SOCKETS.redirect_skb(&ctx,&mut target,0) as u32
}
#[panic_handler] fn panic(_: &core::panic::PanicInfo)->!{loop{}}
