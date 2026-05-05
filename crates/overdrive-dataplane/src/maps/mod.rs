//! Typed userspace BPF map handles per architecture.md § 9 +
//! research recommendation #5 (typed map newtype API).
//!
//! Each handle wraps an `aya::maps::*` value and exposes an API
//! that hides `BPF_MAP_TYPE_*` choice + endianness conversion at
//! the call site. This is the
//! "make invalid states unrepresentable" discipline applied to
//! BPF map access (`.claude/rules/development.md` § Type-driven
//! design).
//!
//! **RED scaffolds** — every handle's bodies panic via `todo!()`
//! until DELIVER fills them slice by slice.

pub mod backend_map_handle;
pub mod drop_counter_handle;
pub mod maglev_map_handle;
pub mod reverse_nat_map_handle;
pub mod service_map_handle;
