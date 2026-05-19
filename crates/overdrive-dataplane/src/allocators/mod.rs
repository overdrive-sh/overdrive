//! `allocators` — userspace allocator primitives.
//!
//! Two concrete allocators with deliberately divergent reuse policies
//! (ADR-0049 § Amendments → 2026-05-19); no shared abstraction:
//!
//! - [`BackendIdAllocator`] (ADR-0046) — monotonic-counter `BackendId`
//!   allocator keyed by `(ip, port, proto)`. Userspace-only;
//!   re-hydrated from observation on restart (no persistence). Counter
//!   advances on every miss; released slots are NOT reclaimed (correct
//!   for the effectively-unbounded `u32` identifier space).
//! - [`ServiceVipAllocator`] (ADR-0049 / step 01-01) — scan-based VIP
//!   pool allocator keyed by service-spec digest. Bounded by an
//!   operator-configured [`VipRange`]; released VIPs return to the
//!   pool (so a finite /16 default can serve effectively-unbounded
//!   lifetimes provided the simultaneously-held count stays below
//!   capacity). The redb-backed persistence wrapper is
//!   [`PersistentServiceVipAllocator`] (step 01-03).
//!
//! Both allocators are sync, no I/O, no DB handle, and share the
//! memo-table-deduplication shape — but the release policy diverges
//! per the bullets above. There is no shared trait: VIP allocation
//! carries range/capacity/exhaustion concerns that BackendId does not,
//! and a prior attempt to factor them baked `VipRange` into a generic
//! `PoolAllocator` that BackendId could not use.

mod backend_id;
pub mod entry;
mod error;
mod persistent_service_vip;
mod service_vip;
mod vip_range;

pub use backend_id::BackendIdAllocator;
// `ServiceVipAllocatorEntry` (= V1 payload alias) is re-exported so
// callers can construct entries with struct-literal syntax. The
// codec-internal envelope enum `ServiceVipAllocatorEntryEnvelope` is
// deliberately NOT re-exported here per ADR-0048 § 2 Layer 1
// (non-re-export discipline); cross-crate writers reach it via the
// verbose `overdrive_dataplane::allocators::entry::*` module path.
pub use entry::{ServiceVipAllocatorEntry, ServiceVipAllocatorEntryLatest};
pub use error::{Result, ServiceVipAllocatorError, VipAllocatorConfigError};
pub use persistent_service_vip::{PersistentAllocatorError, PersistentServiceVipAllocator};
pub use service_vip::{ServiceSpecDigest, ServiceVip, ServiceVipAllocator};
pub use vip_range::VipRange;
