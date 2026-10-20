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
// attempts + 1 < CEILING, with no exemptions — on the model-checking instance (CEIL 2) ...
module lease_bugRetryOffByOne { import cid_lease(OFF = Set("retryOffByOne"), CFG = "ok", REC = false).* from "../cid_lease" }
// ... and on the conformance configuration (CEIL 5, the production ceiling), for the named runs
module lease_conf_bugRetryOffByOne { import cid_lease(OFF = Set("retryOffByOne"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// RESTART-SSOT off (reclaimNotExempt): only the platform-reclaim exemption removed
module lease_bugReclaimNotExempt { import cid_lease(OFF = Set("reclaimNotExempt"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugReclaimNotExempt { import cid_lease(OFF = Set("reclaimNotExempt"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// RESTART-SSOT off (generationNotExempt): only the generation exemption removed (a Redeploy of a workload at the ceiling
// does not place it: a removed transition, which no state invariant sees; the named run is the discriminator)
module lease_bugGenerationNotExempt { import cid_lease(OFF = Set("generationNotExempt"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugGenerationNotExempt { import cid_lease(OFF = Set("generationNotExempt"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// PENDING GH #325 — crashLosesPlacedGeneration: today's production (RESTART-GEN-CRASH, not fixed in #295) — a generation
// replacement leased at a serve crash is lost and the workload returns to its pre-Redeploy done / failed. The conformance
// instance keeps the #325 window (CFG = "conf_325", feature-delta `cid_lease` item 9), so its run fails for the hazard's
// reason, not for the window guard
module lease_bugCrashLosesPlacedGeneration { import cid_lease(OFF = Set("crashLosesPlacedGeneration"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugCrashLosesPlacedGeneration { import cid_lease(OFF = Set("crashLosesPlacedGeneration"), CFG = "conf_325", REC = false).* from "../cid_lease" }
// DR6-CLOSE, one hazard per item-7 rule. The `lease_bug*` instances use CFG = "ok", the `lease_conf_bug*` instances
// CFG = "conf_ok", all non-recording (REC = false: the #325-window guard of item 9 applies only with REC).
// item 7.2 off (predecessorFirst): the restart frees the crashed allocation's lease before it leases the successor.
// A dispatch order, not a CID-safety rule: Safety holds; the conformance run is the discriminator
module lease_bugPredecessorFirst { import cid_lease(OFF = Set("predecessorFirst"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugPredecessorFirst { import cid_lease(OFF = Set("predecessorFirst"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// item 7.3 off (predecessorCleanedAtAssign): the predecessor is retired at the successor's assign, not after its launch
module lease_bugPredecessorCleanedAtAssign { import cid_lease(OFF = Set("predecessorCleanedAtAssign"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugPredecessorCleanedAtAssign { import cid_lease(OFF = Set("predecessorCleanedAtAssign"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// item 7.4 off (refusalKeepsPredecessor): a pool refusal leaves the predecessor held until the next launch. Its run is a
// `lease_ok` run (three offsets), so this one instance carries both its Safety and its run check
module lease_bugRefusalKeepsPredecessor { import cid_lease(OFF = Set("refusalKeepsPredecessor"), CFG = "ok", REC = false).* from "../cid_lease" }
// item 7.5 off (terminalBeforeRelease): FinalizeFailed writes the terminal before it releases the lease
module lease_bugTerminalBeforeRelease { import cid_lease(OFF = Set("terminalBeforeRelease"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugTerminalBeforeRelease { import cid_lease(OFF = Set("terminalBeforeRelease"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
// item 7.3 step boundary off (tickDuringPredecessorTeardown): a tick runs while the workload's dispatch is held at a
// predecessor teardown; Assign from `ended` then overwrites the retiring predecessor lease
module lease_bugTickDuringPredecessorTeardown { import cid_lease(OFF = Set("tickDuringPredecessorTeardown"), CFG = "ok", REC = false).* from "../cid_lease" }
// item 7.1 off (vmmExitRetiresLease): the VMM's exit retires and releases its lease in the same step (the DR5
// decomposition the adversarial check refuted). The device and the lease are freed together, so no CID leaks and
// Safety holds (documented); the conformance run is the discriminator (its lease is gone right after VmmCrash)
module lease_bugVmmExitRetiresLease { import cid_lease(OFF = Set("vmmExitRetiresLease"), CFG = "ok", REC = false).* from "../cid_lease" }
module lease_conf_bugVmmExitRetiresLease { import cid_lease(OFF = Set("vmmExitRetiresLease"), CFG = "conf_ok", REC = false).* from "../cid_lease" }
