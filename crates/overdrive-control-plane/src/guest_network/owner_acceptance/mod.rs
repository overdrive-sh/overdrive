//! Source-local acceptance bodies over the production host guest-flow owner
//! (`HostSharedGuestNetworkOwner::with_flow_io` over `SimGuestFlowIo`, DG-3,
//! and through it the `SimFlowForwarder` arbiter, EVMOD-PORT).
//!
//! Source-local because the owner's effect seam and its sim are
//! crate-private (QPROJ-1). Default lane: in-process owner over in-memory
//! sims, no real infrastructure. Every body is complete and compiled, and
//! ignored pending the DELIVER step that implements the owner surface it
//! drives (`.claude/rules/testing.md` § "RED acceptance scaffolds and
//! activation"): the attachment-cut step (7) for the owner's flows, step (8)
//! for the mesh registration, step (9) for the dial and its stream (INB-1),
//! step 11 for the VIP branch, step 13 for the guest-prefix boot checks and
//! the gateway route (GW-LOCAL).
//!
//! The oracle of every body is port-observable: QPROJ-3
//! `flow_observation()`, the QPROJ-7 / QPROJ-8 arbiters
//! (`socket_observation`, the forwarder's inventory, pair state and context
//! table, the sim sockets' far-reader facts), the control messages the owner
//! wrote on the session, the effects it issued, its pinned telemetry events,
//! and the driving ports' own results. No body writes owner state and no body
//! queues an end record: every lease, session, flow and dial is produced by
//! the owner's driving ports, and every flow end is a cause scripted on a
//! socket that the sim forwarder classifies.

#![allow(
    clippy::redundant_pub_crate,
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::doc_markdown,
    clippy::too_many_lines,
    reason = "acceptance bodies assert their fixture preconditions; CONTRACT_SHAPE tokens are literal markers"
)]
#![allow(
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::print_stderr,
    reason = "test support and conformance drivers: fixture preconditions are asserted with expect, seeds are printed, and handler signatures follow the action map"
)]

pub(crate) mod support;

mod admission;
mod datagrams;
mod dial;
mod errors;
mod flow_end;
mod legs_dial;
mod policy;
mod quota;
