//! Guest-local TCP handshake correlation (ADR-0150, TCP-CORRELATION).
use core::{ffi::c_void, ptr};

use aya_ebpf::{
    bindings::{
        BPF_EXIST, BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB, BPF_SOCK_OPS_HDR_OPT_LEN_CB,
        BPF_SOCK_OPS_PARSE_ALL_HDR_OPT_CB_FLAG, BPF_SOCK_OPS_PARSE_HDR_OPT_CB,
        BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB, BPF_SOCK_OPS_STATE_CB, BPF_SOCK_OPS_STATE_CB_FLAG,
        BPF_SOCK_OPS_TCP_CONNECT_CB, BPF_SOCK_OPS_WRITE_HDR_OPT_CB,
        BPF_SOCK_OPS_WRITE_HDR_OPT_CB_FLAG, BPF_TCP_CLOSE, BPF_TCP_ESTABLISHED, bpf_map_def,
    },
    helpers::r#gen::{
        bpf_get_socket_cookie, bpf_load_hdr_opt, bpf_reserve_hdr_opt, bpf_sk_storage_delete,
        bpf_sk_storage_get, bpf_store_hdr_opt,
    },
    macros::{map, sock_ops},
    maps::HashMap,
    programs::SockOpsContext,
};

use super::guest_capture::{CFG, ORIGIN, USER_EVENTS, UserEvent};

const TCP_OPTION_KIND: u8 = 254;
const TCP_OPTION_LEN: u8 = 18;
const OPTION_MAGIC: u16 = 0x4f56;
const TCP_SYN: u32 = 0x02;
const TCP_ACK: u32 = 0x10;
const AF_INET_FAMILY: u32 = 2;
const CHILD_PENDING: u32 = 1;
const CHILD_READY: u32 = 2;
const CHILD_REJECTED: u32 = 3;
const CHILD_CANDIDATE_READY: u32 = 4;
const CHILD_CANDIDATE_REJECTED: u32 = 5;
const ACTIVE_ARMED: u32 = 1;
const ACTIVE_EMIT: u32 = 2;
const BPF_MAP_TYPE_SK_STORAGE: u32 = 24;
const BPF_F_NO_PREALLOC: u32 = 1;
const BPF_SK_STORAGE_GET_F_CREATE: u64 = 1;
const EVENT_PENDING_READY: u32 = 2;
const ACTIVE_STATE_VALUE_SIZE: u32 = 24;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CorrelationMeta {
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
    pub meta: CorrelationMeta,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ActiveState {
    pub phase: u32,
    pub pad: u32,
    pub meta: CorrelationMeta,
}

const _: () = assert!(core::mem::size_of::<ActiveState>() == 24);

impl ChildState {
    const fn pending() -> Self {
        Self {
            state: CHILD_PENDING,
            pad: 0,
            meta: CorrelationMeta {
                application_cookie: 0,
                destination_ip: 0,
                destination_port: 0,
                pad: 0,
            },
        }
    }

    const fn ready(meta: CorrelationMeta) -> Self {
        Self { state: CHILD_READY, pad: 0, meta }
    }

    const fn rejected() -> Self {
        Self { state: CHILD_REJECTED, ..Self::pending() }
    }
}

#[map]
pub static CHILD_CORRELATION: HashMap<u64, ChildState> = HashMap::pinned(8_194, 0);

/// Aya 0.13.1 needs a BTF-backed storage map to be pinned before it can reuse
/// it. Setup unlinks this staging pin before loading any program.
#[unsafe(link_section = "maps")]
#[unsafe(export_name = "ACTIVE_CORRELATION")]
pub static mut ACTIVE_CORRELATION_MAP: bpf_map_def = bpf_map_def {
    type_: BPF_MAP_TYPE_SK_STORAGE,
    key_size: 4,
    value_size: ACTIVE_STATE_VALUE_SIZE,
    max_entries: 0,
    map_flags: BPF_F_NO_PREALLOC,
    id: 0,
    pinning: 1,
};

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

const _: () = assert!(core::mem::size_of::<CorrelationOption>() == 18);

impl CorrelationOption {
    const fn search() -> Self {
        Self {
            kind: TCP_OPTION_KIND,
            len: 4,
            magic: OPTION_MAGIC.to_be(),
            application_cookie: 0,
            destination_ip: 0,
            destination_port: 0,
        }
    }

    const fn from_meta(meta: CorrelationMeta) -> Self {
        Self {
            kind: TCP_OPTION_KIND,
            len: TCP_OPTION_LEN,
            magic: OPTION_MAGIC.to_be(),
            application_cookie: meta.application_cookie.to_be(),
            destination_ip: meta.destination_ip,
            destination_port: meta.destination_port.to_be(),
        }
    }

    fn decode(self) -> Option<CorrelationMeta> {
        if self.kind != TCP_OPTION_KIND
            || self.len != TCP_OPTION_LEN
            || self.magic != OPTION_MAGIC.to_be()
        {
            return None;
        }
        Some(CorrelationMeta {
            application_cookie: u64::from_be(self.application_cookie),
            destination_ip: self.destination_ip,
            destination_port: u16::from_be(self.destination_port),
            pad: 0,
        })
    }
}

#[inline(always)]
fn tcp_flags(ctx: &SockOpsContext) -> u32 {
    unsafe { (*ctx.ops).skb_tcp_flags }
}

#[inline(always)]
fn is_syn(flags: u32) -> bool {
    flags & TCP_SYN != 0
}

#[inline(always)]
fn is_non_syn_ack(flags: u32) -> bool {
    flags & (TCP_SYN | TCP_ACK) == TCP_ACK
}

#[inline(always)]
fn cookie(ctx: &SockOpsContext) -> u64 {
    unsafe { bpf_get_socket_cookie(ctx.ops.cast::<c_void>()) }
}

#[inline(always)]
fn is_tcp_intake_child(ctx: &SockOpsContext) -> bool {
    let ops = unsafe { &*ctx.ops };
    let workload = CFG.get(0).copied().unwrap_or(0);
    let tcp_intake_port = CFG.get(1).copied().unwrap_or(0);
    ops.is_fullsock != 0
        && ops.family == AF_INET_FAMILY
        && ops.local_ip4 == workload
        && ops.local_port == tcp_intake_port
}

#[inline(always)]
fn socket(ctx: &SockOpsContext) -> *mut c_void {
    unsafe { (*ctx.ops).__bindgen_anon_2.sk }.cast::<c_void>()
}

#[inline(always)]
unsafe fn active_get(ctx: &SockOpsContext) -> *mut ActiveState {
    let sk = socket(ctx);
    if sk.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        bpf_sk_storage_get(ptr::addr_of_mut!(ACTIVE_CORRELATION_MAP).cast(), sk, ptr::null_mut(), 0)
            .cast::<ActiveState>()
    }
}

#[inline(always)]
unsafe fn active_create(ctx: &SockOpsContext, meta: CorrelationMeta) -> *mut ActiveState {
    let sk = socket(ctx);
    if sk.is_null() {
        return ptr::null_mut();
    }
    let mut initial = ActiveState { phase: ACTIVE_ARMED, pad: 0, meta };
    unsafe {
        bpf_sk_storage_get(
            ptr::addr_of_mut!(ACTIVE_CORRELATION_MAP).cast(),
            sk,
            (&raw mut initial).cast(),
            BPF_SK_STORAGE_GET_F_CREATE,
        )
        .cast::<ActiveState>()
    }
}

#[inline(always)]
unsafe fn active_delete(ctx: &SockOpsContext) -> i64 {
    let sk = socket(ctx);
    if sk.is_null() {
        return -1;
    }
    unsafe { bpf_sk_storage_delete(ptr::addr_of_mut!(ACTIVE_CORRELATION_MAP).cast(), sk) }
}

#[inline(always)]
fn load_current_option(ctx: &SockOpsContext) -> Option<CorrelationMeta> {
    let mut option = CorrelationOption::search();
    let loaded = unsafe {
        bpf_load_hdr_opt(ctx.ops, (&raw mut option).cast(), u32::from(TCP_OPTION_LEN), 0)
    };
    (loaded == i64::from(TCP_OPTION_LEN)).then(|| option.decode()).flatten()
}

#[inline(always)]
fn wake_pending_child(child_cookie: u64) {
    let event = UserEvent { kind: EVENT_PENDING_READY, pad: 0, value: child_cookie };
    let _ = USER_EVENTS.output(&event, 0);
}

#[inline(always)]
fn promote_candidate(ctx: &SockOpsContext) {
    let child_cookie = cookie(ctx);
    let Some(current) = (unsafe { CHILD_CORRELATION.get(&child_cookie) }).copied() else {
        return;
    };
    let terminal = match current.state {
        CHILD_CANDIDATE_READY => Some(ChildState::ready(current.meta)),
        CHILD_CANDIDATE_REJECTED => Some(ChildState::rejected()),
        _ => None,
    };
    if let Some(terminal) = terminal
        && CHILD_CORRELATION.insert(&child_cookie, &terminal, u64::from(BPF_EXIST)).is_ok()
    {
        wake_pending_child(child_cookie);
    }
}

#[sock_ops]
pub fn tcp_correlation(ctx: SockOpsContext) -> u32 {
    let flags = tcp_flags(&ctx);
    match ctx.op() {
        BPF_SOCK_OPS_TCP_CONNECT_CB => {
            let active_cookie = cookie(&ctx);
            if let Some(origin) = unsafe { ORIGIN.get(&active_cookie) } {
                let meta = CorrelationMeta {
                    application_cookie: active_cookie,
                    destination_ip: origin.ip,
                    destination_port: origin.port,
                    pad: 0,
                };
                let storage = unsafe { active_create(&ctx, meta) };
                if !storage.is_null() {
                    let callback_flags = (BPF_SOCK_OPS_WRITE_HDR_OPT_CB_FLAG
                        | BPF_SOCK_OPS_PARSE_ALL_HDR_OPT_CB_FLAG
                        | BPF_SOCK_OPS_STATE_CB_FLAG)
                        .cast_signed();
                    if ctx.set_cb_flags(callback_flags).is_err() {
                        let _ = unsafe { active_delete(&ctx) };
                    }
                }
            }
        }
        BPF_SOCK_OPS_ACTIVE_ESTABLISHED_CB => {
            let active = unsafe { active_get(&ctx) };
            if !active.is_null() {
                unsafe { (*active).phase = ACTIVE_EMIT };
            }
        }
        BPF_SOCK_OPS_HDR_OPT_LEN_CB => {
            let active = unsafe { active_get(&ctx) };
            if is_non_syn_ack(flags)
                && !active.is_null()
                && unsafe { (*active).phase == ACTIVE_EMIT }
            {
                let _ = unsafe { bpf_reserve_hdr_opt(ctx.ops, u32::from(TCP_OPTION_LEN), 0) };
            }
        }
        BPF_SOCK_OPS_WRITE_HDR_OPT_CB => {
            let active = unsafe { active_get(&ctx) };
            if is_non_syn_ack(flags)
                && !active.is_null()
                && unsafe { (*active).phase == ACTIVE_EMIT }
            {
                let option = CorrelationOption::from_meta(unsafe { (*active).meta });
                let _ = unsafe {
                    bpf_store_hdr_opt(
                        ctx.ops,
                        (&raw const option).cast(),
                        u32::from(TCP_OPTION_LEN),
                        0,
                    )
                };
            }
        }
        BPF_SOCK_OPS_PASSIVE_ESTABLISHED_CB => {
            if !is_tcp_intake_child(&ctx) {
                return 1;
            }
            let child_cookie = cookie(&ctx);
            if is_syn(flags) {
                if CHILD_CORRELATION.insert(&child_cookie, &ChildState::pending(), 0).is_ok() {
                    let _ = ctx.set_cb_flags(BPF_SOCK_OPS_STATE_CB_FLAG.cast_signed());
                }
            } else if let Some(meta) = load_current_option(&ctx) {
                let _ = CHILD_CORRELATION.insert(&child_cookie, &ChildState::ready(meta), 0);
            } else {
                let _ = CHILD_CORRELATION.insert(&child_cookie, &ChildState::rejected(), 0);
            }
        }
        BPF_SOCK_OPS_PARSE_HDR_OPT_CB => {
            let active = unsafe { active_get(&ctx) };
            if !is_syn(flags) && !active.is_null() && unsafe { (*active).phase == ACTIVE_EMIT } {
                let _ = unsafe { active_delete(&ctx) };
            }
        }
        BPF_SOCK_OPS_STATE_CB => {
            let new_state = ctx.arg(1);
            if new_state == BPF_TCP_ESTABLISHED {
                promote_candidate(&ctx);
            }
            if new_state == BPF_TCP_CLOSE {
                let _ = unsafe { active_get(&ctx) };
            }
        }
        _ => {}
    }
    1
}
