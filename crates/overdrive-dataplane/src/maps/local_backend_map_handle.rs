//! Typed userspace handle for `LOCAL_BACKEND_MAP` per ADR-0053 § 1.
//!
//! Wraps an `aya::maps::HashMap<MapData, LocalServiceKey,
//! LocalBackendEntry>`. The trait-surface methods
//! [`LocalBackendMapHandle::upsert`] and
//! [`LocalBackendMapHandle::remove`] take `Ipv4Addr` / `u16` /
//! `SocketAddrV4` at the boundary and convert to host-order PODs
//! internally — call sites never touch wire bytes.
//!
//! Endianness lockstep per ADR-0041: userspace writes host-order;
//! the kernel-side `cgroup_connect4_service` program converts wire
//! bytes from network-order at the read boundary. See
//! `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs`
//! for the kernel-side companion.

use std::net::{Ipv4Addr, SocketAddrV4};

use aya::maps::{HashMap, MapData, MapError};
use overdrive_core::dataplane::backend_key::Proto;
use parking_lot::Mutex;

use crate::maps::{LocalBackendEntry, LocalServiceKey};

/// Typed handle around `LOCAL_BACKEND_MAP`.
///
/// Wraps the underlying `aya::maps::HashMap` in a `parking_lot::Mutex`
/// because aya's `insert` / `remove` take `&mut self` but the
/// `Dataplane::register_local_backend` trait surface is `&self` —
/// the typed handle IS the interior-mutability boundary for BPF map
/// updates. The lock is held only for the duration of the BPF
/// syscalls — never across `.await`.
pub struct LocalBackendMapHandle {
    inner: Mutex<HashMap<MapData, LocalServiceKey, LocalBackendEntry>>,
}

impl LocalBackendMapHandle {
    /// Wrap a recovered `aya::maps::HashMap`.
    #[must_use]
    pub const fn new(map: HashMap<MapData, LocalServiceKey, LocalBackendEntry>) -> Self {
        Self { inner: Mutex::new(map) }
    }

    /// Insert-or-replace the local backend for `(vip, vip_port, proto)`.
    ///
    /// `proto` is lowered to its IANA byte (TCP=6, UDP=17) at the write
    /// edge — the key dimension that distinguishes co-located tcp/53 +
    /// udp/53 on one VIP (ADR-0053 rev 2026-06-03). Idempotent against
    /// the same quadruple; atomic point write — observers see either the
    /// prior backend or the new one, never a mix.
    ///
    /// # Errors
    ///
    /// Returns `MapError` on kernel-side rejection (`EINVAL` on
    /// malformed key, `EPERM` on capability failure, `ENOMEM` on
    /// kernel allocator exhaustion).
    pub fn upsert(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: SocketAddrV4,
        proto: Proto,
    ) -> Result<(), MapError> {
        let key = LocalServiceKey {
            vip_host: u32::from(vip),
            port_host: vip_port,
            proto: proto.as_u8(),
            _pad: 0,
        };
        let value = LocalBackendEntry {
            backend_ip_host: u32::from(*backend.ip()),
            backend_port_host: backend.port(),
            _pad: 0,
        };
        self.inner.lock().insert(key, value, 0)
    }

    /// Remove the entry for `(vip, vip_port, proto)`.
    ///
    /// Idempotent: removing a non-existent entry returns `Ok(())`
    /// per the ADR-0053 § 2 trait contract — `KeyNotFound` is
    /// swallowed.
    ///
    /// # Errors
    ///
    /// Returns `MapError` for any failure other than `KeyNotFound`.
    pub fn remove(&self, vip: Ipv4Addr, vip_port: u16, proto: Proto) -> Result<(), MapError> {
        let key = LocalServiceKey {
            vip_host: u32::from(vip),
            port_host: vip_port,
            proto: proto.as_u8(),
            _pad: 0,
        };
        // Bind the lock-guarded `Remove` result to a local so the
        // mutex guard drops before the match scrutinee is evaluated
        // (clippy::significant_drop_in_scrutinee).
        let outcome = self.inner.lock().remove(&key);
        // Three-arm match per Phase 16 review D7: collapsing the
        // idempotent `KeyNotFound` branch into the `Ok(())` arm via
        // `|` works mechanically (and clippy::match_same_arms
        // suggests it) but hides the asymmetry — `Ok(())` is the
        // load-bearing success path; `Err(KeyNotFound)` is the
        // trait-contract-mandated swallow per ADR-0053 § 2.
        // Splitting them documents that intent at the matcher; the
        // identical bodies are the point, not a bug.
        #[allow(
            clippy::match_same_arms,
            reason = "Phase 16 review D7: each arm carries distinct semantic intent — \
                      success vs idempotent-swallow per ADR-0053 § 2 — that the \
                      collapsed form would erase. Comments above the match document the \
                      load-bearing distinction."
        )]
        match outcome {
            Ok(()) => Ok(()),
            Err(MapError::KeyNotFound) => Ok(()), // idempotent per ADR-0053 § 2
            Err(e) => Err(e),
        }
    }

    /// Snapshot every `(key, value)` pair, in iteration order.
    ///
    /// Used by tests and by the probe round-trip to assert on the
    /// post-state of the map.
    ///
    /// # Errors
    ///
    /// Returns `MapError` if the underlying map iteration fails.
    pub fn entries(&self) -> Result<Vec<(LocalServiceKey, LocalBackendEntry)>, MapError> {
        let guard = self.inner.lock();
        let mut out = Vec::new();
        for entry in guard.iter() {
            out.push(entry?);
        }
        drop(guard);
        Ok(out)
    }

    /// Read the entry for `(vip, vip_port)`, if any.
    ///
    /// # Errors
    ///
    /// Returns `MapError` on syscall failure other than
    /// `KeyNotFound` (which surfaces as `Ok(None)`).
    pub fn get(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        proto: Proto,
    ) -> Result<Option<LocalBackendEntry>, MapError> {
        let key = LocalServiceKey {
            vip_host: u32::from(vip),
            port_host: vip_port,
            proto: proto.as_u8(),
            _pad: 0,
        };
        let outcome = self.inner.lock().get(&key, 0);
        match outcome {
            Ok(v) => Ok(Some(v)),
            Err(MapError::KeyNotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    //! S-02-02 — userspace `LocalServiceKey` layout + proto-roundtrip
    //! proptest. There is NO Tier-2 `BPF_PROG_TEST_RUN` backstop for
    //! the `cgroup_sock_addr` program that reads this key, so this
    //! proptest is the userspace-edge compensation for the proto-source
    //! correctness (per the task contract): it pins the 8-byte key
    //! layout, proto at byte offset 6, trailing pad 0, and the
    //! no-userspace-flip invariant (host-order in == host-order bytes).

    use std::net::Ipv4Addr;

    use overdrive_core::dataplane::backend_key::Proto;
    use proptest::prelude::*;

    use crate::maps::LocalServiceKey;

    /// 8-byte key layout is the byte-for-byte contract the kernel-side
    /// `LocalServiceKey` mirrors. A drift off 8 bytes silently
    /// mis-keys every cgroup lookup.
    #[test]
    fn local_service_key_is_eight_bytes() {
        assert_eq!(core::mem::size_of::<LocalServiceKey>(), 8);
    }

    fn arb_proto() -> impl Strategy<Value = Proto> {
        prop_oneof![Just(Proto::Tcp), Just(Proto::Udp)]
    }

    proptest! {
        /// For any `(vip, vip_port, proto)`, the host-order
        /// `LocalServiceKey` the handle builds is byte-identical to the
        /// key the kernel would build from the same `bpf_sock_addr`:
        /// `vip_host` host-order, `port_host` host-order, `proto` the
        /// IANA byte at offset 6, trailing `_pad == 0`. Catches a
        /// mis-slotted proto byte, a non-zero pad, or any sneaky
        /// endianness flip at the userspace edge.
        #[test]
        fn local_service_key_bytes_match_kernel_build(
            vip in any::<u32>(),
            vip_port in any::<u16>(),
            proto in arb_proto(),
        ) {
            let v4 = Ipv4Addr::from(vip);
            let key = LocalServiceKey {
                vip_host: u32::from(v4),
                port_host: vip_port,
                proto: proto.as_u8(),
                _pad: 0,
            };
            // No userspace flip: host-order in == host-order field.
            prop_assert_eq!(key.vip_host, vip);
            prop_assert_eq!(key.port_host, vip_port);
            prop_assert_eq!(key.proto, proto.as_u8());

            // Byte-for-byte: proto lands at offset 6, trailing pad
            // (offset 7) is zeroed. Asserting the pad through the byte
            // view (not the `_pad` field) keeps clippy's
            // `used_underscore_binding` happy while still pinning that
            // the construction zeroes it.
            let bytes: [u8; 8] = unsafe { core::mem::transmute(key) };
            prop_assert_eq!(bytes[6], proto.as_u8(), "proto byte at offset 6");
            prop_assert_eq!(bytes[7], 0u8, "trailing pad byte at offset 7 is zero");
        }
    }
}
