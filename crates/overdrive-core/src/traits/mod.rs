//! Injectable trait boundaries.
//!
//! Every source of non-determinism in Overdrive — time, network, entropy,
//! kernel, drivers, storage, LLM — crosses one of the traits in this
//! module. Core logic depends on these traits, never on concrete
//! implementations. The wiring crates (`overdrive-node`,
//! `overdrive-control-plane`) pick the real impls; test crates pick the
//! `Sim*` impls.
//!
//! This is the seam Deterministic Simulation Testing (see
//! `docs/whitepaper.md` §21) stands on. A lint gate in CI forbids
//! `std::time::*::now`, `rand::{random, thread_rng}`, `tokio::net::*`, and
//! direct `aya-rs` / kernel calls from anywhere that is not a wiring
//! crate.

pub mod ca;
pub mod cgroup_fs;
pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod intent_store;
pub mod llm;
pub mod observation_store;
// SCAFFOLD: true — service-health-check-probes feature.
// Three port traits (`TcpProber` / `HttpProber` / `ExecProber`) per
// ADR-0054 §3. Lands GREEN across slices 01-03.
pub mod prober;
pub mod transport;

pub use ca::{
    Ca, CaCertDer, CaCertPem, CaError, CaKeyPem, IntermediateHandle, RootCaHandle, SvidMaterial,
    SvidRequest, TrustBundle, TrustBundlePem,
};
pub use cgroup_fs::{CgroupFs, ProbeError};
pub use clock::Clock;
pub use dataplane::Dataplane;
pub use driver::{Driver, DriverType};
pub use entropy::Entropy;
pub use intent_store::IntentStore;
pub use llm::Llm;
pub use observation_store::ObservationStore;
pub use transport::Transport;
