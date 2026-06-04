//! `ServiceMapHandle` — typed userspace wrapper around the
//! `SERVICE_MAP` outer `BPF_MAP_TYPE_HASH_OF_MAPS` per
//! architecture.md § 10.
//!
//! Outer key = `(ServiceVip, u16 port)` (host-order in the
//! map; converted at the kernel boundary § 11). Inner =
//! per-service `BPF_MAP_TYPE_HASH` of `BackendId` →
//! `BackendEntry`.
//!
//! # Slice 02 scope
//!
//! Phase 2.2 Slice 02 (this step, 02-02) lands the **userspace
//! half**: a typed wrapper over an in-memory backing store with
//! the exact key/value shape and host-order encoding the kernel
//! will see in Slice 03. Slice 03 (US-03; S-2.2-09..11) wraps
//! this in `aya::maps::HashMap` against the real BPF object once
//! `SERVICE_MAP` is declared in `crates/overdrive-bpf/src/maps/
//! service_map.rs`. Slice 03 also lands the atomic-swap surface
//! (`swap_inner(service_id, vip, new_inner)`).
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! Map storage format is **host byte order** (LE on every kernel
//! matrix entry per `testing.md` § Kernel matrix). Userspace
//! reads / writes maps in host order **without** `htonl` /
//! `ntohl` calls; only the kernel-side hot path performs the
//! wire→host conversion in `crates/overdrive-bpf/src/shared/
//! sanity.rs`. This module's proptest pins the no-userspace-flip
//! invariant: a host-order write read back as host-order bytes
//! is byte-for-byte identical to the input — no sneaky
//! `to_be_bytes` slipping in at the userspace edge.
//!
//! See test-scenarios.md S-2.2-04..06 (Slice 02), S-2.2-09..11
//! (Slice 03).

use std::collections::BTreeMap;

use overdrive_core::dataplane::backend_key::Proto;
use overdrive_core::id::ServiceVip;
use overdrive_core::traits::dataplane::{Backend, DataplaneError};

/// Outer-map key for SERVICE_MAP. 8-byte POD; all fields
/// host-order. The `_pad` field exists to make the struct's
/// in-memory size match the BPF map's byte layout (kernel-side
/// reads will `&` the same 8 bytes; padding alignment is
/// load-bearing for BPF).
///
/// Construction from `(ServiceVip, u16)` is the **only** edge at
/// which a userspace caller's IPv4 address is converted to a
/// `u32`. The conversion preserves host-order semantics — IPv4
/// octets `a.b.c.d` map to the host-order `u32` whose four bytes
/// (LE on x86-64 / aarch64) are `[d, c, b, a]`. The kernel reads
/// the BPF map with `bpf_map_lookup_elem` against the same
/// 8-byte struct value and never touches `to_be` / `from_be` on
/// it (architecture.md § 11). IPv6 VIPs are not yet supported in
/// the SERVICE_MAP key — the outer-map shape is fixed-width
/// 8 bytes; IPv6 lands in GH #155 (architecture.md § 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub(crate) struct ServiceKey {
    /// IPv4 VIP, host-order. Octets `a.b.c.d` = `u32::from(Ipv4Addr::new(a, b, c, d))`.
    pub(crate) vip_host: u32,
    /// VIP port (the port the client connected to), host-order.
    pub(crate) port_host: u16,
    /// IANA L4 protocol byte — TCP=6, UDP=17 (`Proto::as_u8()`). Step
    /// 02-01 widened the outer key from `(vip, port)` to
    /// `(vip, port, proto)` IPVS-style (ADR-0040 rev 2026-06-03):
    /// `proto` absorbs one of the two reserved `_pad` bytes so
    /// tcp/8080 and udp/8080 occupy distinct outer-map slots. No
    /// endianness concern — a single byte is orthogonal to the
    /// u16/u32 host-order lockstep.
    pub(crate) proto: u8,
    /// Padding to 8-byte alignment. Always zero — deterministic BPF
    /// hashing keys on raw bytes, so a non-zero pad would split
    /// logically-equal keys across slots.
    pub(crate) _pad: u8,
}

// Compile-time guard: the outer key MUST stay 8 bytes (ADR-0040 rev
// "absorb the pad byte, keep 8 bytes"). A layout drift off 8 bytes
// breaks kernel/userspace byte-for-byte parity and fails here at
// build time rather than as a silent misroute at runtime.
const _: () = assert!(
    core::mem::size_of::<ServiceKey>() == 8,
    "ServiceKey must be exactly 8 bytes (vip_host:4 + port_host:2 + proto:1 + _pad:1)"
);

impl ServiceKey {
    /// Encode `(ServiceVip, u16, Proto)` to the host-order 8-byte POD.
    /// `proto` lowers to its IANA byte via [`Proto::as_u8`] (TCP=6,
    /// UDP=17). Returns `DataplaneError::LoadFailed` for IPv6 VIPs (the
    /// SERVICE_MAP outer-key shape is fixed-width 4-byte IPv4 in
    /// Phase 2.2; IPv6 deferred per architecture.md § 6).
    fn from_vip_port(vip: ServiceVip, port: u16, proto: Proto) -> Result<Self, DataplaneError> {
        match vip.get() {
            std::net::IpAddr::V4(v4) => Ok(Self {
                vip_host: u32::from(v4),
                port_host: port,
                proto: proto.as_u8(),
                _pad: 0,
            }),
            std::net::IpAddr::V6(_) => Err(DataplaneError::LoadFailed(
                "ServiceMapHandle: IPv6 VIP not supported in Phase 2.2 SERVICE_MAP key (deferred to GH #155)".into(),
            )),
        }
    }
}

/// Inner-map value for a single backend. 12 bytes, all host-
/// order. Matches the BACKEND_MAP value shape from
/// architecture.md § 10. Slice 03 wraps a per-service inner map
/// of `BackendId → BackendEntry`; for Slice 02's userspace half
/// the handle stores a single backend per `(VIP, port)` directly,
/// pending HASH_OF_MAPS landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct BackendEntry {
    /// Backend IPv4 address, host-order.
    pub(crate) ipv4_host: u32,
    /// Backend port, host-order.
    pub(crate) port_host: u16,
    /// Load-balancing weight, host-order.
    pub(crate) weight: u16,
    /// Liveness flag. 1 = healthy, 0 = unhealthy.
    pub(crate) healthy: u8,
    /// Padding to 12-byte alignment. Always zero.
    pub(crate) _pad: [u8; 3],
}

impl BackendEntry {
    /// Encode a `Backend` (from the trait surface) into the
    /// host-order POD. Returns `DataplaneError::LoadFailed` for
    /// IPv6 backend addresses (Phase 2.2 is IPv4-only end-to-
    /// end per architecture.md § 6).
    fn from_backend(backend: &Backend) -> Result<Self, DataplaneError> {
        match backend.addr.ip() {
            std::net::IpAddr::V4(v4) => Ok(Self {
                ipv4_host: u32::from(v4),
                port_host: backend.addr.port(),
                weight: backend.weight,
                healthy: u8::from(backend.healthy),
                _pad: [0; 3],
            }),
            std::net::IpAddr::V6(_) => Err(DataplaneError::LoadFailed(
                "ServiceMapHandle: IPv6 backend address not supported in Phase 2.2 (deferred to GH #155)".into(),
            )),
        }
    }
}

/// Typed wrapper around the SERVICE_MAP backing store.
///
/// # Phase 2.2 Slice 02
///
/// Backed by an in-memory `BTreeMap` with the exact byte layout
/// the kernel-side BPF program will see in Slice 03. The choice
/// of `BTreeMap` (not `HashMap`) follows
/// `.claude/rules/development.md` § Ordered-collection choice —
/// proptest assertions iterate the map and would race on
/// `HashMap`'s `RandomState`-keyed traversal under DST seeds.
///
/// # Slice 03 graduation
///
/// Slice 03 replaces the `BTreeMap` field with an
/// `aya::maps::HashMap<MapData, ServiceKey, ServiceValue>`
/// retrieved from `aya::Ebpf::map_mut("SERVICE_MAP")`. The public
/// `insert` / `remove` surface stays unchanged; the wrap point
/// for atomic-swap (`swap_inner`) lands in the same step.
pub struct ServiceMapHandle {
    /// In-memory backing store. `BTreeMap` (not `HashMap`) per
    /// the deterministic-iteration rule above.
    backing: BTreeMap<ServiceKey, BackendEntry>,
}

impl ServiceMapHandle {
    /// Construct an empty handle. The Slice 02 in-memory backing
    /// is created here; Slice 03 reshapes the constructor to
    /// accept an `aya::maps::HashMap` instead.
    #[must_use]
    pub const fn new() -> Self {
        Self { backing: BTreeMap::new() }
    }

    /// Insert a single backend under the `(VIP, port)` outer
    /// key. Host-order encoding happens here and only here —
    /// callers pass the typed `ServiceVip` / `Backend` and never
    /// see the raw `u32` / `[u8; …]` representation.
    ///
    /// Errors:
    /// - `DataplaneError::LoadFailed` for IPv6 VIPs or IPv6
    ///   backend addresses (Phase 2.2 deferral).
    pub fn insert(
        &mut self,
        vip: ServiceVip,
        port: u16,
        proto: Proto,
        backend: &Backend,
    ) -> Result<(), DataplaneError> {
        let key = ServiceKey::from_vip_port(vip, port, proto)?;
        let value = BackendEntry::from_backend(backend)?;
        self.backing.insert(key, value);
        Ok(())
    }

    /// Remove the entry under `(VIP, port)`. Idempotent — a
    /// missing entry is not an error (matches the kernel
    /// `bpf_map_delete_elem`'s `ENOENT` semantics, which
    /// userspace treats as no-op for the hydrator's converge-on-
    /// retry loop).
    pub fn remove(
        &mut self,
        vip: ServiceVip,
        port: u16,
        proto: Proto,
    ) -> Result<(), DataplaneError> {
        let key = ServiceKey::from_vip_port(vip, port, proto)?;
        self.backing.remove(&key);
        Ok(())
    }

    /// Test-only readback — returns the host-order
    /// `BackendEntry` written under `(VIP, port)`, or `None` if
    /// no entry exists. Lives `pub(crate)` because the proptest
    /// in `mod tests` below is the only legitimate consumer; the
    /// production `EbpfDataplane::update_service` write path
    /// does not read back through the handle.
    #[cfg(test)]
    pub(crate) fn get_for_test(
        &self,
        vip: ServiceVip,
        port: u16,
        proto: Proto,
    ) -> Option<BackendEntry> {
        ServiceKey::from_vip_port(vip, port, proto)
            .ok()
            .and_then(|key| self.backing.get(&key).copied())
    }
}

impl Default for ServiceMapHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    //! S-2.2-04 (handle endianness roundtrip portion) — userspace
    //! proptest over `ServiceMapHandle::insert` /
    //! `get_for_test`. Pins the no-userspace-flip invariant: a
    //! host-order write read back as host-order bytes is
    //! byte-for-byte identical to the input.
    //!
    //! Architecture.md § 11 requires that userspace **never**
    //! flip endianness — the kernel-side hot path is the only
    //! conversion site. A regression that sneaks `to_be_bytes` /
    //! `from_be_bytes` into the handle's encode/decode would
    //! break this proptest at the round-trip equality
    //! assertion.

    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

    use overdrive_core::dataplane::backend_key::Proto;
    use overdrive_core::id::{ServiceVip, SpiffeId};
    use overdrive_core::traits::dataplane::Backend;
    use proptest::prelude::*;

    use super::{BackendEntry, ServiceKey, ServiceMapHandle};

    /// Generator over the two recognised L4 protocols. Step 02-01
    /// widened the SERVICE_MAP key with a proto byte; every key-shape
    /// proptest exercises both arms.
    fn arb_proto() -> impl Strategy<Value = Proto> {
        prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
    }

    /// Generator for an arbitrary IPv4 `ServiceVip`. Includes
    /// edge cases (0.0.0.0, 255.255.255.255, common host bits)
    /// because proptest's default `u32` shrinker covers the
    /// boundary cleanly.
    fn arb_ipv4_vip() -> impl Strategy<Value = ServiceVip> {
        any::<u32>().prop_map(|raw| {
            let v4 = Ipv4Addr::from(raw);
            ServiceVip::new(IpAddr::V4(v4)).expect("ServiceVip::new accepts every IPv4")
        })
    }

    /// Generator for an arbitrary IPv4 `Backend`. The SPIFFE ID
    /// is a fixed valid sentinel — it does not participate in
    /// the SERVICE_MAP key/value shape; the proptest covers the
    /// IP/port/weight/healthy axes.
    fn arb_ipv4_backend() -> impl Strategy<Value = Backend> {
        (any::<u32>(), any::<u16>(), any::<u16>(), any::<bool>()).prop_map(
            |(ip, port, weight, healthy)| Backend {
                alloc: SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/test")
                    .expect("sentinel SVID parses"),
                addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(ip), port)),
                weight,
                healthy,
            },
        )
    }

    proptest! {
        /// S-02-01 — endianness lockstep over the FULL 8 key bytes.
        /// For any `(vip, port, proto)`, the host-order `ServiceKey`
        /// from `from_vip_port` is byte-identical to the key the
        /// kernel would build from the same IPv4 header: `vip_host`
        /// via `u32::from_be_bytes(octets)`, `port_host` the raw
        /// host-order port, `proto` the IPv4 proto byte, and trailing
        /// `_pad == 0`. Catches any sneaky `to_be_bytes` /
        /// `from_be_bytes` slipping into the userspace edge, a
        /// mis-sloated proto byte, or a non-zero pad.
        #[test]
        fn service_key_bytes_match_kernel_header_build(
            vip in arb_ipv4_vip(),
            vip_port in any::<u16>(),
            proto in arb_proto(),
            backend in arb_ipv4_backend(),
        ) {
            let mut handle = ServiceMapHandle::new();
            handle.insert(vip, vip_port, proto, &backend).expect("IPv4 inputs always insert");

            let stored = handle.get_for_test(vip, vip_port, proto)
                .expect("just-inserted key must read back");

            let backend_v4 = match backend.addr.ip() {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => unreachable!("arb_ipv4_backend only emits IPv4"),
            };
            let expected = BackendEntry {
                ipv4_host: u32::from(backend_v4),
                port_host: backend.addr.port(),
                weight: backend.weight,
                healthy: u8::from(backend.healthy),
                _pad: [0; 3],
            };
            prop_assert_eq!(stored, expected);

            // The kernel builds the outer key from the IPv4 header:
            //   vip_host = u32::from_be_bytes(dst_ip_octets)
            //   port_host = dst_port (host-order)
            //   proto = ipv4 proto byte (IANA u8)
            //   _pad = 0
            let vip_v4 = match vip.get() {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => unreachable!("arb_ipv4_vip only emits IPv4"),
            };
            let kernel_key = ServiceKey {
                vip_host: u32::from_be_bytes(vip_v4.octets()),
                port_host: vip_port,
                proto: proto.as_u8(),
                _pad: 0,
            };
            // Byte-for-byte equality over all 8 bytes — the handle's
            // host-order key and the kernel's header-built key must
            // be identical, including the zeroed trailing pad.
            let handle_bytes: [u8; 8] = unsafe {
                core::mem::transmute(ServiceKey {
                    vip_host: u32::from(vip_v4),
                    port_host: vip_port,
                    proto: proto.as_u8(),
                    _pad: 0,
                })
            };
            let kernel_bytes: [u8; 8] = unsafe { core::mem::transmute(kernel_key) };
            prop_assert_eq!(handle_bytes, kernel_bytes);
            prop_assert_eq!(handle_bytes[7], 0u8, "trailing _pad byte must be zero");

            // The stored entry is reachable under the proto-bearing key.
            prop_assert!(handle.backing.contains_key(&kernel_key));
        }

        /// S-02-01 — proto is a load-bearing key component. Inserting
        /// under `(vip, port, Tcp)` does NOT make the entry reachable
        /// under `(vip, port, Udp)` — distinct outer-map slots, the
        /// DNS co-location unlock.
        #[test]
        fn distinct_proto_is_distinct_slot(
            vip in arb_ipv4_vip(),
            vip_port in any::<u16>(),
            backend in arb_ipv4_backend(),
        ) {
            let mut handle = ServiceMapHandle::new();
            handle.insert(vip, vip_port, Proto::Tcp, &backend)
                .map_err(|e| TestCaseError::fail(e.to_string()))?;

            // Same (vip, port) under the OTHER proto must miss.
            prop_assert!(handle.get_for_test(vip, vip_port, Proto::Udp).is_none());
            // The inserted proto reads back.
            prop_assert!(handle.get_for_test(vip, vip_port, Proto::Tcp).is_some());
        }

        /// Remove is idempotent and only affects the targeted
        /// `(VIP, port, proto)` — adjacent entries survive.
        #[test]
        fn service_map_handle_remove_is_targeted(
            vip in arb_ipv4_vip(),
            port_a in any::<u16>(),
            port_b in any::<u16>(),
            proto in arb_proto(),
            backend in arb_ipv4_backend(),
        ) {
            prop_assume!(port_a != port_b);
            let mut handle = ServiceMapHandle::new();
            handle.insert(vip, port_a, proto, &backend).map_err(|e| TestCaseError::fail(e.to_string()))?;
            handle.insert(vip, port_b, proto, &backend).map_err(|e| TestCaseError::fail(e.to_string()))?;

            handle.remove(vip, port_a, proto).map_err(|e| TestCaseError::fail(e.to_string()))?;

            prop_assert!(handle.get_for_test(vip, port_a, proto).is_none());
            prop_assert!(handle.get_for_test(vip, port_b, proto).is_some());

            // Idempotent — second remove is a no-op, not an error.
            handle.remove(vip, port_a, proto).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
    }

    /// IPv6 VIP rejection — `ServiceMapHandle::insert` returns
    /// `LoadFailed` on IPv6 inputs (Phase 2.2 IPv4-only end-to-
    /// end per architecture.md § 6 / GH #155 deferral).
    #[test]
    fn ipv6_vip_is_rejected_with_load_failed() {
        let v6_vip = ServiceVip::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST))
            .expect("ServiceVip::new accepts IPv6 at the type level");
        let backend = Backend {
            alloc: SpiffeId::new("spiffe://overdrive.local/job/svc/alloc/test").unwrap(),
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 80)),
            weight: 1,
            healthy: true,
        };
        let mut handle = ServiceMapHandle::new();
        match handle.insert(v6_vip, 8080, Proto::Tcp, &backend) {
            Err(super::DataplaneError::LoadFailed(msg)) => {
                assert!(msg.contains("IPv6"), "unexpected diagnostic: {msg}");
            }
            other => panic!("expected LoadFailed for IPv6 VIP, got {other:?}"),
        }
    }
}
