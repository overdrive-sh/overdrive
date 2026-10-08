// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module A (../owner_flows.qnt): the design with ONE
// rule or kernel assumption switched off (OFF). Expected verdicts are in ../checks.toml.

// L-3 off: session check and claim are two steps
module flows_bugSplitClaim { import owner_flows(OFF = Set("sessionSplitClaim")).* from "../owner_flows" }
// H-1 off: install before the request is read
module flows_bugInstallEarly { import owner_flows(OFF = Set("installBeforeRead")).* from "../owner_flows" }
// U-1 off: admission check and flow-table registration are two steps
module flows_bugAdmitSplit { import owner_flows(OFF = Set("admitSplit")).* from "../owner_flows" }
// U-1 off: the step-6 continuation ignores that the flow was closed
module flows_bugPairedNoRecheck { import owner_flows(OFF = Set("pairedNoRecheck")).* from "../owner_flows" }
// session loss does not abort the CID's flows
module flows_bugNoAbortOnLoss { import owner_flows(OFF = Set("noAbortOnLoss")).* from "../owner_flows" }
// teardown does not abort the CID's flows
module flows_bugTeardownSkipsFlows { import owner_flows(OFF = Set("teardownSkipsFlows")).* from "../owner_flows" }
// U-5 off: the host acts on a Paired for a flow it already closed
module flows_bugLatePairedResurrects { import owner_flows(OFF = Set("latePairedResurrects")).* from "../owner_flows" }
// K-A2 dropped: a dead VM's queued host connections survive CID reuse (V-22)
module flows_envStaleConn { import owner_flows(OFF = Set("K-A2")).* from "../owner_flows" }
// K-A4 dropped: the guest never observes the host's close of V_h (V-24)
module flows_envGuestMissesHostClose { import owner_flows(OFF = Set("K-A4")).* from "../owner_flows" }
// candidate: the guest tombstones an Abort for an unaccepted flow, with K-A4 dropped
module flows_altTombstoneMissesClose { import owner_flows(OFF = Set("K-A4", "altTombstone")).* from "../owner_flows" }
