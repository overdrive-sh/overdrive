// -*- mode: Bluespec; -*-
// Teeth for the empty-datagram notice handling (../udp_empty.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// A2 shape off: a notice is acted on whatever its epoch (a stale notice sends an empty datagram on the new association)
module empty_bugNoEpoch { import udp_empty(OFF = Set("noEpoch")).* from "../udp_empty" }
// ring-drop compensation off: the owner ignores a raised lost counter
module empty_bugNoCompensation { import udp_empty(OFF = Set("noCompensation")).* from "../udp_empty" }
// the cell's lost counter is not reset when the cell is initialised for a new flow: a lost count of the earlier
// epoch is compensated on the new flow
module empty_bugNoResetAtReuse { import udp_empty(OFF = Set("noResetAtReuse")).* from "../udp_empty" }
