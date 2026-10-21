// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed
// compile heap. Expected verdicts are in ../checks.toml.

// EVMOD-PAIR off: attach reads the half's state and installs its callbacks in a later step (the transition between
// them is recorded by neither; the unclassified_eof alarm then aborts the pair)
module pair_bugCheckThenHook { import fwd_pair(OFF = Set("checkThenHook"), CFG = "ok", REC = false).* from "../fwd_pair" }
// EVMOD-ATTACH-END off: on HalfEnded the owner closes the surviving TCP half without SO_LINGER{1,0} (a FIN, not a reset)
module pair_bugEndedAttachGracefulClose { import fwd_pair(OFF = Set("endedAttachGracefulClose"), CFG = "ok", REC = false).* from "../fwd_pair" }
