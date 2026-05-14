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
mod error;
mod service_vip;
mod vip_range;

pub use backend_id::BackendIdAllocator;
pub use error::{Result, ServiceVipAllocatorError, VipAllocatorConfigError};
pub use service_vip::{ServiceSpecDigest, ServiceVip, ServiceVipAllocator};
pub use vip_range::VipRange;
