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
//! **RED scaffold** — bodies panic via `todo!()` until DELIVER
//! fills them per Slice 04.

pub mod permutation;
pub mod table;
