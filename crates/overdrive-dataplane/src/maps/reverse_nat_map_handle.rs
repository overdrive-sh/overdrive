//! `ReverseNatMapHandle` — typed userspace wrapper around the
//! `REVERSE_NAT_MAP` `BPF_MAP_TYPE_HASH` per architecture.md § 10.
//!
//! Outer key = `BackendKey { ip_host, port_host, proto, _pad }`
//! (host-order in the map); value = `Vip { ip_host, port_host,
//! _pad }` (host-order). Matches the kernel-side declaration in
//! `crates/overdrive-bpf/src/maps/reverse_nat_map.rs` byte-for-byte.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Map storage format is **host byte order** (LE on every kernel
//! matrix entry per `testing.md` § Kernel matrix). Userspace reads
//! / writes maps in host order **without** `htonl` / `ntohl` calls;
//! only the kernel-side hot path performs the wire→host conversion
//! in `crates/overdrive-bpf/src/shared/sanity.rs`'s
//! `reverse_key_from_packet`. This module's proptest pins the
//! no-userspace-flip invariant: a host-order write read back as
//! host-order bytes is byte-for-byte identical to the input — no
//! sneaky `to_be_bytes` slipping in at the userspace edge.
//!
//! # Slice 05 scope
//!
//! Phase 2.2 Slice 05 ships the **userspace half**: a typed wrapper
//! over an in-memory backing store with the exact key/value shape
//! and host-order encoding the kernel sees. Promotion to wrap an
//! `aya::maps::HashMap` against the real BPF object lands in the
//! same Slice 05 work as the production-binding closure. The
//! Tier 2 test (`crates/overdrive-bpf/tests/integration/
//! reverse_key_roundtrip.rs`) is the kernel-side complement and
//! exercises the real map; this proptest exercises the userspace
//! lockstep contract in isolation.

use std::collections::BTreeMap;

use overdrive_core::dataplane::backend_key::BackendKey;

/// `REVERSE_NAT_MAP` outer-key POD. 8 bytes, all fields host-order.
/// Matches `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`'s
/// kernel-side `BackendKey` byte-for-byte. Renamed to
/// `BackendKeyPod` here to avoid a name clash with the typed
/// `overdrive_core::dataplane::backend_key::BackendKey` newtype that
/// the public API accepts; the POD is the wire-shape the BPF map
/// keys on, the newtype is the type-system distinctness layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub(crate) struct BackendKeyPod {
    /// Backend IPv4 address. Host-order.
    pub(crate) ip_host: u32,
    /// Backend port. Host-order.
    pub(crate) port_host: u16,
    /// L4 protocol — IANA proto number (TCP=6, UDP=17).
    pub(crate) proto: u8,
    /// Padding for 8-byte alignment in BPF map storage. Always 0.
    pub(crate) _pad: u8,
}

impl BackendKeyPod {
    /// Encode a typed `BackendKey` newtype to the host-order POD.
    /// Infallible for IPv4 — every `(Ipv4Addr, u16, Proto)` triple
    /// is a valid POD shape; the newtype already validates the
    /// inputs at construction.
    fn from_typed(key: BackendKey) -> Self {
        Self { ip_host: u32::from(key.ip), port_host: key.port, proto: key.proto.as_u8(), _pad: 0 }
    }
}

/// `REVERSE_NAT_MAP` value POD. 8 bytes, host-order. The original
/// VIP the client connected to — used by the egress reverse-NAT
/// program to rewrite the source 5-tuple of the backend response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct VipPod {
    /// VIP IPv4 address. Host-order.
    pub(crate) ip_host: u32,
    /// VIP port. Host-order.
    pub(crate) port_host: u16,
    /// Padding for 8-byte alignment. Always 0.
    pub(crate) _pad: u16,
}

/// Typed wrapper around the `REVERSE_NAT_MAP` backing store.
///
/// # Phase 2.2 Slice 05
///
/// Backed by an in-memory `BTreeMap` with the exact byte layout the
/// kernel-side BPF program sees. Choice of `BTreeMap` (not
/// `HashMap`) follows `.claude/rules/development.md`
/// § Ordered-collection choice — proptest assertions iterate the
/// map and would race on `HashMap`'s `RandomState` traversal under
/// DST seeds.
///
/// The public `insert` / `remove` surface mirrors `ServiceMapHandle`
/// — host-order encoding happens here and only here. Callers pass
/// the typed `BackendKey` newtype and never see the raw `u32` /
/// `u8` POD fields.
pub struct ReverseNatMapHandle {
    /// In-memory backing store. `BTreeMap` (not `HashMap`) per the
    /// deterministic-iteration rule above.
    backing: BTreeMap<BackendKeyPod, VipPod>,
}

impl ReverseNatMapHandle {
    /// Construct an empty handle.
    #[must_use]
    pub const fn new() -> Self {
        Self { backing: BTreeMap::new() }
    }

    /// Insert a `BackendKey → Vip` mapping. Host-order encoding
    /// happens here — the typed `BackendKey` newtype carries
    /// `Ipv4Addr`/`u16`/`Proto`; the stored POD carries
    /// `u32`/`u16`/`u8` in host order.
    ///
    /// `vip_ip` and `vip_port` are the *original* VIP the client
    /// connected to — what the reverse-NAT program rewrites the
    /// backend response source 5-tuple back to.
    ///
    /// Infallible today: `BackendKey` is statically IPv4 and every
    /// `(Ipv4Addr, u16, Proto)` triple maps to a valid POD shape.
    /// When IPv6 support lands (GH #155), this signature gains a
    /// `Result` return at the point the failure mode becomes real
    /// — speculative `Result` wrapping today would just trip
    /// clippy's `unnecessary_wraps` lint per
    /// `.claude/rules/development.md` § Errors.
    pub fn insert(&mut self, key: BackendKey, vip_ip: std::net::Ipv4Addr, vip_port: u16) {
        let key_pod = BackendKeyPod::from_typed(key);
        let vip_pod = VipPod { ip_host: u32::from(vip_ip), port_host: vip_port, _pad: 0 };
        self.backing.insert(key_pod, vip_pod);
    }

    /// Remove a `BackendKey` entry. Idempotent — a missing entry is
    /// not an error (matches the kernel `bpf_map_delete_elem`'s
    /// `ENOENT` semantics, which userspace treats as no-op for the
    /// hydrator's converge-on-retry loop).
    pub fn remove(&mut self, key: BackendKey) {
        let key_pod = BackendKeyPod::from_typed(key);
        self.backing.remove(&key_pod);
    }

    /// Test-only readback — returns the stored host-order
    /// `(ip, port)` for a given `BackendKey`, or `None` if no entry
    /// exists. The proptest in `mod tests` is the only legitimate
    /// consumer; production write paths do not read back through
    /// the handle.
    #[cfg(test)]
    pub(crate) fn get_for_test(&self, key: BackendKey) -> Option<VipPod> {
        let key_pod = BackendKeyPod::from_typed(key);
        self.backing.get(&key_pod).copied()
    }
}

impl Default for ReverseNatMapHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    //! S-2.2-17 (handle endianness roundtrip portion) — userspace
    //! proptest over `ReverseNatMapHandle::insert` /
    //! `get_for_test`. Pins the no-userspace-flip invariant: a
    //! host-order write read back as host-order bytes is
    //! byte-for-byte identical to the input.
    //!
    //! Architecture.md § 11 requires that userspace **never** flip
    //! endianness — the kernel-side hot path is the only conversion
    //! site. A regression that sneaks `to_be_bytes` /
    //! `from_be_bytes` into the handle's encode/decode would break
    //! this proptest at the round-trip equality assertion.

    use std::net::Ipv4Addr;

    use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
    use proptest::prelude::*;

    use super::{BackendKeyPod, ReverseNatMapHandle, VipPod};

    /// Generator for an arbitrary `Proto`. Phase 2.2 supports
    /// exactly TCP and UDP — IPv6 / ICMP / SCTP are GH #155 / future
    /// deferrals (architecture.md § 6).
    fn arb_proto() -> impl Strategy<Value = Proto> {
        prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
    }

    /// Generator for an arbitrary IPv4 `BackendKey`. Includes edge
    /// cases (0.0.0.0, 255.255.255.255, common host bits) because
    /// proptest's default `u32` shrinker covers the boundary
    /// cleanly.
    fn arb_backend_key() -> impl Strategy<Value = BackendKey> {
        (any::<u32>(), any::<u16>(), arb_proto())
            .prop_map(|(ip, port, proto)| BackendKey::new(Ipv4Addr::from(ip), port, proto))
    }

    proptest! {
        /// S-2.2-17 (userspace half) — host-order write → host-order
        /// read produces byte-for-byte identical bytes for every
        /// IPv4 `BackendKey` × `(VIP, port)` tuple. Catches any
        /// sneaky `to_be_bytes` / `from_be_bytes` slipping into the
        /// userspace edge.
        #[test]
        fn reverse_nat_handle_endianness_roundtrip(
            key in arb_backend_key(),
            vip_raw in any::<u32>(),
            vip_port in any::<u16>(),
        ) {
            let vip_ip = Ipv4Addr::from(vip_raw);
            let mut handle = ReverseNatMapHandle::new();
            handle.insert(key, vip_ip, vip_port);

            let stored = handle.get_for_test(key).expect("just-inserted key must read back");

            // Reconstruct the expected host-order POD directly from
            // the typed input — this is the load-bearing assertion.
            // If the handle slipped a network-order flip in
            // anywhere, `stored.ip_host` would not equal
            // `u32::from(vip_ip)` and the test fails at the
            // field-by-field assert below.
            let expected = VipPod {
                ip_host: u32::from(vip_ip),
                port_host: vip_port,
                _pad: 0,
            };
            prop_assert_eq!(stored, expected);

            // Round-trip the key as well — the same no-flip rule
            // applies to the outer-map key. `backing` is
            // `pub(crate)`-accessible from the proptest because the
            // test lives in `mod tests` inside the same crate.
            let expected_key = BackendKeyPod {
                ip_host: u32::from(key.ip),
                port_host: key.port,
                proto: key.proto.as_u8(),
                _pad: 0,
            };
            prop_assert!(handle.backing.contains_key(&expected_key));
        }

        /// `remove` is idempotent and only affects the targeted key
        /// — adjacent entries survive.
        #[test]
        fn reverse_nat_handle_remove_is_targeted(
            key_a in arb_backend_key(),
            key_b in arb_backend_key(),
            vip_raw in any::<u32>(),
            vip_port in any::<u16>(),
        ) {
            prop_assume!(key_a != key_b);
            let vip_ip = Ipv4Addr::from(vip_raw);
            let mut handle = ReverseNatMapHandle::new();
            handle.insert(key_a, vip_ip, vip_port);
            handle.insert(key_b, vip_ip, vip_port);

            handle.remove(key_a);

            prop_assert!(handle.get_for_test(key_a).is_none());
            prop_assert!(handle.get_for_test(key_b).is_some());

            // Idempotent — second remove is a no-op.
            handle.remove(key_a);
        }
    }
}
