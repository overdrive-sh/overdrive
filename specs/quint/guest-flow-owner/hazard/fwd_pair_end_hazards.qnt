// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): ends present at attach, the TCP reset after a FIN, and the early shutdown on the liveness instance (EVMOD-PAIR, EVMOD-END). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EVMOD-PAIR off: attach reads the half's state and installs its callbacks in a later step (the transition between
// them is recorded by neither; the unclassified_eof alarm then aborts the pair)
module pair_bugCheckThenHook { import fwd_pair(OFF = Set("checkThenHook"), CFG = "ok", REC = false).* from "../fwd_pair" }
// the same, with the unclassified end-of-stream forwarded as a FIN instead of the alarm's abort
module pair_bugCheckThenHookFin { import fwd_pair(OFF = Set("checkThenHook", "unclassifiedAsFin"), CFG = "ok", REC = false).* from "../fwd_pair" }
