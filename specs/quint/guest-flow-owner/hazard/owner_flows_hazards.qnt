// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of owner_flows (../owner_flows.qnt): the design with ONE
// rule or kernel assumption switched off (OFF), or a candidate alternative switched on, on the
// instance (CFG) that carries the rule. Expected verdicts are in ../checks.toml.
// This file: CFG "core" (control sessions, flows, host-opened dials). The beacon hazards are in
// owner_flows_beacon_hazards.qnt (one file per group keeps the compiled IR of each within the Apalache server heap).

// ---- CFG "core": control sessions, flows, TcpAccept
// L-3 off: session check and claim are two steps
module flows_bugSplitClaim { import owner_flows(OFF = Set("sessionSplitClaim"), CFG = "core").* from "../owner_flows" }
// H-1 off: install before the request is read
module flows_bugInstallEarly { import owner_flows(OFF = Set("installBeforeRead"), CFG = "core").* from "../owner_flows" }
// U-1 off: admission check and flow-table registration are two steps
module flows_bugAdmitSplit { import owner_flows(OFF = Set("admitSplit"), CFG = "core").* from "../owner_flows" }
// U-1 off: the step-6 continuation ignores that the flow was closed
module flows_bugPairedNoRecheck { import owner_flows(OFF = Set("pairedNoRecheck"), CFG = "core").* from "../owner_flows" }
// session loss does not abort the CID's flows
module flows_bugNoAbortOnLoss { import owner_flows(OFF = Set("noAbortOnLoss"), CFG = "core").* from "../owner_flows" }
// teardown does not abort the CID's flows
module flows_bugTeardownSkipsFlows { import owner_flows(OFF = Set("teardownSkipsFlows"), CFG = "core").* from "../owner_flows" }
// U-5 off: the host acts on a Paired for a flow it already closed
module flows_bugLatePairedResurrects { import owner_flows(OFF = Set("latePairedResurrects"), CFG = "core").* from "../owner_flows" }
// ATTR-TOKEN off: attribution by peer CID alone (the peer-CID-only rule), under K-A2′ (V-22, P-37)
module flows_bugPeerCidOnly { import owner_flows(OFF = Set("peerCidOnly"), CFG = "core").* from "../owner_flows" }
// ATTR-LIVE-CLAIM order off: the session claim is taken at accept, before the ControlOpen is read
module flows_bugClaimBeforeToken { import owner_flows(OFF = Set("claimBeforeToken"), CFG = "core").* from "../owner_flows" }
// ATTR-LIVE-CLAIM off (comparison with the lease token, its counterexample kept as teeth): the CID state is
// checked at accept only, and the claim step compares with the lease token, Retiring included
module flows_bugNoClaimRecheck { import owner_flows(OFF = Set("noClaimRecheck"), CFG = "core").* from "../owner_flows" }
// K-A4 dropped: the guest never observes the host's close of V_h (V-24)
module flows_envGuestMissesHostClose { import owner_flows(OFF = Set("K-A4"), CFG = "core").* from "../owner_flows" }
// candidate: the guest tombstones an Abort for an unaccepted flow, with K-A4 dropped
module flows_altTombstoneMissesClose { import owner_flows(OFF = Set("K-A4", "altTombstone"), CFG = "core").* from "../owner_flows" }
// rejected alternative to ATTR-LIVE-CLAIM: teardown also closes connections in `reading`, and the
// claim step compares with the lease token (no live check)
module flows_altTeardownClosesReading { import owner_flows(OFF = Set("altTeardownClosesReading"), CFG = "core").* from "../owner_flows" }

// ---- CFG "core": DG-5-F (guest session loss) and DG-5-E (installed, unrouted V_g)
// DG-5-F off: the guest may open a new control session while a flow of the lost session is still open
// (the loss is observed per flow, later, and a reconnect can overtake it)
module flows_bugReconnectBeforeLossAbort { import owner_flows(OFF = Set("reconnectBeforeLossAbort"), CFG = "core").* from "../owner_flows" }
// DG-5-E off: an installed, unrouted V_g is not watched for terminal readiness (the host's close of V_h
// and an error on it go unnoticed until the flow is routed)
module flows_bugNoInstalledWatch { import owner_flows(OFF = Set("noInstalledWatch"), CFG = "core").* from "../owner_flows" }

// ---- CFG "core": INB-1 (a dial through the owner)
// the owner opens a dial without the Active ∧ live session ∧ not-quiesced check
module flows_bugDialNoAdmission { import owner_flows(OFF = Set("dialNoAdmission"), CFG = "core").* from "../owner_flows" }
// GUEST_DIAL_PAIRING_DEADLINE not above GUEST_INBOUND_CONNECT_TIMEOUT: the owner may abandon a dial the guest can still answer
module flows_bugOwnerDeadlineShorter { import owner_flows(OFF = Set("ownerDeadlineShorter"), CFG = "core").* from "../owner_flows" }
