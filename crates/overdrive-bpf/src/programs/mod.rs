//! Kernel-side eBPF program modules for Phase 2.2 (XDP service map
//! + Maglev + `REVERSE_NAT`).
//!
//! Each program is a `#[xdp]` or `#[classifier]` function compiled
//! into the same `overdrive_bpf.o` ELF artifact. The userspace
//! loader in `overdrive-dataplane` resolves them by name via
//! `aya::Ebpf::program_mut(...)`.
//!
//!
//! See `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 9 module layout.

pub mod cgroup_connect4_service;
// unconnected-udp-sendmsg4 (GH #200, ADR-0053 rev 2026-06-05) — the two
// new cgroup_sock_addr hooks for the unconnected same-host UDP path.
pub mod cgroup_recvmsg4_service;
pub mod cgroup_sendmsg4_service;
pub mod sanity;
pub mod xdp_reverse_nat;
pub mod xdp_service_map;
