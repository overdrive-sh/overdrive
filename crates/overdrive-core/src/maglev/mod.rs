//! Userspace Maglev consistent-hashing primitives for Slice 04
//! (US-04; ASR-2.2-02 ≤ 1 % incidental disruption).
//!
//! Two pure modules:
//!
//! - [`permutation`] — Eisenbud (NSDI 2016) permutation generator
//!   with weighted multiplicity expansion per architecture.md
//!   D6.
//! - [`table`] — weighted-multiplicity expansion shaping that
//!   takes `&BTreeMap<BackendId, Weight>` (deterministic order)
//!   and produces the Maglev table for the kernel-side
//!   `MAGLEV_MAP` inner array.
//!
//! Both are **pure functions** per Mandate 4 — `BTreeMap`-keyed
//! inputs, deterministic output, no I/O. proptest-shaped (see
//! S-2.2-12, S-2.2-13).
//!
//! Lives in `overdrive-core` because the algorithm is pure
//! arithmetic over `BackendId` and `MaglevTableSize` — both
//! already core types — and is consumed by both the production
//! `overdrive-dataplane` userspace BPF map handle (writes the
//! table into the kernel-side `MAGLEV_MAP` inner array) and the
//! `overdrive-sim` DST invariants `MaglevDistributionEven` /
//! `MaglevDeterministic`.

pub mod permutation;
pub mod table;
