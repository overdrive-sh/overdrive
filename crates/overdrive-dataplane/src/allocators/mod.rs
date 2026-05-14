//! `allocators` — userspace allocator primitives.
//!
//! Two concrete allocators, no shared abstraction:
//!
//! - [`BackendIdAllocator`] (ADR-0046) — monotonic-counter `BackendId`
//!   allocator keyed by `(ip, port, proto)`. Userspace-only; re-hydrated
//!   from observation on restart (no persistence).
//! - [`ServiceVipAllocator`] (ADR-0049 / step 01-01) — monotonic VIP
//!   pool allocator keyed by service-spec digest. Bounded by an
//!   operator-configured [`VipRange`]. Persistence wrapper lives in
//!   `IntentBackedAllocator` (step 01-03; not in this module).
//!
//! Both allocators are sync, no I/O, no DB handle. Both follow the
//! memo + monotonic-counter shape; neither reclaims slots on release.
//! Beyond that surface similarity there is no shared trait — VIP
//! allocation carries range/capacity/exhaustion concerns that BackendId
//! does not, and trying to factor them produced unjustified complexity
//! (the previous attempt baked `VipRange` into a generic `PoolAllocator`,
//! which BackendId could not use).

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
