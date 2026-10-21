// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_safe (../fwd_safe.qnt): one per rule of EVMOD-SAFE (ADR-0204, rules 1-4 and the
// write-space entry's removal) and per wakeup of § Forwarding. Each is the design with ONE rule switched off (OFF) on
// the instance (CFG) that carries it. Expected verdicts are in ../checks.toml.

// rule 2 off (alternative (a)): rcu_assign_pointer(…, NULL) + rcu_dereference
module safe_bugRcuAssignNull { import fwd_safe(OFF = Set("rcuAssignNull"), CFG = "pair").* from "../fwd_safe" }
// rule 3 off: no grace period before the free
module safe_bugNoGracePeriod { import fwd_safe(OFF = Set("noGracePeriod"), CFG = "pair").* from "../fwd_safe" }
// rule 4 off (alternative (d)): the id looked up under RCU, the work queued after rcu_read_unlock
module safe_bugCtlQueueOutsideRcu { import fwd_safe(OFF = Set("ctlQueueOutsideRcu"), CFG = "pair").* from "../fwd_safe" }
// rule 4 off (watch): the override queues the record without checking the id under the lock
module safe_bugWatchEndNoIdCheck { import fwd_safe(OFF = Set("watchEndNoIdCheck"), CFG = "watch").* from "../fwd_safe" }
// the write-space entry removed by the free work after its cancel_work_sync, i.e. after the free work was queued
// (violates NoTouchAfterFree)
module safe_bugRemoveWaitAfterFreeQueued { import fwd_safe(OFF = Set("removeWaitAfterFreeQueued"), CFG = "pair").* from "../fwd_safe" }
// EVMOD-SWEEP off (F-S1): a run acts on an end it read with no recorded class without the vsock_table_lock re-check
module safe_bugNoSweepRecheck { import fwd_safe(OFF = Set("noSweepRecheck"), CFG = "pair").* from "../fwd_safe" }
// § Forwarding, Trigger off: the target's write-space wait-queue entry is not added
module safe_liveNoWriteSpace { import fwd_safe(OFF = Set("noWriteSpace"), CFG = "plive").* from "../fwd_safe" }
// § Forwarding, Trigger off: the source's sk_data_ready is not overridden
module safe_liveNoDataReady { import fwd_safe(OFF = Set("noDataReady"), CFG = "plive").* from "../fwd_safe" }
// § Forwarding, Trigger off: the source's sk_error_report is not overridden
module safe_liveNoErrorReport { import fwd_safe(OFF = Set("noErrorReport"), CFG = "plive").* from "../fwd_safe" }
// the workqueue's clear-before-run replaced by a flag cleared at return (a lost wakeup)
module safe_liveLateClearPending { import fwd_safe(OFF = Set("lateClearPending"), CFG = "plive").* from "../fwd_safe" }
// rule 4 / EVMOD-REC off: OVD_FWD_ABORT does not queue the work item
module safe_liveCtlAbortNoQueue { import fwd_safe(OFF = Set("ctlAbortNoQueue"), CFG = "plive").* from "../fwd_safe" }
// § Order of a pair's end off: the end stops after the unhook and does not requeue for its record and free
module safe_liveEndSplitNoRequeue { import fwd_safe(OFF = Set("endSplitNoRequeue"), CFG = "plive").* from "../fwd_safe" }
