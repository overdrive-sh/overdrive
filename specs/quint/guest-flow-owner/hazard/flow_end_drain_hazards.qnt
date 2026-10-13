// -*- mode: Bluespec; -*-
// Hazard instances of the flow end (../flow_end.qnt), the drain and ring rules (A1, A2, A3, A7, R2-DRAIN, DG-5-C): the
// design with ONE rule switched off (OFF). Split from flow_end_hazards.qnt to keep the compiled IR of each hazard file
// within the Apalache server heap. Expected verdicts are in ../checks.toml.

// A2 off: a record of another epoch is acted on
module end_bugNoEpoch { import flow_end(OFF = Set("noEpoch"), CFG = "dv").* from "../flow_end" }
// A3 off: a refused redirect writes no record
module end_bugNoRedirectNotice { import flow_end(OFF = Set("noRedirectNotice"), CFG = "dv").* from "../flow_end" }
// A7 off: the owner ignores a raised lost counter
module end_bugIgnoreRingDrop { import flow_end(OFF = Set("ignoreRingDrop"), CFG = "dv").* from "../flow_end" }
// A1 off: no level backstop
module end_bugNoBackstop { import flow_end(OFF = Set("noBackstop"), CFG = "dv").* from "../flow_end" }
// A1 off: the cell counts calls instead of reading the cumulative kernel quantity (two bytes)
module end_bugPerCallCounter { import flow_end(OFF = Set("perCallCounter"), CFG = "two").* from "../flow_end" }
// R2-DRAIN off: the owner arms before it stores the goal (two bytes)
module end_bugArmFirst { import flow_end(OFF = Set("armFirst"), CFG = "two").* from "../flow_end" }
// R2-DRAIN off: the owner reads sent before it arms
module end_bugReadBeforeArm { import flow_end(OFF = Set("readBeforeArm"), CFG = "clean").* from "../flow_end" }
// an abort leaves the cells armed
module end_bugAbortKeepsArmed { import flow_end(OFF = Set("abortKeepsArmed"), CFG = "abort").* from "../flow_end" }
// DG-5-C off: the clean close is a step of its own after the second direction completes
module end_bugSeparateClose { import flow_end(OFF = Set("separateClose"), CFG = "all").* from "../flow_end" }
// DG-5-C off on the fault-free instance (both directions active): the invariant check NoCompletedFlowStaysOpen
// needs no fault, and the instance is small enough for exhaustive search
module end_bugSeparateCloseClean { import flow_end(OFF = Set("separateClose"), CFG = "clean").* from "../flow_end" }
