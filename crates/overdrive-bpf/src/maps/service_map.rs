//! `SERVICE_MAP` — kernel-side `BPF_MAP_TYPE_HASH_OF_MAPS` per
//! ADR-0040 § 2 + architecture.md § 10.
//!
//! Phase 2.2 Slice 03 reshape: outer HoM keyed on `ServiceKey` (8-byte
//! host-order POD), inner ARRAY of `BackendId` (raw `u32`) size 256.
//! Atomic per-service backend swap is `bpf_map_update_elem` against
//! the outer map (step 3 of the 5-step swap; the kernel ref-counts
//! inner maps so concurrent XDP readers see either the old or the
//! new pointer atomically).
//!
//! # Pinning by name
//!
//! aya 0.13.x's ELF loader cannot create a HASH_OF_MAPS map directly
//! (its `bpf_create_map` does not set `inner_map_fd` in the
//! `BPF_MAP_CREATE` syscall — research § D.3 (b)). The workaround is
//! to declare `pinning: PINNING_BY_NAME` on the kernel-side static
//! and pre-create + pre-pin the outer map from userspace before
//! calling `EbpfLoader::load_file(...)`. aya's loader sees the
//! pinning field, joins `<map_pin_path>/SERVICE_MAP`, calls
//! `BPF_OBJ_GET`, and reuses the pre-pinned FD — no second
//! `BPF_MAP_CREATE` is attempted. See
//! `.claude/rules/development.md` § "Sharing the outer HoM between
//! userspace and the kernel-side ELF — `pinning = ByName`" and aya
//! 0.13.1 source `aya/src/bpf.rs:495-503` +
//! `aya/src/maps/mod.rs::MapData::create_pinned_by_name`.
//!
//! # Endianness lockstep (architecture.md § 11)
//!
//! `ServiceKey` is host-order. The kernel-side lookup converts wire-
//! order packet fields into host-order before keying SERVICE_MAP.
//! Userspace stores host-order without flipping. The Slice 02
//! proptest in the userspace handle is the byte-level pin (carries
//! over unchanged across this restructure — only the underlying
//! kernel map type changes, not the key shape).
//!
//! # Inner ARRAY shape (Slice 04 — Maglev-sized)
//!
//! Inner key = `u32` (slot index `0..M`), value = `BackendId` (raw
//! `u32`). Slot count `M` is the Maglev table size — `MaglevTableSize::DEFAULT.get()`
//! = 16_381 — per architecture.md § 5 Q-Sig D2 / D6 and ADR-0041.
//!
//! Pre-Slice-04 (Slice 03 lay) used `M = 256` with a placeholder
//! `(src_port ^ dst_port) & 0xff` slot hash; Slice 04 replaces both
//! with the proper Maglev-permuted slot table populated from userspace
//! by `crate::maglev::permutation::generate(...)`, indexed in the
//! XDP fast path by FNV-1a over the 5-tuple `mod M`. The kernel-side
//! map declaration here is the single source of `M` shared by the
//! XDP program (`programs/xdp_service_map.rs`) and the userspace
//! handle (`crates/overdrive-dataplane/src/lib.rs::SERVICE_MAP_INNER_CAPACITY`).
//!
//! # Naming
//!
//! The map is named `SERVICE_MAP` for ELF-export stability across
//! Slice 03 / 04. Architecturally Slice 04 reframes it as the
//! Maglev permutation table — see `crate::maps::maglev_map` for the
//! conceptual relabeling and migration notes. The kernel-side type
//! and pin path are unchanged; userspace populates Maglev-permuted
//! slots instead of round-robin slots.

#![allow(dead_code)]

use aya_ebpf::{macros::map, maps::Array};

use crate::maps::hash_of_maps::{HashOfMaps, PINNING_BY_NAME};

/// Outer-map key for SERVICE_MAP. 8 bytes, host-order. Matches
/// `crates/overdrive-dataplane/src/maps/service_map_handle.rs`
/// `ServiceKey` byte-for-byte (vip_host: u32, port_host: u16,
/// _pad: u16). Kept `#[repr(C)]` so the kernel-side and
/// userspace structs share an identical memory layout — aya's
/// `bpf_map_lookup_elem` keys the map by raw bytes.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct ServiceKey {
    pub vip_host: u32,
    pub port_host: u16,
    pub _pad: u16,
}

/// Outer-map *capacity* in service slots. 4096 per architecture.md
/// § 10. Sized to comfortably hold every `(VIP, port)` tuple a single
/// node will route in Phase 2.
pub const MAX_OUTER_ENTRIES: u32 = 4096;

/// Inner-ARRAY size in slots. Equals `MaglevTableSize::DEFAULT.get()`
/// = 16_381 per architecture.md § 5 Q-Sig D6 (Cilium's prime list,
/// largest ≤ 2¹⁴). The XDP fast path indexes this map by FNV-1a over
/// the 5-tuple `mod 16_381`; userspace populates each slot via
/// `crate::maglev::permutation::generate(...)`.
///
/// **MUST stay in lockstep** with
/// `overdrive_core::dataplane::MaglevTableSize::DEFAULT.get()` (the
/// userspace SSOT) and `SERVICE_MAP_INNER_CAPACITY` in
/// `crates/overdrive-dataplane/src/lib.rs`. The userspace handle
/// allocates inner ARRAYs of this size; kernel-side bounds checks
/// and verifier complexity assume this exact value.
pub const INNER_TABLE_SIZE: u32 = 16_381;

/// `SERVICE_MAP` — outer `BPF_MAP_TYPE_HASH_OF_MAPS` keyed by
/// `ServiceKey`, inner `BPF_MAP_TYPE_ARRAY` of `BackendId` (raw `u32`)
/// size 256.
///
/// `PINNING_BY_NAME` is mandatory for HoM under aya 0.13.x — the ELF
/// loader cannot create the outer map itself (no `inner_map_fd`
/// support in aya's `bpf_create_map`). Userspace pre-creates and
/// pre-pins the outer FD at `/sys/fs/bpf/overdrive/SERVICE_MAP`
/// before calling `EbpfLoader::map_pin_path("/sys/fs/bpf/overdrive")`;
/// aya's loader picks up the pinned FD via `BPF_OBJ_GET` and reuses
/// it (kernel ref-counted, so userspace and kernel-side share the
/// same map identity).
///
/// The XDP fast path looks up `(VIP, port) → inner ARRAY` (outer
/// lookup; verifier-tagged `inner_map`), then chains
/// `bpf_map_lookup_elem(inner_ptr, &slot_index)` for the
/// `BackendId`, then resolves `BackendId → Backend` against
/// BACKEND_MAP. Single-level nesting only — kernel rejects
/// HoM-of-HoM at outer-map create time (research § D.6).
#[map]
pub static SERVICE_MAP: HashOfMaps<ServiceKey, u32, Array<u32>> =
    HashOfMaps::with_max_entries_pinned(MAX_OUTER_ENTRIES, 0, PINNING_BY_NAME);
