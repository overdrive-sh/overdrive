//! V-26 probe programs (aya-rs, Rust). Models the D8a-LOOKUP decision.
//!   v26_steer     : the design's decision program (prefix 100.95.0.0/16, gateway 100.95.0.1)
//!   v26_ifx       : always SK_PASS, counts TCP lookups to the prefix by (ingress_ifindex, local_port)
//!   v26_wrongtype : an XDP program, used as an incompatible BPF_LINK_UPDATE replacement
#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{sk_action, xdp_action},
    macros::{map, sk_lookup, xdp},
    maps::{HashMap, SockHash},
    programs::{SkLookupContext, XdpContext},
};

const PREFIX_HI16: u32 = (100 << 8) | 95; // 100.95.x.x
const GATEWAY: u32 = (100 << 24) | (95 << 16) | 1; // 100.95.0.1
const LOOPBACK_IFINDEX: u32 = 1;
const IPPROTO_TCP: u32 = 6;
const AF_INET: u32 = 2;

/// Key: (destination address, byte order as in the lookup context; destination port, host order).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SteerKey {
    pub addr: u32,
    pub port: u32,
}

#[map]
static STEER: SockHash<SteerKey> = SockHash::with_max_entries(262_144, 0);

#[map]
static IFX: HashMap<u64, u64> = HashMap::with_max_entries(1024, 0);

#[sk_lookup]
pub fn v26_steer(ctx: SkLookupContext) -> u32 {
    let l = unsafe { &*ctx.lookup };
    if l.family != AF_INET {
        return sk_action::SK_PASS;
    }
    let dst = u32::from_be(l.local_ip4);
    if (dst >> 16) != PREFIX_HI16 || dst == GATEWAY {
        return sk_action::SK_PASS;
    }
    // In the prefix. Only loopback-ingress TCP with an entry is assigned.
    if l.protocol != IPPROTO_TCP || l.ingress_ifindex != LOOPBACK_IFINDEX {
        return sk_action::SK_DROP;
    }
    let key = SteerKey { addr: l.local_ip4, port: l.local_port };
    // SAFETY: the map handle is a static; redirect_sk_lookup only looks up and assigns.
    let map = unsafe { &mut *(&STEER as *const _ as *mut SockHash<SteerKey>) };
    match map.redirect_sk_lookup(&ctx, key, 0) {
        Ok(()) => sk_action::SK_PASS,
        Err(_) => sk_action::SK_DROP,
    }
}

#[sk_lookup]
pub fn v26_ifx(ctx: SkLookupContext) -> u32 {
    let l = unsafe { &*ctx.lookup };
    if l.family == AF_INET && l.protocol == IPPROTO_TCP {
        let dst = u32::from_be(l.local_ip4);
        if (dst >> 16) == PREFIX_HI16 {
            let k: u64 = ((l.ingress_ifindex as u64) << 32) | (l.local_port as u64);
            match IFX.get_ptr_mut(&k) {
                Some(p) => unsafe { *p += 1 },
                None => {
                    let _ = IFX.insert(&k, &1u64, 0);
                }
            }
        }
    }
    sk_action::SK_PASS
}

#[xdp]
pub fn v26_wrongtype(_ctx: XdpContext) -> u32 {
    xdp_action::XDP_PASS
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
