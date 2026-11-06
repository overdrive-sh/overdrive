//! `cgroup_connect4_gateway` — the gateway selection `BPF_CGROUP_INET4_CONNECT`
//! program (public-ingress-gateway; ADR-0224 / ADR-0232 / ADR-0243, R-1).
//!
//! Attached beside `cgroup_connect4_service` at the same cgroup ancestor. The
//! kernel runs every attached program over the same context; any verdict other
//! than 1 makes `connect(2)` fail with `-EPERM`
//! (`kernel/bpf/cgroup.c:79-89, 1664-1665, 1694-1696 @ v7.0`).
//!
//! The arm is keyed on the socket cookie, which `bpf_get_socket_cookie` returns
//! for a `cgroup/connect4` context (`net/core/filter.c:5151-5154, 8199-8200 @
//! v7.0`) and which equals userspace `getsockopt(SO_COOKIE)` of the same socket
//! (`net/core/sock.c:2109-2113 @ v7.0`).
//!
//! - **Unregistered socket** (no `GATEWAY_CONNECT_INTENT_MAP` entry): allow,
//!   context untouched. Today's behaviour is unchanged.
//! - **Registered socket**: resolve the intent's Service-map key in
//!   `SERVICE_MAP` (outer lookup, NULL check, chained inner Maglev lookup
//!   indexed by a deterministic fold of the cookie), then `BACKEND_MAP[id]`.
//!   An absent outer entry, an empty slot or a missing backend record writes
//!   a `NoBackend` receipt and denies. Otherwise the receipt `Selected(id)` is
//!   written and the destination is rewritten to the backend. `healthy` is not
//!   consulted (R-2). Any map-write failure denies; there is no fall-through to
//!   the unchanged VIP.
//!
//! The program reads neither `user_ip4` nor `user_port`; it writes both. The
//! port is written low-16 network order inside the `u32`
//! (`development.md` § "`bpf_sock_addr.user_port`").

#![allow(dead_code)]

use aya_ebpf::{
    cty::c_void, helpers::bpf_get_socket_cookie, helpers::bpf_map_lookup_elem,
    macros::cgroup_sock_addr, programs::SockAddrContext,
};

use crate::maps::{
    backend_map::BACKEND_MAP,
    gateway_connect::{
        GATEWAY_CONNECT_INTENT_MAP, GATEWAY_RECEIPT_NO_BACKEND, GATEWAY_RECEIPT_SELECTED,
        GATEWAY_SELECTION_RECEIPT_MAP, GatewaySelectionReceiptPod,
    },
    service_map::{INNER_TABLE_SIZE, SERVICE_MAP, ServiceKey},
};

/// Verdict: allow the connect.
const ALLOW: i32 = 1;
/// Verdict: deny the connect (`-EPERM` to the caller).
const DENY: i32 = 0;

/// `BPF_CGROUP_INET4_CONNECT` entry point of the gateway selection arm.
#[cgroup_sock_addr(connect4)]
pub fn cgroup_connect4_gateway(ctx: SockAddrContext) -> i32 {
    // SAFETY: aya's `SockAddrContext` exposes the kernel-provided
    // `*mut bpf_sock_addr`, valid for the duration of the program run.
    let cookie = unsafe { bpf_get_socket_cookie(ctx.sock_addr.cast::<c_void>()) };

    // SAFETY: canonical aya-ebpf map read; the reference lives for this run.
    let Some(intent) = (unsafe { GATEWAY_CONNECT_INTENT_MAP.get(&cookie) }) else {
        return ALLOW;
    };
    let key = ServiceKey {
        vip_host: intent.vip_host,
        port_host: intent.port_host,
        proto: intent.proto,
        _pad: 0,
    };

    let Some(inner) = SERVICE_MAP.lookup_inner(&key) else {
        return no_backend(cookie);
    };
    // The low 32 bits of the folded cookie; the mask makes `try_from` total.
    let folded = u32::try_from((cookie ^ (cookie >> 17)) & u64::from(u32::MAX)).unwrap_or(0);
    let slot: u32 = folded % INNER_TABLE_SIZE;
    // SAFETY: `inner` is the verifier-tagged inner-map pointer from the outer
    // lookup, NULL-checked by `NonNull`; `slot` is a stack local.
    let backend_id_ptr =
        unsafe { bpf_map_lookup_elem(inner.as_ptr(), (&raw const slot).cast::<c_void>()) };
    if backend_id_ptr.is_null() {
        return no_backend(cookie);
    }
    // SAFETY: non-null, points at the inner ARRAY's `u32` value.
    let backend_id: u32 = unsafe { *(backend_id_ptr.cast::<u32>()) };
    if backend_id == 0 {
        return no_backend(cookie);
    }
    // SAFETY: canonical aya-ebpf map read.
    let Some(backend) = (unsafe { BACKEND_MAP.get(&backend_id) }) else {
        return no_backend(cookie);
    };

    if write_receipt(cookie, GATEWAY_RECEIPT_SELECTED, backend_id).is_err() {
        return DENY;
    }
    // SAFETY: `user_ip4` and `user_port` are writable `bpf_sock_addr` fields in
    // `cgroup/connect4`. The port occupies the low 16 bits, network order.
    unsafe {
        (*ctx.sock_addr).user_ip4 = backend.ipv4_host.to_be();
        (*ctx.sock_addr).user_port = u32::from(backend.port_host.to_be());
    }
    ALLOW
}

/// Record `NoBackend` for `cookie` and deny the connect.
#[inline(always)]
fn no_backend(cookie: u64) -> i32 {
    // The connect is denied whether or not the receipt lands: a failed write
    // leaves no receipt, which userspace reads as a missing receipt (a failed
    // dial), never as a selection.
    let _ = write_receipt(cookie, GATEWAY_RECEIPT_NO_BACKEND, 0);
    DENY
}

#[inline(always)]
fn write_receipt(cookie: u64, outcome: u32, backend_id: u32) -> Result<(), i64> {
    let receipt = GatewaySelectionReceiptPod { outcome, backend_id };
    GATEWAY_SELECTION_RECEIPT_MAP.insert(&cookie, &receipt, 0)
}
