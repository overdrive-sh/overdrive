//! Kernel-side BPF map declarations for Phase 2.2 (XDP service map
//! + Maglev + REVERSE_NAT + drop counter).
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
pub mod maglev_map;
pub mod reverse_nat_map;
pub mod service_map;
