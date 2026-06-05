//! Built-in Certificate Authority — pure policy surface (GH #28, ADR-0063).
//!
//! `overdrive-core` (class `core`) owns the **decisions** of the CA: which
//! X.509 extensions and constraints each certificate role carries, and the
//! single-URI-SAN invariant (KPI K2). These are pure policy — no `rcgen`, no
//! crypto backend, dst-lint-clean — so they are DST-testable and the sim
//! adapter shares the exact same surface as the host adapter (ADR-0063 D5).
//!
//! The `Ca` port trait, the `RcgenCa` host adapter, the `SimCa` sim adapter,
//! and the root-key envelope land in later slices and consume this policy;
//! none of them live here.

pub mod cert_spec;
