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

// Typed userspace handle around `BPF_MAP_TYPE_HASH_OF_MAPS` —
// hand-rolled until aya 0.14+ / PR #1446. Linux-only because the
// `bpf()` syscall surface is Linux-specific. See research § D.3 +
// Appendix A.3.
#[cfg(target_os = "linux")]
pub mod hash_of_maps;
