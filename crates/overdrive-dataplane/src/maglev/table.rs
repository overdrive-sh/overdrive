//! Maglev weighted-multiplicity expansion: take
//! `&BTreeMap<BackendId, Weight>` and produce the per-backend
//! offset / skip pairs that the [`super::permutation::generate`]
//! function consumes.
//!
//! Pure synchronous function. `BTreeMap` order is canonical input
//! per `.claude/rules/development.md` § Ordered-collection choice.
//!
//! **RED scaffold** — body panics via `todo!()` until DELIVER
//! fills it per Slice 04.

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
