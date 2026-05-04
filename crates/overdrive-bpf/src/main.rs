//! Kernel-side eBPF programs for the Overdrive dataplane.
//!
//! Compiled against `bpfel-unknown-none` with `#![no_std]`; loaded
//! into the kernel by `overdrive-dataplane` via aya. Phase 2.1 step
//! 01-01 ships exactly one no-op XDP program (`xdp_pass`) plus an
//! `LruHashMap<u32, u64>` packet counter (`PKTS`) to exercise the
//! build → load → attach → observe → detach pipeline end-to-end.
//!
//! Real dataplane work (SERVICE_MAP, sockops+kTLS, BPF LSM,
//! telemetry ringbuf) lands in subsequent Phase 2 slices.
//!
//! See `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md`
//! and `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md`.

#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::xdp_action,
    macros::{map, xdp},
    maps::LruHashMap,
    programs::XdpContext,
};

/// Packet counter map. Capacity 1024 entries; key is a placeholder
/// `u32` (currently always 0), value is the running packet count.
/// Future slices replace this with proper per-flow / per-identity
/// keys once the relevant map shapes land.
#[map]
static PKTS: LruHashMap<u32, u64> = LruHashMap::with_max_entries(1024, 0);

/// No-op XDP program — increments `PKTS[0]` and returns `XDP_PASS`.
/// The Tier 3 smoke test asserts the counter increments after
/// traffic is generated against the attached interface.
#[xdp]
pub fn xdp_pass(ctx: XdpContext) -> u32 {
    match try_xdp_pass(&ctx) {
        Ok(action) => action,
        Err(()) => xdp_action::XDP_PASS,
    }
}

#[inline(always)]
fn try_xdp_pass(_ctx: &XdpContext) -> Result<u32, ()> {
    let key: u32 = 0;
    // SAFETY: `PKTS.get` is unsafe per aya-ebpf's API — without
    // `BPF_F_NO_PREALLOC` the kernel does not guarantee atomicity
    // between concurrent `insert`/`remove`. For a packet counter
    // the worst case is a momentary stale read, which is
    // acceptable for the smoke-test scaffold. Future slices that
    // need stronger guarantees will switch to per-CPU maps or
    // atomic helpers.
    let next = unsafe { PKTS.get(&key).copied().unwrap_or(0).wrapping_add(1) };
    let _ = PKTS.insert(&key, &next, 0);
    Ok(xdp_action::XDP_PASS)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
