//! Node-shared guest TCX endpoint and classifier-counter maps.

use aya_ebpf::{
    macros::map,
    maps::{Array, HashMap},
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Endpoint {
    pub source_ip: u32,
    pub source_mac: [u8; 6],
    pub source_pad: [u8; 2],
    pub bridge_mac: [u8; 6],
    pub bridge_pad: [u8; 2],
}

#[map]
pub static ENDPOINTS: HashMap<u32, Endpoint> = HashMap::with_max_entries(65_536, 0);

#[map]
pub static COUNTERS: Array<u64> = Array::with_max_entries(8, 0);
