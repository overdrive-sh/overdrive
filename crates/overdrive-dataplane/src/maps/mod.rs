//! Typed userspace BPF map handles per architecture.md § 9 +
//! research recommendation #5 (typed map newtype API).
//!
//! Each handle wraps an `aya::maps::*` value and exposes an API
//! that hides `BPF_MAP_TYPE_*` choice + endianness conversion at
//! the call site. This is the
//! "make invalid states unrepresentable" discipline applied to
//! BPF map access (`.claude/rules/development.md` § Type-driven
//! design).
//!
//! **RED scaffolds** — every handle's bodies panic via `todo!()`
//! until DELIVER fills them slice by slice.

pub mod drop_counter_handle;
pub mod maglev_map_handle;
pub mod reverse_nat_map_handle;
pub mod service_map_handle;

// Typed userspace handle around `BPF_MAP_TYPE_HASH_OF_MAPS` —
// hand-rolled until aya 0.14+ / PR #1446. See research § D.3 +
// Appendix A.3.
pub mod hash_of_maps;

// Shared wire-shape POD types used across the typed handles AND the
// `EbpfDataplane` struct fields. `ServiceKey` is the 8-byte outer-map
// key; `BackendEntryPod` is the 12-byte BACKEND_MAP value. Both are
// host-order per architecture.md § 11. Kept here (not in
// `service_map_handle.rs` `pub(crate)`) so `EbpfDataplane` fields can
// name them without re-exporting handle internals.
//
// `pub _pad` is a load-bearing wire-shape padding field — its byte
// position determines the struct's kernel-readable layout. Renaming
// it loses the "this is padding" signal at every call site.
#[allow(clippy::pub_underscore_fields)]
pub mod wire {
    use overdrive_core::dataplane::backend_key::{BackendKey, Proto};
    use overdrive_core::id::ServiceVip;
    use overdrive_core::traits::dataplane::{Backend, DataplaneError};

    /// Outer-map key for SERVICE_MAP. 8-byte POD; host-order. Matches
    /// `crates/overdrive-bpf/src/maps/service_map.rs` `ServiceKey`
    /// byte-for-byte (vip_host: u32, port_host: u16, proto: u8, _pad: u8).
    ///
    /// Step 02-01 widened the key from `(vip, port)` to
    /// `(vip, port, proto)` IPVS-style (ADR-0040 rev 2026-06-03): the
    /// IANA L4 proto byte (TCP=6, UDP=17) absorbs one reserved pad byte
    /// so tcp/8080 and udp/8080 occupy distinct outer-map slots. The
    /// struct stays 8 bytes; the trailing `_pad` stays zeroed (BPF hashes
    /// raw key bytes).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct ServiceKey {
        /// IPv4 VIP, host-order. Octets `a.b.c.d` =
        /// `u32::from(Ipv4Addr::new(a, b, c, d))`.
        pub vip_host: u32,
        /// VIP port, host-order.
        pub port_host: u16,
        /// IANA L4 protocol byte — TCP=6, UDP=17 (`Proto::as_u8()`).
        pub proto: u8,
        /// Padding to 8-byte alignment. Always zero.
        pub _pad: u8,
    }

    // Compile-time guard: the outer key MUST stay 8 bytes (ADR-0040 rev
    // "absorb the pad byte, keep 8 bytes").
    const _: () = assert!(
        core::mem::size_of::<ServiceKey>() == 8,
        "wire::ServiceKey must be exactly 8 bytes (vip_host:4 + port_host:2 + proto:1 + _pad:1)"
    );

    // SAFETY: repr(C), all fields fully byte-addressable, no
    // padding-uninit issues (we always set _pad to 0).
    unsafe impl crate::maps::hash_of_maps::Pod for ServiceKey {}

    // SAFETY: same as above; `aya::Pod` permits raw map insert.
    unsafe impl aya::Pod for ServiceKey {}

    impl ServiceKey {
        /// Encode `(ServiceVip, u16, Proto)` to the host-order POD.
        /// `proto` lowers to its IANA byte via [`Proto::as_u8`]. Returns
        /// `LoadFailed` for IPv6 VIPs (Phase 2.2 IPv4-only per
        /// architecture.md § 6 / GH #155 deferral).
        pub fn from_vip_port(
            vip: ServiceVip,
            port: u16,
            proto: Proto,
        ) -> Result<Self, DataplaneError> {
            match vip.get() {
                std::net::IpAddr::V4(v4) => Ok(Self {
                    vip_host: u32::from(v4),
                    port_host: port,
                    proto: proto.as_u8(),
                    _pad: 0,
                }),
                std::net::IpAddr::V6(_) => Err(DataplaneError::LoadFailed(
                    "ServiceKey: IPv6 VIP not supported in Phase 2.2 SERVICE_MAP key (deferred to GH #155)"
                        .into(),
                )),
            }
        }
    }

    /// BACKEND_MAP value — resolved backend POD. 12 bytes, all
    /// host-order. Matches `crates/overdrive-bpf/src/maps/backend_map.rs`
    /// `BackendEntry` byte-for-byte.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(C)]
    pub struct BackendEntryPod {
        /// Backend IPv4, host-order.
        pub ipv4_host: u32,
        /// Backend port, host-order.
        pub port_host: u16,
        /// Load-balancing weight, host-order.
        pub weight: u16,
        /// 1 = healthy, 0 = unhealthy.
        pub healthy: u8,
        /// Padding to 12-byte alignment.
        pub _pad: [u8; 3],
    }

    // SAFETY: repr(C), no padding-uninit issues (we always zero
    // `_pad`); aya needs the marker for raw map access.
    unsafe impl aya::Pod for BackendEntryPod {}

    impl BackendEntryPod {
        /// Encode a `Backend` (from the trait surface) into the
        /// host-order POD. Returns `LoadFailed` for IPv6 backend
        /// addresses (Phase 2.2 IPv4-only per architecture.md § 6).
        pub fn from_backend(backend: &Backend) -> Result<Self, DataplaneError> {
            match backend.addr.ip() {
                std::net::IpAddr::V4(v4) => Ok(Self {
                    ipv4_host: u32::from(v4),
                    port_host: backend.addr.port(),
                    weight: backend.weight,
                    healthy: u8::from(backend.healthy),
                    _pad: [0; 3],
                }),
                std::net::IpAddr::V6(_) => Err(DataplaneError::LoadFailed(
                    "BackendEntryPod: IPv6 backend address not supported in Phase 2.2 (deferred to GH #155)"
                        .into(),
                )),
            }
        }
    }

    /// `REVERSE_NAT_MAP` outer-key POD.
    ///
    /// 8-byte host-order tuple
    /// `(backend_ip, backend_port, proto, _pad)`. Matches
    /// `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`'s kernel
    /// `BackendKey` byte-for-byte. Used by the egress reverse-NAT
    /// program to look up `(backend_ip, backend_port, proto)` →
    /// `Vip` for source-rewrite on response packets.
    ///
    /// Slice 05-04 promotion: `BackendKeyPod` lives in the public
    /// `wire` module so `EbpfDataplane` can name it as a struct
    /// field type (the typed BPF map handle). The earlier
    /// `pub(crate)` POD in `reverse_nat_map_handle.rs` is the same
    /// shape, distinct only because that module ships an in-memory
    /// proptest stand-in.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct BackendKeyPod {
        /// Backend IPv4 address. Host-order.
        pub ip_host: u32,
        /// Backend port. Host-order.
        pub port_host: u16,
        /// L4 protocol — IANA proto number (TCP=6, UDP=17).
        pub proto: u8,
        /// Padding for 8-byte alignment in BPF map storage. Always 0.
        pub _pad: u8,
    }

    // SAFETY: repr(C), all fields fully byte-addressable, `_pad`
    // always set to 0. aya needs the marker for raw map access.
    unsafe impl aya::Pod for BackendKeyPod {}

    impl BackendKeyPod {
        /// Encode a typed `BackendKey` newtype to the host-order POD.
        /// Infallible for IPv4 — every `(Ipv4Addr, u16, Proto)` triple
        /// is a valid POD shape; the newtype already validates the
        /// inputs at construction.
        #[must_use]
        pub fn from_typed(key: BackendKey) -> Self {
            Self {
                ip_host: u32::from(key.ip),
                port_host: key.port,
                proto: key.proto.as_u8(),
                _pad: 0,
            }
        }
    }

    /// `REVERSE_NAT_MAP` value POD. 8-byte host-order
    /// `(vip_ip, vip_port)` tuple. Matches
    /// `crates/overdrive-bpf/src/maps/reverse_nat_map.rs`'s kernel
    /// `Vip` byte-for-byte.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(C)]
    pub struct VipPod {
        /// VIP IPv4 address. Host-order.
        pub ip_host: u32,
        /// VIP port. Host-order.
        pub port_host: u16,
        /// Padding for 8-byte alignment. Always 0.
        pub _pad: u16,
    }

    // SAFETY: repr(C), no padding-uninit issues (we always zero
    // `_pad`); aya needs the marker for raw map access.
    unsafe impl aya::Pod for VipPod {}

    /// `LOCAL_BACKEND_MAP` outer-key POD per ADR-0053 § 1.
    ///
    /// 8-byte host-order tuple `(vip, vip_port, _pad)`. Matches
    /// `crates/overdrive-bpf/src/maps/local_backend_map.rs`'s
    /// kernel `LocalServiceKey` byte-for-byte.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(C)]
    pub struct LocalServiceKey {
        /// VIP IPv4, host-order. `u32::from(Ipv4Addr::new(a, b, c, d))`.
        pub vip_host: u32,
        /// VIP port, host-order.
        pub port_host: u16,
        /// Padding to 8-byte alignment. Always zero.
        pub _pad: u16,
    }

    // SAFETY: repr(C), `_pad` always 0, all fields fully
    // byte-addressable. aya requires the marker for raw map access.
    unsafe impl aya::Pod for LocalServiceKey {}

    /// `LOCAL_BACKEND_MAP` outer-value POD per ADR-0053 § 1.
    /// 8-byte host-order tuple `(backend_ip, backend_port, _pad)`.
    /// Matches kernel `LocalBackendEntry` byte-for-byte.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    #[repr(C)]
    pub struct LocalBackendEntry {
        /// Backend IPv4, host-order.
        pub backend_ip_host: u32,
        /// Backend port, host-order.
        pub backend_port_host: u16,
        /// Padding for 8-byte alignment. Always zero.
        pub _pad: u16,
    }

    // SAFETY: same as LocalServiceKey.
    unsafe impl aya::Pod for LocalBackendEntry {}
}

// Re-export at the crate-public level for `EbpfDataplane` field
// naming and the integration tests.
pub use wire::{
    BackendEntryPod, BackendKeyPod, LocalBackendEntry, LocalServiceKey, ServiceKey, VipPod,
};

// Typed userspace handle for `LOCAL_BACKEND_MAP` per ADR-0053 § 1.
pub mod local_backend_map_handle;
