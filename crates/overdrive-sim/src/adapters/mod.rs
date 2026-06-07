//! Sim adapters — one module per injectable port trait.
//!
//! Each sub-module contains the `Sim*` implementation of one
//! `overdrive_core::traits::*` trait:
//!
//! * [`clock`] — `SimClock`, logical-time clock driven by harness ticks.
//! * [`transport`] — `SimTransport`, in-process datagram router with
//!   injectable partition matrix.
//! * [`entropy`] — `SimEntropy`, seeded `StdRng`.
//! * [`dataplane`] — `SimDataplane`, in-memory policy / service /
//!   flow-event storage.
//! * [`driver`] — `SimDriver`, in-memory allocation table with
//!   configurable failure modes.
//! * [`llm`] — `SimLlm`, transcript-replay adapter.
//! * [`observation_store`] — `SimObservationStore` + gossip cluster.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc, dead_code)]

pub mod ca;
pub mod cgroup_fs;
pub mod clock;
pub mod dataplane;
pub mod driver;
pub mod entropy;
pub mod llm;
pub mod observation_store;
pub mod transport;
// reconciler-memory-redb step 01-03 — `SimViewStore` impl of
// `overdrive_control_plane::view_store::ViewStore` per ADR-0035 §2.
pub mod view_store;
// workflow-primitive step 01-03 — `SimJournalStore` impl of
// `overdrive_control_plane::journal::JournalStore` per ADR-0066. In-memory
// `BTreeMap<(WorkflowId, u32), Vec<u8>>` with injectable fsync-failure.
pub mod journal;
// SCAFFOLD: true — service-health-check-probes feature.
// Sim bindings for `TcpProber` / `HttpProber` / `ExecProber` per
// ADR-0054 §2. Queue-driven outcome injection. Lands GREEN across
// slices 01-03.
pub mod probers;

pub use ca::SimCa;
pub use cgroup_fs::{SimCgroupFs, SimEntry, SimOp};
