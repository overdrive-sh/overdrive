#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{TC_ACT_OK, TC_ACT_SHOT},
    macros::{classifier, map},
    maps::{Array, HashMap},
    programs::TcContext,
};

const ETH_P_IPV4: u16 = 0x0800;
const ETH_P_ARP: u16 = 0x0806;
const IPPROTO_TCP: u8 = 6;
const MARK_TPROXY_SOURCE: u32 = 0x295a;
const PACKET_HOST: u32 = 0;

const COUNT_GATEWAY_PASS: u32 = 0;
const COUNT_INTERCEPT: u32 = 1;
const COUNT_MAP_MISS: u32 = 2;
const COUNT_SPOOF_MAC: u32 = 3;
const COUNT_SPOOF_IP: u32 = 4;
const COUNT_DIRECT_BYPASS_DROP: u32 = 5;
const COUNT_ARP_PASS: u32 = 6;
const COUNT_MALFORMED_DROP: u32 = 7;
const COUNTER_SLOTS: u32 = 8;

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
static ENDPOINTS: HashMap<u32, Endpoint> = HashMap::with_max_entries(16, 0);

#[map]
static COUNTERS: Array<u64> = Array::with_max_entries(COUNTER_SLOTS, 0);

#[inline(always)]
fn bump(slot: u32) {
    if let Some(counter) = COUNTERS.get_ptr_mut(slot) {
        unsafe {
            *counter = (*counter).wrapping_add(1);
        }
    }
}

#[inline(always)]
fn drop_with(slot: u32) -> i32 {
    bump(slot);
    TC_ACT_SHOT as i32
}

#[classifier]
pub fn gh295c_endpoint(mut ctx: TcContext) -> i32 {
    match classify(&mut ctx) {
        Ok(action) => action,
        Err(()) => drop_with(COUNT_MALFORMED_DROP),
    }
}

#[inline(always)]
fn classify(ctx: &mut TcContext) -> Result<i32, ()> {
    let ifindex = unsafe { (*ctx.skb.skb).ingress_ifindex };
    let Some(endpoint) = (unsafe { ENDPOINTS.get(&ifindex) }) else {
        return Ok(drop_with(COUNT_MAP_MISS));
    };

    let destination_mac = ctx.load::<[u8; 6]>(0).map_err(|_| ())?;
    let source_mac = ctx.load::<[u8; 6]>(6).map_err(|_| ())?;
    if source_mac != endpoint.source_mac {
        return Ok(drop_with(COUNT_SPOOF_MAC));
    }

    let ether_type = u16::from_be(ctx.load::<u16>(12).map_err(|_| ())?);
    if ether_type == ETH_P_ARP {
        let sender_ip = u32::from_be(ctx.load::<u32>(28).map_err(|_| ())?);
        if sender_ip != endpoint.source_ip {
            return Ok(drop_with(COUNT_SPOOF_IP));
        }
        bump(COUNT_ARP_PASS);
        return Ok(TC_ACT_OK as i32);
    }
    if ether_type != ETH_P_IPV4 {
        return Ok(drop_with(COUNT_DIRECT_BYPASS_DROP));
    }

    let version_ihl = ctx.load::<u8>(14).map_err(|_| ())?;
    if version_ihl != 0x45 {
        return Ok(drop_with(COUNT_MALFORMED_DROP));
    }
    let source_ip = u32::from_be(ctx.load::<u32>(26).map_err(|_| ())?);
    if source_ip != endpoint.source_ip {
        return Ok(drop_with(COUNT_SPOOF_IP));
    }

    if destination_mac == endpoint.bridge_mac {
        bump(COUNT_GATEWAY_PASS);
        return Ok(TC_ACT_OK as i32);
    }

    let protocol = ctx.load::<u8>(23).map_err(|_| ())?;
    if protocol != IPPROTO_TCP {
        return Ok(drop_with(COUNT_DIRECT_BYPASS_DROP));
    }

    ctx.store(0, &endpoint.bridge_mac, 0).map_err(|_| ())?;
    ctx.set_mark(MARK_TPROXY_SOURCE);
    ctx.change_type(PACKET_HOST).map_err(|_| ())?;
    bump(COUNT_INTERCEPT);
    Ok(TC_ACT_OK as i32)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
