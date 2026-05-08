//! Direct `bpf(2)` syscall helpers used where aya 0.13.x ships no
//! typed surface.
//!
//! Specifically: `BPF_MAP_TYPE_HASH_OF_MAPS` creation with
//! `inner_map_fd`, atomic `bpf_map_update_elem` against an outer-map
//! key (the load-bearing single-syscall pointer swap of ADR-0040 § 2
//! step 3), and `BPF_PROG_TEST_RUN` for Tier 2.
//!
//! When aya PR #1446 lands and the project upgrades, the HoM helpers
//! here are deleted and call sites move to `aya::maps::HashOfMaps`.
//! See `docs/research/dataplane/aya-rs-usage-comprehensive-research.md`
//! § F.1 for the migration recipe.
//!

pub mod bpf;
pub mod prog_test_run;
