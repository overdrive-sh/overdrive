//! Kernel-side BPF map declarations for Phase 2.2 (XDP service map
//! + Maglev + `REVERSE_NAT` + drop counter).
//!
//! Each map module declares a `#[map]` static of the appropriate
//! `BPF_MAP_TYPE_*` per
//! `docs/feature/phase-2-xdp-service-map/design/architecture.md`
//! § 10.
//!
//! **RED scaffolds** — the `#[map]` static declarations are NOT
//! yet emitted; DELIVER lands them per the carpaccio slice plan
//! (Slice 02 / 03 / 04 / 05 / 06). The shared sanity prologue's
//! `DROP_COUNTER` lands in Slice 06.

pub mod backend_map;
pub mod drop_counter;
pub mod hash_of_maps;
pub mod local_backend_map;
pub mod maglev_map;
pub mod reverse_nat_map;
// unconnected-udp-sendmsg4 (GH #200, ADR-0053 rev 2026-06-05) — the
// reply store for the unconnected same-host cgroup path + its miss
// counter. RED scaffolds: the `#[map]` attribute is absent until
// DELIVER GREEN (Slice 01 map, Slice 03 counter).
pub mod reverse_local_map;
pub mod reverse_local_miss_counter;
pub mod service_map;
