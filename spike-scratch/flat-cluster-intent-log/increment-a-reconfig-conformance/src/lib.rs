//! SPIKE (throwaway): a deterministic harness ("rig") that holds N real `viewstamp-proto` endpoints over
//! a virtual network with SELECTIVE per-message delivery, per-node virtual clocks and crash/restart, so a
//! Quint trace can be replayed against the real Sans-I/O core one protocol step at a time.
//!
//! The embedding mirrors `viewstamp_simulation::Cluster` (@ f7fbcd96): slot<->MemberId translation at
//! egress/ingress, storage completions pumped after every input (`handle_storage` + block jobs), crash =
//! drop the endpoint + discard in-flight writes + fresh storage session, restart = `recover_with_reconfig`.
//! Unlike `Cluster`, nothing is delivered unless the driver says so, and every emitted message is kept in
//! a history so a trace may re-deliver (duplicate) it.

pub mod rig;
pub mod monitor;
pub mod driver;
