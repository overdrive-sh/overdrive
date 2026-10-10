// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module A (../owner_flows.qnt): the design with ONE
// rule or kernel assumption switched off (OFF), or a candidate alternative switched on, on the
// instance (CFG) that carries the rule. Expected verdicts are in ../checks.toml.
// This file: CFG "beacon" and CFG "half" (see owner_flows_hazards.qnt for CFG "core").

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

// ---- CFG "half": DG-5-D (drain wait) and DG-5-C (merged clean close)
// DG-5-D off: a vsock end for an acknowledged direction shuts the outer leg at once, the drain still owed
module flows_bugNoDrainWaitOuter { import owner_flows(OFF = Set("noDrainWaitOuter"), CFG = "half").* from "../owner_flows" }
// DG-5-D off: HalfClose is sent at the outer EOF, the drain toward the vsock still owed
module flows_bugNoDrainWaitVsock { import owner_flows(OFF = Set("noDrainWaitVsock"), CFG = "half").* from "../owner_flows" }
// DG-5-D off: an abort leaves the direction's drain wait armed
module flows_bugAbortKeepsDrain { import owner_flows(OFF = Set("abortKeepsDrain"), CFG = "half").* from "../owner_flows" }
// DG-5-C off: the clean close is a step of its own after the second direction completes
module flows_bugSeparateClose { import owner_flows(OFF = Set("separateClose"), CFG = "half").* from "../owner_flows" }
