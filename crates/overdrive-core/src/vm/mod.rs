//! VM-workload value types (`microvm-driver-cloud-hypervisor`, GH #42).
//!
//! Per ADR-0082 — the anti-corruption layer against Cloud Hypervisor's
//! substrate lies. [`config`] holds `VmConfig`'s pure value family: types
//! whose sole public constructor makes one silent substrate lie
//! structurally discouraged.
//!
//! **Landed incrementally.** Step 01-01 lands [`config::DiskAttachment`],
//! [`config::MemoryPlan`] + [`config::reserve_bytes`],
//! [`config::KernelImage`], and [`config::VmConfinement`] — see
//! `config`'s module doc for exactly what is landed and, importantly,
//! what remains deferred (`VmRunDir`, the outer `VmConfig` aggregate, and
//! the `Vmm` port trait that references `VmConfig`) and why.
pub mod config;
