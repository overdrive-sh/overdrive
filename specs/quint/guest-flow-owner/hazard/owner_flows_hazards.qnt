// -*- mode: Bluespec; -*-
// Hazard / teeth instances of owner_flows (../owner_flows.qnt): one per design decision, the design with ONE rule
// switched off (OFF) on the instance (CFG) that carries the rule. Expected verdicts are in ../checks.toml.
// This file: CFG "core" (control sessions, flows, host-opened dials) and "one" (the instance of GuestHalfEnds). The
// admission, beacon and session hazards are in their own files (one file per group keeps the compiled IR of each within
// the Apalache server heap).

// L-3 off: session check and claim are two steps
module flows_bugSplitClaim { import owner_flows(OFF = Set("sessionSplitClaim"), CFG = "core", REC = false).* from "../owner_flows" }
// H-1 off: connect and attach before the request is read
module flows_bugAttachEarly { import owner_flows(OFF = Set("attachBeforeRead"), CFG = "core", REC = false).* from "../owner_flows" }
// DG-5-F off: session loss does not abort the CID's flows
module flows_bugNoAbortOnLoss { import owner_flows(OFF = Set("noAbortOnLoss"), CFG = "core", REC = false).* from "../owner_flows" }
// U-5 off: the host acts on a Paired for a flow it already closed
module flows_bugLatePairedResurrects { import owner_flows(OFF = Set("latePairedResurrects"), CFG = "core", REC = false).* from "../owner_flows" }
// ATTR-TOKEN off: attribution by peer CID alone (the peer-CID-only rule), under K-A2′ (V-22, P-37)
module flows_bugPeerCidOnly { import owner_flows(OFF = Set("peerCidOnly"), CFG = "core", REC = false).* from "../owner_flows" }
// ATTR-LIVE-CLAIM off (comparison with the lease token): the CID state is checked at accept only, and the claim step
// compares with the lease token, Retiring included
module flows_bugNoClaimRecheck { import owner_flows(OFF = Set("noClaimRecheck"), CFG = "core", REC = false).* from "../owner_flows" }
// INB-1: the owner opens a dial without the Active ∧ live session ∧ not-quiesced check
module flows_bugDialNoAdmission { import owner_flows(OFF = Set("dialNoAdmission"), CFG = "core", REC = false).* from "../owner_flows" }
// INB-1: GUEST_DIAL_PAIRING_DEADLINE not above GUEST_INBOUND_CONNECT_TIMEOUT: the owner may abandon a dial the guest can
// still answer
module flows_bugOwnerDeadlineShorter { import owner_flows(OFF = Set("ownerDeadlineShorter"), CFG = "core", REC = false).* from "../owner_flows" }
// DG-5-E off, on the instance GuestHalfEnds is checked on: an accepted, unattached V_g is not watched for terminal
// readiness (the host's close of V_h and an error on it go unnoticed until the flow is attached)
module flows_bugNoAcceptedWatch { import owner_flows(OFF = Set("noAcceptedWatch"), CFG = "one", REC = false).* from "../owner_flows" }
