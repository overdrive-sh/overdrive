//! Throwaway spike (public-ingress-gateway V-GW-1..5). Aya Rust eBPF only.
//! Map/struct shapes copied from crates/overdrive-bpf/src/maps/*.rs (SERVICE_MAP, BACKEND_MAP).
//! SERVICE_MAP and BACKEND_MAP are pinned ByName: the loader points aya at a bpffs dir where the
//! LIVE maps of a running `overdrive serve` were re-pinned (bpftool map pin), so the probe reads
//! the production Service-map state.
#![no_std]
#![no_main]
#![allow(dead_code)]

#[path = "../../../../crates/overdrive-bpf/src/maps/hash_of_maps.rs"]
mod hash_of_maps;

use aya_ebpf::{
    cty::c_void,
    helpers::{bpf_get_current_comm, bpf_get_socket_cookie, bpf_map_lookup_elem},
    macros::{cgroup_sock_addr, map},
    maps::{Array, HashMap},
    programs::SockAddrContext,
};
use hash_of_maps::{HashOfMaps, PINNING_BY_NAME};

#[derive(Clone, Copy)]
#[repr(C)]
pub struct ServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct BackendEntry {
    pub ipv4_host: u32,
    pub port_host: u16,
    pub weight: u16,
    pub healthy: u8,
    pub _pad: [u8; 3],
}

/// verdict 1 = Selected, 2 = NoBackend. reason (NoBackend): 1 no outer entry, 2 empty slot,
/// 3 missing BACKEND_MAP entry, 4 healthy == 0.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Receipt {
    pub verdict: u32,
    pub reason: u32,
    pub backend_id: u32,
    pub ip_host: u32,
    pub port_host: u32,
    pub healthy: u32,
}

pub const INNER_TABLE_SIZE: u32 = 16_381;

#[map]
static SERVICE_MAP: HashOfMaps<ServiceKey, u32, Array<u32>> =
    HashOfMaps::with_max_entries_pinned(4096, 0, PINNING_BY_NAME);
#[map]
static BACKEND_MAP: HashMap<u32, BackendEntry> = HashMap::pinned(65_536, 0);
#[map]
static INTENT: HashMap<u64, ServiceKey> = HashMap::pinned(4096, 0);
#[map]
static RECEIPT: HashMap<u64, Receipt> = HashMap::pinned(4096, 0);
#[map]
static COOKIE_BY_PORT: HashMap<u32, u64> = HashMap::pinned(4096, 0);
/// connects observed by the counting hook, keyed by host-order dst port
#[map]
static SEEN: HashMap<u32, u64> = HashMap::pinned(4096, 0);
/// connects observed, keyed by task comm (V-GW-2: which thread names create hooked sockets)
#[map]
static SEEN_COMM: HashMap<[u8; 16], u64> = HashMap::pinned(256, 0);

#[inline(always)]
fn dport(ctx: &SockAddrContext) -> u32 {
    u32::from(u16::from_be(unsafe { (*ctx.sock_addr).user_port } as u16))
}

/// V-GW-5 probe: record the hook-side cookie under the dialed port; V-GW-2 counter.
#[cgroup_sock_addr(connect4)]
pub fn vgw_cookie(ctx: SockAddrContext) -> i32 {
    let c = unsafe { bpf_get_socket_cookie(ctx.sock_addr as *mut c_void) };
    let p = dport(&ctx);
    let _ = COOKIE_BY_PORT.insert(&p, &c, 0);
    let n = unsafe { SEEN.get(&p) }.copied().unwrap_or(0) + 1;
    let _ = SEEN.insert(&p, &n, 0);
    if let Ok(comm) = bpf_get_current_comm() {
        let m = unsafe { SEEN_COMM.get(&comm) }.copied().unwrap_or(0) + 1;
        let _ = SEEN_COMM.insert(&comm, &m, 0);
    }
    1
}

#[inline(always)]
fn note(cookie: u64, r: Receipt) {
    let _ = RECEIPT.insert(&cookie, &r, 0);
}

/// V-GW-4 / V-GW-3 / V-GW-1: registered arm. Unregistered sockets (no INTENT entry) return 1
/// unchanged. Registered: SERVICE_MAP -> inner Maglev slot -> BACKEND_MAP; healthy==0 => deny.
#[cgroup_sock_addr(connect4)]
pub fn vgw_select(ctx: SockAddrContext) -> i32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr as *mut c_void) };
    let Some(key) = (unsafe { INTENT.get(&cookie) }) else {
        return 1;
    };
    let key = *key;
    let mut r = Receipt { verdict: 2, reason: 0, backend_id: 0, ip_host: 0, port_host: 0, healthy: 0 };
    let Some(inner) = SERVICE_MAP.lookup_inner(&key) else {
        r.reason = 1;
        note(cookie, r);
        return 0;
    };
    let slot: u32 = ((cookie ^ (cookie >> 17)) as u32) % INNER_TABLE_SIZE;
    let bid_ptr = unsafe { bpf_map_lookup_elem(inner.as_ptr(), (&raw const slot).cast::<c_void>()) };
    if bid_ptr.is_null() {
        r.reason = 2;
        note(cookie, r);
        return 0;
    }
    let bid: u32 = unsafe { *(bid_ptr as *const u32) };
    if bid == 0 {
        r.reason = 2;
        note(cookie, r);
        return 0;
    }
    r.backend_id = bid;
    let Some(be) = (unsafe { BACKEND_MAP.get(&bid) }) else {
        r.reason = 3;
        note(cookie, r);
        return 0;
    };
    let be = *be;
    r.ip_host = be.ipv4_host;
    r.port_host = u32::from(be.port_host);
    r.healthy = u32::from(be.healthy);
    if be.healthy == 0 {
        r.reason = 4;
        note(cookie, r);
        return 0;
    }
    r.verdict = 1;
    note(cookie, r);
    unsafe {
        (*ctx.sock_addr).user_ip4 = be.ipv4_host.to_be();
        (*ctx.sock_addr).user_port = u32::from(be.port_host.to_be());
    }
    1
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
