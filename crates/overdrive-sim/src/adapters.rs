//! Sim adapters — one module per port.
//!
//! SCAFFOLD: true — DISTILL placeholder per DWD-06. Each sub-module is
//! a RED stub that names the adapter type; the crafter fills in the
//! implementations during DELIVER against the corresponding trait from
//! `overdrive_core::traits::*`.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod clock {
    //! `SimClock` — turmoil-wrapped deterministic clock.
    pub struct SimClock;
}

pub mod transport {
    //! `SimTransport` — turmoil-wrapped network with injectable
    //! partition / loss / delay.
    pub struct SimTransport;
}

pub mod entropy {
    //! `SimEntropy` — `StdRng` seeded from harness seed.
    pub struct SimEntropy;
}

pub mod dataplane {
    //! `SimDataplane` — in-memory HashMap-backed dataplane.
    pub struct SimDataplane;
}

pub mod driver {
    //! `SimDriver` — in-memory allocation table with configurable
    //! failure modes.
    pub struct SimDriver;
}

pub mod llm {
    //! `SimLlm` — transcript-replay LLM adapter.
    pub struct SimLlm;
}

pub mod observation_store {
    //! `SimObservationStore` — in-memory LWW CRDT with injectable
    //! gossip delay + partition.
    pub struct SimObservationStore;
}
