//! Throwaway Aya probe: carry the capture identity in an experimental TCP
//! option on the ACK that completes the handshake. SockOps handles normal and
//! syncookie children. A separate throwaway CO-RE object observes and promotes
//! the Fast-Open final ACK. No production source depends on this object.
#![no_std]
#![no_main]

use core::{ffi::c_void, ptr};

use aya_ebpf::{
    bindings::{
        bpf_map_def, BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB, BPF_SOCK_OPS_HDR_OPT_LEN_CB,
        BPF_SOCK_OPS_PARSE_ALL_HDR_OPT_CB_FLAG, BPF_SOCK_OPS_PARSE_HDR_OPT_CB,
        BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB, BPF_SOCK_OPS_STATE_CB, BPF_SOCK_OPS_STATE_CB_FLAG,
        BPF_SOCK_OPS_TCP_CONNECT_CB, BPF_SOCK_OPS_WRITE_HDR_OPT_CB,
        BPF_SOCK_OPS_WRITE_HDR_OPT_CB_FLAG, BPF_TCP_CLOSE,
    },
    helpers::gen::{
        bpf_get_socket_cookie, bpf_load_hdr_opt, bpf_reserve_hdr_opt, bpf_sk_storage_delete,
        bpf_sk_storage_get, bpf_store_hdr_opt,
    },
    macros::{cgroup_sock, map, sock_ops},
    maps::{HashMap, PerCpuArray},
    programs::{SockContext, SockOpsContext},
};

const TCPOPT_EXP: u8 = 254;
const OPT_LEN: u8 = 18;
const MAGIC: u16 = 0x4f56;
const TCP_SYN: u32 = 0x02;
const TCP_ACK: u32 = 0x10;
const PENDING: u32 = 1;
const READY: u32 = 2;
const REJECT: u32 = 3;
const CANDIDATE_READY: u32 = 4;
const CANDIDATE_REJECT: u32 = 5;
const ACTIVE_ARMED: u32 = 1;
const ACTIVE_EMIT: u32 = 2;
const BPF_MAP_TYPE_SK_STORAGE: u32 = 24;
const BPF_F_NO_PREALLOC: u32 = 1;
const BPF_SK_STORAGE_GET_F_CREATE: u64 = 1;
const BPF_EXIST: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Meta {
    pub application_cookie: u64,
    pub destination_ip: u32,
    pub destination_port: u16,
    pub pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ChildState {
    pub state: u32,
    pub pad: u32,
    pub meta: Meta,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ActiveState {
    pub phase: u32,
    pub pad: u32,
    pub meta: Meta,
}

impl ChildState {
    const fn pending() -> Self {
        Self {
            state: PENDING,
            pad: 0,
            meta: Meta { application_cookie: 0, destination_ip: 0, destination_port: 0, pad: 0 },
        }
    }

    const fn ready(meta: Meta) -> Self {
        Self { state: READY, pad: 0, meta }
    }

    const fn reject() -> Self {
        Self { state: REJECT, ..Self::pending() }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct CorrelationOption {
    kind: u8,
    len: u8,
    magic: u16,
    application_cookie: u64,
    destination_ip: u32,
    destination_port: u16,
}

impl CorrelationOption {
    const fn search() -> Self {
        Self {
            kind: TCPOPT_EXP,
            len: 4,
            magic: MAGIC.to_be(),
            application_cookie: 0,
            destination_ip: 0,
            destination_port: 0,
        }
    }

    const fn from_meta(meta: Meta) -> Self {
        Self {
            kind: TCPOPT_EXP,
            len: OPT_LEN,
            magic: MAGIC.to_be(),
            application_cookie: meta.application_cookie.to_be(),
            destination_ip: meta.destination_ip,
            destination_port: meta.destination_port.to_be(),
        }
    }

    fn decode(self) -> Option<Meta> {
        if self.kind != TCPOPT_EXP || self.len != OPT_LEN || self.magic != MAGIC.to_be() {
            return None;
        }
        Some(Meta {
            application_cookie: u64::from_be(self.application_cookie),
            destination_ip: self.destination_ip,
            destination_port: u16::from_be(self.destination_port),
            pad: 0,
        })
    }
}

#[map]
static ORIGIN: HashMap<u64, Meta> = HashMap::with_max_entries(4096, 0);

/// Active handshakes copy the origin into socket-local storage after
/// ephemeral-port allocation. It has no max_entries capacity and follows the
/// exact active socket's lifetime.
#[link_section = "maps"]
#[export_name = "ACTIVE"]
static mut ACTIVE: bpf_map_def = bpf_map_def {
    type_: BPF_MAP_TYPE_SK_STORAGE,
    key_size: 4,
    value_size: core::mem::size_of::<ActiveState>() as u32,
    max_entries: 0,
    map_flags: BPF_F_NO_PREALLOC,
    id: 0,
    pinning: 1,
};

/// Child-cookie keyed observation stand-in for the proposed bounded child
/// handoff. Production cleanup and capacity are DESIGN concerns, not inferred
/// from this throwaway map.
#[map]
static CHILD: HashMap<u64, ChildState> = HashMap::pinned(8_194, 0);

#[map]
static COUNTERS: PerCpuArray<u64> = PerCpuArray::pinned(16, 0);

const C_CONNECT_ARMED: u32 = 0;
const C_ACTIVE_EMIT: u32 = 1;
const C_ACK_RESERVED: u32 = 2;
const C_ACK_WRITTEN: u32 = 3;
const C_PASSIVE_READY: u32 = 4;
const C_TFO_PENDING: u32 = 5;
const C_TFO_READY: u32 = 6;
const C_PASSIVE_REJECT: u32 = 7;
const C_STORE_ERROR: u32 = 8;
const C_LOAD_ERROR: u32 = 9;
const C_ACTIVE_PARSE_DELETE: u32 = 10;
const C_ACTIVE_CLOSE_SEEN: u32 = 11;
const C_TFO_CANDIDATE_READY: u32 = 12;
const C_TFO_CANDIDATE_REJECT: u32 = 13;
const C_TERMINAL_NOOP: u32 = 14;

#[inline(always)]
fn bump(i: u32) {
    if let Some(p) = COUNTERS.get_ptr_mut(i) {
        unsafe { *p = (*p).wrapping_add(1) };
    }
}

#[inline(always)]
fn cookie(ctx: &SockOpsContext) -> u64 {
    unsafe { bpf_get_socket_cookie(ctx.ops as *mut c_void) }
}

#[inline(always)]
fn flags(ctx: &SockOpsContext) -> u32 {
    unsafe { (*ctx.ops).skb_tcp_flags }
}

#[inline(always)]
fn is_syn(tcp_flags: u32) -> bool {
    tcp_flags & TCP_SYN != 0
}

#[inline(always)]
fn is_non_syn_ack(tcp_flags: u32) -> bool {
    tcp_flags & (TCP_SYN | TCP_ACK) == TCP_ACK
}

#[inline(always)]
fn sock(ctx: &SockOpsContext) -> *mut c_void {
    (unsafe { (*ctx.ops).__bindgen_anon_2.sk }) as *mut c_void
}

#[inline(always)]
unsafe fn active_get(ctx: &SockOpsContext) -> *mut ActiveState {
    let sk = sock(ctx);
    if sk.is_null() {
        return ptr::null_mut();
    }
    bpf_sk_storage_get(ptr::addr_of_mut!(ACTIVE).cast(), sk, ptr::null_mut(), 0) as *mut ActiveState
}

#[inline(always)]
unsafe fn active_create(ctx: &SockOpsContext, meta: Meta) -> *mut ActiveState {
    let sk = sock(ctx);
    if sk.is_null() {
        return ptr::null_mut();
    }
    let mut init = ActiveState { phase: ACTIVE_ARMED, pad: 0, meta };
    bpf_sk_storage_get(
        ptr::addr_of_mut!(ACTIVE).cast(),
        sk,
        (&mut init as *mut ActiveState).cast(),
        BPF_SK_STORAGE_GET_F_CREATE,
    ) as *mut ActiveState
}

#[inline(always)]
unsafe fn active_delete(ctx: &SockOpsContext) -> i64 {
    let sk = sock(ctx);
    if sk.is_null() {
        return -1;
    }
    bpf_sk_storage_delete(ptr::addr_of_mut!(ACTIVE).cast(), sk)
}

#[inline(always)]
fn load_current(ctx: &SockOpsContext) -> Option<Meta> {
    let mut option = CorrelationOption::search();
    let rc = unsafe {
        bpf_load_hdr_opt(
            ctx.ops,
            (&mut option as *mut CorrelationOption).cast(),
            core::mem::size_of::<CorrelationOption>() as u32,
            0,
        )
    };
    if rc == OPT_LEN as i64 {
        option.decode()
    } else {
        if rc < 0 {
            bump(C_LOAD_ERROR);
        }
        None
    }
}

#[sock_ops]
pub fn tcp_correlation(ctx: SockOpsContext) -> u32 {
    let tcp_flags = flags(&ctx);
    match ctx.op() {
        BPF_SOCK_OPS_TCP_CONNECT_CB => {
            let active = cookie(&ctx);
            if let Some(meta) = unsafe { ORIGIN.get(&active) } {
                let storage = unsafe { active_create(&ctx, *meta) };
                if !storage.is_null()
                    && ctx
                        .set_cb_flags(
                            (BPF_SOCK_OPS_WRITE_HDR_OPT_CB_FLAG
                                | BPF_SOCK_OPS_PARSE_ALL_HDR_OPT_CB_FLAG
                                | BPF_SOCK_OPS_STATE_CB_FLAG) as i32,
                        )
                        .is_ok()
                {
                    bump(C_CONNECT_ARMED);
                } else if !storage.is_null() {
                    let _ = unsafe { active_delete(&ctx) };
                }
            }
        }
        BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB => {
            let active = unsafe { active_get(&ctx) };
            if !active.is_null() {
                unsafe { (*active).phase = ACTIVE_EMIT };
                bump(C_ACTIVE_EMIT);
            }
        }
        BPF_SOCK_OPS_HDR_OPT_LEN_CB => {
            let active = unsafe { active_get(&ctx) };
            if is_non_syn_ack(tcp_flags)
                && !active.is_null()
                && unsafe { (*active).phase == ACTIVE_EMIT }
            {
                let rc = unsafe { bpf_reserve_hdr_opt(ctx.ops, OPT_LEN as u32, 0) };
                if rc == 0 {
                    bump(C_ACK_RESERVED);
                }
            }
        }
        BPF_SOCK_OPS_WRITE_HDR_OPT_CB => {
            let active = unsafe { active_get(&ctx) };
            if is_non_syn_ack(tcp_flags)
                && !active.is_null()
                && unsafe { (*active).phase == ACTIVE_EMIT }
            {
                let option = CorrelationOption::from_meta(unsafe { (*active).meta });
                let rc = unsafe {
                    bpf_store_hdr_opt(
                        ctx.ops,
                        (&option as *const CorrelationOption).cast(),
                        core::mem::size_of::<CorrelationOption>() as u32,
                        0,
                    )
                };
                if rc == 0 {
                    bump(C_ACK_WRITTEN);
                } else {
                    bump(C_STORE_ERROR);
                }
            }
        }
        BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB => {
            let child = cookie(&ctx);
            if is_syn(tcp_flags) {
                if CHILD.insert(&child, &ChildState::pending(), 0).is_ok() {
                    let _ = ctx.set_cb_flags(BPF_SOCK_OPS_STATE_CB_FLAG as i32);
                    bump(C_TFO_PENDING);
                }
            } else if let Some(meta) = load_current(&ctx) {
                if CHILD.insert(&child, &ChildState::ready(meta), 0).is_ok() {
                    bump(C_PASSIVE_READY);
                }
            } else {
                let _ = CHILD.insert(&child, &ChildState::reject(), 0);
                bump(C_PASSIVE_REJECT);
            }
        }
        BPF_SOCK_OPS_PARSE_HDR_OPT_CB => {
            let active = unsafe { active_get(&ctx) };
            if !is_syn(tcp_flags) && !active.is_null() && unsafe { (*active).phase == ACTIVE_EMIT }
            {
                if unsafe { active_delete(&ctx) } == 0 {
                    bump(C_ACTIVE_PARSE_DELETE);
                }
            }
        }
        BPF_SOCK_OPS_STATE_CB => {
            if ctx.arg(0) == aya_ebpf::bindings::BPF_TCP_SYN_RECV
                && ctx.arg(1) == aya_ebpf::bindings::BPF_TCP_ESTABLISHED
            {
                let child = cookie(&ctx);
                if let Some(current) = unsafe { CHILD.get(&child) } {
                    let terminal = if current.state == CANDIDATE_READY {
                        bump(C_TFO_READY);
                        ChildState { state: READY, ..*current }
                    } else if current.state == CANDIDATE_REJECT {
                        bump(C_PASSIVE_REJECT);
                        ChildState { state: REJECT, ..*current }
                    } else {
                        return 1;
                    };
                    let _ = CHILD.insert(&child, &terminal, BPF_EXIST);
                }
            }
            if ctx.arg(1) == BPF_TCP_CLOSE {
                if !unsafe { active_get(&ctx) }.is_null() {
                    bump(C_ACTIVE_CLOSE_SEEN);
                }
            }
        }
        _ => {}
    }
    1
}

/// Mirrors production's exact child-cookie cleanup at file release. Writers
/// update with `BPF_EXIST`, so deletion is final and cannot be recreated.
#[cgroup_sock(sock_release)]
pub fn correlation_release(ctx: SockContext) -> i32 {
    let child = unsafe { bpf_get_socket_cookie(ctx.sock.cast::<c_void>()) };
    let _ = CHILD.remove(&child);
    1
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[no_mangle]
static _license: [u8; 4] = *b"GPL\0";
