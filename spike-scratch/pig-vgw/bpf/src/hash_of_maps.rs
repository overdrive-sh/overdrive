//! Kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` declaration per
//! `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! § D.2 + Appendix A.4.
//!
//! aya-ebpf 0.1.x does NOT ship a `HashOfMaps<K, V, M>` typed surface
//! (the upstream `#[map]` macro is type-agnostic, but no struct
//! exists for `HoM`). This module provides a `#[repr(transparent)]`
//! struct over `bpf_map_def` that works with `aya_ebpf::macros::map`
//! natively — the macro is a `link_section` annotator and does not
//! gate on a type whitelist (research § D.1).
//!
//! # Use site
//!
//! ```ignore
//! use crate::maps::hash_of_maps::HashOfMaps;
//! use aya_ebpf::{macros::map, maps::Array};
//!
//! #[map]
//! pub static SERVICE_MAP: HashOfMaps<ServiceKey, BackendId, Array<u32>> =
//!     HashOfMaps::with_max_entries(4096, 0);
//! ```
//!
//! # Verifier discipline (chained inner-map lookup)
//!
//! Per kernel.org BPF `map_of_maps` doc + research § D.6:
//!
//! 1. `lookup_inner(&key)` returns `Option<NonNull<c_void>>` — the
//!    pointer is verifier-tagged `inner_map`.
//! 2. NULL-check via the `Option::Some` arm before chaining into
//!    a second `bpf_map_lookup_elem`.
//! 3. Single-level nesting only — kernel rejects HoM-of-HoM at
//!    outer-map create time.
//!
//! # Migration
//!
//! When aya 1.0 / PR #1446 lands, this struct collapses to a re-
//! export of `aya_ebpf::maps::HashOfMaps`. The `#[map]` site stays
//! identical — the macro doesn't see the type change.

#![allow(dead_code)]

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ptr::NonNull;

use aya_ebpf::{bindings::bpf_map_def, cty::c_void, helpers::bpf_map_lookup_elem};

// `BPF_MAP_TYPE_HASH_OF_MAPS = 13` per
// `aya_ebpf_bindings::bindings::bpf_map_type` (stable kernel ABI).
// Hard-coded here to avoid importing the bindings module — the
// import surface is arch-fragmented (`x86_64::bindings`,
// `aarch64::bindings`, …) and a re-export isn't exposed.
const BPF_MAP_TYPE_HASH_OF_MAPS: u32 = 13;

/// `bpf_map_def.pinning` field values mirroring aya-ebpf's
/// `pub(crate) PinningType` enum (`maps/mod.rs:81`). The field is
/// load-bearing for the pin-by-name workaround that lets aya 0.13.x
/// share an outer `HoM` map between userspace and the kernel-side ELF
/// (per `.claude/rules/development.md` § "Sharing the outer `HoM`
/// between userspace and the kernel-side ELF — `pinning = ByName`").
///
/// `PINNING_NONE`: aya creates the map fresh via `MapData::create`.
/// `PINNING_BY_NAME`: aya tries `BPF_OBJ_GET("/sys/fs/bpf/<dir>/<name>")`
/// first; on success it reuses the pre-pinned FD (the userspace-
/// owned outer `HoM`); on failure it falls back to `MapData::create`
/// (which fails for `HoM` because aya's `bpf_create_map` does not set
/// `inner_map_fd` — see research § D.3 (b)).
pub const PINNING_NONE: u32 = 0;
pub const PINNING_BY_NAME: u32 = 1;

/// Sealed marker trait for "this type can be used as an inner map of
/// a `HashOfMaps`." Every aya-ebpf inner-map type gets a blanket
/// impl. Sealed so external crates can't mis-implement it (the
/// `INNER_MAP_TYPE` constant is consumed only by userspace
/// inner-prototype creation; a wrong value would silently break the
/// kernel-side lookup chain).
mod sealed {
    pub trait Sealed {}
}

/// Marker trait for types valid as the inner-map of a `HashOfMaps`.
///
/// Kernel-side reads only the type-level metadata — the
/// `INNER_MAP_TYPE` constant is exposed for userspace-side prototype
/// creation in [`crate::maps::hash_of_maps`] (no kernel-side reader
/// today; reserved for future cross-crate plumbing).
pub trait InnerMap: sealed::Sealed {
    /// The kernel `BPF_MAP_TYPE_*` constant for this inner-map kind.
    const INNER_MAP_TYPE: u32;
}

// Blanket impls for the inner-map types Phase 2.2 actually uses.
impl<K, V> sealed::Sealed for aya_ebpf::maps::HashMap<K, V> {}
impl<K, V> InnerMap for aya_ebpf::maps::HashMap<K, V> {
    const INNER_MAP_TYPE: u32 = 1; // BPF_MAP_TYPE_HASH
}
impl<V> sealed::Sealed for aya_ebpf::maps::Array<V> {}
impl<V> InnerMap for aya_ebpf::maps::Array<V> {
    const INNER_MAP_TYPE: u32 = 2; // BPF_MAP_TYPE_ARRAY
}

/// Kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` declaration.
///
/// `K` — outer-map key type. `V` — inner-map *value* type (used at
/// the type level only; the kernel-side helper signature for
/// chained lookup uses raw byte pointers). `M` — inner-map *kind*
/// (e.g. `Array<BackendId>` or `HashMap<u32, BackendId>`).
///
/// # Use with `#[map]`
///
/// The aya-ebpf `#[map]` macro is type-agnostic — it emits
/// `#[link_section = "maps"]` + `#[export_name = "FOO"]` regardless
/// of what's beneath. This struct's `#[repr(transparent)]` over
/// `bpf_map_def` produces the same kernel-readable map definition
/// the upstream typed maps emit.
#[repr(transparent)]
pub struct HashOfMaps<K, V, M: InnerMap> {
    def: UnsafeCell<bpf_map_def>,
    _k: PhantomData<K>,
    _v: PhantomData<V>,
    _m: PhantomData<M>,
}

// SAFETY: `bpf_map_def` is plain-data; the kernel synchronises map
// access internally. The `Sync` bound is the canonical aya-ebpf
// shape for static map declarations.
unsafe impl<K: Sync, V: Sync, M: InnerMap> Sync for HashOfMaps<K, V, M> {}

impl<K, V, M: InnerMap> HashOfMaps<K, V, M> {
    /// Construct an outer `HoM`. `flags` is passed through to
    /// `bpf_map_def::map_flags`; the canonical value is 0.
    ///
    /// `pinning` selects between `PINNING_NONE` (aya creates the map
    /// fresh; fails on `HoM` in aya 0.13.x because `bpf_create_map`
    /// does not set `inner_map_fd`) and `PINNING_BY_NAME` (aya tries
    /// `BPF_OBJ_GET` against `<map_pin_path>/<MAP_NAME>` first, reusing
    /// a pre-pinned userspace-owned outer FD — the only working path
    /// for `HoM` in aya 0.13.x per
    /// `.claude/rules/development.md` § "Sharing the outer `HoM` between
    /// userspace and the kernel-side ELF — `pinning = ByName`").
    ///
    /// # `value_size = sizeof(u32)`
    ///
    /// `HoM` stores inner-map FDs as `u32` regardless of the host's
    /// pointer width — kernel ABI invariant.
    #[allow(clippy::cast_possible_truncation)]
    pub const fn with_max_entries_pinned(max_entries: u32, flags: u32, pinning: u32) -> Self {
        Self {
            def: UnsafeCell::new(bpf_map_def {
                type_: BPF_MAP_TYPE_HASH_OF_MAPS,
                key_size: core::mem::size_of::<K>() as u32,
                value_size: core::mem::size_of::<u32>() as u32,
                max_entries,
                map_flags: flags,
                id: 0,
                pinning,
            }),
            _k: PhantomData,
            _v: PhantomData,
            _m: PhantomData,
        }
    }

    /// Convenience: construct an outer `HoM` with `PINNING_NONE`. Kept
    /// for use sites that genuinely do not need pin-by-name (e.g.
    /// hypothetical future map types whose ELF declarations aya can
    /// create directly). Phase 2.2 `SERVICE_MAP` uses
    /// [`Self::with_max_entries_pinned`] with `PINNING_BY_NAME`.
    pub const fn with_max_entries(max_entries: u32, flags: u32) -> Self {
        Self::with_max_entries_pinned(max_entries, flags, PINNING_NONE)
    }

    /// Look up the inner map for `key`. Returns `Some(NonNull)` on
    /// hit; the pointer is verifier-tagged `inner_map`.
    ///
    /// # Verifier discipline
    ///
    /// Caller MUST chain to `bpf_map_lookup_elem` only after a
    /// successful lookup here — the verifier rejects unconditional
    /// dereference of the outer-lookup result. The `Option`
    /// representation makes the NULL-check load-bearing in the type
    /// system.
    #[inline(always)]
    pub fn lookup_inner(&self, key: &K) -> Option<NonNull<c_void>> {
        // SAFETY: `bpf_map_lookup_elem` is the canonical verifier-
        // accepted helper for outer-map lookup. The pointer-cast on
        // `key` is sound because the kernel reads `key_size` raw
        // bytes — if `K` has padding it must be zeroed by the caller
        // (this is the standard map-key contract).
        unsafe {
            let p = bpf_map_lookup_elem(
                self.def.get().cast(),
                core::ptr::from_ref(key).cast::<c_void>(),
            );
            NonNull::new(p)
        }
    }
}
