//! `FrontendAddrAllocator` — the per-host source of the **stable per-`<job>`
//! frontend address** the dial-by-name responder answers with (ADR-0072 REV-2
//! "stable-frontend", GH #243; roadmap 01-04 / REV-2 design unit 1a-A).
//!
//! # Why a stable frontend address
//!
//! The responder answers `<job>.svc.overdrive.local` with a STABLE per-`<job>`
//! frontend address `F` drawn from [`WORKLOAD_FRONTEND_BASE`]
//! (`10.98.0.0/16`), NOT the live backend address. `F` is stable across alloc
//! cycles; the dataplane (02-00 re-keyed `MtlsResolve`) translates `F → the
//! live backend`. This eliminates SQ1 (stale-cached-DNS-answer): the workload
//! always dials the SAME `F` regardless of backend churn, so a cached answer
//! is never wrong.
//!
//! This allocator is the SSOT for that stable `F`. It is a per-host,
//! in-memory `<job> → Ipv4Addr` map — empty on boot, rebuilt by re-`assign`ing
//! every still-declared `<job>` (the [`NetSlotAllocator`] restart precedent).
//!
//! # Keyed by the logical `<job>`, released only on logical deletion
//!
//! [`crate::veth_provisioner::NetSlotAllocator`] is the structural precedent
//! — a pure smallest-free scan separated from an atomic held-map wrapper — but
//! this allocator diverges from it on the ONE load-bearing axis:
//!
//! - `NetSlotAllocator` keys on `AllocationId` and releases on alloc terminal
//!   (each alloc cycle ⇒ a new slot).
//! - `FrontendAddrAllocator` keys on the **logical `<job>`**
//!   ([`MeshServiceName`] — the `<job>` label in `<job>.svc.overdrive.local`)
//!   and releases **ONLY on logical-workload deletion** — NEVER on an alloc
//!   cycle, NEVER on a transient zero-healthy window. `F` MUST survive a
//!   stop→restart→new-`AllocationId` cycle and a zero-healthy window, or SQ1
//!   returns. The allocator therefore carries NO health state and NO
//!   `AllocationId` axis. (Zero-healthy is the `name_index`'s WITHHOLD seam in
//!   01-03 — never a release here.)
//!
//! # Disjointness
//!
//! `10.98.0.0/16` is structurally disjoint from the two other Phase-1 /16s —
//! `crate::veth_provisioner::WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`, the
//! per-netns /30s) and `VipRange::default()` (`10.96.0.0/16`, the service VIP
//! range). It is the exact block the spike's rejected `10.96.0.0/16`
//! frontend-base candidate collided with (ADR-0072 § Collision check), which
//! is why the block was moved to `10.98.0.0/16`.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;

use ipnet::Ipv4Net;
use overdrive_core::id::MeshServiceName;
use parking_lot::Mutex;

/// Per-host base block every stable per-`<job>` frontend address is drawn from
/// (`10.98.0.0/16`).
///
/// Lexically distinct from the `ServiceFrontend` VIP type in
/// `overdrive-dataplane` — this is the dial-by-name *frontend* block, not the
/// service-VIP block. It is structurally disjoint (a distinct /16) from
/// [`crate::veth_provisioner::WORKLOAD_SUBNET_BASE`] (`10.99.0.0/16`) and from
/// `VipRange::default()` (`10.96.0.0/16`); the disjointness is asserted in the
/// `dns_frontend_allocator` acceptance proptests against the live named consts,
/// never against a magic number, so a future base-addr edit cannot silently
/// collide.
///
/// Fixed for Phase-1 single-node; making it operator-configurable is out of
/// scope here. `Ipv4Net::new_assert` is `const` in `ipnet` 2.x (mirrors
/// [`crate::veth_provisioner::WORKLOAD_SUBNET_BASE`]), so the base is a
/// compile-time constant and the `/16` prefix is statically valid.
pub const WORKLOAD_FRONTEND_BASE: Ipv4Net = Ipv4Net::new_assert(Ipv4Addr::new(10, 98, 0, 0), 16);

/// The error returned when every address in [`WORKLOAD_FRONTEND_BASE`] is
/// already held, so a NEW `<job>` cannot be assigned a stable frontend address.
///
/// Exhaustion REFUSES the assignment — it is NEVER a panic and NEVER a silent
/// reuse of a held address. A reused address would collide two distinct
/// `<job>`s onto one frontend `F`, defeating the per-`<job>` stability the
/// allocator exists to provide. An already-held `<job>` re-assigns
/// successfully even at full capacity (re-entry is never starved by
/// exhaustion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("no free frontend address: all {capacity} addresses in {base} are held", base = WORKLOAD_FRONTEND_BASE)]
pub struct FrontendAddrExhausted {
    /// The usable host-address capacity of [`WORKLOAD_FRONTEND_BASE`] (the
    /// `/16` block minus its reserved network + broadcast endpoints) — every
    /// one of which is held when this error is returned.
    pub capacity: u64,
}

/// PURE decision: the smallest USABLE host [`Ipv4Addr`] in
/// [`WORKLOAD_FRONTEND_BASE`] that is NOT in `held`.
///
/// Total over the bounded `10.98.0.0/16` USABLE host span (the network and
/// broadcast addresses are reserved — see below), deterministic (same `held` ⇒
/// same address), performs no I/O. This is the assign-smallest-free contract:
/// the lowest GAP, not a next-monotonic counter — so an address freed by a
/// [`FrontendAddrAllocator::release`] is reclaimed by the next `assign`.
///
/// # Reserved endpoints
///
/// The block's network address (`10.98.0.0`) and broadcast address
/// (`10.98.255.255`) are NEVER handed out — the scan runs over
/// `[network()+1, broadcast()-1]`. This mirrors the usable-host discipline of
/// the adjacent networking code (`crate::veth_provisioner` derives the per-netns
/// gateway/peer as `network()+1`/`network()+2`, never the subnet-zero address;
/// `VipRange::default()` reserves both endpoints). A dialer's `connect()` is
/// pointed at the assigned `F`, and a subnet-zero / broadcast destination is not
/// guaranteed routable+capturable through the frontend datapath, so it is
/// excluded by construction rather than relied upon.
///
/// # Errors
///
/// Returns [`FrontendAddrExhausted`] when every usable address in the block is
/// in `held` — never a (reused) address, and never a reserved endpoint.
fn smallest_free_addr(held: &BTreeSet<Ipv4Addr>) -> Result<Ipv4Addr, FrontendAddrExhausted> {
    // Scan the block's USABLE host span ascending for the first address not
    // held. The block is a fixed `10.98.0.0/16`, so `network()+1` and
    // `broadcast()-1` cannot under/overflow a `u32` (the endpoints are interior
    // to the `u32` range). The lowest GAP is returned, so a released address
    // (the lower one) is reclaimed by the next assign.
    let first = u32::from(WORKLOAD_FRONTEND_BASE.network()) + 1;
    let last = u32::from(WORKLOAD_FRONTEND_BASE.broadcast()) - 1;
    for raw in first..=last {
        let candidate = Ipv4Addr::from(raw);
        if !held.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(FrontendAddrExhausted { capacity: frontend_block_capacity() })
}

/// The number of USABLE host addresses in [`WORKLOAD_FRONTEND_BASE`]
/// (`broadcast - network - 1` over the `/16` span = 65534 — the 65536-address
/// block minus the reserved network and broadcast endpoints). Pure,
/// deterministic; the [`FrontendAddrExhausted`] capacity is sourced from here so
/// the error and the scan share one definition of "the block is full".
fn frontend_block_capacity() -> u64 {
    let first = u64::from(u32::from(WORKLOAD_FRONTEND_BASE.network()));
    let last = u64::from(u32::from(WORKLOAD_FRONTEND_BASE.broadcast()));
    last - first - 1
}

/// Per-host stable per-`<job>` frontend-address allocator (ADR-0072 REV-2).
///
/// Hands out the stable frontend address `F` the dial-by-name responder
/// answers `<job>.svc.overdrive.local` with. The held `MeshServiceName →
/// Ipv4Addr` map is the allocator's state.
///
/// Held-state shape mirrors [`crate::veth_provisioner::NetSlotAllocator`]:
/// ephemeral runtime state, NEVER persisted, rebuilt on restart by
/// re-`assign`ing every still-declared `<job>` (single-node Phase 1; no
/// cross-restart persistence). `BTreeMap` (not `HashMap`) for deterministic
/// iteration order (`.claude/rules/development.md` § "Ordered-collection
/// choice" — [`snapshot`](Self::snapshot) is cloned and observed);
/// `parking_lot::Mutex` (not `tokio::sync`) because the only critical section
/// is a point smallest-free-scan + insert / remove that never crosses an
/// `.await`.
///
/// # Atomicity
///
/// [`assign`](Self::assign) takes the lock ONCE and performs the idempotent
/// re-entry check, the smallest-free scan, AND the insert in that single
/// critical section — there is no contains-then-insert TOCTOU window
/// (`.claude/rules/development.md` § "Check-and-act must be atomic").
#[derive(Clone, Debug, Default)]
pub struct FrontendAddrAllocator {
    /// `MeshServiceName → Ipv4Addr` binding for every currently-held `<job>`.
    /// `Arc<Mutex<…>>` so a clone shares the same held map (the allocator is
    /// composed once at boot and shared across readers).
    held: Arc<Mutex<BTreeMap<MeshServiceName, Ipv4Addr>>>,
}

impl FrontendAddrAllocator {
    /// Construct an empty allocator. On a fresh process boot nothing is held —
    /// every still-declared `<job>` is re-assigned on its next `assign`.
    #[must_use]
    pub fn new() -> Self {
        Self { held: Arc::new(Mutex::new(BTreeMap::new())) }
    }

    /// Assign the smallest-free stable frontend address to `job`, recording the
    /// `job → F` binding, and return it.
    ///
    /// **Idempotent per `<job>`:** if `job` is ALREADY held its EXISTING
    /// address is returned unchanged and no new address is consumed — a second
    /// `assign(J)` (the alloc-cycle case: stop → new `AllocationId` → new
    /// backend addr but the SAME logical `<job>`) returns the SAME `F`. The
    /// held check, the smallest-free scan, and the insert are ONE locked
    /// critical section — no contains-then-insert TOCTOU.
    ///
    /// # Errors
    ///
    /// Returns [`FrontendAddrExhausted`] when `job` is NOT already held and
    /// every address in [`WORKLOAD_FRONTEND_BASE`] is taken — refusing the
    /// assignment rather than reusing a held address. An already-held `<job>`
    /// re-assigns successfully even at full capacity.
    ///
    /// # Atomicity
    ///
    /// One `self.held.lock()`; the guard is dropped within the call (never
    /// across an `.await`).
    pub fn assign(&self, job: &MeshServiceName) -> Result<Ipv4Addr, FrontendAddrExhausted> {
        // ONE locked critical section: the idempotent re-entry check, the
        // smallest-free scan, AND the insert all happen under a single guard —
        // no contains-then-insert TOCTOU window. The guard is scoped to this
        // block so it drops before the function returns
        // (clippy::significant_drop_tightening) while still spanning the whole
        // check-and-act.
        let mut held = self.held.lock();
        // Idempotent per `<job>`: an already-held `<job>` returns its existing
        // F unchanged, consuming no new address — there is no window for a racer
        // between "is job held?" and "claim an address for job".
        if let Some(existing) = held.get(job) {
            return Ok(*existing);
        }
        // Smallest-free scan over the addresses currently bound — then bind it
        // to `job` in the SAME critical section.
        let taken: BTreeSet<Ipv4Addr> = held.values().copied().collect();
        let addr = smallest_free_addr(&taken)?;
        held.insert(job.clone(), addr);
        drop(held);
        Ok(addr)
    }

    /// Release `job`'s held frontend address — **logical-workload-DELETION
    /// ONLY**.
    ///
    /// Called when the logical `<job>` is undeployed/deleted, NOT on an alloc
    /// cycle and NOT on a transient zero-healthy window (releasing on
    /// zero-healthy would reintroduce the rejected SQ1 stale-`F` failure). The
    /// released address becomes the smallest-free candidate again iff it is the
    /// lowest free value.
    ///
    /// **Idempotent:** releasing a `<job>` that is not held is a benign no-op
    /// (`BTreeMap::remove` of an absent key), so a double-release does not
    /// panic and does not disturb the held set.
    pub fn release(&self, job: &MeshServiceName) {
        // Lock → remove → drop the guard within the call. `remove` returning
        // `None` (the `<job>` was not held) is the idempotent no-op — exactly
        // the double-release / release-of-never-assigned case.
        self.held.lock().remove(job);
    }

    /// Snapshot the currently-held `<job> → F` bindings.
    ///
    /// A point-in-time clone for read-only observers (e.g. a restart rebuild or
    /// the responder's name index), decoupled from the live map. Iteration
    /// order is `Ord` on [`MeshServiceName`], deterministic across processes
    /// and seeds (`.claude/rules/development.md` § "Ordered-collection
    /// choice").
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<MeshServiceName, Ipv4Addr> {
        self.held.lock().clone()
    }
}
