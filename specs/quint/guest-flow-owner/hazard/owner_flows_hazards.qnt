// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module A (../owner_flows.qnt). Expected
// verdicts are in ../checks.toml.

// L-3 off: session check and claim are two steps
module flows_bugSplitClaim {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = true, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// H-1 off: install before the request is read
module flows_bugInstallEarly {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = true, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// U-1 off: admission check and flow-table registration are two steps
module flows_bugAdmitSplit {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = true, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// U-1 off: the step-6 continuation ignores that the flow was closed
module flows_bugPairedNoRecheck {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = true, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// session loss does not abort the CID's flows
module flows_bugNoAbortOnLoss {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = true, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// teardown does not abort the CID's flows
module flows_bugTeardownSkipsFlows {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = true, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// U-5 off: the host acts on a Paired for a flow it already closed
module flows_bugLatePairedResurrects {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = true, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// K-A2 dropped: a dead VM's queued host connections survive CID reuse (V-22)
module flows_envStaleConn {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = true, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = false, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// K-A4 dropped: the guest never observes the host's close of V_h
module flows_envGuestMissesHostClose {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = true, ALT_GUEST_ABORT_TOMBSTONE = false).* from "../owner_flows"
}

// candidate: guest tombstones an Abort for an unaccepted flow, with K-A4 dropped
module flows_altTombstoneMissesClose {
  import owner_flows(CONNS = Set(1, 2), FLOW_IDS = Set(1, 2), MAXGEN = 2, BUG_SESSION_SPLIT_CLAIM = false, BUG_INSTALL_BEFORE_READ = false, BUG_ADMIT_SPLIT = false, BUG_PAIRED_NO_RECHECK = false, BUG_NO_ABORT_ON_LOSS = false, BUG_TEARDOWN_SKIPS_FLOWS = false, ENV_STALE_CONN_SURVIVES = false, HFLOWS = Set(1), BUG_LATE_PAIRED_RESURRECTS = false, ENV_GUEST_MISSES_HOST_CLOSE = true, ALT_GUEST_ABORT_TOMBSTONE = true).* from "../owner_flows"
}
