// -*- mode: Bluespec; -*-
// Teeth for steering concern 3 (../quiescence.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// D8a-HOLD: one latch — any recovery's restore reopens forwarding and ends every hold
module quiesce_bugAnyRestore { import quiescence(OFF = Set("anyRestore"), CFG = "ok").* from "../quiescence" }
module quiesce_live_bugAnyRestore { import quiescence(OFF = Set("anyRestore"), CFG = "live").* from "../quiescence" }
// D8a-HOLD: a recovery repairs without holding quiescence
module quiesce_bugRepairNoHold { import quiescence(OFF = Set("repairNoHold"), CFG = "ok").* from "../quiescence" }
// UP-7: a failed post-repair audit is not a failed attempt
module quiesce_live_bugRedoNoCount { import quiescence(OFF = Set("redoNoCount"), CFG = "live").* from "../quiescence" }
// G-V5 / UP-12: a recovery's repair also converges the steering and the route; its failure fences
module quiesce_bugOtherRepairsPrefix { import quiescence(OFF = Set("otherRepairsPrefix"), CFG = "ok").* from "../quiescence" }
