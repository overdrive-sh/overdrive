// -*- mode: Bluespec; -*-
// Teeth for the CID lease (../cid_lease.qnt): each instance is the design with ONE rule switched off (OFF).
// Expected verdicts are in ../checks.toml.

// release-last off: the lease is released while the VMM still holds the device
module lease_bugReleaseBeforeVmm { import cid_lease(OFF = Set("releaseBeforeVmm"), CFG = "ok", REC = false).* from "../cid_lease" }
// NO-SURVIVE off: assign runs before boot reclamation killed the previous process's VMMs
module lease_bugAssignBeforeReclaim { import cid_lease(OFF = Set("assignBeforeReclaim"), CFG = "ok", REC = false).* from "../cid_lease" }
// RESET-EXIT off: a guest reset reboots the guest inside the same VMM
module lease_bugResetReboots { import cid_lease(OFF = Set("resetReboots"), CFG = "ok", REC = false).* from "../cid_lease" }
// ADR-0156 off: the next-fit assign walk is not one step under the pool lock (choose, then take later without a recheck)
module lease_bugAssignSplit { import cid_lease(OFF = Set("assignSplit"), CFG = "ok", REC = false).* from "../cid_lease" }
// NO-SURVIVE off (liveness): boot reclamation kills the orphans but never records that it completed, so a waiting workload is never placed
module lease_bugReclaimNeverCompletes { import cid_lease(OFF = Set("reclaimNeverCompletes"), CFG = "live", REC = false).* from "../cid_lease" }
// T-5 ruling off: a serve crash forgets the stop intent, so a stopped workload is placed again after the restart
module lease_bugCrashForgetsStop { import cid_lease(OFF = Set("crashForgetsStop"), CFG = "ok", REC = false).* from "../cid_lease" }
// RESTART-SSOT off (retryOffByOne): the full pre-fix `restart_status_for_alloc` as the restart decision — restart iff
// attempts + 1 < CEILING, with no exemptions
module lease_bugRetryOffByOne { import cid_lease(OFF = Set("retryOffByOne"), CFG = "ok", REC = false).* from "../cid_lease" }
// RESTART-SSOT off (reclaimNotExempt): only the platform-reclaim exemption removed
module lease_bugReclaimNotExempt { import cid_lease(OFF = Set("reclaimNotExempt"), CFG = "ok", REC = false).* from "../cid_lease" }
// pending GH #325 off (crashLosesPlacedGeneration): its named run is the teeth for `lease_conf_325`
module lease_conf_bugCrashLosesPlacedGeneration { import cid_lease(OFF = Set("crashLosesPlacedGeneration"), CFG = "conf_325", REC = false).* from "../cid_lease" }
// item 7.4 off (refusalKeepsPredecessor): a pool refusal leaves the predecessor held until the next launch. Its run is a
// `lease_ok` run (three offsets): the teeth for `lease_conf_three`
module lease_bugRefusalKeepsPredecessor { import cid_lease(OFF = Set("refusalKeepsPredecessor"), CFG = "ok", REC = false).* from "../cid_lease" }
// item 7.5 off (terminalBeforeRelease): FinalizeFailed writes the terminal before it releases the lease
module lease_bugTerminalBeforeRelease { import cid_lease(OFF = Set("terminalBeforeRelease"), CFG = "ok", REC = false).* from "../cid_lease" }
// item 7.3 step boundary off (tickDuringPredecessorTeardown): a tick runs while the workload's dispatch is held at a
// predecessor teardown; Assign from `ended` then overwrites the retiring predecessor lease
module lease_bugTickDuringPredecessorTeardown { import cid_lease(OFF = Set("tickDuringPredecessorTeardown"), CFG = "ok", REC = false).* from "../cid_lease" }
