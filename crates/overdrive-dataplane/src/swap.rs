//! Atomic HASH_OF_MAPS inner-map swap primitive for Slice 03 (US-03;
//! ASR-2.2-01 zero-drop atomic backend swap) per ADR-0040 § 2.
//!
//! # Five-step swap shape
//!
//! From `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 7 / ADR-0040 § 2:
//!
//! 1. Insert / update relevant rows in `BACKEND_MAP`.
//! 2. **Allocate fresh inner map** populated with the new backend
//!    slot table. (Failure surfaces here as
//!    [`AtomicSwapError::MapAllocFailed`].)
//! 3. `bpf_map_update_elem(OUTER_MAP, &service_id, &new_inner_fd)` —
//!    single atomic pointer swap. The load-bearing step.
//! 4. Garbage-collect orphaned `BACKEND_MAP` entries.
//! 5. Release the old inner map (kernel refcounts).
//!
//! On step-2 failure the contract is structural: steps 3–5 do not
//! run, the existing outer-map pointer is unchanged, and traffic
//! continues to forward to the prior backend set. S-2.2-11 pins
//! this property at the trait-surface level.
//!
//! # Phase 2.2 step 03-02 scope
//!
//! This module ships the **alloc-failure surface** required to flip
//! S-2.2-11 GREEN: the typed [`AtomicSwapError`] enum, the
//! `From<AtomicSwapError>` conversion to `DataplaneError`, and the
//! [`atomic_inner_map_swap_create`] primitive that issues the actual
//! `bpf(BPF_MAP_CREATE)` syscall. Steps 1, 3, 4, and 5 of the
//! orchestration land in subsequent slices once the kernel-side
//! HASH_OF_MAPS map declaration is in place — aya-rs 0.13 does not
//! ship a typed userspace `HashOfMaps<K, V>` wrapper or a kernel-
//! side `aya-ebpf` macro for the type, so the full kernel-side
//! restructure is structurally larger than this slice's hour budget
//! and is sequenced into a follow-up step.
//!
//! The error-surface IS load-bearing in isolation: ADR-0040's
//! preservation invariant (step-2 failure leaves the prior pointer
//! untouched) is structurally guaranteed by short-circuiting before
//! step 3, and the distinct typed variant prevents future refactors
//! from collapsing into `LoadFailed(String)` and losing the
//! operator-facing remediation distinction. See
//! `.claude/rules/development.md` § Errors — distinct failure modes
//! get distinct variants.

// `bpf(2)` syscall surface — FD <-> u32 casts (kernel ABI), raw
// pointer borrows for `bpf_attr` arg buffers. Same allow scope as
// `crate::sys::bpf`.
#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr
)]

use overdrive_core::traits::dataplane::DataplaneError;

/// Errors from the atomic-swap primitive. Mapped through
/// `From<AtomicSwapError> for DataplaneError` at the trait
/// boundary so callers branch on the typed variant rather than
/// parsing strings.
#[derive(Debug, thiserror::Error)]
pub enum AtomicSwapError {
    /// Kernel rejected the inner-map allocation (step 2 of the
    /// 5-step swap). `source` carries the `bpf(BPF_MAP_CREATE)`
    /// errno — typically `EINVAL` (invalid params), `EPERM` (no
    /// `CAP_BPF`), or `ENOMEM` (memlock exhausted). Receiving this
    /// error is the structural guarantee that the outer-map
    /// pointer is unchanged.
    #[error("inner-map allocation rejected by kernel: {source}")]
    MapAllocFailed {
        #[source]
        source: std::io::Error,
    },
}

impl From<AtomicSwapError> for DataplaneError {
    fn from(value: AtomicSwapError) -> Self {
        match value {
            AtomicSwapError::MapAllocFailed { source } => Self::MapAllocFailed { source },
        }
    }
}

// ---------------------------------------------------------------
// Linux: real `bpf(BPF_MAP_CREATE)` issuance.
// ---------------------------------------------------------------

/// Allocate a fresh inner map for the HASH_OF_MAPS swap (step 2 of
/// the 5-step orchestration). Returns the file descriptor of the
/// freshly-created map on success.
///
/// `max_entries` is the BPF map size; per architecture.md § 10 /
/// Q5=A the inner-map size is fixed at 256, so production callers
/// pass `256`. The parameter is exposed so the test harness can
/// drive it to known invalid values (`0` is the canonical EINVAL
/// trigger from `bpf(BPF_MAP_CREATE)`) to exercise the alloc-
/// failure path without depending on host memlock state.
///
/// # Errors
///
/// Returns [`AtomicSwapError::MapAllocFailed`] if the kernel
/// rejects the map-create syscall. Failure preserves the outer-
/// map pointer (the swap orchestration aborts before step 3).
#[cfg(target_os = "linux")]
pub fn atomic_inner_map_swap_create(max_entries: u32) -> Result<MapFd, AtomicSwapError> {
    use std::mem;
    use std::os::fd::FromRawFd;

    use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

    // BPF map type 1 = BPF_MAP_TYPE_HASH (matches ADR-0040: inner =
    // BPF_MAP_TYPE_HASH keyed by BackendId → BackendEntry, size 256).
    // Constants are stable kernel ABI; the value is the same on every
    // arch in the kernel matrix. Sourced from
    // `aya_obj::generated::linux_bindings_*::bpf_map_type` —
    // `BPF_MAP_TYPE_HASH = 1`.
    const BPF_MAP_TYPE_HASH: u32 = 1;
    // BPF_MAP_CREATE = 0; first arg to the bpf() syscall.
    const BPF_MAP_CREATE: c_long = 0;

    // The kernel reads a `union bpf_attr` from userspace; we populate
    // the `map_create` fields and zero the rest. The struct layout
    // is stable kernel ABI per `include/uapi/linux/bpf.h`.
    #[repr(C)]
    #[derive(Default)]
    struct BpfMapCreateAttr {
        map_type: u32,
        key_size: u32,
        value_size: u32,
        max_entries: u32,
        map_flags: u32,
        inner_map_fd: u32,
        numa_node: u32,
        map_name: [u8; 16],
        map_ifindex: u32,
        btf_fd: u32,
        btf_key_type_id: u32,
        btf_value_type_id: u32,
        // Trailing kernel-reserved fields — kept zero. The kernel
        // reads only as many bytes as we declare via the syscall's
        // size argument.
        _pad: [u8; 32],
    }

    let attr = BpfMapCreateAttr {
        map_type: BPF_MAP_TYPE_HASH,
        // `BackendId` is `u32` per architecture.md § 6.
        key_size: mem::size_of::<u32>() as u32,
        // `BackendEntry` is 12 bytes per architecture.md § 10.
        value_size: 12,
        max_entries,
        ..Default::default()
    };

    // SAFETY: the syscall reads `attr` for `size_of::<BpfMapCreateAttr>()`
    // bytes. The struct is fully initialised, `#[repr(C)]`, and lives
    // for the duration of the call. The syscall itself does not retain
    // the pointer past return.
    let raw = unsafe {
        syscall(
            SYS_bpf,
            BPF_MAP_CREATE,
            &attr as *const _ as *const c_void,
            mem::size_of::<BpfMapCreateAttr>() as c_int,
        )
    };

    if raw < 0 {
        return Err(AtomicSwapError::MapAllocFailed { source: std::io::Error::last_os_error() });
    }

    // SAFETY: `raw >= 0` is a kernel-issued, owned file descriptor;
    // we transfer ownership into `MapFd`'s `OwnedFd` so it closes on
    // drop. `raw as c_int` is a lossless narrowing — the bpf()
    // syscall returns int-shaped fds.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw as c_int) };
    Ok(MapFd { inner: fd })
}

/// Owned file descriptor handle for a freshly-allocated inner map.
/// Drops on `Drop` via `OwnedFd`'s `close(2)` — kernel reaps the
/// map once refcount hits 0.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct MapFd {
    inner: std::os::fd::OwnedFd,
}

#[cfg(target_os = "linux")]
impl MapFd {
    /// Borrow the raw fd. Kept narrow — the swap orchestration is
    /// the only legitimate caller; future steps 3–5 will pass it
    /// to `bpf(BPF_MAP_UPDATE_ELEM)`.
    #[must_use]
    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.inner.as_raw_fd()
    }
}

// ---------------------------------------------------------------
// Non-Linux: stub branch so the workspace compiles on macOS dev.
// ---------------------------------------------------------------

/// Non-Linux fallthrough — returns `MapAllocFailed`.
///
/// Carries a synthetic `io::Error`. Lives behind
/// `#[cfg(not(target_os = "linux"))]` so the macOS-side workspace
/// continues to compile without aya in the dep graph.
#[cfg(not(target_os = "linux"))]
pub fn atomic_inner_map_swap_create(_max_entries: u32) -> Result<MapFd, AtomicSwapError> {
    Err(AtomicSwapError::MapAllocFailed {
        source: std::io::Error::other("atomic_inner_map_swap_create: non-Linux build target"),
    })
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct MapFd {
    _private: (),
}

#[cfg(test)]
mod tests {
    //! Cross-platform sanity tests for the `From<AtomicSwapError>
    //! for DataplaneError` conversion. The Linux-side `bpf()`
    //! syscall path is exercised by the Tier 3 integration test
    //! `kernel_rejects_inner_map_alloc_existing_mapping_preserved`
    //! in `tests/integration/atomic_swap.rs` — that test is the
    //! load-bearing pin against the typed-variant preservation
    //! property S-2.2-11 cares about.

    use overdrive_core::traits::dataplane::DataplaneError;

    use super::AtomicSwapError;

    #[test]
    fn atomic_swap_error_converts_to_dataplane_error_map_alloc_failed() {
        let err = AtomicSwapError::MapAllocFailed { source: std::io::Error::other("synthetic") };
        let trait_err: DataplaneError = err.into();
        match trait_err {
            DataplaneError::MapAllocFailed { source } => {
                assert!(source.to_string().contains("synthetic"));
            }
            other => panic!("expected DataplaneError::MapAllocFailed, got {other:?}"),
        }
    }
}
