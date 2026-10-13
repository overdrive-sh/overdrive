// -*- mode: Bluespec; -*-
// Teeth for the CID lease (../cid_lease.qnt): each instance is the design with ONE rule switched off (OFF).
// Expected verdicts are in ../checks.toml.

// release-last off: the lease is released while the VMM still holds the device
module lease_bugReleaseBeforeVmm { import cid_lease(OFF = Set("releaseBeforeVmm"), CFG = "ok").* from "../cid_lease" }
// NO-SURVIVE off: assign runs before boot reclamation killed the previous process's VMMs
module lease_bugAssignBeforeReclaim { import cid_lease(OFF = Set("assignBeforeReclaim"), CFG = "ok").* from "../cid_lease" }
// RESET-EXIT off: a guest reset reboots the guest inside the same VMM
module lease_bugResetReboots { import cid_lease(OFF = Set("resetReboots"), CFG = "ok").* from "../cid_lease" }
// ADR-0156 off: the next-fit assign walk is not one step under the pool lock (choose, then take later without a recheck)
module lease_bugAssignSplit { import cid_lease(OFF = Set("assignSplit"), CFG = "ok").* from "../cid_lease" }
// NO-SURVIVE off (liveness): boot reclamation kills the orphans but never records that it completed, so a waiting workload is never placed
module lease_bugReclaimNeverCompletes { import cid_lease(OFF = Set("reclaimNeverCompletes"), CFG = "live").* from "../cid_lease" }
