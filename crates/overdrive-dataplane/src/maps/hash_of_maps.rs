//! Typed userspace handle around a `BPF_MAP_TYPE_HASH_OF_MAPS` outer
//! map, per
//! `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! § D.3 + Appendix A.3.
//!
//! aya 0.13.x ships no typed `HashOfMaps<K, V>` userspace surface
//! (PR #1446 is the upstream migration target). This handle is the
//! ~200-line bridge from the project's typed call sites
//! (`EbpfDataplane::update_service`) to the raw `bpf()` syscall
//! surface in [`crate::sys::bpf`].
//!
//! # The atomic-swap shape
//!
//! Step 3 of ADR-0040 § 2's 5-step swap is `set(&service_id,
//! new_inner_fd)` — a single `bpf_map_update_elem` against the outer
//! map. The kernel ref-counts inner maps; concurrent XDP readers see
//! either the old or the new inner-map pointer atomically. There is
//! no torn state — that property is a hard kernel invariant, not a
//! Overdrive-side guarantee.
//!
//! # Migration
//!
//! When aya PR #1446 ships, this handle becomes a thin wrapper around
//! `aya::maps::HashOfMaps<MapData, K, M>`. The public method set
//! is intentionally signature-compatible — `new_with_*_inner`,
//! `set`, `delete`, `pin`, `as_fd`, `create_inner`. The migration
//! recipe is in research § F.1.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::marker::PhantomData;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

use crate::sys::bpf::{
    BPF_ANY, bpf_create_map, bpf_map_delete_elem, bpf_map_update_elem, bpf_obj_pin,
};

// `bpf_map_type` discriminators from `include/uapi/linux/bpf.h`.
// Stable kernel ABI; values identical across the kernel matrix.
const BPF_MAP_TYPE_HASH: u32 = 1;
const BPF_MAP_TYPE_ARRAY: u32 = 2;
const BPF_MAP_TYPE_HASH_OF_MAPS: u32 = 13;

/// Marker for types safely transmutable to/from raw bytes.
/// Equivalent to aya's own internal `Pod` bound. Implementations
/// must be `#[repr(C)]` or `#[repr(transparent)]` and contain no
/// padding bytes.
///
/// # Safety
///
/// Implementor must guarantee the type is fully byte-addressable
/// with no uninitialised padding. A type with internal padding will
/// produce undefined map keys on insertion (the kernel hashes raw
/// bytes including padding).
pub unsafe trait Pod: Copy + 'static {}

// SAFETY: u32 / u64 / fixed-size byte arrays are fully byte-
// addressable with no padding.
unsafe impl Pod for u32 {}
unsafe impl Pod for u64 {}
unsafe impl<const N: usize> Pod for [u8; N] {}

/// Errors from the typed userspace HoM handle. Distinct variants per
/// `.claude/rules/development.md` § Errors so callers can branch on
/// alloc failure (the load-bearing S-2.2-11 surface) versus generic
/// I/O.
#[derive(Debug, thiserror::Error)]
pub enum HashOfMapsError {
    /// Kernel rejected `bpf(BPF_MAP_CREATE)` for the outer map or an
    /// inner-map prototype. Carries the originating errno (typically
    /// `EINVAL`, `EPERM`, or `ENOMEM`). The S-2.2-11 alloc-failure
    /// path is structurally identical to this variant — see
    /// [`crate::swap::AtomicSwapError::MapAllocFailed`] for the
    /// step-2 specific shape that converts to
    /// `DataplaneError::MapAllocFailed`.
    #[error("bpf(BPF_MAP_CREATE) rejected by kernel: {source}")]
    MapAllocFailed {
        #[source]
        source: std::io::Error,
    },
    /// Generic syscall failure on `bpf_map_update_elem` /
    /// `bpf_map_delete_elem` / `bpf_obj_pin`. Wraps the originating
    /// `io::Error`.
    #[error("bpf() syscall failed: {0}")]
    Syscall(#[from] std::io::Error),
}

pub type Result<T, E = HashOfMapsError> = std::result::Result<T, E>;

/// Typed userspace handle around a `BPF_MAP_TYPE_HASH_OF_MAPS` outer
/// map.
///
/// Owns the outer map fd and the inner-map prototype fd. Drops both
/// on deallocation; pinned outer maps survive process exit (see
/// [`Self::pin`]).
///
/// `K` is the outer-map key type (e.g. `ServiceKey` for SERVICE_MAP).
/// `V` is the inner-map *value* type (e.g. `BackendId` for the
/// per-service Maglev table). The inner-map *key* is always `u32`
/// (the inner map being either a HASH or an ARRAY).
pub struct HashOfMapsHandle<K: Pod, V: Pod> {
    outer_fd: OwnedFd,
    /// Prototype inner-map fd. Retained so the outer map's
    /// inner-map metadata stays valid for the lifetime of the
    /// handle. Drops on `Self::drop` close both.
    _inner_proto_fd: OwnedFd,
    inner_max_entries: u32,
    inner_map_type: u32,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
}

impl<K: Pod, V: Pod> HashOfMapsHandle<K, V> {
    /// Construct a new outer HoM whose inner maps are flat HASH maps
    /// of `(u32, V)`. Used by SERVICE_MAP when the inner table is
    /// keyed by a non-Maglev slot index.
    ///
    /// `name` is the bpffs name (≤ 15 bytes). `max_outer_entries` is
    /// the outer-map slot count (e.g. 4096 services per
    /// architecture.md § 10). `max_inner_entries` is the per-service
    /// inner-map cap.
    pub fn new_with_hash_inner(
        name: &str,
        max_outer_entries: u32,
        max_inner_entries: u32,
    ) -> Result<Self> {
        Self::new_with_inner(BPF_MAP_TYPE_HASH, name, max_outer_entries, max_inner_entries)
    }

    /// Construct a new outer HoM whose inner maps are ARRAY maps of
    /// `V` (indexed by `u32`). This is the SERVICE_MAP shape per
    /// architecture.md § 5 / Q5=A — inner ARRAY of `BackendId` size
    /// 256 (the Maglev table slot count).
    pub fn new_with_array_inner(
        name: &str,
        max_outer_entries: u32,
        max_inner_entries: u32,
    ) -> Result<Self> {
        Self::new_with_inner(BPF_MAP_TYPE_ARRAY, name, max_outer_entries, max_inner_entries)
    }

    fn new_with_inner(
        inner_map_type: u32,
        name: &str,
        max_outer_entries: u32,
        max_inner_entries: u32,
    ) -> Result<Self> {
        // 1. Inner-map prototype. The prototype is never inserted
        //    into the outer map — it exists solely to give the kernel
        //    the inner-map shape (key/value sizes) at outer-map create
        //    time.
        let inner_fd = bpf_create_map(
            inner_map_type,
            // Inner-map key: HASH and ARRAY both use u32 keys for
            // our use case (slot index for ARRAY; backend-set hash
            // for HASH).
            core::mem::size_of::<u32>() as u32,
            core::mem::size_of::<V>() as u32,
            max_inner_entries,
            0,
            None,
            None,
        )
        .map_err(|source| HashOfMapsError::MapAllocFailed { source })?;

        // 2. Outer HoM map referencing the prototype. value_size for
        //    HoM is sizeof(u32) — the kernel stores inner-map FDs as
        //    `u32` regardless of the host's pointer width.
        let outer_fd = bpf_create_map(
            BPF_MAP_TYPE_HASH_OF_MAPS,
            core::mem::size_of::<K>() as u32,
            core::mem::size_of::<u32>() as u32,
            max_outer_entries,
            0,
            Some(inner_fd.as_fd()),
            Some(name),
        )
        .map_err(|source| HashOfMapsError::MapAllocFailed { source })?;

        Ok(Self {
            outer_fd,
            _inner_proto_fd: inner_fd,
            inner_max_entries: max_inner_entries,
            inner_map_type,
            _k: PhantomData,
            _v: PhantomData,
        })
    }

    /// Atomically replace the inner map at `key`. This is the
    /// load-bearing step 3 of ADR-0040 § 2's 5-step swap — a single
    /// `bpf_map_update_elem` syscall against the outer map. Concurrent
    /// XDP readers see either the old or the new inner-map pointer
    /// atomically; the kernel ref-counts the previous inner map and
    /// reaps it once all in-flight programs return.
    pub fn set(&self, key: &K, inner: BorrowedFd<'_>) -> Result<()> {
        // The outer map's value size is sizeof(u32) — we write the
        // inner FD as a host-order u32. SAFETY: `K: Pod` guarantees
        // raw-byte-addressability of the key.
        let key_bytes = unsafe {
            core::slice::from_raw_parts(key as *const _ as *const u8, core::mem::size_of::<K>())
        };
        let inner_fd_u32: u32 = inner.as_raw_fd_u32();
        let value_bytes = inner_fd_u32.to_ne_bytes();
        bpf_map_update_elem(self.outer_fd.as_fd(), key_bytes, &value_bytes, BPF_ANY)?;
        Ok(())
    }

    /// Remove the slot at `key`. `ENOENT` is folded to `Ok(())` — the
    /// kernel returns `ENOENT` for absent keys, which we treat as
    /// idempotent no-op (matches the typed handle's
    /// idempotent-remove convention).
    pub fn delete(&self, key: &K) -> Result<()> {
        let key_bytes = unsafe {
            core::slice::from_raw_parts(key as *const _ as *const u8, core::mem::size_of::<K>())
        };
        bpf_map_delete_elem(self.outer_fd.as_fd(), key_bytes)?;
        Ok(())
    }

    /// Pin the outer fd to a bpffs path so external callers (e.g. a
    /// kernel-side BPF program declaring an external map) can
    /// recover it via `BPF_OBJ_GET`. The path's parent must already
    /// be a bpffs mount.
    pub fn pin<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        bpf_obj_pin(self.outer_fd.as_fd(), path.as_ref())?;
        Ok(())
    }

    /// Borrow the outer fd. Rare — exposed for direct syscalls (e.g.
    /// `bpf_map_lookup_elem` in test code that needs to verify
    /// post-swap state).
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.outer_fd.as_fd()
    }

    /// Allocate a fresh inner map matching the prototype's shape.
    /// Returns the new map's fd, ready to populate via direct
    /// `bpf_map_update_elem` calls before passing to [`Self::set`].
    ///
    /// Returns [`HashOfMapsError::MapAllocFailed`] on kernel
    /// rejection — this is the S-2.2-11 alloc-failure entry point.
    /// Failure leaves any existing outer-map slot unchanged because
    /// the alloc happens BEFORE the [`Self::set`] call.
    pub fn create_inner(&self, max_entries: Option<u32>) -> Result<OwnedFd> {
        bpf_create_map(
            self.inner_map_type,
            core::mem::size_of::<u32>() as u32,
            core::mem::size_of::<V>() as u32,
            max_entries.unwrap_or(self.inner_max_entries),
            0,
            None,
            None,
        )
        .map_err(|source| HashOfMapsError::MapAllocFailed { source })
    }
}

// Helper trait to extract the raw fd as u32. Needed because
// `BorrowedFd::as_raw_fd` returns `RawFd` (= `c_int`) and the
// kernel's HoM ABI stores fds as u32.
trait AsRawFdU32 {
    fn as_raw_fd_u32(self) -> u32;
}

impl AsRawFdU32 for BorrowedFd<'_> {
    fn as_raw_fd_u32(self) -> u32 {
        use std::os::fd::AsRawFd;
        // `RawFd` is `c_int` (i32). Negative fds are an error
        // condition the kernel never returns; the cast is well-
        // defined for valid fds.
        self.as_raw_fd() as u32
    }
}

#[cfg(test)]
mod tests {
    //! Linux-only unit tests exercising the real `bpf()` syscall
    //! surface. Skipped on unprivileged invocations (the bpf()
    //! syscall requires CAP_BPF or root).

    use super::{HashOfMapsError, HashOfMapsHandle};

    /// Constructing a HoM with `inner_max_entries = 0` is the
    /// canonical EINVAL trigger from `bpf(BPF_MAP_CREATE)`. The
    /// failure must surface as
    /// [`HashOfMapsError::MapAllocFailed`] — never collapse to a
    /// generic `Syscall` variant, because S-2.2-11 pins the
    /// distinct-typed-variant property.
    #[test]
    fn alloc_failure_with_zero_inner_entries_yields_typed_variant() {
        // SAFETY: `geteuid` is `unsafe` because the libc binding
        // family is. The call has no preconditions.
        let euid = unsafe { libc::geteuid() };
        if euid != 0 {
            eprintln!("[skip] requires root (CAP_BPF) for bpf(BPF_MAP_CREATE); euid={euid}");
            return;
        }

        let result = HashOfMapsHandle::<u32, u32>::new_with_array_inner(
            "test_hom_zero_inner",
            16,
            0, // EINVAL trigger
        );
        match result {
            Err(HashOfMapsError::MapAllocFailed { .. }) => {}
            Err(other) => panic!("expected MapAllocFailed, got {other:?}"),
            Ok(_) => panic!("expected alloc to fail with inner_max_entries=0"),
        }
    }

    /// Successful HoM construction + atomic swap end-to-end. Exercises
    /// the full path from `bpf(BPF_MAP_CREATE)` × 2 through
    /// `HashOfMapsHandle::set` (the load-bearing
    /// `bpf_map_update_elem` swap of step 3).
    #[test]
    fn create_outer_and_inner_then_atomic_swap() {
        let euid = unsafe { libc::geteuid() };
        if euid != 0 {
            eprintln!("[skip] requires root (CAP_BPF); euid={euid}");
            return;
        }

        let hom = HashOfMapsHandle::<u32, u32>::new_with_array_inner("test_hom_swap", 16, 256)
            .expect("HoM construction with valid params must succeed");

        let key: u32 = 42;
        let inner_v1 =
            hom.create_inner(None).expect("inner-map alloc with valid params must succeed");
        hom.set(&key, inner_v1.as_fd_borrowed()).expect("step-3 atomic update must succeed");

        // Second set with a fresh inner — this is the swap path.
        let inner_v2 = hom.create_inner(None).expect("inner-map alloc must succeed");
        hom.set(&key, inner_v2.as_fd_borrowed()).expect("atomic swap must succeed");

        // Idempotent delete.
        hom.delete(&key).expect("delete must succeed");
        hom.delete(&key).expect("delete is idempotent on absent key");
    }

    // OwnedFd → BorrowedFd helper for the swap test above. `OwnedFd`
    // exposes `as_fd()` via the `AsFd` trait; the local helper makes
    // the call site read straight.
    trait OwnedFdExt {
        fn as_fd_borrowed(&self) -> std::os::fd::BorrowedFd<'_>;
    }
    impl OwnedFdExt for std::os::fd::OwnedFd {
        fn as_fd_borrowed(&self) -> std::os::fd::BorrowedFd<'_> {
            use std::os::fd::AsFd;
            self.as_fd()
        }
    }
}
