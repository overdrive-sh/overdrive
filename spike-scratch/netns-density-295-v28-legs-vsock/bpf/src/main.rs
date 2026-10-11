//! v28 throwaway (netns-density-295). Aya Rust eBPF only. One tracepoint program:
//! vsock:virtio_transport_recv_pkt -> ringbuf record for every RST / SHUTDOWN op received by this
//! machine's virtio vsock transport (op, flags, addressing). Same program shape as v27 T5
//! (findings-drain-t1-t6.md, T5: 0 misclassified). Userspace classifies half-close vs abort.
#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::bpf_ktime_get_ns,
    macros::{map, tracepoint},
    maps::RingBuf,
    programs::TracePointContext,
};

const RB_FORCE_WAKEUP: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShutRec {
    pub t_ns: u64,
    pub src_cid: u32,
    pub src_port: u32,
    pub dst_cid: u32,
    pub dst_port: u32,
    pub op: u32,
    pub flags: u32,
}

#[map]
static SHUTRB: RingBuf = RingBuf::with_byte_size(1 << 22, 0);

#[tracepoint(category = "vsock", name = "virtio_transport_recv_pkt")]
pub fn vsock_recv(ctx: TracePointContext) -> u32 {
    unsafe {
        let op: u16 = ctx.read_at(30).unwrap_or(0);
        if op != 3 && op != 4 {
            return 0;
        }
        let rec = ShutRec {
            t_ns: bpf_ktime_get_ns(),
            src_cid: ctx.read_at(8).unwrap_or(0),
            src_port: ctx.read_at(12).unwrap_or(0),
            dst_cid: ctx.read_at(16).unwrap_or(0),
            dst_port: ctx.read_at(20).unwrap_or(0),
            op: op as u32,
            flags: ctx.read_at(32).unwrap_or(0),
        };
        let _ = SHUTRB.output(&rec, RB_FORCE_WAKEUP);
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
