//! `overdrive-host` — production bindings from `overdrive-core` port
//! traits to the host OS, kernel, and network.
//!
//! Every injectable port trait in [`overdrive_core::traits`] has a
//! matching sim implementation in `overdrive-sim` (classed
//! `adapter-sim`) and a matching host implementation here (classed
//! `adapter-host`). Wiring crates pick host impls for production;
//! test crates pick Sim impls under the turmoil harness.
//!
//! Crate boundary: depending on `overdrive-host` is the opt-in to
//! real I/O — there is no feature flag. `overdrive-core` stays
//! infra-free; no reconciler or policy crate should ever list this
//! crate in its `[dependencies]`.
//!
//! Phase 1 ships minimal bindings — `SystemClock`, `OsEntropy`, and
//! a placeholder `TcpTransport` whose network methods still return
//! `Unsupported`. Phase 2 wires `TcpTransport` to `tokio::net::*`
//! and adds host impls for `Dataplane`, `Driver`, `IntentStore`,
//! `ObservationStore`, and `Llm`.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod clock;
pub mod entropy;
pub mod transport;

pub use clock::SystemClock;
pub use entropy::{CountingOsEntropy, OsEntropy};
pub use transport::TcpTransport;
