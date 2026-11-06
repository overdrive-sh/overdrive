//! Gateway connect-intent and selection-receipt maps (public-ingress-gateway,
//! ADR-0232 / ADR-0243).
//!
//! `GATEWAY_CONNECT_INTENT_MAP` holds one registered intent per prepared
//! gateway upstream socket, keyed by the socket cookie's host-order `u64`;
//! its value is the exact Service-map key the gateway selection arm looks up.
//! `GATEWAY_SELECTION_RECEIPT_MAP` holds the receipt the arm writes before it
//! allows (rewritten) or denies the connect, keyed by the same cookie.
//!
//! Both maps are HASH maps with `max_entries = 128`, unpinned: they live with
//! the loaded BPF object, so a dataplane restart starts them empty (SR-2).
//! The two value structs are exactly eight bytes, fully initialised
//! (`_pad` is always zero) and mirror the userspace PODs in
//! `overdrive-dataplane::gateway_connect`.
//!
//! This file declares the maps only. The separate gateway selection
//! `cgroup/connect4` program that reads and writes them is
//! `programs/cgroup_connect4_gateway.rs`.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

/// Capacity of both maps.
pub const GATEWAY_CONNECT_MAP_CAPACITY: u32 = 128;

/// Receipt outcome: no usable backend (no outer entry, empty slot, missing
/// `BACKEND_MAP` entry).
pub const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
/// Receipt outcome: a nonzero `BackendId` was selected.
pub const GATEWAY_RECEIPT_SELECTED: u32 = 2;

/// Registered intent: the exact Service-map key, host-order.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

/// Selection receipt written by the gateway selection arm.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct GatewaySelectionReceiptPod {
    /// [`GATEWAY_RECEIPT_NO_BACKEND`] or [`GATEWAY_RECEIPT_SELECTED`].
    pub outcome: u32,
    /// The selected nonzero `BackendId`, `0` for `NoBackend`.
    pub backend_id: u32,
}

const _: () = assert!(core::mem::size_of::<GatewayConnectIntentPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewayConnectIntentPod>() == 4);
const _: () = assert!(core::mem::size_of::<GatewaySelectionReceiptPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewaySelectionReceiptPod>() == 4);

/// `GATEWAY_CONNECT_INTENT_MAP` — socket cookie (host-order `u64`) → intent.
#[map]
pub static GATEWAY_CONNECT_INTENT_MAP: HashMap<u64, GatewayConnectIntentPod> =
    HashMap::with_max_entries(GATEWAY_CONNECT_MAP_CAPACITY, 0);

/// `GATEWAY_SELECTION_RECEIPT_MAP` — socket cookie (host-order `u64`) → receipt.
#[map]
pub static GATEWAY_SELECTION_RECEIPT_MAP: HashMap<u64, GatewaySelectionReceiptPod> =
    HashMap::with_max_entries(GATEWAY_CONNECT_MAP_CAPACITY, 0);
