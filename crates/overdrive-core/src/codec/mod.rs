//! Codec — versioned envelope primitives shared across every
//! rkyv-persisted type that crosses a durable-storage boundary.
//!
//! Per ADR-0048 and `.claude/rules/development.md` § "rkyv schema
//! evolution", every rkyv-persisted value (observation rows, intent
//! aggregates, future durable types) is wrapped in a per-type
//! versioned envelope enum. The shared trait and error type live
//! here; the per-type envelopes (e.g. `AllocStatusRowEnvelope`,
//! `JobEnvelope`) are co-located with their domain types.
//!
//! See `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`.

pub mod envelope;

pub use envelope::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
