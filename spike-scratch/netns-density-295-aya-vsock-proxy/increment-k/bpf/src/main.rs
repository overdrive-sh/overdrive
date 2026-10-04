#![no_std]
#![no_main]
use aya_ebpf::{
    helpers::bpf_get_socket_cookie,
    macros::{map, stream_verdict},
    maps::{Array, HashMap, PerCpuArray, SockMap},
    programs::SkBuffContext,
};
#[map]
static SOCKETS: SockMap = SockMap::with_max_entries(65536, 0);
#[map]
static ROUTES: HashMap<u64, u32> = HashMap::with_max_entries(65536, 0);
#[map]
static FLAGS: Array<u64> = Array::with_max_entries(1, 0);
#[map]
static FAILURES: PerCpuArray<u64> = PerCpuArray::with_max_entries(4, 0);
#[map]
static COUNTERS: PerCpuArray<u64> = PerCpuArray::with_max_entries(65536, 0);
#[stream_verdict]
pub fn route(ctx: SkBuffContext) -> u32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.skb.skb as *mut _) };
    let Some(key) = (unsafe { ROUTES.get(&cookie) }) else {
        if let Some(ptr) = FAILURES.get_ptr_mut(0) {
            unsafe {
                *ptr = (*ptr).wrapping_add(1);
            }
        }
        return 0;
    };
    if let Some(ptr) = COUNTERS.get_ptr_mut(*key) {
        unsafe {
            *ptr = (*ptr).wrapping_add(1);
        }
    }
    if ctx.pull_data(ctx.len()).is_err() {
        if let Some(ptr) = FAILURES.get_ptr_mut(2) {
            unsafe {
                *ptr = (*ptr).wrapping_add(1);
            }
        }
        return 0;
    }
    let flags = unsafe { FLAGS.get(0).copied().unwrap_or(0) };
    let result = unsafe { SOCKETS.redirect_skb(&ctx, *key, flags) as u32 };
    if result == 0 {
        if let Some(ptr) = FAILURES.get_ptr_mut(1) {
            unsafe {
                *ptr = (*ptr).wrapping_add(1);
            }
        }
    }
    result
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
