//! `DropCounterHandle` — typed userspace wrapper around the
//! `DROP_COUNTER` `BPF_MAP_TYPE_PERCPU_ARRAY` per architecture.md
//! § 10.
//!
//! Key = `DropClass as u32`; value = per-CPU `u64`. The
//! [`DropCounterHandle::read`] operation sums across CPUs at read
//! time via `overdrive_core::dataplane::aggregate_per_cpu`.
//!
//! # Endianness
//!
//! `u64` counter values are host-order on every CPU. PERCPU_ARRAY
//! has no endianness flip — each per-CPU slot is a plain `u64`
//! incremented in-place by the kernel-side program.
//!
//! # Tier mapping
//!
//! Tier 1 (DST) exercises `SimDataplane::record_drop` /
//! `read_drop_counter` — the sim's collapsed in-memory counter
//! mirrors this handle's surface shape. Tier 2
//! (`BPF_PROG_TEST_RUN`) exercises the kernel-side `DROP_COUNTER`
//! through this handle. Tier 3 (real veth) exercises the full
//! load-and-attach path.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use overdrive_core::dataplane::{DropClass, aggregate_per_cpu};

/// Number of slots in the kernel-side `DROP_COUNTER`. Lockstep with
/// `DropClass::VARIANT_COUNT` (Q7=B / ADR-0040 D8).
pub const SLOT_COUNT: u32 = DropClass::VARIANT_COUNT;

/// Aggregate per-CPU counter values for a single `DropClass` slot.
///
/// `per_cpu_values` is the `Vec<u64>` returned by
/// `aya::maps::PerCpuArray::get(&class.as_index(), 0)` — one entry
/// per online CPU at the time of the call. The returned `u64` is
/// the bit-exact saturating sum.
///
/// Mirrors the production read path: the kernel-side program
/// increments per-CPU slots lock-free; userspace folds the per-CPU
/// view into a single counter at read time.
#[must_use]
pub fn aggregate_for_class(_class: DropClass, per_cpu_values: &[u64]) -> u64 {
    aggregate_per_cpu(per_cpu_values)
}

#[cfg(target_os = "linux")]
mod linux {
    use aya::Ebpf;
    use aya::maps::{MapData, PerCpuArray};
    use overdrive_core::dataplane::{DropClass, aggregate_per_cpu};
    use overdrive_core::traits::dataplane::DataplaneError;

    /// Typed userspace handle around the kernel-side `DROP_COUNTER`
    /// `BPF_MAP_TYPE_PERCPU_ARRAY`. Slot count =
    /// `DropClass::VARIANT_COUNT` (= 6).
    ///
    /// Construct via [`DropCounterHandle::from_ebpf`] against the
    /// loaded `Ebpf` object. The handle owns a `PerCpuArray<MapData,
    /// u64>` typed wrapper and exposes per-class read access.
    pub struct DropCounterHandle {
        map: PerCpuArray<MapData, u64>,
    }

    impl DropCounterHandle {
        /// Construct the typed handle from a loaded `Ebpf`. The map
        /// must have been declared in the BPF ELF as
        /// `DROP_COUNTER: PerCpuArray<u64>` of size
        /// `DropClass::VARIANT_COUNT` (matched by
        /// `crates/overdrive-bpf/src/maps/drop_counter.rs`).
        pub fn from_ebpf(bpf: &mut Ebpf) -> Result<Self, DataplaneError> {
            let map_data = bpf.take_map("DROP_COUNTER").ok_or_else(|| {
                DataplaneError::LoadFailed("DROP_COUNTER not present in BPF object".into())
            })?;
            let map: PerCpuArray<MapData, u64> = PerCpuArray::try_from(map_data).map_err(|e| {
                DataplaneError::LoadFailed(format!("DROP_COUNTER PerCpuArray::try_from: {e}"))
            })?;
            Ok(Self { map })
        }

        /// Read the aggregated count for `class` — sums per-CPU
        /// values across every online CPU. Mirrors
        /// `SimDataplane::read_drop_counter` so DST tests and the
        /// production read path share the same surface.
        pub fn read(&self, class: DropClass) -> Result<u64, DataplaneError> {
            let per_cpu_values = self.map.get(&class.as_index(), 0).map_err(|e| {
                DataplaneError::LoadFailed(format!(
                    "DROP_COUNTER PerCpuArray::get(slot={}): {e}",
                    class.as_index()
                ))
            })?;
            Ok(aggregate_per_cpu(&per_cpu_values))
        }

        /// Snapshot every slot in canonical `DropClass` order.
        /// Returns `[u64; DropClass::VARIANT_COUNT as usize]`.
        pub fn snapshot(&self) -> Result<[u64; DropClass::VARIANT_COUNT as usize], DataplaneError> {
            let mut out = [0_u64; DropClass::VARIANT_COUNT as usize];
            for class in [
                DropClass::MalformedHeader,
                DropClass::UnknownVip,
                DropClass::NoHealthyBackend,
                DropClass::SanityPrologue,
                DropClass::ReverseNatMiss,
                DropClass::OversizePacket,
            ] {
                out[class.as_index() as usize] = self.read(class)?;
            }
            Ok(out)
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::DropCounterHandle;
