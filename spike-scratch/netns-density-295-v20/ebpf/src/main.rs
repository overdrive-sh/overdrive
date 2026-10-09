//! V-20 probe programs (aya-rs, Rust). Models M-1: intake-child identity by clone-flagged socket storage.
//!   v20_sockops : cgroup sock_ops; at PASSIVE_ESTABLISHED reads the child's tag from TAGS (BPF_F_CLONE)
//!                 and from TAGS_NOCLONE (negative control), records per-cookie what it saw.
//!   v20_steer   : sk_lookup, the D8a-LOOKUP decision: (dst addr, dst port) -> listener in STEER.
//! TAGS / TAGS_NOCLONE are BPF_MAP_TYPE_SK_STORAGE maps; they need BTF, so userspace creates and pins them
//! and this object declares them legacy-style with pinning = ByName (aya reuses the pinned fd).
#![no_std]
#![no_main]

use core::{ffi::c_void, ptr};

use aya_ebpf::{
    bindings::{bpf_map_def, sk_action},
    helpers::gen::{bpf_get_socket_cookie, bpf_sk_storage_get},
    macros::{map, sk_lookup, sock_ops},
    maps::{HashMap, PerCpuArray, SockHash},
    programs::{SkLookupContext, SockOpsContext},
};

const BPF_MAP_TYPE_SK_STORAGE: u32 = 24;
const BPF_F_NO_PREALLOC: u32 = 1;
const BPF_F_CLONE: u32 = 1 << 9;
const NONE: u32 = 0xFFFF_FFFF;

#[link_section = "maps"]
#[export_name = "TAGS"]
static mut TAGS: bpf_map_def = bpf_map_def {
    type_: BPF_MAP_TYPE_SK_STORAGE,
    key_size: 4,
    value_size: 4,
    max_entries: 0,
    map_flags: BPF_F_NO_PREALLOC | BPF_F_CLONE,
    id: 0,
    pinning: 1,
};

#[link_section = "maps"]
#[export_name = "TAGS_NOCLONE"]
static mut TAGS_NOCLONE: bpf_map_def = bpf_map_def {
    type_: BPF_MAP_TYPE_SK_STORAGE,
    key_size: 4,
    value_size: 4,
    max_entries: 0,
    map_flags: BPF_F_NO_PREALLOC,
    id: 0,
    pinning: 1,
};

/// socket cookie -> (clone-map tag | noclone-map tag << 32); NONE = absent.
#[map]
static SEEN: HashMap<u64, u64> = HashMap::with_max_entries(65536, 0);

/// Counters (per CPU; userspace sums).
#[map]
static CNT: PerCpuArray<u64> = PerCpuArray::with_max_entries(16, 0);

const C_PASSIVE: u32 = 0;
const C_PASSIVE_TAGGED: u32 = 1;
const C_PASSIVE_UNTAGGED: u32 = 2;
const C_ACTIVE: u32 = 3;
const C_ACTIVE_TAGGED: u32 = 4;
const C_SHARED: u32 = 5; // passive, local addr in 100.96.0.0/24 and local port 8080
const C_SHARED_TAGGED: u32 = 6;
const C_SHARED_UNTAGGED: u32 = 7;
const C_NOCLONE_TAGGED: u32 = 8;
const C_STATECB: u32 = 9;
const C_STATECB_SEEN_TAGGED: u32 = 10;
const C_STATECB_NOW_UNTAGGED: u32 = 11;
const C_NOSK: u32 = 12;
const C_INSERT_FAIL: u32 = 13;

#[inline(always)]
fn bump(i: u32) {
    if let Some(p) = CNT.get_ptr_mut(i) {
        unsafe { *p += 1 };
    }
}

#[inline(always)]
unsafe fn tag_of(map: *mut bpf_map_def, sk: *mut c_void) -> u32 {
    let p = bpf_sk_storage_get(map as *mut c_void, sk, ptr::null_mut(), 0) as *const u32;
    if p.is_null() {
        NONE
    } else {
        *p
    }
}

#[sock_ops]
pub fn v20_sockops(ctx: SockOpsContext) -> u32 {
    let op = ctx.op();
    // 4 ACTIVE_ESTABLISHED_CB, 5 PASSIVE_ESTABLISHED_CB, 10 STATE_CB
    if op != 4 && op != 5 && op != 10 {
        return 1;
    }
    let ops = ctx.ops;
    let sk = unsafe { (*ops).__bindgen_anon_2.sk } as *mut c_void;
    if sk.is_null() {
        bump(C_NOSK);
        return 1;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ops as *mut c_void) };
    let t = unsafe { tag_of(ptr::addr_of_mut!(TAGS), sk) };
    if op == 10 {
        bump(C_STATECB);
        if let Some(prev) = unsafe { SEEN.get(&cookie) } {
            if (*prev as u32) != NONE {
                bump(C_STATECB_SEEN_TAGGED);
                if t != (*prev as u32) {
                    bump(C_STATECB_NOW_UNTAGGED);
                }
            }
        }
        return 1;
    }
    let tn = unsafe { tag_of(ptr::addr_of_mut!(TAGS_NOCLONE), sk) };
    if op == 4 {
        bump(C_ACTIVE);
        if t != NONE {
            bump(C_ACTIVE_TAGGED);
        }
        return 1;
    }
    // PASSIVE_ESTABLISHED
    bump(C_PASSIVE);
    if t != NONE {
        bump(C_PASSIVE_TAGGED);
    } else {
        bump(C_PASSIVE_UNTAGGED);
    }
    if tn != NONE {
        bump(C_NOCLONE_TAGGED);
    }
    let lip = u32::from_be(unsafe { (*ops).local_ip4 });
    let lport = unsafe { (*ops).local_port };
    if (lip >> 8) == ((100 << 16) | (96 << 8)) && lport == 8080 {
        bump(C_SHARED);
        if t != NONE {
            bump(C_SHARED_TAGGED);
        } else {
            bump(C_SHARED_UNTAGGED);
        }
    }
    let v: u64 = (t as u64) | ((tn as u64) << 32);
    if SEEN.insert(&cookie, &v, 0).is_err() {
        bump(C_INSERT_FAIL);
    }
    // ask for STATE_CB so the tag can be re-read later in the child's life
    let _ = ctx.set_cb_flags(4);
    1
}

const PREFIX_HI24: u32 = (100 << 16) | (96 << 8); // 100.96.0.x
const AF_INET: u32 = 2;
const IPPROTO_TCP: u32 = 6;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SteerKey {
    pub addr: u32,
    pub port: u32,
}

#[map]
static STEER: SockHash<SteerKey> = SockHash::with_max_entries(1024, 0);

#[sk_lookup]
pub fn v20_steer(ctx: SkLookupContext) -> u32 {
    let l = unsafe { &*ctx.lookup };
    if l.family != AF_INET || l.protocol != IPPROTO_TCP {
        return sk_action::SK_PASS;
    }
    let dst = u32::from_be(l.local_ip4);
    if (dst >> 8) != PREFIX_HI24 {
        return sk_action::SK_PASS;
    }
    let key = SteerKey { addr: l.local_ip4, port: l.local_port };
    let map = unsafe { &mut *(&STEER as *const _ as *mut SockHash<SteerKey>) };
    match map.redirect_sk_lookup(&ctx, key, 0) {
        Ok(()) => sk_action::SK_PASS,
        Err(_) => sk_action::SK_DROP,
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
