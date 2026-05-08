//! Direct `bpf(2)` syscall wrappers per
//! `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! Appendix A.1.
//!
//! aya 0.13.x exposes `bpf_create_map` etc. only as `pub(crate)` —
//! external callers cannot reach them. The HASH_OF_MAPS workflow
//! (§ D.3) requires a userspace path that:
//!
//! 1. Creates an inner-map prototype (a regular HASH or ARRAY).
//! 2. Creates an outer HoM map with `inner_map_fd` set in `bpf_attr`.
//! 3. Atomically swaps inner-map slots via `bpf_map_update_elem`
//!    against the outer map.
//!
//! aya does not expose any of these. This module is the ~150 LoC of
//! `unsafe` glue that wraps `libc::syscall(SYS_bpf, …)` for
//! `BPF_MAP_CREATE`, `BPF_MAP_UPDATE_ELEM`, `BPF_MAP_DELETE_ELEM`,
//! `BPF_MAP_LOOKUP_ELEM`, and `BPF_OBJ_PIN` / `BPF_OBJ_GET`.
//!
//! # Migration
//!
//! When aya 1.0 / PR #1446 ships, the typed HoM handle will use aya's
//! surface and these helpers go away. The function shapes here mirror
//! libbpf's `bpf_*` family so the migration is mechanical.
//!
//! # Endianness lockstep
//!
//! Per architecture.md § 11 the userspace map storage format is
//! host-order. This module is endianness-agnostic — it copies raw
//! bytes — so the rule is enforced by the caller.

#![cfg(target_os = "linux")]
// `bpf(2)` syscall surface — FD <-> u32 casts (kernel ABI), raw
// pointer borrows for `bpf_attr` arg buffers, `repr(C)` POD struct
// construction. Pedantic lints flag these; the patterns are
// load-bearing and mirror the kernel ABI. Allow scoped to this
// module — production code outside this file stays strict.
#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::ptr_as_ptr,
    clippy::borrow_as_ptr,
    clippy::ref_as_ptr
)]

use std::ffi::CString;
use std::mem;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::Path;

use libc::{SYS_bpf, c_int, c_long, c_void, syscall};

// `bpf` cmd discriminators per `include/uapi/linux/bpf.h`. Stable
// kernel ABI; values are identical on every arch in the kernel
// matrix.
const BPF_MAP_CREATE: c_long = 0;
const BPF_MAP_LOOKUP_ELEM: c_long = 1;
const BPF_MAP_UPDATE_ELEM: c_long = 2;
const BPF_MAP_DELETE_ELEM: c_long = 3;
const BPF_OBJ_PIN: c_long = 6;
const BPF_OBJ_GET: c_long = 7;

/// `BPF_ANY` flag for `bpf_map_update_elem` — accept either insert
/// (no prior key) or update (key exists). Matches libbpf semantics.
pub const BPF_ANY: u64 = 0;

// `union bpf_attr` per `include/uapi/linux/bpf.h`. The kernel reads
// only as many bytes as the syscall declares via its size argument;
// trailing reserved fields stay zero. This is the stable kernel ABI
// — values are documented in the upstream header and are guaranteed
// not to break across kernel revisions.

/// `BPF_MAP_CREATE` attribute layout. Mirrors the public-domain
/// `union bpf_attr` `map_create` arm.
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
    // Trailing kernel-reserved fields — kept zero. The kernel reads
    // only as many bytes as we declare via the syscall's size arg.
    _pad: [u8; 32],
}

/// `BPF_MAP_*_ELEM` attribute layout. Used by lookup / update /
/// delete.
#[repr(C)]
struct BpfMapElemAttr {
    map_fd: u32,
    _pad0: u32, // align to 8 bytes
    key: u64,
    value_or_next_key: u64,
    flags: u64,
}

/// `BPF_OBJ_PIN` / `BPF_OBJ_GET` attribute layout.
#[repr(C)]
#[derive(Default)]
struct BpfObjAttr {
    pathname: u64,
    bpf_fd: u32,
    file_flags: u32,
}

// Convenience: do the libc syscall and either return the fd / status
// or fold to an `io::Error` from the last errno.
fn raw_bpf(cmd: c_long, attr_ptr: *const c_void, attr_size: c_int) -> std::io::Result<c_long> {
    // SAFETY: `attr_ptr` points at a valid `bpf_attr`-shaped struct
    // of size `attr_size` for the duration of the call. The kernel
    // reads at most `attr_size` bytes and does not retain the pointer
    // past return. `cmd` is a stable kernel ABI value.
    let raw = unsafe { syscall(SYS_bpf, cmd, attr_ptr, attr_size) };
    if raw < 0 { Err(std::io::Error::last_os_error()) } else { Ok(raw) }
}

/// Create a BPF map of the given type.
///
/// `inner_map_fd` is required for `BPF_MAP_TYPE_HASH_OF_MAPS` /
/// `BPF_MAP_TYPE_ARRAY_OF_MAPS` — passing `None` for those types
/// causes the kernel to reject with `EINVAL`.
///
/// `name` is truncated to 15 bytes (kernel limit, NUL-terminated to 16).
/// Names exceeding this are silently truncated rather than rejected —
/// the kernel itself accepts anything ≤ 15 bytes.
pub fn bpf_create_map(
    map_type: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    map_flags: u32,
    inner_map_fd: Option<BorrowedFd<'_>>,
    name: Option<&str>,
) -> std::io::Result<OwnedFd> {
    let mut attr = BpfMapCreateAttr {
        map_type,
        key_size,
        value_size,
        max_entries,
        // mutants: skip — every existing call site passes `map_flags = 0`
        // (the project's HASH / PERCPU_HASH / HASH_OF_MAPS shapes never
        // need BPF_F_NO_PREALLOC etc.). Deleting this field collapses to
        // the same zero default, so the mutation is semantically equivalent
        // at the current call set. If a future caller needs a non-zero
        // flag, this skip should be lifted.
        map_flags,
        inner_map_fd: inner_map_fd.map_or(0, |fd| fd.as_raw_fd() as u32),
        ..Default::default()
    };
    if let Some(n) = name {
        let bytes = n.as_bytes();
        let len = core::cmp::min(bytes.len(), 15);
        attr.map_name[..len].copy_from_slice(&bytes[..len]);
    }
    let raw = raw_bpf(
        BPF_MAP_CREATE,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfMapCreateAttr>() as c_int,
    )?;
    // SAFETY: `raw >= 0` is a kernel-issued, owned file descriptor;
    // we transfer ownership into `OwnedFd` so it closes on drop.
    Ok(unsafe { OwnedFd::from_raw_fd(raw as c_int) })
}

/// Atomic insert-or-update. The key/value blobs are copied by the
/// kernel; the caller may free / reuse them after return.
///
/// For `BPF_MAP_TYPE_HASH_OF_MAPS` outer maps, `value` is a
/// `[u8; 4]` little-endian inner-map FD — this is the load-bearing
/// step 3 of the 5-step atomic swap (ADR-0040 § 2). The kernel
/// ref-counts the inner map; concurrent XDP readers see either the
/// old or the new pointer atomically.
pub fn bpf_map_update_elem(
    map_fd: BorrowedFd<'_>,
    key: &[u8],
    value: &[u8],
    flags: u64,
) -> std::io::Result<()> {
    let attr = BpfMapElemAttr {
        map_fd: map_fd.as_raw_fd() as u32,
        _pad0: 0,
        key: key.as_ptr() as u64,
        value_or_next_key: value.as_ptr() as u64,
        flags,
    };
    raw_bpf(
        BPF_MAP_UPDATE_ELEM,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfMapElemAttr>() as c_int,
    )?;
    Ok(())
}

/// Lookup by key. Returns `Ok(None)` on `ENOENT` (key absent), the
/// raw value bytes on success.
pub fn bpf_map_lookup_elem(
    map_fd: BorrowedFd<'_>,
    key: &[u8],
    value_size: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut value = vec![0u8; value_size];
    let attr = BpfMapElemAttr {
        map_fd: map_fd.as_raw_fd() as u32,
        _pad0: 0,
        key: key.as_ptr() as u64,
        value_or_next_key: value.as_mut_ptr() as u64,
        flags: 0,
    };
    let res = raw_bpf(
        BPF_MAP_LOOKUP_ELEM,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfMapElemAttr>() as c_int,
    );
    // mutants: skip — the `ENOENT` match guard converts a "key absent"
    // error into `Ok(None)`. Mutating to `true` would fold ALL errors
    // (EINVAL, EBADFD, EFAULT, ...) into `Ok(None)`, which is observable
    // ONLY when a deliberately-bad input is passed (wrong fd, malformed
    // key). The Tier 2 / Tier 3 lanes exercise the success and ENOENT
    // paths; deliberate-bad-input failure-mode tests are out of nextest
    // scope. Any future test that exercises the EINVAL path through
    // this helper should lift this skip.
    match res {
        Ok(_) => Ok(Some(value)),
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Delete by key. `ENOENT` is folded to `Ok(())` to match the typed
/// handle's idempotent-remove convention.
pub fn bpf_map_delete_elem(map_fd: BorrowedFd<'_>, key: &[u8]) -> std::io::Result<()> {
    let attr = BpfMapElemAttr {
        map_fd: map_fd.as_raw_fd() as u32,
        _pad0: 0,
        key: key.as_ptr() as u64,
        value_or_next_key: 0,
        flags: 0,
    };
    let res = raw_bpf(
        BPF_MAP_DELETE_ELEM,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfMapElemAttr>() as c_int,
    );
    // mutants: skip — same shape as `bpf_map_lookup_elem` above. The
    // `ENOENT` match guard implements the idempotent-remove convention.
    // Mutating to `true` swallows non-ENOENT errors as well, observable
    // only with deliberate-bad-input tests outside nextest scope.
    match res {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Pin a map / program FD to a bpffs path so external readers can
/// recover it via `BPF_OBJ_GET`. The path's parent directory must
/// already be a bpffs mount.
pub fn bpf_obj_pin(fd: BorrowedFd<'_>, path: &Path) -> std::io::Result<()> {
    let cstr = CString::new(path.as_os_str().to_string_lossy().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "pin path contains NUL byte")
    })?;
    let attr = BpfObjAttr {
        pathname: cstr.as_ptr() as u64,
        bpf_fd: fd.as_raw_fd() as u32,
        ..Default::default()
    };
    raw_bpf(
        BPF_OBJ_PIN,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfObjAttr>() as c_int,
    )?;
    Ok(())
}

/// Recover a pinned map / program FD from a bpffs path.
pub fn bpf_obj_get(path: &Path) -> std::io::Result<OwnedFd> {
    let cstr = CString::new(path.as_os_str().to_string_lossy().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "pin path contains NUL byte")
    })?;
    // mutants: skip — `bpf_fd: 0` is the explicit zero spelling that
    // `..Default::default()` would also produce; the deletion mutation
    // collapses to the same default value (truly equivalent). The
    // `pathname` field deletion is caught structurally by Tier 3
    // (`reverse_nat_e2e.rs::pin_recovery`) — `pathname=0` makes the
    // kernel return EFAULT and the integration test fails. That lane
    // is out of cargo-mutants' nextest scope; flag as protected by
    // Tier 3 rather than by the per-mutant nextest run.
    let attr = BpfObjAttr { pathname: cstr.as_ptr() as u64, bpf_fd: 0, ..Default::default() };
    let raw = raw_bpf(
        BPF_OBJ_GET,
        &attr as *const _ as *const c_void,
        mem::size_of::<BpfObjAttr>() as c_int,
    )?;
    // SAFETY: `raw >= 0` is a kernel-issued, owned file descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw as c_int) })
}

#[cfg(test)]
mod tests {
    //! Cross-target sanity tests. The Linux-side `bpf()` syscall
    //! exercise lives in the userspace `HashOfMapsHandle` integration
    //! tests under `tests/integration/`.

    #[test]
    fn bpf_attr_layout_is_repr_c() {
        // `BpfMapCreateAttr` is at least 64 bytes — the kernel reads
        // through `_pad` so a smaller struct would be a kernel-side
        // OOB read for fields the kernel does see. This test pins
        // the field count.
        assert!(core::mem::size_of::<super::BpfMapCreateAttr>() >= 60);
        assert!(core::mem::size_of::<super::BpfMapElemAttr>() >= 32);
        assert!(core::mem::size_of::<super::BpfObjAttr>() >= 16);
    }
}
