//! Cross-file net-slot partitioning registry — the SINGLE SOURCE OF TRUTH for
//! which per-allocation [`NetSlot`] band each real-netns integration test file
//! draws from.
//!
//! # The defect this closes
//!
//! [`overdrive_control_plane::veth_provisioner::derive_workload_netns_plan`]
//! derives SYSTEM-GLOBAL kernel names from the net-slot index: the netns
//! `ovd-ns-<4hex>`, the veth ends `ovd-hv-<slot>` / `ovd-wl-<slot>`, a per-slot
//! `/30` subnet, and the host-side `/etc/netns/<netns>/` dir. `nextest` runs
//! every test in its OWN process, in parallel, so two tests ANYWHERE in the
//! integration binary that each build a fresh
//! [`NetSlotAllocator`](overdrive_control_plane::veth_provisioner::NetSlotAllocator)
//! both derive smallest-free slot `0` and collide on the same kernel objects
//! (`ovd-ns-0000`, …). The collision is on the shared KERNEL, not the process,
//! so per-test process isolation does not help — only DISJOINT SLOT VALUES do.
//!
//! # The convention
//!
//! Each participating file is granted a NAMED, DISJOINT band of
//! [`SLOTS_PER_FILE`] slots. Every test in a file draws its slot through
//! [`FileSlotBand::nth`] (`base + offset`) rather than spelling a raw slot
//! number, so cross-file disjointness is STRUCTURAL and the per-file offsets
//! are the only thing a file's author picks. Per `.claude/rules/rust.md`
//! § "A key prefix and its record classifier are ONE SSOT": the band table
//! lives in exactly ONE place; a file hand-picking a raw slot outside it is the
//! anti-pattern this exists to kill. The [`tests`] disjointness guard fails the
//! build the moment two bands overlap or a band leaves the valid domain, so the
//! registry cannot silently drift.
//!
//! # The reserved production band
//!
//! Slots `0..PRODUCTION_BAND` are RESERVED for netns that PRODUCTION
//! assigns: the full-server walking-skeleton tests (`dns_responder_*`,
//! `canonical_address_inbound_walking_skeleton`) boot `run_server`, whose
//! INTERNAL `NetSlotAllocator` hands smallest-free starting at `0` and is not
//! reachable by the test to pre-adopt. Every TEST-CONTROLLED band below
//! therefore starts ABOVE the reserved band, so a test-pinned netns can never
//! collide with a production-assigned one. Those full-server tests deploy only
//! a handful of workloads (production stays well inside `0..PRODUCTION_BAND`)
//! and are additionally serialized against each other via the
//! `host-kernel-shared` `max-threads = 1` nextest group.
//!
//! # What slots CANNOT partition (out of this convention's scope)
//!
//! Slot partitioning makes per-test netns/veth/subnet NAMES disjoint. It does
//! NOT isolate resources that are global regardless of slot; those stay
//! serialized via the `host-kernel-shared` nextest group:
//!
//! - **`adopt_on_restart`'s global netns GC** — its recovery pass enumerates
//!   EVERY `ovd-ns-*` on the host and reaps any not in ITS observation Running
//!   set, so a concurrent test's netns (any slot) would be GC'd. Serialized.
//! - **The node-global `overdrive-mtls` nft table** — the worker/dataplane mTLS
//!   surface and `adopt_on_restart`'s tproxy-sweep test share one table name.
//!   Serialized.
//! - **The process-global `:53` DNS bind + `FrontendAddrAllocator`** — the
//!   `dns_responder_*` full-server tests share both. Serialized.

use overdrive_control_plane::veth_provisioner::{NET_SLOT_MAX, NetSlot};

/// Slots each file's band reserves. Sixteen gives every current participant
/// ample headroom (the busiest file uses five offsets) while keeping the whole
/// registry far inside the `0..=NET_SLOT_MAX` domain.
pub const SLOTS_PER_FILE: u16 = 16;

/// Width of the reserved production band. Slots `0..PRODUCTION_BAND` are left
/// for netns that `run_server`'s internal allocator assigns (smallest-free from
/// `0`); the test-controlled bands below begin at this offset. See the module
/// docs for why the full-server tests own the low band.
pub const PRODUCTION_BAND: u16 = SLOTS_PER_FILE;

/// One file's reserved, disjoint band of net slots. A plain value object; the
/// `base` is the first slot the file owns and the band spans
/// `base..base + SLOTS_PER_FILE`.
#[derive(Clone, Copy, Debug)]
pub struct FileSlotBand {
    name: &'static str,
    base: u16,
}

impl FileSlotBand {
    const fn new(name: &'static str, base: u16) -> Self {
        Self { name, base }
    }

    /// The `offset`-th slot in this file's band (`base + offset`). Each
    /// concurrently-schedulable test in the file passes a DISTINCT `offset` so
    /// its netns / veth / `/30` names are disjoint from every sibling's.
    ///
    /// # Panics
    ///
    /// Panics (test-only) if `offset >= SLOTS_PER_FILE` — the band is
    /// exhausted, which the author must fix by widening [`SLOTS_PER_FILE`] or
    /// re-scoping the file, never by reaching into a neighbouring band.
    #[must_use]
    pub fn nth(self, offset: u16) -> NetSlot {
        assert!(
            offset < SLOTS_PER_FILE,
            "net-slot band `{}` exhausted: offset {offset} >= SLOTS_PER_FILE {SLOTS_PER_FILE}",
            self.name,
        );
        NetSlot::new(self.base + offset)
            .expect("registry band is proven within 0..=NET_SLOT_MAX by the disjointness guard")
    }

    /// This band's first slot value.
    #[must_use]
    pub const fn base(self) -> u16 {
        self.base
    }

    /// This band's human-readable owner name (the test file it belongs to).
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }
}

// -----------------------------------------------------------------------------
// The registry: one disjoint band per real-netns integration test file whose
// TEST controls the slot. APPEND-ONLY — the disjointness guard below fails the
// build if a new band overlaps an existing one, drops into the reserved
// production band, or leaves the valid domain.
//
// Sim-only files (crash_observability_two_cycles, lww_counter_survives_restart,
// vip_allocator_lifecycle) construct a NetSlotAllocator but never arm the mTLS
// worker, so they provision NO real netns and need no band. The dns_responder_*
// full-server tests are production-slot-owned (+ share :53 / FrontendAddrAllocator)
// and are serialized, not slot-partitioned — see the module docs.
// -----------------------------------------------------------------------------

/// `alloc_netns_lifecycle.rs` — the C3 action-shim netns lifecycle acceptance
/// (`alloc_lands`, the two `finalize_failed_*` gates, and the VM TAP converge
/// plus incompatible-name refusal scenarios).
pub const ALLOC_NETNS_LIFECYCLE: FileSlotBand =
    FileSlotBand::new("alloc_netns_lifecycle", PRODUCTION_BAND);

/// `mtls_install_fail_closed.rs` — the intercept-install fail-closed ordering
/// acceptance (four Start/Restart × supersede arms).
pub const MTLS_INSTALL_FAIL_CLOSED: FileSlotBand =
    FileSlotBand::new("mtls_install_fail_closed", PRODUCTION_BAND + SLOTS_PER_FILE);

/// `adopt_on_restart.rs` — the boot-recovery slot re-adoption acceptance
/// (survivor + orphan netns). Its tproxy-sweep sibling test is on the nft-table
/// axis, not the slot axis, and needs no band.
pub const ADOPT_ON_RESTART: FileSlotBand =
    FileSlotBand::new("adopt_on_restart", PRODUCTION_BAND + 2 * SLOTS_PER_FILE);

/// `workload_netns_provision.rs` — the per-allocation netns provision / converge
/// / resolv.conf acceptance.
pub const WORKLOAD_NETNS_PROVISION: FileSlotBand =
    FileSlotBand::new("workload_netns_provision", PRODUCTION_BAND + 3 * SLOTS_PER_FILE);

/// Every registered band — consumed ONLY by the disjointness guard below.
const ALL_BANDS: &[FileSlotBand] =
    &[ALLOC_NETNS_LIFECYCLE, MTLS_INSTALL_FAIL_CLOSED, ADOPT_ON_RESTART, WORKLOAD_NETNS_PROVISION];

#[cfg(test)]
mod tests {
    use super::{ALL_BANDS, NET_SLOT_MAX, PRODUCTION_BAND, SLOTS_PER_FILE};

    /// Default-lane structural guard: every registered band is pairwise disjoint
    /// from every other, sits ABOVE the reserved production band, and tiles
    /// entirely within `0..=NET_SLOT_MAX`. A band that overlaps a neighbour,
    /// drops into the production band, or overflows the domain fails HERE — so
    /// the registry cannot silently drift as files are added.
    #[test]
    fn registered_bands_are_pairwise_disjoint_and_within_domain() {
        // (1) Each band is above the reserved production band, its top slot is
        //     within the domain, and `nth` is total across the whole band.
        for band in ALL_BANDS {
            assert!(
                band.base() >= PRODUCTION_BAND,
                "band `{}` (base {}) intrudes on the reserved production band 0..{PRODUCTION_BAND}",
                band.name(),
                band.base(),
            );
            let top = u32::from(band.base()) + u32::from(SLOTS_PER_FILE) - 1;
            assert!(
                top <= u32::from(NET_SLOT_MAX),
                "band `{}` top slot {top} exceeds NET_SLOT_MAX {NET_SLOT_MAX}",
                band.name(),
            );
            // Both extremes construct without panicking (offset in range, slot
            // in domain) — this is the `nth` totality half of the guard.
            let _first = band.nth(0);
            let _last = band.nth(SLOTS_PER_FILE - 1);
        }

        // (2) Pairwise disjoint: no slot value belongs to two bands.
        for (i, a) in ALL_BANDS.iter().enumerate() {
            for b in &ALL_BANDS[i + 1..] {
                let (a_lo, a_hi) = (a.base(), a.base() + SLOTS_PER_FILE - 1);
                let (b_lo, b_hi) = (b.base(), b.base() + SLOTS_PER_FILE - 1);
                assert!(
                    a_hi < b_lo || b_hi < a_lo,
                    "bands `{}` ({a_lo}..={a_hi}) and `{}` ({b_lo}..={b_hi}) overlap",
                    a.name(),
                    b.name(),
                );
            }
        }
    }
}
