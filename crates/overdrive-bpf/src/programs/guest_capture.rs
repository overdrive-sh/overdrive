//! The guest's cgroup socket capture and release hooks (ADR-0150).
use core::ffi::c_void;

use aya_ebpf::{
    helpers::r#gen::bpf_get_socket_cookie,
    macros::{cgroup_sock, cgroup_sock_addr, map},
    maps::{Array, HashMap, Queue, RingBuf},
    programs::{SockAddrContext, SockContext},
};

use super::tcp_correlation::CHILD_CORRELATION;

const EVENT_SOCKET_RELEASED: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Destination {
    pub ip: u32,
    pub port: u16,
    pub pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SlotKey {
    pub cookie: u64,
    pub ip: u32,
    pub port: u16,
    pub pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SlotValue {
    pub cookie: u64,
    pub ip: u32,
    pub port: u16,
    pub pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserEvent {
    pub kind: u32,
    pub pad: u32,
    pub value: u64,
}

#[map]
pub static CFG: Array<u32> = Array::with_max_entries(4, 0);
#[map]
pub static ORIGIN: HashMap<u64, Destination> = HashMap::with_max_entries(65_536, 0);
#[map]
pub static UDP_ORIGIN: HashMap<u64, Destination> = HashMap::with_max_entries(65_536, 0);
#[map]
pub static SLOT_OWNER: HashMap<SlotKey, u32> = HashMap::with_max_entries(1_024, 0);
#[map]
pub static SLOT_META: Array<SlotValue> = Array::with_max_entries(1_024, 0);
#[map]
pub static FREE_SLOTS: Queue<u32> = Queue::with_max_entries(1_024, 0);
#[map]
pub static USER_EVENTS: RingBuf = RingBuf::with_byte_size(8_192, 0);

#[inline(always)]
fn cfg(index: u32) -> u32 {
    CFG.get(index).copied().unwrap_or(0)
}

#[inline(always)]
fn is_local_destination(ip: u32, workload: u32) -> bool {
    let host = u32::from_be(ip);
    host >> 24 == 127 || host == 0 || ip == workload
}

#[inline(always)]
fn destination(ctx: &SockAddrContext) -> (u64, u32, Destination) {
    let addr = unsafe { &mut *ctx.sock_addr };
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr.cast::<c_void>()) };
    let dest = Destination { ip: addr.user_ip4, port: port_from_sock_addr(addr.user_port), pad: 0 };
    (cookie, addr.protocol, dest)
}

#[inline(always)]
fn rewrite(addr: &mut aya_ebpf::bindings::bpf_sock_addr, port: u32) {
    addr.user_ip4 = cfg(0);
    addr.user_port = u32::from(u16::try_from(port).unwrap_or(0).to_be());
}

#[inline(always)]
fn port_from_sock_addr(user_port: u32) -> u16 {
    // The ABI stores low-16 network-order port bits in a u32 with an
    // undefined high half. Select those bytes explicitly, then convert to
    // host order without narrowing the u32.
    let bytes = user_port.to_be_bytes();
    u16::from_be(u16::from_be_bytes([bytes[2], bytes[3]]))
}

#[inline(always)]
fn alloc_slot(cookie: u64, dest: Destination) -> Option<u32> {
    let key = SlotKey { cookie, ip: dest.ip, port: dest.port, pad: 0 };
    if let Some(slot) = unsafe { SLOT_OWNER.get(&key) } {
        return Some(*slot);
    }
    let slot = FREE_SLOTS.pop()?;
    if let Some(value) = SLOT_META.get_ptr_mut(slot) {
        unsafe { *value = SlotValue { cookie, ip: dest.ip, port: dest.port, pad: 0 } };
        if SLOT_OWNER.insert(&key, &slot, 0).is_ok() {
            return Some(slot);
        }
        let _ = FREE_SLOTS.push(&slot, 0);
    }
    None
}

#[inline(always)]
fn capture_tcp(ctx: &SockAddrContext) -> i32 {
    let (cookie, protocol, original) = destination(ctx);
    let workload = cfg(0);
    if protocol != 6 || is_local_destination(original.ip, workload) {
        return 1;
    }
    if ORIGIN.insert(&cookie, &original, 0).is_err() {
        return 0;
    }
    let addr = unsafe { &mut *ctx.sock_addr };
    rewrite(addr, cfg(1));
    1
}

#[inline(always)]
fn capture_udp(ctx: &SockAddrContext) -> i32 {
    let (cookie, protocol, mut original) = destination(ctx);
    if protocol != 17 {
        return 1;
    }
    if original.ip == 0 && original.port == 0 {
        let Some(connected) = (unsafe { UDP_ORIGIN.get(&cookie) }).copied() else {
            return 1;
        };
        original = connected;
    } else if is_local_destination(original.ip, cfg(0)) {
        return 1;
    }
    let Some(slot) = alloc_slot(cookie, original) else {
        return 0;
    };
    let port = cfg(2).wrapping_add(slot);
    if original.ip == (unsafe { &*ctx.sock_addr }).user_ip4 {
        let _ = UDP_ORIGIN.insert(&cookie, &original, 0);
    }
    let addr = unsafe { &mut *ctx.sock_addr };
    rewrite(addr, port);
    1
}

/// Rewrite TCP connects to the guest TCP intake after retaining the original
/// peer by the application's exact `SO_COOKIE`.
#[cgroup_sock_addr(connect4)]
pub fn connect4(ctx: SockAddrContext) -> i32 {
    let protocol = unsafe { (*ctx.sock_addr).protocol };
    if protocol == 6 { capture_tcp(&ctx) } else { capture_udp(&ctx) }
}

/// Capture unconnected UDP sends and route them through the matching slot.
#[cgroup_sock_addr(sendmsg4)]
pub fn sendmsg4(ctx: SockAddrContext) -> i32 {
    capture_udp(&ctx)
}

/// Restore the application's original TCP or UDP peer address.
#[cgroup_sock_addr(getpeername4)]
pub fn getpeername4(ctx: SockAddrContext) -> i32 {
    let addr = unsafe { &mut *ctx.sock_addr };
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr.cast::<c_void>()) };
    let port = port_from_sock_addr(addr.user_port);
    if u32::from(port) == cfg(1) && addr.user_ip4 == cfg(0) {
        if let Some(original) = unsafe { ORIGIN.get(&cookie) } {
            addr.user_ip4 = original.ip;
            addr.user_port = u32::from(original.port.to_be());
        }
    } else if addr.user_ip4 == cfg(0) && u32::from(port) >= cfg(2) {
        let index = u32::from(port).wrapping_sub(cfg(2));
        if index < 1_024
            && let Some(original) = SLOT_META.get(index)
            && original.cookie == cookie
        {
            addr.user_ip4 = original.ip;
            addr.user_port = u32::from(original.port.to_be());
        }
    }
    1
}

/// Restore source addresses on UDP receives from a captured slot.
#[cgroup_sock_addr(recvmsg4)]
pub fn recvmsg4(ctx: SockAddrContext) -> i32 {
    let addr = unsafe { &mut *ctx.sock_addr };
    let port = port_from_sock_addr(addr.user_port);
    if addr.user_ip4 == cfg(0) && u32::from(port) >= cfg(2) {
        let index = u32::from(port).wrapping_sub(cfg(2));
        if index < 1_024
            && let Some(original) = SLOT_META.get(index)
        {
            addr.user_ip4 = original.ip;
            addr.user_port = u32::from(original.port.to_be());
        }
    }
    1
}

/// Remove application origin and slot ownership for the exact released socket.
#[cgroup_sock(sock_release)]
pub fn sock_release(ctx: SockContext) -> i32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock.cast::<c_void>()) };
    let _ = ORIGIN.remove(&cookie);
    let _ = UDP_ORIGIN.remove(&cookie);
    let _ = CHILD_CORRELATION.remove(&cookie);
    let mut released = false;
    for slot in 0..1_024 {
        if let Some(value) = SLOT_META.get_ptr_mut(slot) {
            let value = unsafe { &mut *value };
            if value.cookie == cookie {
                let key = SlotKey { cookie, ip: value.ip, port: value.port, pad: 0 };
                let _ = SLOT_OWNER.remove(&key);
                *value = SlotValue { cookie: 0, ip: 0, port: 0, pad: 0 };
                let _ = FREE_SLOTS.push(&slot, 0);
                released = true;
            }
        }
    }
    if released {
        let event = UserEvent { kind: EVENT_SOCKET_RELEASED, pad: 0, value: cookie };
        let _ = USER_EVENTS.output(&event, 0);
    }
    1
}
