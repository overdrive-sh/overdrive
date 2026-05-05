//! Kernel-side eBPF program modules for Phase 2.2 (XDP service map
//! + Maglev + REVERSE_NAT).
//!
//! Each program is a `#[xdp]` or `#[classifier]` function compiled
//! into the same `overdrive_bpf.o` ELF artifact. The userspace
//! loader in `overdrive-dataplane` resolves them by name via
//! `aya::Ebpf::program_mut(...)`.
//!
//! **RED scaffolds** — bodies panic via `core::hint::black_box`-
//! shaped placeholder until DELIVER fills them per the carpaccio
//! slice plan. Note: `panic!` cannot expand cleanly inside
//! `#[xdp]` handlers (the panic_handler is `loop {}`); the RED
//! signal in the kernel-side scaffolds is the absence of the
//! `#[xdp]` / `#[classifier]` attribute itself — adding the
//! attribute is part of DELIVER's GREEN pass per Slice 02 / 04 /
//! 05 / 06.
//!
//! See `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 9 module layout.

pub mod tc_reverse_nat;
pub mod xdp_service_map;
