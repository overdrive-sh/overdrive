//! Guest-only eBPF programs for the guest capture owner.
//!
//! This object contains only the guest cgroup hooks and TCP correlation
//! program; it does not include host dataplane maps.

#![no_std]
#![no_main]
#![allow(
    clippy::needless_pass_by_value,
    clippy::inline_always,
    clippy::unnecessary_wraps,
    clippy::missing_const_for_fn
)]

#[path = "programs/guest_capture.rs"]
mod guest_capture;
#[path = "programs/tcp_correlation.rs"]
mod tcp_correlation;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
