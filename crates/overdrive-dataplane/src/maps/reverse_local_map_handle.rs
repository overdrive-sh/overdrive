//! Typed userspace handle for `REVERSE_LOCAL_MAP` per ADR-0053 rev
//! 2026-06-05 (GH #200).
//!
//! Wraps an `aya::maps::HashMap<MapData, ReverseLocalKeyPod,
//! ReverseLocalEntryPod>` — the reply store for the UNCONNECTED-UDP
//! same-host cgroup path. Keyed by the backend identity
//! `(backend_ip, backend_port, proto)` (the `BackendKey` newtype,
//! byte-parity with the three existing keys, DDD-2), value = the
//! original VIP `(address, port)` (the 8-byte `ReverseLocalEntryPod`,
//! byte-parity with the kernel `ReverseLocalEntry`). The
//! `cgroup_recvmsg4_service` program reads it to rewrite the reply
//! source the app sees backend→VIP — BOTH address and port (ADR-0053
//! §D4).
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

/// `REVERSE_LOCAL_MAP` value POD — the original VIP `(address, port)`.
///
/// 8 bytes, host-order, byte-parity with the kernel `ReverseLocalEntry`.
/// The value width grew 4→8 in step 01-02 (the VIP port joined the VIP
/// address) so the recvmsg4 reverse rewrite can restore BOTH the source
/// address and the source port (ADR-0053 §D4).
///
/// `pub _pad` is a load-bearing wire-shape padding field — its byte
/// position determines the struct's kernel-readable layout. Renaming it
/// loses the "this is padding" signal at every call site.
#[allow(clippy::pub_underscore_fields)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct ReverseLocalEntryPod {
    /// Original VIP IPv4, host-order.
    pub vip_host: u32,
    /// Original VIP port, host-order.
    pub vip_port_host: u16,
    /// Padding to 8-byte alignment. Always zero.
    pub _pad: u16,
}

// Compile-time guard: byte-parity with the kernel `ReverseLocalEntry`.
const _: () = assert!(core::mem::size_of::<ReverseLocalEntryPod>() == 8);

// SAFETY: repr(C), `_pad` always 0, all fields byte-addressable. aya
// needs the marker for raw map access.
unsafe impl aya::Pod for ReverseLocalEntryPod {}

/// `REVERSE_LOCAL_MAP` value codec — the original VIP `(address, port)`,
/// stored as a host-order 8-byte POD.
///
/// Endianness lockstep (ADR-0041): the userspace handle stores the VIP
/// address and port host-order with NO flip; the
/// `cgroup_recvmsg4_service` program converts each to network-order at
/// the write boundary when it rewrites the reply source. `encode` packs
/// `(vip, vip_port)` into the POD host-order; `decode` is the inverse. A
/// unit struct rather than free functions so the encode/decode pair is
/// the single named codec site for the VIP value, mirroring the key's
/// `from_typed`.
pub struct ReverseLocalMapValue;

impl ReverseLocalMapValue {
    /// Host-order POD the handle writes into the map value for
    /// `(vip, vip_port)`.
    #[must_use]
    pub fn encode(vip: Ipv4Addr, vip_port: u16) -> ReverseLocalEntryPod {
        ReverseLocalEntryPod { vip_host: u32::from(vip), vip_port_host: vip_port, _pad: 0 }
    }

    /// Recover the VIP `(address, port)` from the host-order POD map
    /// value.
    #[must_use]
    pub fn decode(entry: ReverseLocalEntryPod) -> (Ipv4Addr, u16) {
        (Ipv4Addr::from(entry.vip_host), entry.vip_port_host)
    }
}

/// Typed handle around `REVERSE_LOCAL_MAP`.
pub struct ReverseLocalMapHandle {
    inner: Mutex<HashMap<MapData, ReverseLocalKeyPod, ReverseLocalEntryPod>>,
}

impl ReverseLocalMapHandle {
    /// Wrap a recovered `aya::maps::HashMap`.
    #[must_use]
    pub const fn new(map: HashMap<MapData, ReverseLocalKeyPod, ReverseLocalEntryPod>) -> Self {
        Self { inner: Mutex::new(map) }
    }

    /// Insert-or-replace the reverse mapping
    /// `(backend_ip, backend_port, proto) → (vip, vip_port)`. Written
    /// reverse-first in the `register_local_backend` dual-write (DDD-1).
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
        vip_port: u16,
    ) -> Result<(), MapError> {
        let key = ReverseLocalKeyPod::from_typed(BackendKey::new(backend_ip, backend_port, proto));
        let value = ReverseLocalMapValue::encode(vip, vip_port);
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

    /// Dump every `(ReverseLocalKeyPod, ReverseLocalEntryPod)` entry —
    /// the `bpftool map dump`-equivalent surface the Tier-3 acceptance
    /// asserts on (S-01-02, S-02-03). Mirrors
    /// `EbpfDataplane::local_backend_map_entries`.
    ///
    /// # Errors
    ///
    /// Returns `MapError` on a kernel-side read failure.
    pub fn entries(&self) -> Result<Vec<(ReverseLocalKeyPod, ReverseLocalEntryPod)>, MapError> {
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

    use crate::maps::reverse_local_map_handle::{
        ReverseLocalEntryPod, ReverseLocalKeyPod, ReverseLocalMapValue,
    };

    /// 8-byte key layout is the byte-for-byte contract the kernel-side
    /// `ReverseLocalKey` mirrors (byte-parity with the three shipped
    /// keys, DDD-2). A drift off 8 bytes silently mis-keys every
    /// recvmsg4 reverse lookup.
    #[test]
    fn reverse_local_key_pod_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<ReverseLocalKeyPod>(), 8);
    }

    /// 8-byte VALUE layout is the byte-for-byte contract the kernel-side
    /// `ReverseLocalEntry { vip_host: u32, vip_port_host: u16, _pad: u16 }`
    /// mirrors. The value width grows 4→8 in step 01-02 (ADR-0053 §D4 —
    /// the reverse rewrite restores BOTH addr and port); a drift off 8
    /// bytes silently mismatches the kernel value read and corrupts the
    /// rewritten reply source.
    #[test]
    fn reverse_local_entry_pod_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<ReverseLocalEntryPod>(), 8);
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
        /// - the VIP `(addr, port)` pair round-trips host-order through
        ///   the value encode/decode pair,
        /// - the 8-byte value POD lays out `vip_host` (offset 0..4),
        ///   `vip_port_host` (offset 4..6) host-order, trailing pad
        ///   (offset 6..8) zeroed.
        ///
        /// Catches a mis-slotted proto byte, a non-zero pad, a sneaky
        /// endianness flip at the userspace edge, a VIP byte-swap, or a
        /// dropped/byte-swapped VIP port.
        #[test]
        fn reverse_local_key_pod_bytes_match_kernel_build(
            backend_ip in any::<u32>(),
            backend_port in 1u16..=u16::MAX,
            proto in arb_proto(),
            vip in any::<u32>(),
            vip_port in any::<u16>(),
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

            // VIP (addr, port) value lockstep: the handle stores the VIP
            // as an 8-byte host-order POD (vip_host:4 + vip_port_host:2 +
            // _pad:2); encoding then decoding the value round-trips the
            // original host-order (addr, port) pair (the recvmsg4 program
            // reads these bytes and converts BOTH to NBO at the write
            // boundary). Exercises the (addr, port) value codec that
            // GREEN lands; RED against the IP-only codec.
            let entry = ReverseLocalMapValue::encode(vip_v4, vip_port);
            let (decoded_vip, decoded_port) = ReverseLocalMapValue::decode(entry);
            prop_assert_eq!(decoded_vip, vip_v4, "VIP addr host-order round-trip");
            prop_assert_eq!(decoded_port, vip_port, "VIP port host-order round-trip");
            prop_assert_eq!(entry.vip_host, vip, "VIP addr stored host-order, no flip");
            prop_assert_eq!(entry.vip_port_host, vip_port, "VIP port stored host-order, no flip");

            // Byte-for-byte value layout: vip_host at offset 0..4
            // host-order, vip_port_host at offset 4..6 host-order,
            // trailing pad (offset 6..8) zeroed. Pins the 8-byte kernel
            // value contract (`ReverseLocalEntry`).
            let entry_bytes: [u8; 8] = unsafe { core::mem::transmute(entry) };
            prop_assert_eq!(&entry_bytes[0..4], &vip.to_ne_bytes(), "vip_host at offset 0..4");
            prop_assert_eq!(
                &entry_bytes[4..6],
                &vip_port.to_ne_bytes(),
                "vip_port_host at offset 4..6"
            );
            prop_assert_eq!(entry_bytes[6], 0u8, "value pad byte at offset 6 is zero");
            prop_assert_eq!(entry_bytes[7], 0u8, "value pad byte at offset 7 is zero");
        }
    }
}

#[cfg(all(test, feature = "integration-tests", target_os = "linux"))]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]
mod real_map_tests {
    //! Deregister-path coverage for [`ReverseLocalMapHandle::remove`]
    //! against a REAL `aya::maps::HashMap` over a kernel-created
    //! `BPF_MAP_TYPE_HASH` fd. The in-memory proptest above pins the
    //! key/value byte layout; it CANNOT exercise `remove`, because the
    //! handle wraps a live aya map (the layout proptest never touches the
    //! map). This module is the missing half: it drives the real
    //! upsert→entries→remove→entries cycle so a no-op `remove` (which the
    //! body's success path is — `bpf_map_delete_elem`) is observable as a
    //! still-present entry.
    //!
    //! Tier 3 (real kernel — privileged `bpf(BPF_MAP_CREATE)` + map
    //! delete). Gated behind `integration-tests` + `target_os = "linux"`;
    //! runs under `cargo xtask lima run -- cargo nextest run ... --features
    //! integration-tests`. The map is constructed exactly as production
    //! does at `EbpfDataplane::new` (lib.rs) — `Map::HashMap(MapData)` →
    //! `HashMap::try_from` → `ReverseLocalMapHandle::new` — except the
    //! `MapData` is sourced from a self-created fd rather than the loaded
    //! BPF object, so no ELF / cgroup attach is needed.

    use std::net::Ipv4Addr;

    use aya::maps::{HashMap, Map, MapData};
    use overdrive_core::dataplane::backend_key::Proto;

    use crate::maps::reverse_local_map_handle::{
        ReverseLocalEntryPod, ReverseLocalKeyPod, ReverseLocalMapHandle, ReverseLocalMapValue,
    };

    /// `BPF_MAP_TYPE_HASH = 1` — stable kernel ABI (matches the
    /// production REVERSE_LOCAL_MAP shape: 8-byte key, 8-byte
    /// `ReverseLocalEntryPod` value).
    const BPF_MAP_TYPE_HASH: u32 = 1;

    /// Build a real `ReverseLocalMapHandle` over a freshly kernel-created
    /// HASH map. Mirrors the production `EbpfDataplane::new` construction
    /// (`Map::HashMap(MapData)` → `HashMap::try_from` → `new`).
    fn real_handle() -> ReverseLocalMapHandle {
        let key_size = u32::try_from(core::mem::size_of::<ReverseLocalKeyPod>())
            .expect("ReverseLocalKeyPod is 8 bytes — fits u32");
        let value_size = u32::try_from(core::mem::size_of::<ReverseLocalEntryPod>())
            .expect("ReverseLocalEntryPod is 8 bytes — fits u32");
        let fd = crate::sys::bpf::bpf_create_map(
            BPF_MAP_TYPE_HASH,
            key_size,   // 8-byte key
            value_size, // 8-byte VIP (addr, port) value
            64,
            0,
            None,
            Some("REVLOCAL_TEST"),
        )
        .expect("bpf(BPF_MAP_CREATE) for a HASH map — needs CAP_BPF / root (Lima default-root)");
        let map_data = MapData::from_fd(fd).expect("MapData::from_fd over the created HASH map");
        let typed = HashMap::<_, ReverseLocalKeyPod, ReverseLocalEntryPod>::try_from(Map::HashMap(
            map_data,
        ))
        .expect("typed HashMap<ReverseLocalKeyPod, ReverseLocalEntryPod> from the created map");
        ReverseLocalMapHandle::new(typed)
    }

    /// `remove` actually deletes the reverse entry — the deregister-path
    /// guarantee the `cgroup_recvmsg4_service` reply rewrite depends on. A
    /// stale entry left after deregister would mis-rewrite a later
    /// non-service reply's source to a stale VIP.
    ///
    /// Load-bearing assertion: after `remove`, the map's full entry set is
    /// EMPTY. A no-op `remove` (the `remove -> Ok(())` mutant) leaves the
    /// entry present and fails this test. The assertions compare against the
    /// exact `(key, vip)` set rather than a membership predicate, so there
    /// is no compound boolean for a mutator to collapse into a vacuous pass.
    #[test]
    fn remove_deletes_the_reverse_entry() {
        let handle = real_handle();
        let backend_ip = Ipv4Addr::new(10, 244, 1, 7);
        let backend_port = 34567u16;
        let proto = Proto::Udp;
        let vip = Ipv4Addr::new(10, 96, 0, 10);
        let vip_port = 53u16;

        let want_key = ReverseLocalKeyPod::from_typed(
            overdrive_core::dataplane::backend_key::BackendKey::new(
                backend_ip,
                backend_port,
                proto,
            ),
        );
        let want_value = ReverseLocalMapValue::encode(vip, vip_port);

        // Upsert, then confirm the map holds EXACTLY the upserted entry —
        // establishes the subsequent absence is a real deletion, not an
        // empty map to start. Exact-set equality, no membership predicate.
        handle
            .upsert(backend_ip, backend_port, proto, vip, vip_port)
            .expect("upsert reverse entry");
        assert_eq!(
            handle.entries().expect("dump REVERSE_LOCAL_MAP entries (after upsert)"),
            vec![(want_key, want_value)],
            "precondition: after one upsert the reverse map holds exactly the \
             (backend, proto) → (vip, vip_port) entry"
        );

        // Remove, then confirm the map is EMPTY. This is the assertion the
        // `remove -> Ok(())` mutant cannot satisfy: a no-op leaves the entry
        // present, so the set is non-empty and this equality fails.
        handle.remove(backend_ip, backend_port, proto).expect("remove reverse entry");
        assert_eq!(
            handle.entries().expect("dump REVERSE_LOCAL_MAP entries (after remove)"),
            Vec::new(),
            "after remove, REVERSE_LOCAL_MAP MUST be empty — a stale reverse entry would \
             mis-rewrite a later non-service reply's source to a stale VIP. A no-op remove \
             leaves the entry present and fails here."
        );
    }
}
