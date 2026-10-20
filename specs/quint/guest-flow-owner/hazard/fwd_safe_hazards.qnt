// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_safe (../fwd_safe.qnt): the stale-invocation rules of EVMOD-SAFE (ADR-0204) and the
// wakeups of § Forwarding. Each is the design with ONE rule switched off (OFF), or a rejected alternative switched on,
// on the instance (CFG) that carries it. Expected verdicts are in ../checks.toml.

// rule 2 off (alternative (a)): rcu_assign_pointer(…, NULL) + rcu_dereference
module safe_bugRcuAssignNull { import fwd_safe(OFF = Set("rcuAssignNull"), CFG = "pair").* from "../fwd_safe" }
module safe_runsRcuAssignNull { import fwd_safe(OFF = Set("rcuAssignNull"), CFG = "runs").* from "../fwd_safe" }
// rule 2 off (alternative (b)): an override that finds no pair returns
module safe_bugNoFallback { import fwd_safe(OFF = Set("noFallback"), CFG = "pair").* from "../fwd_safe" }
// rule 3 off: no grace period before the free
module safe_bugNoGracePeriod { import fwd_safe(OFF = Set("noGracePeriod"), CFG = "pair").* from "../fwd_safe" }
// rule 3 off: no cancel_work_sync before the free
module safe_bugNoCancelSync { import fwd_safe(OFF = Set("noCancelSync"), CFG = "pair").* from "../fwd_safe" }
module safe_runsNoCancelSync { import fwd_safe(OFF = Set("noCancelSync"), CFG = "runs").* from "../fwd_safe" }
// rule 4 off (alternative (d)): the id looked up under RCU, the work queued after rcu_read_unlock
module safe_bugCtlQueueOutsideRcu { import fwd_safe(OFF = Set("ctlQueueOutsideRcu"), CFG = "pair").* from "../fwd_safe" }
// alternative (d) read the other way: the work queued before rcu_read_unlock
module safe_altCtlQueueInsideRcu { import fwd_safe(OFF = Set("ctlQueueInsideRcu"), CFG = "pair").* from "../fwd_safe" }
// rule 4 order off (watch): the owner's abort queues the free before it removes the id
module safe_bugWatchFreeBeforeId { import fwd_safe(OFF = Set("watchFreeBeforeId"), CFG = "watch").* from "../fwd_safe" }
// rule 4 off (watch): the override queues the record without checking the id under the lock
module safe_bugWatchEndNoIdCheck { import fwd_safe(OFF = Set("watchEndNoIdCheck"), CFG = "watch").* from "../fwd_safe" }
// § Forwarding, Trigger off: the target's write-space wait-queue entry not added / the source's sk_data_ready not
// overridden
module safe_liveNoWriteSpace { import fwd_safe(OFF = Set("noWriteSpace"), CFG = "plive").* from "../fwd_safe" }
module safe_liveNoDataReady { import fwd_safe(OFF = Set("noDataReady"), CFG = "plive").* from "../fwd_safe" }
// the workqueue's clear-before-run replaced by a flag cleared at return (a lost wakeup)
module safe_liveLateClearPending { import fwd_safe(OFF = Set("lateClearPending"), CFG = "plive").* from "../fwd_safe" }
module safe_runsLateClearPending { import fwd_safe(OFF = Set("lateClearPending"), CFG = "runs").* from "../fwd_safe" }
// rule 4 / EVMOD-REC off: OVD_FWD_ABORT does not queue the work item
module safe_liveCtlAbortNoQueue { import fwd_safe(OFF = Set("ctlAbortNoQueue"), CFG = "plive").* from "../fwd_safe" }
// § Order of a pair's end off: the end stops after the unhook and does not requeue for its record and free
module safe_liveEndSplitNoRequeue { import fwd_safe(OFF = Set("endSplitNoRequeue"), CFG = "plive").* from "../fwd_safe" }
// § Forwarding, Trigger off: the source's sk_state_change / sk_error_report not overridden (an end queues no run)
module safe_liveNoStateChange { import fwd_safe(OFF = Set("noStateChange"), CFG = "plive").* from "../fwd_safe" }
module safe_liveNoErrorReport { import fwd_safe(OFF = Set("noErrorReport"), CFG = "plive").* from "../fwd_safe" }
// EVMOD-SWEEP off (F-S1): a run acts on an end it read with no recorded class without the vsock_table_lock re-check
module safe_bugNoSweepRecheck { import fwd_safe(OFF = Set("noSweepRecheck"), CFG = "pair").* from "../fwd_safe" }
module safe_runsNoSweepRecheck { import fwd_safe(OFF = Set("noSweepRecheck"), CFG = "runs").* from "../fwd_safe" }
