// -*- mode: Bluespec; -*-
// Hazard instances of the flow end (../flow_end.qnt), counter reads (H-3): the design with ONE rule switched off (OFF).
// A file of its own keeps the compiled IR of each hazard file within the Apalache server heap. Expected verdicts are
// in ../checks.toml.

// H-3 off: the owner reads recursion_misses and the lost count only at a record dispatch, never on the audit tick: a
// both-skip on the last send, or a lost completion record, with no later record strands the direction
module end_bugNoPeriodicRead { import flow_end(OFF = Set("noPeriodicRead"), CFG = "dv").* from "../flow_end" }
