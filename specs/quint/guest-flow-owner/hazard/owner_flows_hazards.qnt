// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module A (../owner_flows.qnt): the design with ONE
// rule or kernel assumption switched off (OFF), or a candidate alternative switched on, on the
// instance (CFG) that carries the rule. Expected verdicts are in ../checks.toml.

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
// ATTR-TOKEN off: attribution by peer CID alone (the round-8 rule), under K-A2′ (V-22, P-37)
module flows_bugPeerCidOnly { import owner_flows(OFF = Set("peerCidOnly"), CFG = "core").* from "../owner_flows" }
// ATTR-LIVE-CLAIM order off: the session claim is taken at accept, before the ControlOpen is read
module flows_bugClaimBeforeToken { import owner_flows(OFF = Set("claimBeforeToken"), CFG = "core").* from "../owner_flows" }
// ATTR-LIVE-CLAIM off (the first round-9 design, its counterexample kept as teeth): the CID state is
// checked at accept only, and the claim step compares with the lease token, Retiring included
module flows_bugNoClaimRecheck { import owner_flows(OFF = Set("noClaimRecheck"), CFG = "core").* from "../owner_flows" }
// K-A4 dropped: the guest never observes the host's close of V_h (V-24)
module flows_envGuestMissesHostClose { import owner_flows(OFF = Set("K-A4"), CFG = "core").* from "../owner_flows" }
// candidate: the guest tombstones an Abort for an unaccepted flow, with K-A4 dropped
module flows_altTombstoneMissesClose { import owner_flows(OFF = Set("K-A4", "altTombstone"), CFG = "core").* from "../owner_flows" }
// rejected alternative to ATTR-LIVE-CLAIM: teardown also closes connections in `reading`, and the
// claim step compares with the lease token (no live check)
module flows_altTeardownClosesReading { import owner_flows(OFF = Set("altTeardownClosesReading"), CFG = "core").* from "../owner_flows" }

// ---- CFG "beacon": the VM driver's beacon
// ATTR-LIVE-CLAIM off for the beacon: READY compared with the lease's token, without the live check
module flows_bugBeaconNoLiveRecheck { import owner_flows(OFF = Set("beaconNoLiveRecheck"), CFG = "beacon").* from "../owner_flows" }
// ATTR-TOKEN off for the beacon: READY attributed by peer CID alone
module flows_bugBeaconPeerCidOnly { import owner_flows(OFF = Set("beaconPeerCidOnly"), CFG = "beacon").* from "../owner_flows" }

// ---- CFG "half": the half-close handshake (HALFCLOSE-SIGNAL, ADR-0180)
// the round-8 rule: a flow vsock's end-of-stream is a half-close
module flows_bugRdhupIsHalfClose { import owner_flows(OFF = Set("rdhupIsHalfClose"), CFG = "half").* from "../owner_flows" }
// no acknowledgement: HalfClose sent and the vsock shut down at once; the receiver still aborts on a
// bare vsock end
module flows_bugNoAck { import owner_flows(OFF = Set("noAck"), CFG = "half").* from "../owner_flows" }
// the acknowledging side waits (acks only once its own outer leg read EOF)
module flows_bugAckWaits { import owner_flows(OFF = Set("ackWaits"), CFG = "half").* from "../owner_flows" }
// an outer EOF before Paired aborts the flow
module flows_bugEarlyEofAborts { import owner_flows(OFF = Set("earlyEofAborts"), CFG = "half").* from "../owner_flows" }
// rejected alternative (V-13 variant C): HalfClose without acknowledgement, a bare vsock end held
// until the message or session loss
module flows_altPendingWait { import owner_flows(OFF = Set("altPendingWait"), CFG = "half").* from "../owner_flows" }
