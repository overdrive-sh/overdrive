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

pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod intent_store;
pub mod llm;
pub mod observation_store;
pub mod transport;

pub use clock::Clock;
pub use dataplane::Dataplane;
pub use driver::{Driver, DriverType};
pub use entropy::Entropy;
pub use intent_store::IntentStore;
pub use llm::Llm;
pub use observation_store::ObservationStore;
pub use transport::Transport;
