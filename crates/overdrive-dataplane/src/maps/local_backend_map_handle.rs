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

    /// Insert-or-replace the local backend for `(vip, vip_port)`.
    ///
    /// Idempotent against the same triple; atomic point write —
    /// observers see either the prior backend or the new one,
    /// never a mix.
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
    ) -> Result<(), MapError> {
        let key = LocalServiceKey { vip_host: u32::from(vip), port_host: vip_port, _pad: 0 };
        let value = LocalBackendEntry {
            backend_ip_host: u32::from(*backend.ip()),
            backend_port_host: backend.port(),
            _pad: 0,
        };
        self.inner.lock().insert(key, value, 0)
    }

    /// Remove the entry for `(vip, vip_port)`.
    ///
    /// Idempotent: removing a non-existent entry returns `Ok(())`
    /// per the ADR-0053 § 2 trait contract — `KeyNotFound` is
    /// swallowed.
    ///
    /// # Errors
    ///
    /// Returns `MapError` for any failure other than `KeyNotFound`.
    pub fn remove(&self, vip: Ipv4Addr, vip_port: u16) -> Result<(), MapError> {
        let key = LocalServiceKey { vip_host: u32::from(vip), port_host: vip_port, _pad: 0 };
        let outcome = self.inner.lock().remove(&key);
        match outcome {
            Ok(()) | Err(MapError::KeyNotFound) => Ok(()),
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
    pub fn get(&self, vip: Ipv4Addr, vip_port: u16) -> Result<Option<LocalBackendEntry>, MapError> {
        let key = LocalServiceKey { vip_host: u32::from(vip), port_host: vip_port, _pad: 0 };
        let outcome = self.inner.lock().get(&key, 0);
        match outcome {
            Ok(v) => Ok(Some(v)),
            Err(MapError::KeyNotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
