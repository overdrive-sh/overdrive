//! `BPF_PROG_TEST_RUN` userspace helper per
//! `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! § C.1 + Appendix A.2.
//!
//! aya 0.13.x does NOT expose `BPF_PROG_TEST_RUN` as a typed surface
//! for XDP / TC programs in a way external callers can reach (the
//! relevant attr layout is `pub(crate)`). Tier 2 PKTGEN/SETUP/CHECK
//! tests use this shim — a thin wrapper around `libc::syscall(SYS_bpf,
//! BPF_PROG_TEST_RUN, …)`. Kept narrow: input bytes, repeat count,
//! returns the kernel verdict + the post-run packet bytes + the
//! kernel-reported program execution time.
//!
//! # Migration
//!
//! Research § F.2: no upstream typed-wrapper effort visible across
//! aya releases. This helper is expected to remain load-bearing
//! across multiple aya versions; signature is kept stable.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::mem;
use std::os::fd::{AsRawFd, BorrowedFd};

use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

/// `bpf` cmd discriminator for `BPF_PROG_TEST_RUN`. Stable kernel ABI
/// per `include/uapi/linux/bpf.h`.
const BPF_PROG_TEST_RUN: c_long = 10;

/// Result of a `BPF_PROG_TEST_RUN` invocation.
pub struct ProgTestRunOutput {
    /// Kernel-side return value — for XDP this is `XDP_PASS` /
    /// `XDP_DROP` / `XDP_TX` / `XDP_ABORTED`. For TC it is
    /// `TC_ACT_OK` / `TC_ACT_SHOT` / etc.
    pub retval: u32,
    /// Packet bytes after the program ran. Header rewrites are
    /// visible here; the buffer is truncated to the kernel-reported
    /// `data_size_out`.
    pub data_out: Vec<u8>,
    /// Kernel-reported program execution time, nanoseconds.
    pub duration_ns: u32,
}

/// `BPF_PROG_TEST_RUN` attribute layout. Mirrors the public-domain
/// `union bpf_attr` `test` arm.
#[repr(C)]
#[derive(Default)]
struct BpfTestRunAttr {
    prog_fd: u32,
    retval: u32,
    data_size_in: u32,
    data_size_out: u32,
    data_in: u64,
    data_out: u64,
    repeat: u32,
    duration: u32,
    ctx_size_in: u32,
    ctx_size_out: u32,
    ctx_in: u64,
    ctx_out: u64,
    flags: u32,
    cpu: u32,
    batch_size: u32,
    _pad: [u8; 4],
}

/// Drive a loaded BPF program against synthetic input.
pub fn prog_test_run(
    prog_fd: BorrowedFd<'_>,
    input: &[u8],
    repeat: u32,
) -> std::io::Result<ProgTestRunOutput> {
    // Headroom for skb_shared_info / xdp_buff metadata the kernel
    // appends. 256 bytes is more than enough for any L2/L3/L4
    // header rewrite case.
    let mut data_out = vec![0u8; input.len() + 256];
    let mut attr = BpfTestRunAttr {
        prog_fd: prog_fd.as_raw_fd() as u32,
        data_in: input.as_ptr() as u64,
        data_size_in: input.len() as u32,
        data_out: data_out.as_mut_ptr() as u64,
        data_size_out: data_out.len() as u32,
        repeat: repeat.max(1),
        ..Default::default()
    };

    // SAFETY: `attr` is a `#[repr(C)]` struct of the size we declare
    // to the kernel; `data_in` / `data_out` point at the
    // caller-owned buffers for the duration of the call. The kernel
    // does not retain pointers past return.
    let raw = unsafe {
        syscall(
            SYS_bpf,
            BPF_PROG_TEST_RUN,
            &mut attr as *mut _ as *const c_void,
            mem::size_of::<BpfTestRunAttr>() as c_int,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }

    data_out.truncate(attr.data_size_out as usize);
    Ok(ProgTestRunOutput { retval: attr.retval, data_out, duration_ns: attr.duration })
}
