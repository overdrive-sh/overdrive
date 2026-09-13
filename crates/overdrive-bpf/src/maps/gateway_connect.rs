//! Registered gateway-connect intent and transient selection-receipt maps.
//!
//! SCAFFOLD: true. The ABI and capacities are exact; the cgroup-connect
//! branch that consumes and publishes these PODs remains RED for DELIVER.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::HashMap};

pub const GATEWAY_CONNECT_MAP_CAPACITY: u32 = 128;
pub const GATEWAY_RECEIPT_NO_BACKEND: u32 = 1;
pub const GATEWAY_RECEIPT_SELECTED: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GatewayConnectIntentPod {
    pub vip_host: u32,
    pub port_host: u16,
    pub proto: u8,
    pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GatewaySelectionReceiptPod {
    pub outcome: u32,
    pub backend_id: u32,
}

const _: () = assert!(core::mem::size_of::<GatewayConnectIntentPod>() == 8);
const _: () = assert!(core::mem::size_of::<GatewaySelectionReceiptPod>() == 8);
const _: () = assert!(core::mem::align_of::<GatewayConnectIntentPod>() == 4);
const _: () = assert!(core::mem::align_of::<GatewaySelectionReceiptPod>() == 4);

#[map]
pub static GATEWAY_CONNECT_INTENT_MAP: HashMap<u64, GatewayConnectIntentPod> =
    HashMap::with_max_entries(GATEWAY_CONNECT_MAP_CAPACITY, 0);

#[map]
pub static GATEWAY_SELECTION_RECEIPT_MAP: HashMap<u64, GatewaySelectionReceiptPod> =
    HashMap::with_max_entries(GATEWAY_CONNECT_MAP_CAPACITY, 0);
