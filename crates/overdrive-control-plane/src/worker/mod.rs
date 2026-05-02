//! Worker-side subsystems wired by the control plane.
//!
//! Despite living in the control-plane crate, these subsystems
//! materialise observations the worker layer (the `Driver` trait
//! object) is the natural owner of — but which require coordination
//! with the control plane's `EvaluationBroker` and `ObservationStore`
//! references that already flow through `AppState`.
//!
//! Per whitepaper §4 *owner-writer model*: every node writes its own
//! observation rows. The driver is the owner of process exits; this
//! subsystem is the writer surface.
//!
//! Phase 1 ships exactly one such subsystem — `exit_observer`. As
//! more arrive (e.g. node-health heartbeat, persistent-microvm
//! agent-acks), they will live alongside.

pub mod exit_observer;
