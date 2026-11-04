// -*- mode: Bluespec; -*-
// Teeth for ../progress_deadline.qnt. Expected verdicts are in ../checks.toml.

// D10: the fixed 90 s cap regardless of the workload's startup deadline (a startup deadline above the cap)
module pd_bugFixedCap { import progress_deadline(OFF = Set("fixedCap"), REC = false, SLOTS = Set(1)).* from "../progress_deadline" }
