//! VM-workload value types (`microvm-driver-cloud-hypervisor`, GH #42).
//!
//! Per ADR-0082 — the anti-corruption layer against Cloud Hypervisor's
//! substrate lies. [`config`] holds `VmConfig`'s pure value family: types
//! whose sole public constructor makes one silent substrate lie
//! structurally discouraged.
//!
//! **Landed incrementally.** Step 01-01 PART 1 landed
//! [`config::DiskAttachment`], [`config::MemoryPlan`] +
//! [`config::reserve_bytes`], [`config::KernelImage`], and
//! [`config::VmConfinement`]. PART 2 lands [`config::RootfsPlan`],
//! [`config::KernelCmdline`], [`config::VsockPort`],
//! [`config::VmRunDir`], and the outer [`config::VmConfig`] aggregate,
//! plus the `Vmm` port trait (`crate::traits::vmm`) that references it
//! — see `config`'s module doc for exactly what is landed and what
//! remains deferred (Landlock, Slice 03 / US-VM-7) and why. Step 01-03
//! lands [`beacon`] — the host<->guest vsock beacon Published Language
//! (ADR-0082 §D7) shared by `overdrive-init` (the guest-side PID 1) and
//! the host-side beacon session (`VmDriver`, landing step 01-07).
pub mod beacon;
pub mod config;
