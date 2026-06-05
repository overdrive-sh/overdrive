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
//! # RED scaffold (Slice 01)
//!
//! Method bodies are `todo!("RED scaffold: …")` until DELIVER fills
//! them (Slice 01 GREEN). The Tier-3 acceptance (the unconnected
//! round-trip) is THE gate — there is no Tier-2 backstop for the
//! cgroup path; the handle's host-order proto-roundtrip proptest
//! compensates at the userspace edge.

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
    // __SCAFFOLD__
    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in Slice 01")]
    pub fn upsert(
        &self,
        backend_ip: Ipv4Addr,
        backend_port: u16,
        proto: Proto,
        vip: Ipv4Addr,
    ) -> Result<(), MapError> {
        let _ = (&self.inner, backend_ip, backend_port, proto, vip);
        todo!(
            "RED scaffold: REVERSE_LOCAL_MAP upsert (backend,proto)->vip, reverse-first (Slice 01 / S-01-02)"
        )
    }

    /// Remove the reverse mapping for `(backend_ip, backend_port,
    /// proto)`. Idempotent — removing an absent entry is `Ok(())`.
    /// Called by `deregister_local_backend` (DDD-5a).
    ///
    /// # Errors
    ///
    /// Returns `MapError` on kernel-side rejection (NOT on a missing key).
    // __SCAFFOLD__
    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in Slice 01")]
    pub fn remove(
        &self,
        backend_ip: Ipv4Addr,
        backend_port: u16,
        proto: Proto,
    ) -> Result<(), MapError> {
        let _ = (&self.inner, backend_ip, backend_port, proto);
        todo!("RED scaffold: REVERSE_LOCAL_MAP remove (Slice 01 / deregister)")
    }

    /// Dump every `(ReverseLocalKeyPod, vip_host)` entry — the
    /// `bpftool map dump`-equivalent surface the Tier-3 acceptance
    /// asserts on (S-01-02, S-02-03). Mirrors
    /// `EbpfDataplane::local_backend_map_entries`.
    ///
    /// # Errors
    ///
    /// Returns `MapError` on a kernel-side read failure.
    // __SCAFFOLD__
    #[expect(clippy::todo, reason = "RED scaffold; lands GREEN in Slice 01")]
    pub fn entries(&self) -> Result<Vec<(ReverseLocalKeyPod, u32)>, MapError> {
        let _ = &self.inner;
        todo!("RED scaffold: REVERSE_LOCAL_MAP entries dump (Slice 01 / S-01-02)")
    }
}
