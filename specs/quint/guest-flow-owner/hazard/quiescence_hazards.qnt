// -*- mode: Bluespec; -*-
// Teeth for the counted hold and the boot write (../quiescence.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// QHOLD: a boolean latch — the first recovery's restore reopens while the second is still repairing
module quiesce_bugBoolLatch { import quiescence(OFF = Set("boolLatch"), CFG = "ok").* from "../quiescence" }
module quiesce_live_bugBoolLatch { import quiescence(OFF = Set("boolLatch"), CFG = "live").* from "../quiescence" }
// QHOLD: a dropped hold releases the count (the guard's drop decrements)
module quiesce_bugDropDecrements { import quiescence(OFF = Set("dropDecrements"), CFG = "ok").* from "../quiescence" }
// a recovery repairs without holding quiescence
module quiesce_bugRepairNoHold { import quiescence(OFF = Set("repairNoHold"), CFG = "ok").* from "../quiescence" }
// UP-7: a failed post-repair audit is not a failed attempt
module quiesce_live_bugRedoNoCount { import quiescence(OFF = Set("redoNoCount"), CFG = "live").* from "../quiescence" }
// ADR-0184: boot continues with serve running after a failed write of the shared table
module quiesce_bugBootContinues { import quiescence(OFF = Set("bootContinues"), CFG = "ok").* from "../quiescence" }
// ADR-0124: the supervisor does not consult the leaked-hold count — a dropped hold keeps forwarding quiesced for good
module quiesce_live_bugLeakIgnored { import quiescence(OFF = Set("leakIgnored"), CFG = "live").* from "../quiescence" }
// QHOLD: a restore reopens admission whatever the remaining hold count (the latch no longer IS the count)
module quiesce_bugRestoreClearsLatch { import quiescence(OFF = Set("restoreClearsLatch"), CFG = "ok").* from "../quiescence" }
// QHOLD: the first hold latches admission but does not abort the open flows
module quiesce_bugQuiesceSkipsFlows { import quiescence(OFF = Set("quiesceSkipsFlows"), CFG = "ok").* from "../quiescence" }
