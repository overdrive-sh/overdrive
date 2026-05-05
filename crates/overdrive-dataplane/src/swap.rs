//! Atomic HASH_OF_MAPS inner-map swap primitive for Slice 03 (US-03;
//! ASR-2.2-01 zero-drop atomic backend swap).
//!
//! Five-step swap shape per `docs/feature/phase-2-xdp-service-map/
//! design/architecture.md` § 7:
//!
//! 1. Insert / update relevant rows in `BACKEND_MAP`.
//! 2. Allocate a fresh inner map populated with the new backend
//!    slot table (or Maglev permutation table for `MAGLEV_MAP`).
//! 3. `bpf_map_update_elem(OUTER_MAP, &service_id, &new_inner_fd)`
//!    — single atomic pointer swap.
//! 4. Release the old inner map (kernel refcounts).
//! 5. Garbage-collect orphaned `BACKEND_MAP` entries.
//!
//! **RED scaffold** — body panics via `todo!()` until DELIVER
//! fills it per Slice 03 (S-2.2-09..11).

#![allow(dead_code)]

pub const SCAFFOLD: bool = true;
