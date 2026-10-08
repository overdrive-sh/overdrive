// -*- mode: Bluespec; -*-
// Teeth for steering concern 2 (../intake.qnt): each instance is the design with ONE rule switched
// off (OFF). Expected verdicts are in ../checks.toml.

// K-L3: a steering entry outlives its listener's close
module intake_bugEntryOutlives { import intake(OFF = Set("entryOutlives"), CFG = "ok").* from "../intake" }
// D8a-LOOKUP: revision-8 take-down — a separate entry removal precedes the close and can fail
module intake_bugTakedownBeforeClose { import intake(OFF = Set("takedownBeforeClose"), CFG = "ok").* from "../intake" }
// U-6: the activation rollback closes only the failing port (two declared ports)
module intake_bugRollbackPartial { import intake(OFF = Set("rollbackPartial"), CFG = "two").* from "../intake" }
// U-6: activation returns Ok without every wanted port steered
module intake_bugActOkPartial { import intake(OFF = Set("actOkPartial"), CFG = "ok").* from "../intake" }
// U-3: teardown returns Ok with listeners open; release follows
module intake_bugTeardownKeeps { import intake(OFF = Set("teardownKeeps"), CFG = "ok").* from "../intake" }
// M-7: quiescence leaves intake listeners open
module intake_bugQuiesceKeeps { import intake(OFF = Set("quiesceKeeps"), CFG = "ok").* from "../intake" }
