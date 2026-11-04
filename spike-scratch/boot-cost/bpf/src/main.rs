//! Throwaway spike object (listen-hooks): a COPY of the guest object
//! (`crates/overdrive-bpf/src/guest_main.rs`: tcp_correlation sockops +
//! guest_capture cgroup programs), plus, behind feature `listen`, a
//! TCP_LISTEN_CB arm and CgroupSock post_bind4/6 programs that push events
//! into a ringbuf. Not production code.
#![no_std]
#![no_main]
#![allow(
    clippy::needless_pass_by_value,
    clippy::inline_always,
    clippy::unnecessary_wraps,
    clippy::missing_const_for_fn
)]

mod guest_capture;
#[cfg(feature = "listen")]
mod listen;
mod tcp_correlation;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
