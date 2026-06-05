//! Typed userspace handle for `REVERSE_LOCAL_MAP` per ADR-0053 rev
//! 2026-06-05 (GH #200).
//!
//! Wraps an `aya::maps::HashMap<MapData, ReverseLocalKeyPod, u32>` — the
//! reply store for the UNCONNECTED-UDP same-host cgroup path. Keyed by
//! the backend identity `(backend_ip, backend_port, proto)` (the
//! `BackendKey` newtype, byte-parity with the three existing keys,
//! DDD-2), value = the original VIP `u32`. The
//! `cgroup_recvmsg4_service` program reads it to rewrite the reply
//! source the app sees backend→VIP.
//!
//! DISTINCT from `reverse_nat_map_handle` — that is the XDP wire path
//! (`REVERSE_NAT_MAP`, connected/remote). This is the cgroup reply path
//! (`REVERSE_LOCAL_MAP`, unconnected/same-host). The two MUST NOT be
//! conflated (the wrong-map-on-wrong-hook trap the sibling journey
//! defuses).
//!
//! Mirrors `LocalBackendMapHandle`'s shape: the typed handle IS the
//! interior-mutability boundary (aya `insert`/`remove` take `&mut self`;
//! the `Dataplane::register_local_backend` trait surface is `&self`).
//! The lock is held only for the BPF syscalls — never across `.await`.
//!
//! Written **ordered (reverse-first)** by the same
//! `register_local_backend` call that writes `LOCAL_BACKEND_MAP` (DDD-1,
//! DDD-5a). No new trait method — `EbpfDataplane::register_local_backend`
//! gains the second write.
//!
//! The Tier-3 acceptance (the unconnected round-trip) is THE gate —
//! there is no Tier-2 backstop for the cgroup path; the handle's
//! host-order roundtrip proptest compensates at the userspace edge.

#![allow(dead_code)]

use std::net::Ipv4Addr;

use aya::maps::{HashMap, MapData, MapError};
use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
use parking_lot::Mutex;

/// `REVERSE_LOCAL_MAP` outer-key POD — the backend identity.
///
/// 8 bytes, host-order, byte-parity with `wire::LocalServiceKey` and the
/// kernel `ReverseLocalKey`. Distinct type from `BackendKeyPod` only in
/// name; kept local so this handle does not couple to the XDP
/// reverse-NAT wire module.
///
/// `pub _pad` is a load-bearing wire-shape padding field — its byte
/// position determines the struct's kernel-readable layout. Renaming it
/// loses the "this is padding" signal at every call site.
#[allow(clippy::pub_underscore_fields)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct ReverseLocalKeyPod {
    /// Backend IPv4, host-order.
    pub backend_ip_host: u32,
    /// Backend port, host-order.
    pub backend_port_host: u16,
    /// IANA L4 protocol byte — TCP=6, UDP=17.
    pub proto: u8,
    /// Padding to 8-byte alignment. Always zero.
    pub _pad: u8,
}

// Compile-time guard: byte-parity with the kernel key + the other three
// 8-byte keys.
const _: () = assert!(core::mem::size_of::<ReverseLocalKeyPod>() == 8);

// SAFETY: repr(C), `_pad` always 0, all fields byte-addressable. aya
// needs the marker for raw map access.
unsafe impl aya::Pod for ReverseLocalKeyPod {}

impl ReverseLocalKeyPod {
    /// Encode a typed `BackendKey` newtype to the host-order POD.
    #[must_use]
    pub fn from_typed(key: BackendKey) -> Self {
        Self {
            backend_ip_host: u32::from(key.ip),
            backend_port_host: key.port,
            proto: key.proto.as_u8(),
            _pad: 0,
        }
    }
}

/// `REVERSE_LOCAL_MAP` value codec — the original VIP, stored as a
/// host-order `u32`.
///
/// Endianness lockstep (ADR-0041): the userspace handle stores the VIP
/// host-order with NO flip; the `cgroup_recvmsg4_service` program
/// converts to network-order at the write boundary when it rewrites the
/// reply source. `encode` is `u32::from(Ipv4Addr)` (host-order on every
/// supported arch); `decode` is the inverse. A unit struct rather than
/// free functions so the encode/decode pair is the single named codec
/// site for the VIP value, mirroring the key's `from_typed`.
pub struct ReverseLocalMapValue;

impl ReverseLocalMapValue {
    /// Host-order `u32` the handle writes into the map value for `vip`.
    #[must_use]
    pub fn encode(vip: Ipv4Addr) -> u32 {
        u32::from(vip)
    }

    /// Recover the VIP from the host-order `u32` map value.
    #[must_use]
    pub fn decode(vip_host: u32) -> Ipv4Addr {
        Ipv4Addr::from(vip_host)
    }
}

/// Typed handle around `REVERSE_LOCAL_MAP`.
pub struct ReverseLocalMapHandle {
    inner: Mutex<HashMap<MapData, ReverseLocalKeyPod, u32>>,
}

impl ReverseLocalMapHandle {
    /// Wrap a recovered `aya::maps::HashMap`.
    #[must_use]
    pub const fn new(map: HashMap<MapData, ReverseLocalKeyPod, u32>) -> Self {
        Self { inner: Mutex::new(map) }
    }

    /// Insert-or-replace the reverse mapping
    /// `(backend_ip, backend_port, proto) → vip`. Written reverse-first
    /// in the `register_local_backend` dual-write (DDD-1).
    ///
    /// # Errors
    ///
    /// Returns `MapError` on kernel-side rejection.
    pub fn upsert(
        &self,
        backend_ip: Ipv4Addr,
        backend_port: u16,
        proto: Proto,
        vip: Ipv4Addr,
    ) -> Result<(), MapError> {
        let key = ReverseLocalKeyPod::from_typed(BackendKey::new(backend_ip, backend_port, proto));
        let value = ReverseLocalMapValue::encode(vip);
        self.inner.lock().insert(key, value, 0)
    }

    /// Remove the reverse mapping for `(backend_ip, backend_port,
    /// proto)`. Idempotent — removing an absent entry is `Ok(())`.
    /// Called by `deregister_local_backend` (DDD-5a).
    ///
    /// # Errors
    ///
    /// Returns `MapError` on kernel-side rejection (NOT on a missing key).
    pub fn remove(
        &self,
        backend_ip: Ipv4Addr,
        backend_port: u16,
        proto: Proto,
    ) -> Result<(), MapError> {
        let key = ReverseLocalKeyPod::from_typed(BackendKey::new(backend_ip, backend_port, proto));
        // Bind the lock-guarded `Remove` result to a local so the mutex
        // guard drops before the match scrutinee is evaluated
        // (clippy::significant_drop_in_scrutinee). Idempotent per the
        // ADR-0053 deregister contract — a missing key is `Ok(())`,
        // mirroring `LocalBackendMapHandle::remove`.
        let outcome = self.inner.lock().remove(&key);
        // Three-arm match (mirrors `LocalBackendMapHandle::remove`,
        // Phase 16 review D7): `Ok(())` is the load-bearing success
        // path; `Err(KeyNotFound)` is the idempotent swallow the
        // ADR-0053 deregister contract mandates. The identical bodies
        // are the point — collapsing them would erase the distinct
        // semantic intent.
        #[allow(
            clippy::match_same_arms,
            reason = "each arm carries distinct semantic intent — success vs \
                      idempotent-swallow per the ADR-0053 deregister contract — \
                      that the collapsed form would erase"
        )]
        match outcome {
            Ok(()) => Ok(()),
            Err(MapError::KeyNotFound) => Ok(()), // idempotent per ADR-0053 deregister
            Err(e) => Err(e),
        }
    }

    /// Dump every `(ReverseLocalKeyPod, vip_host)` entry — the
    /// `bpftool map dump`-equivalent surface the Tier-3 acceptance
    /// asserts on (S-01-02, S-02-03). Mirrors
    /// `EbpfDataplane::local_backend_map_entries`.
    ///
    /// # Errors
    ///
    /// Returns `MapError` on a kernel-side read failure.
    pub fn entries(&self) -> Result<Vec<(ReverseLocalKeyPod, u32)>, MapError> {
        let guard = self.inner.lock();
        let mut out = Vec::new();
        for entry in guard.iter() {
            out.push(entry?);
        }
        drop(guard);
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    //! S-01-02 — userspace `ReverseLocalKeyPod` host-order layout +
    //! `BackendKey → vip` roundtrip proptest.
    //!
    //! There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for the
    //! `cgroup_recvmsg4_service` program that reads `REVERSE_LOCAL_MAP`
    //! (verifier `[1,1]`, research Q1 / `.claude/rules/development.md` §
    //! "`bpf_sock_addr.user_port`"). This proptest is the userspace-edge
    //! compensation: it pins the 8-byte key layout (backend_ip:4 +
    //! backend_port:2 + proto:1 + _pad:1), proto at byte offset 6,
    //! trailing pad zeroed, the no-userspace-flip invariant (host-order
    //! in == host-order bytes out), and the VIP value round-trips
    //! host-order. Endianness lockstep per ADR-0041: userspace stores
    //! host-order; the kernel converts at the read boundary.

    use std::net::Ipv4Addr;

    use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
    use proptest::prelude::*;

    use crate::maps::reverse_local_map_handle::{ReverseLocalKeyPod, ReverseLocalMapValue};

    /// 8-byte key layout is the byte-for-byte contract the kernel-side
    /// `ReverseLocalKey` mirrors (byte-parity with the three shipped
    /// keys, DDD-2). A drift off 8 bytes silently mis-keys every
    /// recvmsg4 reverse lookup.
    #[test]
    fn reverse_local_key_pod_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<ReverseLocalKeyPod>(), 8);
    }

    fn arb_proto() -> impl Strategy<Value = Proto> {
        prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
    }

    proptest! {
        /// For any `(backend_ip, NonZeroU16 backend_port, proto)` and any
        /// VIP, the host-order `ReverseLocalKeyPod` the handle builds from
        /// the `BackendKey` newtype is byte-identical to the key the
        /// kernel rebuilds from the same backend identity, and the VIP
        /// value round-trips host-order:
        ///
        /// - `backend_ip_host` host-order (no userspace flip),
        /// - `backend_port_host` host-order,
        /// - `proto` the IANA byte at offset 6,
        /// - trailing `_pad` (offset 7) zeroed,
        /// - the VIP `u32` round-trips host-order through the value
        ///   encode/decode pair.
        ///
        /// Catches a mis-slotted proto byte, a non-zero pad, a sneaky
        /// endianness flip at the userspace edge, or a VIP byte-swap.
        #[test]
        fn reverse_local_key_pod_bytes_match_kernel_build(
            backend_ip in any::<u32>(),
            backend_port in 1u16..=u16::MAX,
            proto in arb_proto(),
            vip in any::<u32>(),
        ) {
            let backend_v4 = Ipv4Addr::from(backend_ip);
            let vip_v4 = Ipv4Addr::from(vip);
            let key = ReverseLocalKeyPod::from_typed(BackendKey::new(
                backend_v4,
                backend_port,
                proto,
            ));

            // No userspace flip: host-order in == host-order field.
            prop_assert_eq!(key.backend_ip_host, backend_ip);
            prop_assert_eq!(key.backend_port_host, backend_port);
            prop_assert_eq!(key.proto, proto.as_u8());

            // Byte-for-byte: proto lands at offset 6, trailing pad
            // (offset 7) is zeroed. Asserting the pad through the byte
            // view (not the `_pad` field) keeps clippy's
            // `used_underscore_binding` happy while still pinning that
            // the construction zeroes it.
            let key_bytes: [u8; 8] = unsafe { core::mem::transmute(key) };
            prop_assert_eq!(key_bytes[6], proto.as_u8(), "proto byte at offset 6");
            prop_assert_eq!(key_bytes[7], 0u8, "trailing pad byte at offset 7 is zero");

            // VIP value lockstep: the handle stores the VIP as a
            // host-order u32; encoding then decoding the value byte
            // sequence round-trips the original host-order VIP (the
            // recvmsg4 program reads these bytes and converts to NBO at
            // the write boundary). Exercises the value codec that GREEN
            // lands; RED against the absent decode helper.
            let vip_host = ReverseLocalMapValue::encode(vip_v4);
            let decoded = ReverseLocalMapValue::decode(vip_host);
            prop_assert_eq!(decoded, vip_v4, "VIP host-order round-trip");
            prop_assert_eq!(vip_host, vip, "VIP stored host-order, no flip");
        }
    }
}
