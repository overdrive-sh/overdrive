//! SCHED_CLS guest endpoint classifier for the node-shared switch.

use aya_ebpf::{bindings::TC_ACT_OK, macros::classifier, programs::TcContext};

use crate::maps::guest_tcx::{COUNTERS, ENDPOINTS, Endpoint};

const TC_ACT_SHOT: i32 = 2;
const INTERCEPT_MARK: u32 = 0x295a;
const ACCEPTED_MARK: u32 = 0x295b;
const ETH_IPV4: u16 = 0x0800;
const ETH_ARP: u16 = 0x0806;
const TCP: u8 = 6;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;
const ENDPOINT_MAP_MISS: u32 = 2;
const SOURCE_MAC_SPOOF: u32 = 3;
const SOURCE_IP_ARP_SPOOF: u32 = 4;
const DIRECT_BYPASS_DROP: u32 = 5;
const ARP_PASS: u32 = 6;
const MALFORMED_DROP: u32 = 7;
const GATEWAY_HOST_PASS: u32 = 0;
const INTERCEPT: u32 = 1;

#[inline(always)]
fn bump(index: u32) {
    if let Some(value) = COUNTERS.get_ptr_mut(index) {
        unsafe { *value = (*value).wrapping_add(1) };
    }
}

#[inline(always)]
fn same(left: &[u8; 6], right: &[u8; 6]) -> bool {
    left[0] == right[0]
        && left[1] == right[1]
        && left[2] == right[2]
        && left[3] == right[3]
        && left[4] == right[4]
        && left[5] == right[5]
}

#[classifier]
pub fn gh295c_endpoint(mut ctx: TcContext) -> i32 {
    let ifindex = unsafe { (*ctx.skb.skb).ingress_ifindex };
    let Some(endpoint) = (unsafe { ENDPOINTS.get(&ifindex) }) else {
        bump(ENDPOINT_MAP_MISS);
        return TC_ACT_SHOT;
    };
    let Ok(eth) = ctx.load::<[u8; 14]>(0) else {
        bump(MALFORMED_DROP);
        return TC_ACT_SHOT;
    };
    let destination = [eth[0], eth[1], eth[2], eth[3], eth[4], eth[5]];
    let source = [eth[6], eth[7], eth[8], eth[9], eth[10], eth[11]];
    let ether_type = u16::from_be_bytes([eth[12], eth[13]]);
    if ether_type == ETH_ARP {
        let Ok(arp) = ctx.load::<[u8; 28]>(14) else {
            bump(MALFORMED_DROP);
            return TC_ACT_SHOT;
        };
        let hardware_type = u16::from_be_bytes([arp[0], arp[1]]);
        let protocol_type = u16::from_be_bytes([arp[2], arp[3]]);
        let hardware_len = arp[4];
        let protocol_len = arp[5];
        let opcode = u16::from_be_bytes([arp[6], arp[7]]);
        if hardware_type != 1
            || protocol_type != 0x0800
            || hardware_len != 6
            || protocol_len != 4
            || (opcode != ARP_REQUEST && opcode != ARP_REPLY)
        {
            bump(MALFORMED_DROP);
            return TC_ACT_SHOT;
        }
        let sender_mac = [arp[8], arp[9], arp[10], arp[11], arp[12], arp[13]];
        let sender_ip = u32::from_be_bytes([arp[14], arp[15], arp[16], arp[17]]);
        if !same(&source, &endpoint.source_mac) || !same(&sender_mac, &endpoint.source_mac) {
            bump(SOURCE_MAC_SPOOF);
            return TC_ACT_SHOT;
        }
        if sender_ip != endpoint.source_ip {
            bump(SOURCE_IP_ARP_SPOOF);
            return TC_ACT_SHOT;
        }
        ctx.set_mark(ACCEPTED_MARK);
        bump(ARP_PASS);
        return TC_ACT_OK;
    }
    if ether_type != ETH_IPV4 {
        bump(DIRECT_BYPASS_DROP);
        return TC_ACT_SHOT;
    }
    let Ok(ip) = ctx.load::<[u8; 20]>(14) else {
        bump(MALFORMED_DROP);
        return TC_ACT_SHOT;
    };
    let source_ip = u32::from_be_bytes([ip[12], ip[13], ip[14], ip[15]]);
    if !same(&source, &endpoint.source_mac) {
        bump(SOURCE_MAC_SPOOF);
        return TC_ACT_SHOT;
    }
    if source_ip != endpoint.source_ip {
        bump(SOURCE_IP_ARP_SPOOF);
        return TC_ACT_SHOT;
    }
    let protocol = ip[9];
    let destination_ip = u32::from_be_bytes([ip[16], ip[17], ip[18], ip[19]]);
    if protocol == TCP {
        let Ok(l4) = ctx.load::<[u8; 4]>(34) else {
            bump(MALFORMED_DROP);
            return TC_ACT_SHOT;
        };
        let _ = l4;
        // TCP to either the peer or the bridge is intercepted.  The original
        // destination bytes remain untouched for transparent proxy recovery.
        let _ = destination_ip;
        let _ = ctx.store(0, &endpoint.bridge_mac, 0);
        ctx.set_mark(INTERCEPT_MARK);
        bump(INTERCEPT);
        return TC_ACT_OK;
    }
    if destination_ip == endpoint.source_ip || same(&destination, &endpoint.bridge_mac) {
        ctx.set_mark(ACCEPTED_MARK);
        bump(GATEWAY_HOST_PASS);
        return TC_ACT_OK;
    }
    bump(DIRECT_BYPASS_DROP);
    TC_ACT_SHOT
}
